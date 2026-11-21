//! The desktop's way to the user's wallets: a page of the app's own, open
//! in the user's browser.
//!
//! A wallet is a browser extension, and it shows itself only to pages. So
//! the app serves one small page ([`PAGE`], `bridge.html`) on this machine
//! alone (`127.0.0.1`) and has the system open it in the user's browser.
//! The page finds the wallets there, asks the app what to ask them, hands
//! each request to the wallet (which prompts its user as it would for any
//! site) and brings back the answer:
//!
//! | Request | What it is |
//! | --- | --- |
//! | `GET /<secret>` | the page |
//! | `POST /<secret>/hello` | the page has looked: who it is, the wallets it found |
//! | `GET /<secret>/next?page=<id>` | what there is to ask, held open until there is something or [`HOLD`] has passed |
//! | `POST /<secret>/reply` | a wallet's answer to one request |
//! | `POST /<secret>/event` | a wallet's `accountsChanged` or `chainChanged` |
//!
//! What keeps this the user's own:
//!
//! - It listens on the loopback address only, so nothing off this machine
//!   connects, and it answers only requests that name it as this machine
//!   (`Host`), so another site's name pointed here does not reach it.
//! - Every path starts with the session's secret, made new for each
//!   connection from the system's random source. A page of another site
//!   can send requests to a local port but cannot know the secret, so it
//!   can neither read what is asked nor answer in the wallet's place.
//! - Bodies must be declared JSON, which a page of another site may not
//!   send here without the browser asking first, and nothing here grants it.
//! - The page asks for nothing by itself and the wallet approves nothing by
//!   itself: every signature is still the user's click in the wallet.
//!
//! The page is one tab. A reload, or the link opened again, takes over
//! (the newest to say hello is asked, and is given again what its
//! predecessor had not answered); the older one is told so. A tab that
//! stops polling for [`GRACE`] is a closed one, and that ends the session:
//! the app says the page was closed and the user connects again.
//!
//! The port is [`PORT`] whenever it is free, so the page's address is the
//! same from one connection to the next and the wallet knows it as the
//! site it connected before; any free port otherwise.

mod http;

use std::{cell::RefCell, collections::BTreeMap, net::Ipv4Addr, pin::pin, rc::Rc, time::Duration};

use alloy::primitives::hex;
use nsh_core::{
    prelude::*,
    time::{Instant, now},
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{Notify, mpsc, oneshot},
};

use self::http::{Request, Response};
use super::{Call, Connector, Event, Events, Link, Reach, Refusal, Session, WalletInfo};
use crate::data::net::{TaskHandle, TaskSet, deadline};

/// The page the user's browser is given.
const PAGE: &str = include_str!("bridge.html");
/// The port the page is served on while it is free.
const PORT: u16 = 47_821;
/// How long a poll is held while there is nothing to ask.
const HOLD: Duration = Duration::from_secs(25);
/// How long a request may take to arrive whole.
const READ: Duration = Duration::from_secs(10);
/// How long the page may go without polling before it counts as closed. A
/// reload is back well within it.
const GRACE: Duration = Duration::from_secs(4);
/// How often the page is checked for having gone.
const WATCH: Duration = Duration::from_secs(1);
/// The most wallets a page is believed to have found, and the longest id (a
/// wallet's, a page's) one is believed to have.
const WALLETS: usize = 16;
const ID: usize = 64;

/// The wallets of the user's browser.
pub(super) struct Bridge {
    /// How long the page may go without polling before it counts as
    /// closed: [`GRACE`], but for a test that will not wait that long.
    grace: Duration,
}

impl Default for Bridge {
    fn default() -> Self {
        Self { grace: GRACE }
    }
}

impl Connector for Bridge {
    fn reach(&self) -> Reach {
        Reach::Browser
    }

    fn open(&self, events: Events) -> Result<Session, String> {
        let mut secret = [0_u8; 16];
        getrandom::getrandom(&mut secret)
            .map_err(|error| format!("The wallet page could not be made private: {error}"))?;
        let (link, calls) = Link::new();
        let state = Rc::new(State {
            token: hex::encode(secret),
            grace: self.grace,
            events,
            queued: Notify::new(),
            inner: RefCell::new(Inner::default()),
        });
        // The connections are tasks of their own (a held poll must not keep
        // the wallet's answers waiting), which end with the session: each
        // holds the state, and nothing of the state holds them. With the
        // last of them the state goes, and whoever still waits for an
        // answer learns there will be none.
        let connections = TaskSet::default();
        let server = TaskHandle::spawn(serve(state, connections.clone(), calls));
        Ok(Session::new(link, (connections, server)))
    }
}

struct State {
    /// The session's secret, which every request's path starts with.
    token: String,
    /// How long the page may go without polling ([`Bridge::grace`]).
    grace: Duration,
    events: Events,
    /// Wakes the held poll: there is something to ask, or another page
    /// took over.
    queued: Notify,
    /// Never borrowed across an `await` or a call of `events`.
    inner: RefCell<Inner>,
}

#[derive(Default)]
struct Inner {
    port: u16,
    /// What the wallets have not answered yet, by the id the page knows
    /// each request by.
    pending: BTreeMap<u64, Pending>,
    last_request: u64,
    /// The page that is asked: the last one to say hello.
    page: Option<String>,
    /// How many of that page's polls are held now.
    held: usize,
    /// When that page last polled, or a poll of its ended.
    seen: Option<Instant>,
}

struct Pending {
    /// The request as the page is given it.
    call: Value,
    reply: oneshot::Sender<Result<Value, Refusal>>,
    /// Whether the page that is asked has been given it.
    sent: bool,
}

/// What a page says of itself and of the wallets it found.
#[derive(Deserialize)]
struct Hello {
    page: String,
    wallets: Vec<WalletInfo>,
}

/// A wallet's answer to the request `id`.
#[derive(Deserialize)]
struct Reply {
    id: u64,
    #[serde(default)]
    result: Value,
    error: Option<Failure>,
}

#[derive(Deserialize)]
struct Failure {
    code: Option<i64>,
    #[serde(default)]
    message: String,
    data: Option<Value>,
}

/// Something a wallet said without being asked.
#[derive(Deserialize)]
struct Notice {
    wallet: String,
    name: String,
    #[serde(default)]
    data: Value,
}

/// Serves the page and passes `calls` to it until the session drops or the
/// page goes.
async fn serve(state: Rc<State>, connections: TaskSet, mut calls: mpsc::UnboundedReceiver<Call>) {
    let lost = |reason: String| (state.events)(Event::Lost(reason));
    let listener = match listen().await {
        Ok(listener) => listener,
        Err(error) => return lost(format!("The wallet page could not be served: {error}")),
    };
    let port = match listener.local_addr() {
        Ok(address) => address.port(),
        Err(error) => return lost(format!("The wallet page could not be served: {error}")),
    };
    state.inner.borrow_mut().port = port;
    (state.events)(Event::Page(format!(
        "http://127.0.0.1:{port}/{}",
        state.token
    )));

    let mut watch = pin!(timer(WATCH));
    loop {
        tokio::select! {
            accepted = listener.accept() => match accepted {
                // Bound to the loopback address, so nothing else connects;
                // checked all the same.
                Ok((stream, peer)) if peer.ip().is_loopback() => {
                    connections.spawn(answer(state.clone(), stream));
                }
                Ok(_) => {}
                Err(error) => {
                    log::warn!("Lattice: the wallet page's server could not accept: {error}");
                    timer(WATCH).await;
                }
            },
            call = calls.recv() => match call {
                Some(call) => state.queue(call),
                None => return,
            },
            () = watch.as_mut() => {
                if state.page_gone() {
                    return lost("The wallet page was closed.".to_owned());
                }
                watch.set(timer(WATCH));
            }
        }
    }
}

/// Listens on [`PORT`], or on any free port when that one is taken (by
/// another Lattice, or by anything else).
async fn listen() -> std::io::Result<TcpListener> {
    match TcpListener::bind((Ipv4Addr::LOCALHOST, PORT)).await {
        Ok(listener) => Ok(listener),
        Err(_) => TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await,
    }
}

/// Answers the one request `stream` brings.
async fn answer(state: Rc<State>, mut stream: TcpStream) {
    let response = match deadline(READ, http::read(&mut stream)).await {
        Some(Ok(request)) => state.respond(&request, &mut stream).await,
        Some(Err(http::Malformed)) => Response::empty(400),
        // Too slow to be the page.
        None => return,
    };
    let _ = stream.write_all(&response.encode()).await;
    let _ = stream.shutdown().await;
}

impl State {
    /// Queues `call` for the page and wakes its poll.
    fn queue(&self, call: Call) {
        {
            let mut inner = self.inner.borrow_mut();
            inner.last_request += 1;
            let id = inner.last_request;
            let request = json!({
                "id": id,
                "wallet": call.wallet,
                "method": call.method,
                "params": call.params,
            });
            inner.pending.insert(
                id,
                Pending {
                    call: request,
                    reply: call.reply,
                    sent: false,
                },
            );
        }
        self.queued.notify_waiters();
    }

    /// Whether a page that was there has stopped polling.
    fn page_gone(&self) -> bool {
        let inner = self.inner.borrow();
        inner.page.is_some()
            && inner.held == 0
            && inner
                .seen
                .is_some_and(|seen| now().saturating_duration_since(seen) > self.grace)
    }

    /// What `path` asks for: what follows the session's secret. `None` for
    /// a path without it.
    fn action<'a>(&self, path: &'a str) -> Option<&'a str> {
        let rest = path.strip_prefix('/')?;
        let (token, action) = rest.split_once('/').unwrap_or((rest, ""));
        same(token.as_bytes(), self.token.as_bytes()).then_some(action)
    }

    async fn respond(&self, request: &Request, stream: &mut TcpStream) -> Response {
        let port = self.inner.borrow().port;
        let named = |host: &String| {
            *host == format!("127.0.0.1:{port}") || *host == format!("localhost:{port}")
        };
        if !request.host.as_ref().is_some_and(named) {
            return Response::empty(403);
        }
        let Some(action) = self.action(&request.path) else {
            return Response::empty(404);
        };
        match (request.method.as_str(), action) {
            ("GET", "") => Response::html(PAGE),
            ("GET", "next") => self.next(request, stream).await,
            ("POST", "hello" | "reply" | "event") => {
                if !request.is_json() {
                    return Response::empty(400);
                }
                let Ok(body) = serde_json::from_slice(&request.body) else {
                    return Response::empty(400);
                };
                match action {
                    "hello" => self.hello(body),
                    "reply" => self.reply(body),
                    _ => self.notice(body),
                }
            }
            (_, "" | "next" | "hello" | "reply" | "event") => Response::empty(405),
            _ => Response::empty(404),
        }
    }

    /// A page has looked for wallets: it is the one asked from now on.
    fn hello(&self, body: Value) -> Response {
        let Ok(Hello { page, mut wallets }) = serde_json::from_value(body) else {
            return Response::empty(400);
        };
        if page.is_empty() || page.len() > ID {
            return Response::empty(400);
        }
        wallets.truncate(WALLETS);
        wallets.retain(|wallet| !wallet.id.is_empty() && wallet.id.len() <= ID);
        {
            let mut inner = self.inner.borrow_mut();
            if inner.page.as_deref() != Some(page.as_str()) {
                // Another tab: it has been given nothing yet, and what its
                // predecessor was given and did not answer is asked again.
                for pending in inner.pending.values_mut() {
                    pending.sent = false;
                }
                inner.page = Some(page);
                inner.held = 0;
            }
            inner.seen = Some(now());
        }
        // The older tab's poll ends, and is told it was replaced.
        self.queued.notify_waiters();
        (self.events)(Event::Wallets(wallets));
        Response::empty(204)
    }

    /// What there is to ask the wallets, for the page `page`: at once when
    /// something waits, else when something comes, else nothing once
    /// [`HOLD`] has passed.
    async fn next(&self, request: &Request, stream: &mut TcpStream) -> Response {
        let Some(page) = request.query_value("page") else {
            return Response::empty(400);
        };
        let mut hold = pin!(timer(HOLD));
        loop {
            {
                let mut inner = self.inner.borrow_mut();
                if inner.page.as_deref() != Some(page) {
                    return Response::empty(409);
                }
                inner.seen = Some(now());
                // What nobody waits for any more is not asked at all.
                inner
                    .pending
                    .retain(|_, pending| pending.sent || !pending.reply.is_closed());
                let calls: Vec<Value> = inner
                    .pending
                    .values_mut()
                    .filter(|pending| !pending.sent)
                    .map(|pending| {
                        pending.sent = true;
                        pending.call.clone()
                    })
                    .collect();
                if !calls.is_empty() {
                    return Response::json(&Value::Array(calls));
                }
                inner.held += 1;
            }
            // The page sends nothing more on this connection, so whatever
            // a read brings is its end: the tab was closed or reloaded.
            let mut end = [0_u8; 1];
            let done = tokio::select! {
                () = self.queued.notified() => false,
                () = hold.as_mut() => true,
                _ = stream.read(&mut end) => true,
            };
            {
                let mut inner = self.inner.borrow_mut();
                if inner.page.as_deref() == Some(page) {
                    inner.held = inner.held.saturating_sub(1);
                    inner.seen = Some(now());
                }
            }
            if done {
                return Response::empty(204);
            }
        }
    }

    /// A wallet's answer goes to whoever asked.
    fn reply(&self, body: Value) -> Response {
        let Ok(Reply { id, result, error }) = serde_json::from_value(body) else {
            return Response::empty(400);
        };
        let Some(pending) = self.inner.borrow_mut().pending.remove(&id) else {
            return Response::empty(404);
        };
        let answer = match error {
            Some(failure) => Err(Refusal {
                code: failure.code.unwrap_or(Refusal::INTERNAL),
                message: failure.message,
                data: failure.data,
            }),
            None => Ok(result),
        };
        // Nobody waiting is nothing to do about.
        let _ = pending.reply.send(answer);
        Response::empty(204)
    }

    /// What a wallet said without being asked goes on to the app.
    fn notice(&self, body: Value) -> Response {
        let Ok(Notice { wallet, name, data }) = serde_json::from_value(body) else {
            return Response::empty(400);
        };
        match name.as_str() {
            "accountsChanged" => (self.events)(Event::Accounts {
                wallet,
                accounts: data,
            }),
            "chainChanged" => (self.events)(Event::Chain {
                wallet,
                chain: data,
            }),
            _ => {}
        }
        Response::empty(204)
    }
}

/// Whether `left` is `right`, taking as long to say so wherever they
/// differ.
fn same(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .fold(0, |differing, (left, right)| differing | (left ^ right))
            == 0
}

#[cfg(test)]
mod tests;
