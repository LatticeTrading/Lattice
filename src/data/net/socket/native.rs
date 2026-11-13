//! The desktop WebSocket stack: tokio-tungstenite over rustls with the
//! webpki roots, the stack nsh-core uses for its own native sockets. Each
//! socket is one engine task; the engine polls tasks inside its tokio
//! runtime, so tokio's networking works there and wakes the engine.

use std::{sync::Arc, time::Duration};

use futures_util::{SinkExt, StreamExt};
use nsh_core::websocket::{
    WebSocketData, WebSocketEvent, WebSocketEvents, WebSocketRequest, WebSocketTask,
    WebSocketTransport,
};
use tokio::sync::mpsc;
use tokio_tungstenite::{
    Connector,
    tungstenite::{
        self, Message,
        client::IntoClientRequest,
        http::{HeaderName, HeaderValue},
        protocol::{CloseFrame, frame::coding::CloseCode},
    },
};

use crate::data::net::task::{TaskHandle, deadline};

/// A backstop for the handshake; [`super::SocketConfig::connect_timeout`]
/// normally gives up first.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(30);
/// The close code of a connection that ended without a close frame.
const ABNORMAL: u16 = 1006;
/// The close code of a close frame without one.
const NO_STATUS: u16 = 1005;

pub(super) struct NativeTransport;

impl WebSocketTransport for NativeTransport {
    fn connect(
        &self,
        request: WebSocketRequest,
        events: WebSocketEvents,
    ) -> Result<Box<dyn WebSocketTask>, String> {
        let mut handshake = request
            .url
            .as_str()
            .into_client_request()
            .map_err(|error| format!("invalid WebSocket request: {error}"))?;
        let headers = handshake.headers_mut();
        for (name, value) in &request.headers {
            let name = HeaderName::from_bytes(name.as_bytes())
                .map_err(|error| format!("invalid header name `{name}`: {error}"))?;
            let value = HeaderValue::from_str(value)
                .map_err(|error| format!("invalid value for header `{name}`: {error}"))?;
            headers.append(name, value);
        }
        if !request.protocols.is_empty() {
            let protocols = HeaderValue::from_str(&request.protocols.join(", "))
                .map_err(|error| format!("invalid subprotocols: {error}"))?;
            headers.insert("sec-websocket-protocol", protocols);
        }
        let tls = tls_config()?;
        let (commands, receiver) = mpsc::unbounded_channel();
        let task = TaskHandle::spawn(drive(handshake, tls, receiver, events));
        Ok(Box::new(NativeSocket {
            commands,
            _task: task,
        }))
    }
}

enum Command {
    Send(Message),
    Close(u16, String),
}

/// Dropping it ends the task, and with it the connection.
struct NativeSocket {
    commands: mpsc::UnboundedSender<Command>,
    _task: TaskHandle,
}

impl WebSocketTask for NativeSocket {
    fn send(&mut self, data: WebSocketData) -> Result<(), String> {
        let message = match data {
            WebSocketData::Text(text) => Message::text(text),
            WebSocketData::Bytes(bytes) => Message::binary(bytes),
        };
        self.commands
            .send(Command::Send(message))
            .map_err(|_| "the connection is closed".to_owned())
    }

    fn close(&mut self, code: u16, reason: &str) -> Result<(), String> {
        self.commands
            .send(Command::Close(code, reason.to_owned()))
            .map_err(|_| "the connection is closed".to_owned())
    }

    fn buffered_amount(&self) -> usize {
        0
    }

    /// Lattice reads every message as it arrives, so it never pauses.
    fn pause(&mut self, _paused: bool) {}
}

/// Connects, then relays frames both ways until the connection ends or the
/// socket handle drops. Events go out only from here, so never while the
/// owner is inside a [`WebSocketTask`] method.
async fn drive(
    request: tungstenite::handshake::client::Request,
    tls: Arc<rustls::ClientConfig>,
    mut commands: mpsc::UnboundedReceiver<Command>,
    mut events: WebSocketEvents,
) {
    let abnormal = |reason: &str| WebSocketEvent::Close {
        code: ABNORMAL,
        reason: reason.to_owned(),
        was_clean: false,
    };
    let connect = tokio_tungstenite::connect_async_tls_with_config(
        request,
        None,
        true,
        Some(Connector::Rustls(tls)),
    );
    let mut socket = match deadline(HANDSHAKE_TIMEOUT, connect).await {
        Some(Ok((socket, response))) => {
            let protocol = response
                .headers()
                .get("sec-websocket-protocol")
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default()
                .to_owned();
            events(WebSocketEvent::Open { protocol });
            socket
        }
        Some(Err(error)) => {
            events(WebSocketEvent::Error(format!(
                "WebSocket handshake failed: {error}"
            )));
            events(abnormal("handshake failed"));
            return;
        }
        None => {
            events(WebSocketEvent::Error(
                "WebSocket handshake timed out".to_owned(),
            ));
            events(abnormal("handshake timed out"));
            return;
        }
    };
    loop {
        tokio::select! {
            biased;
            command = commands.recv() => match command {
                Some(Command::Send(message)) => {
                    if let Err(error) = socket.send(message).await {
                        events(WebSocketEvent::Error(format!("WebSocket write failed: {error}")));
                        events(abnormal("write failed"));
                        return;
                    }
                }
                // Keeps reading until the peer's close frame ends the loop.
                Some(Command::Close(code, reason)) => {
                    let frame = CloseFrame {
                        code: CloseCode::from(code),
                        reason: reason.into(),
                    };
                    if socket.close(Some(frame)).await.is_err() {
                        events(abnormal("close failed"));
                        return;
                    }
                }
                // The handle dropped: nobody is listening.
                None => return,
            },
            incoming = socket.next() => match incoming {
                Some(Ok(Message::Text(text))) => {
                    events(WebSocketEvent::Message(WebSocketData::Text(text.as_str().to_owned())));
                }
                Some(Ok(Message::Binary(bytes))) => {
                    events(WebSocketEvent::Message(WebSocketData::Bytes(bytes.to_vec())));
                }
                // Sends the pong tungstenite queued.
                Some(Ok(Message::Ping(_))) => {
                    let _ = socket.flush().await;
                }
                Some(Ok(Message::Close(frame))) => {
                    let _ = socket.flush().await;
                    let (code, reason) = frame.map_or((NO_STATUS, String::new()), |frame| {
                        (u16::from(frame.code), frame.reason.as_str().to_owned())
                    });
                    events(WebSocketEvent::Close { code, reason, was_clean: true });
                    return;
                }
                Some(Ok(_)) => {}
                Some(Err(tungstenite::Error::ConnectionClosed)) => {
                    events(WebSocketEvent::Close { code: 1000, reason: String::new(), was_clean: true });
                    return;
                }
                Some(Err(error)) => {
                    events(WebSocketEvent::Error(format!("WebSocket read failed: {error}")));
                    events(abnormal("read failed"));
                    return;
                }
                None => {
                    events(abnormal("connection ended without a close frame"));
                    return;
                }
            }
        }
    }
}

/// TLS as nsh-core 0.5 sets it up for its own sockets: the embedding host's
/// provider when it installed one, else ring for this connection only
/// (choosing a fallback must not configure the process's other TLS
/// clients), and the system's trust store, so roots the OS or an
/// administrator trusts (a corporate proxy's) verify too. Android, which
/// has no platform verifier here, uses the bundled webpki roots.
fn tls_config() -> Result<Arc<rustls::ClientConfig>, String> {
    let provider = rustls::crypto::CryptoProvider::get_default()
        .cloned()
        .unwrap_or_else(|| Arc::new(rustls::crypto::ring::default_provider()));
    let failed = |error: rustls::Error| format!("TLS configuration failed: {error}");
    let builder = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(failed)?;
    #[cfg(not(target_os = "android"))]
    let builder = builder
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(
            rustls_platform_verifier::Verifier::new(provider).map_err(failed)?,
        ));
    #[cfg(target_os = "android")]
    let builder = builder.with_root_certificates(rustls::RootCertStore::from_iter(
        webpki_roots::TLS_SERVER_ROOTS.iter().cloned(),
    ));
    Ok(Arc::new(builder.with_no_client_auth()))
}
