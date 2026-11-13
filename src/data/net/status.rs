//! What the connection indicator shows: every socket and query hub reports
//! one [`SourceStatus`] here when its health changes, never per message.

use nsh_core::{prelude::*, time::unix_ms};

use super::error::{ApiError, sanitize};

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

/// Every source's latest report. A `Copy` handle to one signal, so reading
/// [`Self::overall`] in a binding follows health changes.
#[derive(Clone, Copy)]
pub struct StatusBoard(Signal<Vec<SourceStatus>>);

impl Default for StatusBoard {
    fn default() -> Self {
        Self::new()
    }
}

impl StatusBoard {
    /// A board owned by the current owner.
    pub fn new() -> Self {
        Self(signal(Vec::new()))
    }

    /// Every source that has reported, in the order they first did.
    pub fn sources(&self) -> Vec<SourceStatus> {
        self.0.get()
    }

    /// The worst health of any source; `Idle` when none wants data.
    pub fn overall(&self) -> Health {
        self.0.with(|sources| {
            sources
                .iter()
                .map(|source| source.health)
                .max()
                .unwrap_or(Health::Idle)
        })
    }

    /// Replaces `status.id`'s report, notifying only when it changed. Safe
    /// to call from anywhere on the engine thread, including inside a
    /// binding (it subscribes nothing) and after the board's owner is gone.
    pub(crate) fn publish(&self, status: SourceStatus) {
        untracked(|| {
            let Some(mut sources) = self.0.try_get() else {
                return;
            };
            match sources.iter_mut().find(|source| source.id == status.id) {
                Some(existing) if *existing == status => return,
                Some(existing) => *existing = status,
                None => sources.push(status),
            }
            self.0.try_set(sources);
        });
    }
}
