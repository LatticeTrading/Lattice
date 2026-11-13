//! Shared, keyed requests: the one place an integration's REST data is
//! fetched, cached and polled.
//!
//! A [`QuerySpec`] names a request by key. However many consumers ask for a
//! key, at most one request for it is in flight:
//!
//! - [`Queries::watch`] keeps the key's latest value in a signal the
//!   consumer owns. A key watched with a refresh interval is refetched on it
//!   while anyone watches; when the last watcher leaves, polling stops and
//!   an unanswered request is abandoned.
//! - [`Queries::get`] awaits the value once: a fresh cached value at once,
//!   else the request in flight, else a new one.
//!
//! A failed fetch keeps the previous value and reports its error beside it.
//! A key that has never succeeded retries sooner (1s, doubling up to its
//! interval). An unwatched value is kept for ten minutes, so the next
//! widget to ask gets it immediately.
//!
//! A fetch that panics (building its future or in a poll) settles as a
//! failed fetch with a `PANICKED` error, so the key never stays "fetching",
//! its waiters wake and its poll loop goes on; a watcher whose signal write
//! panics misses that one update and the others still get it. Both are
//! logged with the key (desktop; on the web a panic ends the app, see
//! `guard.rs`).

use std::{
    any::Any,
    cell::{Cell, RefCell},
    collections::HashMap,
    future::{Future, poll_fn},
    pin::Pin,
    rc::{Rc, Weak},
    task::{Poll, Waker},
    time::Duration,
};

use nsh_core::{
    prelude::*,
    time::{Instant, now, unix_ms},
};

use super::{
    error::ApiError,
    guard::{contain, contained},
    status::{Failure, Health, SourceStatus, StatusBoard, Unrouted},
    task::TaskHandle,
};

type BoxFuture<T> = Pin<Box<dyn Future<Output = T>>>;
type Erased = Rc<dyn Any>;
type Fetcher = Rc<dyn Fn() -> BoxFuture<Result<Erased, ApiError>>>;
type Push = Rc<dyn Fn(&Snapshot) -> bool>;

/// How long a value nobody watches is kept for the next consumer.
const RETAIN: Duration = Duration::from_secs(10 * 60);
/// Failed fetches of one key in a row after which the hub reports `Failing`.
const FAILING_AFTER: u32 = 3;
/// The first retry of a key that has never succeeded; it doubles from here.
const FIRST_RETRY: Duration = Duration::from_secs(1);

/// One shareable request: its key, how to fetch it, and how fresh it must be.
pub struct QuerySpec<T> {
    key: String,
    fetch: Rc<dyn Fn() -> BoxFuture<Result<T, ApiError>>>,
    refresh: Option<Duration>,
    fresh_for: Option<Duration>,
}

impl<T> Clone for QuerySpec<T> {
    fn clone(&self) -> Self {
        Self {
            key: self.key.clone(),
            fetch: self.fetch.clone(),
            refresh: self.refresh,
            fresh_for: self.fresh_for,
        }
    }
}

impl<T: 'static> QuerySpec<T> {
    /// The request `key` fetched by `fetch`. The key must identify the
    /// request exactly (endpoint plus canonical parameters, see
    /// [`super::canonical_json`]): equal keys share one value.
    pub fn new<F, Fut>(key: impl Into<String>, fetch: F) -> Self
    where
        F: Fn() -> Fut + 'static,
        Fut: Future<Output = Result<T, ApiError>> + 'static,
    {
        Self {
            key: key.into(),
            fetch: Rc::new(move || Box::pin(fetch())),
            refresh: None,
            fresh_for: None,
        }
    }

    /// Refetches every `interval` while watched. A value younger than the
    /// interval is then fresh unless [`Self::fresh_for`] says otherwise.
    #[must_use]
    pub fn refresh_every(mut self, interval: Duration) -> Self {
        self.refresh = Some(interval);
        self
    }

    /// How old a cached value may be and still be served without a fetch.
    /// Without it: the refresh interval, else zero (always fetch).
    #[must_use]
    pub fn fresh_for(mut self, age: Duration) -> Self {
        self.fresh_for = Some(age);
        self
    }

    pub fn key(&self) -> &str {
        &self.key
    }

    fn freshness(&self) -> Duration {
        self.fresh_for.or(self.refresh).unwrap_or_default()
    }

    fn erased(&self) -> Fetcher {
        let fetch = self.fetch.clone();
        Rc::new(move || {
            let pending = fetch();
            Box::pin(async move { pending.await.map(|value| Rc::new(value) as Erased) })
        })
    }
}

/// What a watcher sees of one key.
pub struct QueryState<T> {
    /// The latest good value, kept through later failures.
    pub data: Option<Rc<T>>,
    /// The latest fetch's error, cleared by the next success.
    pub error: Option<ApiError>,
    /// When `data` arrived, in Unix milliseconds.
    pub updated_at_ms: Option<f64>,
    /// Whether a fetch is in flight.
    pub fetching: bool,
}

impl<T> QueryState<T> {
    /// Nothing yet: no value and no error.
    pub fn is_loading(&self) -> bool {
        self.data.is_none() && self.error.is_none()
    }
}

impl<T> Clone for QueryState<T> {
    fn clone(&self) -> Self {
        Self {
            data: self.data.clone(),
            error: self.error.clone(),
            updated_at_ms: self.updated_at_ms,
            fetching: self.fetching,
        }
    }
}

impl<T> Default for QueryState<T> {
    fn default() -> Self {
        Self {
            data: None,
            error: None,
            updated_at_ms: None,
            fetching: false,
        }
    }
}

/// The same fetched value is equal; values are never compared by content.
impl<T> PartialEq for QueryState<T> {
    fn eq(&self, other: &Self) -> bool {
        let same_data = match (&self.data, &other.data) {
            (Some(a), Some(b)) => Rc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        same_data
            && self.error == other.error
            && self.updated_at_ms == other.updated_at_ms
            && self.fetching == other.fetching
    }
}

impl<T: 'static> QueryState<T> {
    fn of(snapshot: &Snapshot, key: &str) -> Self {
        let data = snapshot.data.clone().map(|data| typed::<T>(data, key));
        let (data, error) = match data {
            Some(Err(mismatch)) => (None, Some(mismatch)),
            Some(Ok(data)) => (Some(data), snapshot.error.clone()),
            None => (None, snapshot.error.clone()),
        };
        Self {
            data,
            error,
            updated_at_ms: snapshot.updated_at_ms,
            fetching: snapshot.fetching,
        }
    }
}

/// A key's state before it is typed for a watcher.
#[derive(Clone)]
struct Snapshot {
    data: Option<Erased>,
    error: Option<ApiError>,
    updated_at_ms: Option<f64>,
    fetching: bool,
}

fn typed<T: 'static>(data: Erased, key: &str) -> Result<Rc<T>, ApiError> {
    data.downcast::<T>()
        .map_err(|_| ApiError::invalid_request(format!("query `{key}` holds another type")))
}

/// A consumer's hold on a key. Dropping it stops watching.
#[must_use = "dropping a watch stops it"]
pub struct Watch<T> {
    state: ReadSignal<QueryState<T>>,
    lease: Lease,
}

impl<T: 'static> Watch<T> {
    /// The key's state, a signal the watching owner owns.
    pub fn state(&self) -> ReadSignal<QueryState<T>> {
        self.state
    }

    /// Fetches now, unless a fetch is already in flight.
    pub fn refresh(&self) {
        if let Some(core) = self.lease.hub.upgrade() {
            core.fetch_now(&self.lease.key);
        }
    }

    /// Keeps watching until the current owner (a widget) is cleaned up, and
    /// gives the state signal to read meanwhile.
    pub fn release_on_cleanup(self) -> ReadSignal<QueryState<T>> {
        let Self { state, lease } = self;
        cleanup(move || drop(lease));
        state
    }
}

struct Lease {
    key: String,
    id: u64,
    hub: Weak<Core>,
}

impl Drop for Lease {
    fn drop(&mut self) {
        if let Some(core) = self.hub.upgrade() {
            core.unwatch(&self.key, self.id);
        }
    }
}

/// One integration's shared requests. Cloning is cheap and shares it.
#[derive(Clone)]
pub struct Queries(Rc<Core>);

struct Core {
    me: Weak<Core>,
    status: StatusBoard,
    id: &'static str,
    label: &'static str,
    next_watcher: Cell<u64>,
    last_failure: RefCell<Option<Failure>>,
    entries: RefCell<HashMap<String, Entry>>,
}

struct Entry {
    fetch: Fetcher,
    data: Option<Erased>,
    error: Option<ApiError>,
    updated_at: Option<Instant>,
    updated_at_ms: Option<f64>,
    settled_at: Option<Instant>,
    /// Fetches settled so far; a waiter waits for the count to pass its own.
    settled: u64,
    failures: u32,
    in_flight: Option<TaskHandle>,
    waiters: Vec<Waker>,
    watchers: Vec<Watcher>,
    poller: Option<TaskHandle>,
    idle_since: Option<Instant>,
}

struct Watcher {
    id: u64,
    refresh: Option<Duration>,
    /// Writes the watcher's signal; false once its owner is gone.
    push: Push,
}

impl Entry {
    fn new(fetch: Fetcher) -> Self {
        Self {
            fetch,
            data: None,
            error: None,
            updated_at: None,
            updated_at_ms: None,
            settled_at: None,
            settled: 0,
            failures: 0,
            in_flight: None,
            waiters: Vec::new(),
            watchers: Vec::new(),
            poller: None,
            idle_since: None,
        }
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            data: self.data.clone(),
            error: self.error.clone(),
            updated_at_ms: self.updated_at_ms,
            fetching: self.in_flight.is_some(),
        }
    }

    fn is_fresh(&self, age: Duration, now: Instant) -> bool {
        self.data.is_some() && self.updated_at.is_some_and(|at| at + age > now)
    }

    /// The shortest interval any watcher asked for.
    fn interval(&self) -> Option<Duration> {
        self.watchers
            .iter()
            .filter_map(|watcher| watcher.refresh)
            .min()
    }

    fn demanded(&self) -> bool {
        !self.watchers.is_empty() || self.in_flight.is_some()
    }

    fn evictable(&self, now: Instant) -> bool {
        !self.demanded()
            && self.waiters.is_empty()
            && self.idle_since.is_some_and(|at| at + RETAIN <= now)
    }
}

impl Queries {
    /// A hub that reports to `status` as source `id` (`ferris.queries`)
    /// named `label` (`Ferris requests`).
    pub fn new(status: StatusBoard, id: &'static str, label: &'static str) -> Self {
        let queries = Self(Rc::new_cyclic(|me| Core {
            me: me.clone(),
            status,
            id,
            label,
            next_watcher: Cell::new(0),
            last_failure: RefCell::new(None),
            entries: RefCell::new(HashMap::new()),
        }));
        queries.0.publish();
        queries
    }

    /// Watches `spec`'s key: the returned state follows every fetch of it,
    /// and with a refresh interval it is refetched on it while watched.
    pub fn watch<T: 'static>(&self, spec: QuerySpec<T>) -> Watch<T> {
        let core = &self.0;
        core.evict();
        let id = core.next_watcher.get() + 1;
        core.next_watcher.set(id);
        let state = signal(QueryState::<T>::default());
        let push: Push = {
            let key = spec.key.clone();
            Rc::new(move |snapshot| state.try_set(QueryState::of(snapshot, &key)))
        };
        let (snapshot, started) = {
            let mut entries = core.entries.borrow_mut();
            let entry = entries
                .entry(spec.key.clone())
                .or_insert_with(|| Entry::new(spec.erased()));
            entry.watchers.push(Watcher {
                id,
                refresh: spec.refresh,
                push,
            });
            entry.idle_since = None;
            let started = entry.in_flight.is_none() && !entry.is_fresh(spec.freshness(), now());
            if started {
                core.start_fetch(&spec.key, entry);
            }
            if entry.interval().is_some() && entry.poller.is_none() {
                entry.poller = Some(core.poll(spec.key.clone()));
            }
            (entry.snapshot(), started)
        };
        if started {
            core.notify(&spec.key);
        } else {
            state.set(QueryState::of(&snapshot, &spec.key));
        }
        core.publish();
        Watch {
            state: state.read_only(),
            lease: Lease {
                key: spec.key,
                id,
                hub: Rc::downgrade(core),
            },
        }
    }

    /// `spec`'s value: a fresh cached one at once, else the result of the
    /// fetch in flight, else of a new one.
    pub async fn get<T: 'static>(&self, spec: QuerySpec<T>) -> Result<Rc<T>, ApiError> {
        let core = &self.0;
        core.evict();
        let (target, started) = {
            let mut entries = core.entries.borrow_mut();
            let entry = entries
                .entry(spec.key.clone())
                .or_insert_with(|| Entry::new(spec.erased()));
            if entry.is_fresh(spec.freshness(), now())
                && let Some(data) = &entry.data
            {
                return typed(data.clone(), &spec.key);
            }
            let started = entry.in_flight.is_none();
            if started {
                core.start_fetch(&spec.key, entry);
            }
            (entry.settled + 1, started)
        };
        if started {
            core.notify(&spec.key);
            core.publish();
        }
        let me = Rc::downgrade(core);
        settled(&me, &spec.key, target).await;
        let gone = || ApiError::network("the data services shut down");
        let core = me.upgrade().ok_or_else(gone)?;
        let entries = core.entries.borrow();
        let entry = entries.get(&spec.key).ok_or_else(gone)?;
        if let Some(error) = &entry.error {
            return Err(error.clone());
        }
        let data = entry
            .data
            .clone()
            .ok_or_else(|| ApiError::network("the request was abandoned"))?;
        typed(data, &spec.key)
    }

    /// The cached value of `key`, without fetching.
    pub fn cached<T: 'static>(&self, key: &str) -> Option<Rc<T>> {
        let data = self.0.entries.borrow().get(key)?.data.clone()?;
        typed(data, key).ok()
    }

    /// Fetches `key` now, unless a fetch is already in flight.
    pub fn refresh(&self, key: &str) {
        self.0.fetch_now(key);
    }

    /// Drops every key and abandons every request.
    pub fn shutdown(&self) {
        let entries = std::mem::take(&mut *self.0.entries.borrow_mut());
        for entry in entries.values() {
            for waiter in &entry.waiters {
                waiter.wake_by_ref();
            }
        }
        drop(entries);
        self.0.publish();
    }
}

impl Core {
    fn start_fetch(&self, key: &str, entry: &mut Entry) {
        let fetch = entry.fetch.clone();
        let (me, key, label) = (self.me.clone(), key.to_owned(), self.label);
        entry.in_flight = Some(TaskHandle::spawn(async move {
            // The fetch is the consumer's code: a panic in it settles this
            // fetch as failed instead of killing the task unsettled.
            let result = match contain(|| fetch()) {
                Ok(pending) => contained(pending).await,
                Err(panic) => Err(panic),
            }
            .unwrap_or_else(|panic| {
                log::error!("{label}: fetching `{key}` panicked: {panic}");
                Err(ApiError::panicked(format!("the fetch panicked: {panic}")))
            });
            if let Some(core) = me.upgrade() {
                core.settle(&key, result);
            }
        }));
    }

    /// Starts a fetch of `key` unless one is in flight; the settle count to
    /// wait for.
    fn fetch_now(&self, key: &str) -> Option<u64> {
        let (target, started) = {
            let mut entries = self.entries.borrow_mut();
            let entry = entries.get_mut(key)?;
            let started = entry.in_flight.is_none();
            if started {
                self.start_fetch(key, entry);
            }
            (entry.settled + 1, started)
        };
        if started {
            self.notify(key);
            self.publish();
        }
        Some(target)
    }

    fn settle(&self, key: &str, result: Result<Erased, ApiError>) {
        let waiters = {
            let mut entries = self.entries.borrow_mut();
            let Some(entry) = entries.get_mut(key) else {
                return;
            };
            // This task's own handle: the executor drops it after this poll.
            entry.in_flight = None;
            entry.settled += 1;
            let at = now();
            entry.settled_at = Some(at);
            match result {
                Ok(value) => {
                    entry.data = Some(value);
                    entry.error = None;
                    entry.failures = 0;
                    entry.updated_at = Some(at);
                    entry.updated_at_ms = Some(unix_ms());
                }
                Err(error) => {
                    log::debug!("{}: {key}: {error}", self.label);
                    entry.failures += 1;
                    *self.last_failure.borrow_mut() = Some(Failure::from(&error));
                    entry.error = Some(error);
                }
            }
            if !entry.demanded() {
                entry.idle_since = Some(at);
            }
            std::mem::take(&mut entry.waiters)
        };
        for waiter in waiters {
            waiter.wake();
        }
        self.notify(key);
        self.publish();
    }

    /// Pushes `key`'s state to its watchers, dropping those whose owner is
    /// gone. A push runs the watcher's bindings; one that panics misses
    /// this update and the rest still get it.
    fn notify(&self, key: &str) {
        let (snapshot, pushes) = {
            let entries = self.entries.borrow();
            let Some(entry) = entries.get(key) else {
                return;
            };
            let pushes: Vec<(u64, Push)> = entry
                .watchers
                .iter()
                .map(|watcher| (watcher.id, watcher.push.clone()))
                .collect();
            (entry.snapshot(), pushes)
        };
        for (id, push) in pushes {
            match contain(|| push(&snapshot)) {
                Ok(true) => {}
                Ok(false) => self.unwatch(key, id),
                Err(panic) => {
                    log::error!("{}: a watcher of `{key}` panicked: {panic}", self.label);
                }
            }
        }
    }

    fn unwatch(&self, key: &str, id: u64) {
        let removed = {
            let mut entries = self.entries.borrow_mut();
            let Some(entry) = entries.get_mut(key) else {
                return;
            };
            let Some(index) = entry.watchers.iter().position(|watcher| watcher.id == id) else {
                return;
            };
            let removed = entry.watchers.remove(index);
            if entry.interval().is_none() {
                entry.poller = None;
            }
            if entry.watchers.is_empty() {
                // Nobody waits for the answer: abandon the request.
                if entry.waiters.is_empty() {
                    entry.in_flight = None;
                }
                entry.idle_since = Some(now());
            }
            removed
        };
        drop(removed);
        self.publish();
    }

    /// The task that refetches `key` on its interval while watched.
    fn poll(&self, key: String) -> TaskHandle {
        let me = self.me.clone();
        TaskHandle::spawn(async move {
            loop {
                let Some(due) = me.upgrade().and_then(|core| core.next_poll(&key)) else {
                    return;
                };
                timer_until(due).await;
                let Some(target) = me.upgrade().and_then(|core| core.fetch_now(&key)) else {
                    return;
                };
                settled(&me, &key, target).await;
            }
        })
    }

    fn next_poll(&self, key: &str) -> Option<Instant> {
        let entries = self.entries.borrow();
        let entry = entries.get(key)?;
        let interval = entry.interval()?;
        Some(match entry.settled_at {
            None => now(),
            // Nothing to show yet: retry sooner, backing off to the interval.
            Some(at) if entry.data.is_none() && entry.failures > 0 => {
                let backoff = FIRST_RETRY.saturating_mul(1 << (entry.failures - 1).min(16));
                at + backoff.min(interval)
            }
            Some(at) => at + interval,
        })
    }

    fn evict(&self) {
        let now = now();
        let evicted: Vec<Entry> = {
            let mut entries = self.entries.borrow_mut();
            entries
                .extract_if(|_, entry| entry.evictable(now))
                .map(|(_, entry)| entry)
                .collect()
        };
        drop(evicted);
    }

    fn publish(&self) {
        let status = self.describe();
        self.status.publish(status);
    }

    fn describe(&self) -> SourceStatus {
        let entries = self.entries.borrow();
        let demanded: Vec<&Entry> = entries.values().filter(|entry| entry.demanded()).collect();
        let demand = demanded.len();
        let health = if demand == 0 {
            Health::Idle
        } else if demanded.iter().any(|entry| entry.failures >= FAILING_AFTER) {
            Health::Failing
        } else if demanded.iter().any(|entry| entry.failures > 0) {
            Health::Degraded
        } else if demanded.iter().any(|entry| entry.data.is_none()) {
            Health::Pending
        } else {
            Health::Healthy
        };
        let watched = entries
            .values()
            .filter(|entry| !entry.watchers.is_empty())
            .count();
        let loading = entries
            .values()
            .filter(|entry| entry.in_flight.is_some())
            .count();
        SourceStatus {
            id: self.id,
            label: self.label,
            health,
            demand,
            summary: format!("{watched} watched · {loading} loading"),
            last_failure: self.last_failure.borrow().clone(),
            unrouted: Unrouted::default(),
        }
    }
}

/// Waits until `key` has settled `target` fetches, or its fetch was
/// abandoned, or the key or hub is gone.
async fn settled(me: &Weak<Core>, key: &str, target: u64) {
    poll_fn(|cx| {
        let Some(core) = me.upgrade() else {
            return Poll::Ready(());
        };
        let mut entries = core.entries.borrow_mut();
        match entries.get_mut(key) {
            Some(entry) if entry.settled < target && entry.in_flight.is_some() => {
                if !entry
                    .waiters
                    .iter()
                    .any(|waiter| waiter.will_wake(cx.waker()))
                {
                    entry.waiters.push(cx.waker().clone());
                }
                Poll::Pending
            }
            _ => Poll::Ready(()),
        }
    })
    .await;
}
