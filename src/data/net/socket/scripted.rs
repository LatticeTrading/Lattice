//! A WebSocket server that is not there, for tests: a transport whose
//! connections open at once, answer what the app sends as the test's script
//! says, and deliver whatever the test pushes.
//!
//! The test (or a stub HTTP handler, on whatever thread) talks to it
//! through channels; each connection is one engine task, which is where its
//! events go out from, as a platform's transport's do.

use std::sync::{Arc, Mutex};

use nsh_core::websocket::{
    WebSocketData, WebSocketEvent, WebSocketEvents, WebSocketRequest, WebSocketTask,
    WebSocketTransport,
};
use tokio::sync::mpsc;

use crate::data::net::TaskHandle;

/// What a server would answer a frame the app sent with: frames, in order.
type Script = dyn Fn(&str) -> Vec<String> + Send + Sync;

/// What reaches a connection's task.
enum Frame {
    Message(String),
    Close(u16, String),
}

/// The scripted server. Cloning is cheap: every clone is the same server.
#[derive(Clone)]
pub(crate) struct ScriptedSocket(Arc<Server>);

struct Server {
    script: Box<Script>,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    /// Every text frame the app sent, in order, over every connection.
    sent: Vec<String>,
    /// How many connections were made.
    connections: usize,
    /// The way to the connection that is open, if one is.
    open: Option<mpsc::UnboundedSender<Frame>>,
}

impl ScriptedSocket {
    /// A server that answers each frame the app sends with `script`'s frames.
    pub(crate) fn new(script: impl Fn(&str) -> Vec<String> + Send + Sync + 'static) -> Self {
        Self(Arc::new(Server {
            script: Box::new(script),
            state: Mutex::default(),
        }))
    }

    /// The transport an integration is given in the platform's place.
    pub(crate) fn transport(&self) -> Arc<dyn WebSocketTransport> {
        Arc::new(self.clone())
    }

    /// Sends `frame` to the app over the open connection; with none open it
    /// goes nowhere, as a server's would.
    pub(crate) fn push(&self, frame: impl Into<String>) {
        self.0.deliver(Frame::Message(frame.into()));
    }

    /// Ends the open connection as a network would: no close handshake.
    pub(crate) fn sever(&self) {
        self.0
            .deliver(Frame::Close(1006, "the connection was lost".to_owned()));
    }

    /// Every text frame the app sent so far, in order.
    pub(crate) fn sent(&self) -> Vec<String> {
        self.0.state().sent.clone()
    }

    /// How many connections the app has made.
    pub(crate) fn connections(&self) -> usize {
        self.0.state().connections
    }
}

impl Server {
    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().expect("no test panics holding it")
    }

    fn deliver(&self, frame: Frame) {
        if let Some(open) = &self.state().open {
            // A connection that ended since takes nothing.
            drop(open.send(frame));
        }
    }
}

impl WebSocketTransport for ScriptedSocket {
    fn connect(
        &self,
        _request: WebSocketRequest,
        mut events: WebSocketEvents,
    ) -> Result<Box<dyn WebSocketTask>, String> {
        let (frames, mut arriving) = mpsc::unbounded_channel();
        {
            let mut state = self.0.state();
            state.connections += 1;
            state.open = Some(frames.clone());
        }
        let task = TaskHandle::spawn(async move {
            events(WebSocketEvent::Open {
                protocol: String::new(),
            });
            while let Some(frame) = arriving.recv().await {
                match frame {
                    Frame::Message(text) => {
                        events(WebSocketEvent::Message(WebSocketData::Text(text)));
                    }
                    Frame::Close(code, reason) => {
                        events(WebSocketEvent::Close {
                            code,
                            reason,
                            was_clean: code != 1006,
                        });
                        return;
                    }
                }
            }
        });
        Ok(Box::new(Connection {
            server: self.0.clone(),
            frames,
            _task: task,
        }))
    }
}

/// One connection, as the app holds it. Dropping it ends its task.
struct Connection {
    server: Arc<Server>,
    frames: mpsc::UnboundedSender<Frame>,
    _task: TaskHandle,
}

impl WebSocketTask for Connection {
    fn send(&mut self, data: WebSocketData) -> Result<(), String> {
        let WebSocketData::Text(text) = data else {
            return Ok(());
        };
        let answers = (self.server.script)(&text);
        self.server.state().sent.push(text);
        for answer in answers {
            drop(self.frames.send(Frame::Message(answer)));
        }
        Ok(())
    }

    fn close(&mut self, code: u16, reason: &str) -> Result<(), String> {
        drop(self.frames.send(Frame::Close(code, reason.to_owned())));
        Ok(())
    }

    fn buffered_amount(&self) -> usize {
        0
    }

    fn pause(&mut self, _paused: bool) {}
}
