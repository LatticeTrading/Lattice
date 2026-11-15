//! What the connection indicator shows: every socket and query hub reports
//! one [`SourceStatus`] here when its health changes, never per message.
//! The connection panel reads the rest from the sources themselves while it
//! is open ([`SourceDetails`]).

use std::{
    cell::{Cell, RefCell},
    future::poll_fn,
    rc::Rc,
    task::{Poll, Waker},
    time::Duration,
};

use nsh_core::{prelude::*, time::unix_ms};

use super::{
    consumer::Held,
    error::{ApiError, sanitize},
    socket::TopicPhase,
};

/// How a source is doing, from best to worst; the board's overall health
/// is its worst source's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Health {
    /// Nothing is asking this source for data.
    Idle,
    /// Data is arriving as expected.
    Healthy,
    /// Data is wanted but nothing has confirmed it yet: connecting, or a
    /// first request in flight.
    Pending,
    /// Working, with problems: a failure or two, a reconnect, a missed
    /// keepalive, a warning from the server.
    Degraded,
    /// Repeated failures: data is not arriving.
    Failing,
}

/// A failure kept for diagnostics.
#[derive(Clone, Debug, PartialEq)]
pub struct Failure {
    /// When it happened, in Unix milliseconds.
    pub at_ms: f64,
    pub code: String,
    pub message: String,
}

impl Failure {
    pub(crate) fn now(code: impl AsRef<str>, message: impl AsRef<str>) -> Self {
        Self {
            at_ms: unix_ms(),
            code: sanitize(code.as_ref(), 80),
            message: sanitize(message.as_ref(), 240),
        }
    }
}

impl From<&ApiError> for Failure {
    fn from(error: &ApiError) -> Self {
        Self::now(&error.code, &error.message)
    }
}

/// One source's report.
#[derive(Clone, Debug, PartialEq)]
pub struct SourceStatus {
    /// Stable id, e.g. `ferris.socket`.
    pub id: &'static str,
    /// What people see, e.g. `Ferris realtime`.
    pub label: &'static str,
    pub health: Health,
    /// How many things want data from it: live topics, watched queries.
    pub demand: usize,
    /// One line about its state: `Connected · 3 topics`, `Reconnecting in 2s`.
    pub summary: String,
    pub last_failure: Option<Failure>,
    /// A socket's data that reached no consumer on its current connection
    /// (always empty for a query hub).
    pub unrouted: Unrouted,
}

/// Data frames a socket received for no wanted topic: how "subscribed,
/// acknowledged, no data" explains itself. Routing is exact, so these are
/// usually a key the server spells differently from the subscription (a
/// symbol alias, a param it adds). Frames for a topic still in its release
/// grace or being unsubscribed are expected and not counted.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Unrouted {
    /// How many arrived. Counted per frame but reported with the source's
    /// next status, at the latest its next keepalive check, never a status
    /// per frame.
    pub frames: u64,
    /// What they were for, first seen first, at most 8: the topic key they
    /// named, or `<channel> (no topic)` / `<channel> (no topic; several
    /// wanted)` for frames that named only their channel.
    pub targets: Vec<String>,
    /// Wanted topics the server acknowledged that get no data while frames
    /// for another key of their channel and exchange arrive: most likely
    /// the server's name for them. Each makes the source `Degraded`.
    pub aliases: Vec<Alias>,
}

/// A wanted topic and the key its data most likely arrives under.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Alias {
    /// The key subscribed: acknowledged, no data.
    pub wanted: String,
    /// The unrouted key arriving on the same channel and exchange.
    pub arriving: String,
}

/// How a source carries its data.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Transport {
    /// One shared WebSocket.
    Socket,
    /// Shared, polled HTTP requests.
    Http,
}

impl Transport {
    /// Its short name (`WS`, `HTTP`).
    pub fn label(self) -> &'static str {
        match self {
            Self::Socket => "WS",
            Self::Http => "HTTP",
        }
    }
}

/// What a topic or a request is, as the connection panel names it. The
/// integration that makes the topic or the request says
/// ([`SocketProtocol::subject`](super::SocketProtocol::subject),
/// [`QuerySpec::subject`](super::QuerySpec::subject)).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Subject {
    /// The exchange's id when it is one exchange's (`bybit`), else empty.
    pub exchange: String,
    /// What it is of: a symbol (`BTC/USDT`), `All markets`.
    pub name: String,
    /// What is asked of it: `Order book · 50 levels`.
    pub detail: String,
}

impl Subject {
    /// A subject known only by `name` (a key nobody described).
    pub fn named(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Self::default()
        }
    }
}

/// One thing a source carries, as it stands when asked: a socket's topic
/// or a hub's watched request.
#[derive(Clone, Debug, PartialEq)]
pub struct Feed {
    /// Its key in its source.
    pub key: String,
    pub subject: Subject,
    pub state: FeedState,
    /// Who wants it.
    pub held: Held,
    /// When its data last arrived, in Unix milliseconds.
    pub last_data_ms: Option<f64>,
    /// A topic's latest warning or error from the server (until data flows
    /// again); a request's latest failure (until it succeeds).
    pub failure: Option<Failure>,
}

/// Where a [`Feed`] stands.
#[derive(Clone, Debug, PartialEq)]
pub enum FeedState {
    /// A socket's topic.
    Topic {
        phase: TopicPhase,
        /// The key its data most likely arrives under instead (see
        /// [`Alias`]).
        alias: Option<String>,
    },
    /// A watched request.
    Request {
        /// Whether a value is held (kept through later failures).
        loaded: bool,
        /// Whether a fetch is in flight.
        fetching: bool,
        /// Fetches failed in a row.
        failures: u32,
        /// How often it is refetched.
        every: Option<Duration>,
    },
}

/// A socket's connection, as it stands when asked.
#[derive(Clone, Debug, PartialEq)]
pub struct Link {
    pub state: LinkState,
    /// Whether this connection has delivered a valid frame.
    pub responsive: bool,
    /// Keepalives in a row that got no answer.
    pub missed_keepalives: u32,
    /// The last answered keepalive's round trip.
    pub latency: Option<Duration>,
    /// While [`LinkState::Retrying`]: when the next attempt is made, in
    /// Unix milliseconds.
    pub retry_at_ms: Option<f64>,
    /// Reconnect attempts since the last connection that delivered a frame.
    pub attempt: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LinkState {
    /// Not connected, and nothing asks it to be.
    Idle,
    Connecting,
    Open,
    /// Waiting out the backoff before the next attempt.
    Retrying,
}

/// One source in full: its report and everything it carries. Unlike a
/// [`SourceStatus`], which a source publishes when its health changes, this
/// is read from the source when something asks (the open connection panel):
/// nothing is kept or copied for it meanwhile.
#[derive(Clone, Debug, PartialEq)]
pub struct SourceDetails {
    pub status: SourceStatus,
    pub transport: Transport,
    /// A socket's connection; none for a query hub.
    pub link: Option<Link>,
    /// What it carries that someone wants, in a stable order.
    pub feeds: Vec<Feed>,
}

/// Every source's latest report. A `Copy` handle, and reading
/// [`Self::overall`] in a binding follows health changes.
///
/// A report is not shown as it is made but on the engine's next turn, by
/// the board's own task: a source reports from the middle of its work (a
/// frame half handled, a subscriber being dropped, the app being taken
/// down), and what follows the board is view code, which must never run
/// there. Reports made in one turn are shown together.
#[derive(Clone, Copy)]
pub struct StatusBoard {
    /// What readers follow: the reports as last shown.
    shown: Signal<Vec<SourceStatus>>,
    /// Where reports wait to be shown. Nothing follows it.
    desk: Signal<Desk>,
}

/// The reports as the sources last made them, and the task that shows them.
#[derive(Clone)]
struct Desk(Rc<Waiting>);

/// The one desk is equal to itself.
impl PartialEq for Desk {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

struct Waiting {
    latest: RefCell<Vec<SourceStatus>>,
    /// Whether `latest` has something not shown yet.
    news: Cell<bool>,
    /// The showing task, while it waits for news.
    task: RefCell<Option<Waker>>,
}

impl Default for StatusBoard {
    fn default() -> Self {
        Self::new()
    }
}

impl StatusBoard {
    /// A board owned by the current owner, which its task ends with.
    pub fn new() -> Self {
        let shown = signal(Vec::new());
        let desk = Rc::new(Waiting {
            latest: RefCell::new(Vec::new()),
            news: Cell::new(false),
            task: RefCell::new(None),
        });
        let waiting = Rc::downgrade(&desk);
        spawn(async move {
            loop {
                let news = poll_fn(|cx| {
                    let Some(desk) = waiting.upgrade() else {
                        return Poll::Ready(None);
                    };
                    if desk.news.replace(false) {
                        Poll::Ready(Some(desk.latest.borrow().clone()))
                    } else {
                        *desk.task.borrow_mut() = Some(cx.waker().clone());
                        Poll::Pending
                    }
                })
                .await;
                // The board is gone: so is everything that read it.
                let Some(sources) = news else {
                    return;
                };
                if !shown.try_set(sources) {
                    return;
                }
            }
        });
        Self {
            shown,
            desk: signal(Desk(desk)),
        }
    }

    /// Every source that has reported, in the order they first did.
    pub fn sources(&self) -> Vec<SourceStatus> {
        self.shown.get()
    }

    /// The worst health of any source; `Idle` when none wants data.
    pub fn overall(&self) -> Health {
        self.shown.with(|sources| {
            sources
                .iter()
                .map(|source| source.health)
                .max()
                .unwrap_or(Health::Idle)
        })
    }

    /// Replaces `status.id`'s report, to be shown on the engine's next turn
    /// when it changed. Safe to call from anywhere on the engine thread: it
    /// subscribes nothing, runs nothing that reads the board, and does
    /// nothing once the board's owner is gone.
    pub(crate) fn publish(&self, status: SourceStatus) {
        let Some(Desk(desk)) = untracked(|| self.desk.try_get()) else {
            return;
        };
        {
            let mut latest = desk.latest.borrow_mut();
            match latest.iter_mut().find(|source| source.id == status.id) {
                Some(existing) if *existing == status => return,
                Some(existing) => *existing = status,
                None => latest.push(status),
            }
        }
        desk.news.set(true);
        // Taken before it is woken: the task may run at once.
        let task = desk.task.borrow_mut().take();
        if let Some(task) = task {
            task.wake();
        }
    }
}
