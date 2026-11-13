//! The browser WebSocket stack, through web-sys. Browser callbacks only
//! queue events and wake a pump task, so the app sees them inside the
//! engine's run like any other task wake (NSH's web runtime pumps when a
//! wasm-bindgen callback wakes a task).

use std::{
    cell::RefCell,
    collections::VecDeque,
    future::poll_fn,
    rc::Rc,
    task::{Poll, Waker},
};

use nsh_core::websocket::{
    WebSocketData, WebSocketEvent, WebSocketEvents, WebSocketRequest, WebSocketTask,
    WebSocketTransport,
};
use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use web_sys::{BinaryType, CloseEvent, Event, MessageEvent, WebSocket};

use crate::data::net::task::TaskHandle;

pub(super) struct BrowserTransport;

impl WebSocketTransport for BrowserTransport {
    fn supports_headers(&self) -> bool {
        false
    }

    fn connect(
        &self,
        request: WebSocketRequest,
        events: WebSocketEvents,
    ) -> Result<Box<dyn WebSocketTask>, String> {
        if !request.headers.is_empty() {
            return Err("browser WebSockets cannot send handshake headers".to_owned());
        }
        let socket = if request.protocols.is_empty() {
            WebSocket::new(&request.url)
        } else {
            let protocols = js_sys::Array::new();
            for protocol in &request.protocols {
                protocols.push(&JsValue::from_str(protocol));
            }
            WebSocket::new_with_str_sequence(&request.url, &protocols)
        }
        .map_err(describe)?;
        socket.set_binary_type(BinaryType::Arraybuffer);

        let inbox = Rc::new(RefCell::new(Inbox::default()));
        let handlers = Handlers {
            open: Closure::new({
                let (inbox, socket) = (inbox.clone(), socket.clone());
                move |_: Event| {
                    push(
                        &inbox,
                        WebSocketEvent::Open {
                            protocol: socket.protocol(),
                        },
                    );
                }
            }),
            message: Closure::new({
                let inbox = inbox.clone();
                move |event: MessageEvent| {
                    let data = event.data();
                    let message = if let Some(text) = data.as_string() {
                        WebSocketData::Text(text)
                    } else if let Some(buffer) = data.dyn_ref::<js_sys::ArrayBuffer>() {
                        WebSocketData::Bytes(js_sys::Uint8Array::new(buffer).to_vec())
                    } else {
                        return;
                    };
                    push(&inbox, WebSocketEvent::Message(message));
                }
            }),
            error: Closure::new({
                let inbox = inbox.clone();
                move |_: Event| {
                    let message = "the browser reported a WebSocket error".to_owned();
                    push(&inbox, WebSocketEvent::Error(message));
                }
            }),
            close: Closure::new({
                let inbox = inbox.clone();
                move |event: CloseEvent| {
                    push(
                        &inbox,
                        WebSocketEvent::Close {
                            code: event.code(),
                            reason: event.reason(),
                            was_clean: event.was_clean(),
                        },
                    );
                }
            }),
        };
        socket.set_onopen(Some(handlers.open.as_ref().unchecked_ref()));
        socket.set_onmessage(Some(handlers.message.as_ref().unchecked_ref()));
        socket.set_onerror(Some(handlers.error.as_ref().unchecked_ref()));
        socket.set_onclose(Some(handlers.close.as_ref().unchecked_ref()));

        Ok(Box::new(BrowserSocket {
            socket,
            _handlers: handlers,
            _pump: TaskHandle::spawn(pump(inbox, events)),
        }))
    }
}

/// Events the browser delivered that the pump has not handed on yet.
#[derive(Default)]
struct Inbox {
    events: VecDeque<WebSocketEvent>,
    waker: Option<Waker>,
}

fn push(inbox: &Rc<RefCell<Inbox>>, event: WebSocketEvent) {
    let waker = {
        let mut inbox = inbox.borrow_mut();
        inbox.events.push_back(event);
        inbox.waker.take()
    };
    if let Some(waker) = waker {
        waker.wake();
    }
}

/// Hands each queued event on, in order, until the socket closes.
async fn pump(inbox: Rc<RefCell<Inbox>>, mut events: WebSocketEvents) {
    loop {
        let batch = poll_fn(|cx| {
            let mut inbox = inbox.borrow_mut();
            if inbox.events.is_empty() {
                inbox.waker = Some(cx.waker().clone());
                Poll::Pending
            } else {
                Poll::Ready(std::mem::take(&mut inbox.events))
            }
        })
        .await;
        for event in batch {
            let closed = matches!(event, WebSocketEvent::Close { .. });
            events(event);
            if closed {
                return;
            }
        }
    }
}

struct Handlers {
    open: Closure<dyn FnMut(Event)>,
    message: Closure<dyn FnMut(MessageEvent)>,
    error: Closure<dyn FnMut(Event)>,
    close: Closure<dyn FnMut(CloseEvent)>,
}

struct BrowserSocket {
    socket: WebSocket,
    _handlers: Handlers,
    _pump: TaskHandle,
}

impl WebSocketTask for BrowserSocket {
    fn send(&mut self, data: WebSocketData) -> Result<(), String> {
        match data {
            WebSocketData::Text(text) => self.socket.send_with_str(&text),
            WebSocketData::Bytes(bytes) => self.socket.send_with_u8_array(&bytes),
        }
        .map_err(describe)
    }

    fn close(&mut self, code: u16, reason: &str) -> Result<(), String> {
        self.socket
            .close_with_code_and_reason(code, reason)
            .map_err(describe)
    }

    fn buffered_amount(&self) -> usize {
        self.socket.buffered_amount() as usize
    }

    /// Lattice reads every message as it arrives, so it never pauses.
    fn pause(&mut self, _paused: bool) {}
}

impl Drop for BrowserSocket {
    fn drop(&mut self) {
        // Detach first: the closures are freed with this socket.
        self.socket.set_onopen(None);
        self.socket.set_onmessage(None);
        self.socket.set_onerror(None);
        self.socket.set_onclose(None);
        if matches!(
            self.socket.ready_state(),
            WebSocket::CONNECTING | WebSocket::OPEN
        ) {
            let _ = self.socket.close();
        }
    }
}

fn describe(error: JsValue) -> String {
    error
        .dyn_ref::<js_sys::Error>()
        .map(|error| String::from(error.message()))
        .or_else(|| error.as_string())
        .unwrap_or_else(|| "browser WebSocket error".to_owned())
}
