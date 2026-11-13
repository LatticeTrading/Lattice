//! One physical WebSocket per API, shared by every subscriber.
//!
//! A [`SharedSocket`] keeps one entry per topic key with its subscribers:
//!
//! 1. The first subscriber to a key creates it and, once the socket is
//!    open, sends the protocol's subscribe frame. The socket opens on the
//!    first topic.
//! 2. Later subscribers to the key only join it: one server subscription,
//!    one decoded message fanned out to all of them.
//! 3. When the last subscriber leaves, the key lingers for
//!    [`SocketConfig::release_grace`]; a subscriber arriving within it (a
//!    widget re-rendering, a tab moving) takes it back with no frames sent.
//!    After it, the unsubscribe frame goes out, and when no key is left the
//!    socket closes.
//!
//! The connection reconnects with capped backoff and resubscribes every
//! key on each new socket. While open it sends the protocol's ping every
//! [`SocketConfig::keepalive_interval`]; any valid frame counts as an
//! answer, and a socket that misses [`SocketConfig::max_missed_keepalives`]
//! in a row is replaced. Events from a socket that was already replaced are
//! ignored (each connection has a generation).
//!
//! Callbacks run after the socket's state is released, so a subscriber may
//! subscribe, unsubscribe or drop its subscription from inside one. A
//! callback that panics loses that one call: the panic is logged with its
//! topic and every other subscriber still gets the frame (desktop; on the
//! web a panic ends the app, see [`super::guard`]).
//!
//! Routing is exact, so a data frame whose key matches no wanted topic is
//! dropped. Those frames are counted and reported in the socket's status
//! ([`super::Unrouted`]), each distinct key logged once; frames for a topic
//! still in its release grace or being unsubscribed are expected and not
//! counted. A wanted topic the server acknowledged that gets no data while
//! frames for another key of its channel and exchange arrive is reported as
//! a likely alias ([`SocketProtocol::alias_scope`]).

#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(target_arch = "wasm32")]
mod web;

use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, HashMap, HashSet},
    rc::{Rc, Weak},
    sync::Arc,
    time::Duration,
};

use nsh_core::{
    prelude::*,
    time::{Instant, now},
    websocket::{
        WebSocketData, WebSocketEvent, WebSocketEvents, WebSocketRequest, WebSocketTask,
        WebSocketTransport,
    },
};

use super::{
    guard::contain,
    status::{Alias, Failure, Health, SourceStatus, StatusBoard, Unrouted},
    task::TaskHandle,
};

/// The platform's WebSocket stack: tokio-tungstenite on desktop, the
/// browser's WebSocket on the web. NSH 0.5 gives JS its sockets but not
/// Rust components, so Lattice brings these two; both follow NSH's own
/// transport contract ([`WebSocketTransport`]), so an NSH-provided stack
/// can replace them without touching [`SharedSocket`].
pub fn platform_transport() -> Arc<dyn WebSocketTransport> {
    #[cfg(not(target_arch = "wasm32"))]
    let transport = native::NativeTransport;
    #[cfg(target_arch = "wasm32")]
    let transport = web::BrowserTransport;
    Arc::new(transport)
}

/// An API's streaming wire format: the only part of a shared socket that
/// differs between integrations.
pub trait SocketProtocol: 'static {
    /// What a consumer subscribes to, already normalized.
    type Topic: 'static;
    /// What one data frame becomes. Every subscriber of its topic gets the
    /// same value by reference.
    type Message: 'static;

    /// The topic's identity. Equal keys share one server subscription, so
    /// the key must include everything that changes what the server sends.
    fn key(&self, topic: &Self::Topic) -> String;
    /// The channel the topic is on, for frames that name a channel but no
    /// topic ([`Route::Channel`]).
    fn channel<'a>(&self, topic: &'a Self::Topic) -> &'a str;
    fn subscribe_frame(&self, topic: &Self::Topic) -> String;
    fn unsubscribe_frame(&self, topic: &Self::Topic) -> String;
    /// The application-level keepalive frame; `None` for an API without one
    /// (the socket then sends no pings and never misses one).
    fn ping_frame(&self) -> Option<String> {
        None
    }
    /// What one incoming frame means. `known` says whether a key is one the
    /// socket subscribed (or is unsubscribing): for a frame that names its
    /// topic only partly (a Ferris acknowledgement has no channel), the
    /// protocol may pick the one known key it can be, never a guess between
    /// several.
    fn decode(&self, frame: &WebSocketData, known: &dyn Fn(&str) -> bool)
    -> Inbound<Self::Message>;
    /// The part of a topic key that another name for the same data would
    /// share (Ferris: channel and exchange; the symbol and params are what
    /// an alias changes). When an acknowledged topic gets no data while
    /// frames for another key of its scope arrive, the socket reports it as
    /// a likely alias. `None` (the default): keys are never compared.
    fn alias_scope<'k>(&self, key: &'k str) -> Option<&'k str> {
        let _ = key;
        None
    }
}

/// One decoded frame.
pub enum Inbound<M> {
    /// Data for subscribers.
    Data { route: Route, message: M },
    /// The server confirmed the subscription with this key.
    Acknowledged(String),
    /// The server ended the subscription with this key (or confirmed an
    /// unsubscribe the socket sent).
    Dropped(String),
    /// The answer to a ping.
    Pong,
    /// A warning (`fatal: false`) or error from the server, about a topic
    /// or, with no key, the connection.
    Issue {
        key: Option<String>,
        fatal: bool,
        code: String,
        message: String,
    },
    /// A valid frame with nothing for subscribers.
    Ignored,
    /// Not a frame of this protocol. It does not count as an answer to a
    /// keepalive.
    Malformed,
}

/// Which subscribers a data frame is for.
pub enum Route {
    /// The topic with this key.
    Topic(String),
    /// The one topic on this channel. A frame that names only a channel is
    /// delivered when exactly one topic is on it, and dropped when the
    /// channel is ambiguous: a guess could show one market's data as
    /// another's.
    Channel(String),
}

/// Where one topic is on the server.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TopicPhase {
    /// Waiting for a socket: the first connection, or a reconnect.
    Connecting,
    /// Subscribe frame sent, no answer yet.
    Subscribing,
    /// The server confirmed it; no data yet (a quiet market is normal).
    Subscribed,
    /// Data is arriving.
    Live,
    /// The server refused or ended it.
    Failed,
}

/// How a shared socket behaves. [`SocketConfig::new`] gives the defaults.
#[derive(Clone, Debug)]
pub struct SocketConfig {
    /// The `ws:` / `wss:` URL.
    pub url: String,
    /// Handshake headers. Desktop only: browsers cannot send them, so a
    /// browser socket with headers fails to connect.
    pub headers: Vec<(String, String)>,
    /// The waits before reconnect attempts 1, 2, ...; the last repeats.
    pub reconnect_delays: Vec<Duration>,
    /// How long a handshake may take before it counts as a failure.
    pub connect_timeout: Duration,
    /// How often the ping goes out while the socket is open.
    pub keepalive_interval: Duration,
    /// How long after a ping some valid frame must arrive.
    pub keepalive_timeout: Duration,
    /// Unanswered keepalives in a row before the socket is replaced.
    pub max_missed_keepalives: u32,
    /// How long a topic outlives its last subscriber.
    pub release_grace: Duration,
}

impl SocketConfig {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            headers: Vec::new(),
            reconnect_delays: [500, 1_000, 2_000, 4_000, 8_000]
                .map(Duration::from_millis)
                .to_vec(),
            connect_timeout: Duration::from_secs(15),
            keepalive_interval: Duration::from_secs(20),
            keepalive_timeout: Duration::from_secs(10),
            max_missed_keepalives: 3,
            release_grace: Duration::from_secs(2),
        }
    }
}

/// Failed connections in a row after which the source reports `Failing`.
const FAILING_AFTER: u32 = 3;
/// How long a socket told to close may take to finish its handshake.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(5);
/// How many distinct unrouted targets a status lists.
const UNROUTED_TARGETS: usize = 8;
/// How many distinct unrouted keys (and suspected aliases) are logged over
/// the socket's life; past it, unrouted frames are only counted.
const LOGGED_LIMIT: usize = 256;

/// One API's shared WebSocket. Cloning is cheap and shares it.
pub struct SharedSocket<P: SocketProtocol>(Rc<Core<P>>);

impl<P: SocketProtocol> Clone for SharedSocket<P> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<P: SocketProtocol> SharedSocket<P> {
    /// A shared socket on the platform's stack that reports to `status` as
    /// source `id` (`ferris.socket`) named `label` (`Ferris realtime`).
    pub fn new(
        protocol: P,
        config: SocketConfig,
        status: StatusBoard,
        id: &'static str,
        label: &'static str,
    ) -> Self {
        Self::with_transport(protocol, config, platform_transport(), status, id, label)
    }

    /// [`Self::new`] on another WebSocket stack.
    pub fn with_transport(
        protocol: P,
        config: SocketConfig,
        transport: Arc<dyn WebSocketTransport>,
        status: StatusBoard,
        id: &'static str,
        label: &'static str,
    ) -> Self {
        let socket = Self(Rc::new_cyclic(|me| Core {
            protocol,
            config,
            transport,
            status,
            id,
            label,
            me: me.clone(),
            state: RefCell::new(State::new()),
        }));
        socket.0.publish();
        socket
    }

    /// Subscribes to `topic`; `on_message` gets each of its messages until
    /// the returned subscription drops.
    pub fn subscribe(
        &self,
        topic: P::Topic,
        on_message: impl Fn(&P::Message) + 'static,
    ) -> Subscription {
        self.0.add(topic, Rc::new(on_message), None)
    }

    /// [`Self::subscribe`], also telling `on_phase` the topic's phase now and
    /// each time it changes: what a consumer needs to fall back to polling
    /// while the stream is down, or to refetch a snapshot after a gap.
    pub fn subscribe_with_phase(
        &self,
        topic: P::Topic,
        on_message: impl Fn(&P::Message) + 'static,
        on_phase: impl Fn(TopicPhase) + 'static,
    ) -> Subscription {
        self.0
            .add(topic, Rc::new(on_message), Some(Rc::new(on_phase)))
    }

    /// Drops every topic and closes the socket. Subscriptions still held
    /// become inert.
    pub fn shutdown(&self) {
        self.0.shutdown();
    }
}

/// One subscriber's hold on a topic. Dropping it lets the topic go once no
/// other subscriber holds it.
#[must_use = "dropping a subscription unsubscribes it"]
pub struct Subscription {
    key: String,
    id: u64,
    target: Option<Weak<dyn Target>>,
}

impl Subscription {
    /// The topic's key.
    pub fn key(&self) -> &str {
        &self.key
    }

    /// The topic's phase now; `None` once the socket shut down.
    pub fn phase(&self) -> Option<TopicPhase> {
        self.target()?.phase(&self.key)
    }

    /// Asks the server for the topic afresh (unsubscribe, then subscribe), so
    /// a stream that sends a snapshot on subscribe sends a new one: how a
    /// consumer recovers after it detects a gap. Every subscriber of the
    /// topic sees the new snapshot. False once the socket shut down.
    pub fn resubscribe(&self) -> bool {
        self.target()
            .is_some_and(|target| target.resubscribe(&self.key))
    }

    /// Keeps the subscription until the current owner (a widget) is cleaned
    /// up, instead of until this handle drops.
    pub fn release_on_cleanup(self) {
        cleanup(move || drop(self));
    }

    fn target(&self) -> Option<Rc<dyn Target>> {
        self.target.as_ref()?.upgrade()
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        if let Some(target) = self.target.take().and_then(|target| target.upgrade()) {
            target.release(&self.key, self.id);
        }
    }
}

/// What a subscription asks of its socket, whatever the protocol.
trait Target {
    fn release(&self, key: &str, id: u64);
    fn resubscribe(&self, key: &str) -> bool;
    fn phase(&self, key: &str) -> Option<TopicPhase>;
}

struct Core<P: SocketProtocol> {
    protocol: P,
    config: SocketConfig,
    transport: Arc<dyn WebSocketTransport>,
    status: StatusBoard,
    id: &'static str,
    label: &'static str,
    me: Weak<Core<P>>,
    state: RefCell<State<P>>,
}

struct State<P: SocketProtocol> {
    topics: BTreeMap<String, Topic<P>>,
    next_subscriber: u64,
    /// The current connection's; events of any other are ignored.
    generation: u64,
    link: Link,
    /// Sockets told to close, kept until their close handshake finishes.
    closing: Vec<Closing>,
    /// Reconnect attempts since the last connection that delivered a frame.
    attempt: usize,
    /// Connections in a row that ended before delivering a frame.
    failures: u32,
    /// Whether the current connection has delivered a valid frame.
    proven: bool,
    missed_keepalives: u32,
    last_response: Option<Instant>,
    /// Unsubscribe frames on the current connection whose confirmation has
    /// not arrived, by key; a confirmation is not the server dropping a
    /// topic that was subscribed again since.
    unsubscribes: HashMap<String, Unsubscribing>,
    last_failure: Option<Failure>,
    sweeper: Option<TaskHandle>,
    /// The current connection's data frames that reached no wanted topic.
    unrouted: Unrouted,
    /// `unrouted.frames` as last published: the count is reported with the
    /// next status, never per frame.
    reported_frames: Cell<u64>,
    /// Unrouted targets and suspected aliases already logged, over the
    /// socket's life (at most [`LOGGED_LIMIT`]).
    logged: HashSet<String>,
}

/// Unsubscribe frames sent for one key whose confirmation has not arrived.
struct Unsubscribing {
    pending: u32,
    /// Its topic's channel: a frame naming only the channel may be for it.
    channel: String,
}

enum Link {
    Idle,
    Connecting {
        socket: Box<dyn WebSocketTask>,
        _timeout: TaskHandle,
    },
    Open {
        socket: Box<dyn WebSocketTask>,
        _keepalive: Option<TaskHandle>,
    },
    Retrying {
        _timer: TaskHandle,
    },
}

struct Closing {
    generation: u64,
    _socket: Box<dyn WebSocketTask>,
    _timeout: TaskHandle,
}

struct Topic<P: SocketProtocol> {
    /// Its key, as the map has it (for logs and callbacks).
    key: Rc<str>,
    topic: P::Topic,
    subscribers: Vec<Subscriber<P::Message>>,
    phase: TopicPhase,
    /// When it is unsubscribed, once its last subscriber has left.
    release_at: Option<Instant>,
    /// The server's latest warning or error about it, until data flows.
    issue: Option<Failure>,
    /// While acknowledged with no data: the key of unrouted frames on its
    /// alias scope, most likely what the server calls it.
    alias: Option<String>,
}

struct Subscriber<M> {
    id: u64,
    on_message: MessageCallback<M>,
    on_phase: Option<PhaseCallback>,
}

type MessageCallback<M> = Rc<dyn Fn(&M)>;
type PhaseCallback = Rc<dyn Fn(TopicPhase)>;

/// What a state change does once the state is released.
#[derive(Default)]
struct After {
    /// Phase callbacks to run, with their topic's key.
    phases: Vec<(Rc<str>, PhaseCallback, TopicPhase)>,
    status: bool,
}

/// Where one data frame goes.
enum Routing {
    /// To the wanted topic with this key.
    Deliver(String),
    /// Nowhere, as expected: its topic is in its release grace or being
    /// unsubscribed.
    Lingering,
    /// Nowhere, unexpectedly. `target` is the key it named, or its channel
    /// when it named none; `named` says which.
    Unrouted { target: String, named: bool },
}

impl<P: SocketProtocol> State<P> {
    fn new() -> Self {
        Self {
            topics: BTreeMap::new(),
            next_subscriber: 0,
            generation: 0,
            link: Link::Idle,
            closing: Vec::new(),
            attempt: 0,
            failures: 0,
            proven: false,
            missed_keepalives: 0,
            last_response: None,
            unsubscribes: HashMap::new(),
            last_failure: None,
            sweeper: None,
            unrouted: Unrouted::default(),
            reported_frames: Cell::new(0),
            logged: HashSet::new(),
        }
    }
}

/// Moves `topic` to `phase`, queueing its subscribers' phase callbacks.
fn set_phase<P: SocketProtocol>(topic: &mut Topic<P>, phase: TopicPhase, after: &mut After) {
    if topic.phase == phase {
        return;
    }
    topic.phase = phase;
    // A suspected alias is only evidence while the topic sits acknowledged.
    topic.alias = None;
    after.status = true;
    let key = &topic.key;
    after.phases.extend(
        topic
            .subscribers
            .iter()
            .filter_map(|subscriber| Some((key.clone(), subscriber.on_phase.clone()?, phase))),
    );
}

impl<P: SocketProtocol> Core<P> {
    fn add(
        &self,
        topic: P::Topic,
        on_message: MessageCallback<P::Message>,
        on_phase: Option<PhaseCallback>,
    ) -> Subscription {
        let key = self.protocol.key(&topic);
        let mut after = After {
            status: true,
            ..After::default()
        };
        let id = {
            let mut state = self.state.borrow_mut();
            let state = &mut *state;
            state.next_subscriber += 1;
            let id = state.next_subscriber;
            let subscriber = Subscriber {
                id,
                on_message,
                on_phase: on_phase.clone(),
            };
            let (topic_key, phase) = match state.topics.get_mut(&key) {
                Some(existing) => {
                    existing.release_at = None;
                    existing.subscribers.push(subscriber);
                    (existing.key.clone(), existing.phase)
                }
                None => {
                    let mut entry = Topic {
                        key: Rc::from(key.as_str()),
                        topic,
                        subscribers: vec![subscriber],
                        phase: TopicPhase::Connecting,
                        release_at: None,
                        issue: None,
                        alias: None,
                    };
                    if let Link::Open { socket, .. } = &mut state.link {
                        entry.phase = self.send_subscribe(socket.as_mut(), &entry.topic);
                    }
                    let (topic_key, phase) = (entry.key.clone(), entry.phase);
                    state.topics.insert(key.clone(), entry);
                    if matches!(state.link, Link::Idle) {
                        self.connect(state, &mut after);
                    }
                    (topic_key, phase)
                }
            };
            if let Some(on_phase) = on_phase {
                after.phases.push((topic_key, on_phase, phase));
            }
            id
        };
        self.finish(after);
        let target: Weak<dyn Target> = self.me.clone();
        Subscription {
            key,
            id,
            target: Some(target),
        }
    }

    fn send_subscribe(&self, socket: &mut dyn WebSocketTask, topic: &P::Topic) -> TopicPhase {
        let frame = WebSocketData::Text(self.protocol.subscribe_frame(topic));
        match socket.send(frame) {
            Ok(()) => TopicPhase::Subscribing,
            // The socket is going; its close event resubscribes on the next.
            Err(error) => {
                log::debug!("{}: subscribe not sent: {error}", self.label);
                TopicPhase::Connecting
            }
        }
    }

    /// Opens a new connection (a new generation).
    fn connect(&self, state: &mut State<P>, after: &mut After) {
        state.generation += 1;
        let generation = state.generation;
        state.proven = false;
        state.missed_keepalives = 0;
        state.unsubscribes.clear();
        let me = self.me.clone();
        let events: WebSocketEvents = Box::new(move |event| {
            if let Some(core) = me.upgrade() {
                core.on_event(generation, event);
            }
        });
        let request = WebSocketRequest {
            url: self.config.url.clone(),
            protocols: Vec::new(),
            headers: self.config.headers.clone(),
        };
        match self.transport.connect(request, events) {
            Ok(socket) => {
                let me = self.me.clone();
                let limit = self.config.connect_timeout;
                let timeout = TaskHandle::spawn(async move {
                    timer(limit).await;
                    if let Some(core) = me.upgrade() {
                        core.connect_timed_out(generation);
                    }
                });
                state.link = Link::Connecting {
                    socket,
                    _timeout: timeout,
                };
            }
            Err(message) => self.lost(state, Failure::now("SOCKET_CONNECT_FAILED", message), after),
        }
    }

    /// The connection is gone: closed, failed, timed out or abandoned. Every
    /// topic waits for the next one, which comes after the backoff.
    fn lost(&self, state: &mut State<P>, failure: Failure, after: &mut After) {
        log::debug!("{}: {}: {}", self.label, failure.code, failure.message);
        state.link = Link::Idle;
        state.failures += 1;
        state.proven = false;
        state.last_failure = Some(failure);
        after.status = true;
        // A topic past its last subscriber has nothing left to release.
        state.topics.retain(|_, topic| topic.release_at.is_none());
        for topic in state.topics.values_mut() {
            set_phase(topic, TopicPhase::Connecting, after);
        }
        if state.topics.is_empty() {
            state.attempt = 0;
            return;
        }
        let delays = &self.config.reconnect_delays;
        let delay = delays
            .get(state.attempt.min(delays.len().saturating_sub(1)))
            .copied()
            .unwrap_or(Duration::from_secs(1));
        state.attempt += 1;
        let (me, generation) = (self.me.clone(), state.generation);
        state.link = Link::Retrying {
            _timer: TaskHandle::spawn(async move {
                timer(delay).await;
                if let Some(core) = me.upgrade() {
                    core.retry(generation);
                }
            }),
        };
    }

    fn retry(&self, generation: u64) {
        let mut after = After::default();
        {
            let mut state = self.state.borrow_mut();
            if state.generation != generation || !matches!(state.link, Link::Retrying { .. }) {
                return;
            }
            if state.topics.is_empty() {
                state.link = Link::Idle;
            } else {
                self.connect(&mut state, &mut after);
            }
            after.status = true;
        }
        self.finish(after);
    }

    fn connect_timed_out(&self, generation: u64) {
        let mut after = After::default();
        {
            let mut state = self.state.borrow_mut();
            if state.generation != generation || !matches!(state.link, Link::Connecting { .. }) {
                return;
            }
            let message = format!(
                "no handshake within {}s",
                self.config.connect_timeout.as_secs()
            );
            self.lost(
                &mut state,
                Failure::now("SOCKET_CONNECT_TIMEOUT", message),
                &mut after,
            );
        }
        self.finish(after);
    }

    /// Closes the socket because nothing wants it. It finishes its close
    /// handshake in the background, as an earlier generation.
    fn disconnect(&self, state: &mut State<P>) {
        let generation = state.generation;
        state.generation += 1;
        if let Link::Open { mut socket, .. } | Link::Connecting { mut socket, .. } =
            std::mem::replace(&mut state.link, Link::Idle)
            && socket.close(1000, "no subscriptions").is_ok()
        {
            let me = self.me.clone();
            let timeout = TaskHandle::spawn(async move {
                timer(CLOSE_TIMEOUT).await;
                if let Some(core) = me.upgrade() {
                    core.closed(generation);
                }
            });
            state.closing.push(Closing {
                generation,
                _socket: socket,
                _timeout: timeout,
            });
        }
        state.attempt = 0;
        state.failures = 0;
        state.proven = false;
        state.missed_keepalives = 0;
        state.unsubscribes.clear();
        // Nothing is wanted any more, so there is nothing left to diagnose.
        state.unrouted = Unrouted::default();
    }

    /// A closing socket finished (or ran out of time).
    fn closed(&self, generation: u64) {
        let finished = {
            let mut state = self.state.borrow_mut();
            let (finished, kept) = std::mem::take(&mut state.closing)
                .into_iter()
                .partition(|closing| closing.generation == generation);
            state.closing = kept;
            finished
        };
        drop::<Vec<Closing>>(finished);
    }

    fn on_event(&self, generation: u64, event: WebSocketEvent) {
        if generation != self.state.borrow().generation {
            if let WebSocketEvent::Close { .. } = event {
                self.closed(generation);
            }
            return;
        }
        match event {
            WebSocketEvent::Open { .. } => self.opened(),
            WebSocketEvent::Message(frame) => self.received(&frame),
            // A close event follows a transport error.
            WebSocketEvent::Error(message) => {
                log::debug!("{}: socket error: {message}", self.label);
                self.state.borrow_mut().last_failure = Some(Failure::now("SOCKET_ERROR", message));
            }
            WebSocketEvent::Close { code, reason, .. } => self.on_closed(code, &reason),
        }
    }

    fn opened(&self) {
        let mut after = After {
            status: true,
            ..After::default()
        };
        {
            let mut state = self.state.borrow_mut();
            let state = &mut *state;
            let mut socket = match std::mem::replace(&mut state.link, Link::Idle) {
                Link::Connecting { socket, .. } => socket,
                other => {
                    state.link = other;
                    return;
                }
            };
            // The new socket holds no server subscriptions, so a topic in its
            // release grace has nothing to release.
            state.topics.retain(|_, topic| topic.release_at.is_none());
            for topic in state.topics.values_mut() {
                topic.issue = None;
                let phase = self.send_subscribe(socket.as_mut(), &topic.topic);
                set_phase(topic, phase, &mut after);
            }
            let keepalive = self
                .protocol
                .ping_frame()
                .map(|_| self.keepalive(state.generation));
            state.link = Link::Open {
                socket,
                _keepalive: keepalive,
            };
            state.last_response = Some(now());
            state.missed_keepalives = 0;
            // Each connection's frames are their own evidence.
            state.unrouted = Unrouted::default();
            if state.topics.is_empty() {
                self.disconnect(state);
            }
        }
        self.finish(after);
    }

    fn received(&self, frame: &WebSocketData) {
        let inbound = {
            let state = self.state.borrow();
            let known =
                |key: &str| state.topics.contains_key(key) || state.unsubscribes.contains_key(key);
            self.protocol.decode(frame, &known)
        };
        let mut after = After::default();
        let delivery = {
            let mut state = self.state.borrow_mut();
            let state = &mut *state;
            if !matches!(state.link, Link::Open { .. }) {
                return;
            }
            if !matches!(inbound, Inbound::Malformed) {
                state.last_response = Some(now());
                if !state.proven {
                    // A connection that delivers is a good one: backoff and
                    // failure count start over.
                    state.proven = true;
                    state.failures = 0;
                    state.attempt = 0;
                    after.status = true;
                }
                if state.missed_keepalives > 0 {
                    state.missed_keepalives = 0;
                    after.status = true;
                }
            }
            match inbound {
                Inbound::Data { route, message } => match self.route(state, route) {
                    Routing::Deliver(key) => state.topics.get_mut(&key).map(|topic| {
                        if topic.issue.take().is_some() {
                            after.status = true;
                        }
                        set_phase(topic, TopicPhase::Live, &mut after);
                        let callbacks: Vec<_> = topic
                            .subscribers
                            .iter()
                            .map(|subscriber| subscriber.on_message.clone())
                            .collect();
                        (topic.key.clone(), callbacks, message)
                    }),
                    Routing::Lingering => None,
                    Routing::Unrouted { target, named } => {
                        self.unrouted(state, target, named, &mut after);
                        None
                    }
                },
                Inbound::Acknowledged(key) => {
                    if let Some(topic) = state.topics.get_mut(&key)
                        && matches!(
                            topic.phase,
                            TopicPhase::Connecting | TopicPhase::Subscribing
                        )
                    {
                        set_phase(topic, TopicPhase::Subscribed, &mut after);
                    }
                    None
                }
                Inbound::Dropped(key) => {
                    if let Some(unsubscribing) = state.unsubscribes.get_mut(&key) {
                        unsubscribing.pending -= 1;
                        if unsubscribing.pending == 0 {
                            state.unsubscribes.remove(&key);
                        }
                    } else if let Some(topic) = state.topics.get_mut(&key)
                        && topic.release_at.is_none()
                    {
                        topic.issue = Some(Failure::now(
                            "UNSUBSCRIBED",
                            "the server ended this subscription",
                        ));
                        set_phase(topic, TopicPhase::Failed, &mut after);
                    }
                    None
                }
                Inbound::Issue {
                    key,
                    fatal,
                    code,
                    message,
                } => {
                    let failure = Failure::now(&code, &message);
                    after.status = true;
                    match key.and_then(|key| state.topics.get_mut(&key)) {
                        Some(topic) => {
                            topic.issue = Some(failure);
                            if fatal {
                                set_phase(topic, TopicPhase::Failed, &mut after);
                            }
                        }
                        None => {
                            log::warn!("{}: {code}: {message}", self.label);
                            state.last_failure = Some(failure);
                        }
                    }
                    None
                }
                Inbound::Pong | Inbound::Ignored | Inbound::Malformed => None,
            }
        };
        self.finish(after);
        if let Some((key, callbacks, message)) = delivery {
            // One subscriber's panic costs it this frame; the others still
            // get it, and the transport task that called us keeps running.
            for callback in callbacks {
                if let Err(panic) = contain(|| callback(&message)) {
                    log::error!(
                        "{}: a subscriber of `{key}` panicked on a message: {panic}",
                        self.label
                    );
                }
            }
        }
    }

    /// Where a data frame naming `route` goes. A frame that names only a
    /// channel goes to the one wanted topic on it; with none or several
    /// (a guess could show one market's data as another's) it goes nowhere.
    fn route(&self, state: &State<P>, route: Route) -> Routing {
        match route {
            Route::Topic(key) => match state.topics.get(&key) {
                Some(topic) if topic.release_at.is_none() => Routing::Deliver(key),
                Some(_) => Routing::Lingering,
                None if state.unsubscribes.contains_key(&key) => Routing::Lingering,
                None => Routing::Unrouted {
                    target: key,
                    named: true,
                },
            },
            Route::Channel(channel) => {
                let on_channel = |topic: &&Topic<P>| self.protocol.channel(&topic.topic) == channel;
                let mut wanted = state
                    .topics
                    .values()
                    .filter(|topic| topic.release_at.is_none())
                    .filter(on_channel);
                match (wanted.next(), wanted.next()) {
                    (Some(topic), None) => Routing::Deliver(topic.key.to_string()),
                    (Some(_), Some(_)) => Routing::Unrouted {
                        target: format!("{channel} (no topic; several wanted)"),
                        named: false,
                    },
                    (None, _) => {
                        let lingering = state
                            .topics
                            .values()
                            .filter(|topic| topic.release_at.is_some())
                            .any(|topic| on_channel(&topic))
                            || state
                                .unsubscribes
                                .values()
                                .any(|unsubscribing| unsubscribing.channel == channel);
                        if lingering {
                            Routing::Lingering
                        } else {
                            Routing::Unrouted {
                                target: format!("{channel} (no topic)"),
                                named: false,
                            }
                        }
                    }
                }
            }
        }
    }

    /// Counts a data frame for `target` that reached nobody. Cheap per
    /// frame: no status unless it names something new, and each distinct
    /// target logged once.
    fn unrouted(&self, state: &mut State<P>, target: String, named: bool, after: &mut After) {
        state.unrouted.frames += 1;
        if named {
            self.suspect_aliases(state, &target, after);
        }
        if state.unrouted.targets.contains(&target) {
            return;
        }
        if state.logged.len() < LOGGED_LIMIT && !state.logged.contains(&target) {
            log::warn!(
                "{}: dropping frames for `{target}`: no wanted topic has that key \
                 (logged once; counted in the status)",
                self.label
            );
            state.logged.insert(target.clone());
        }
        if state.unrouted.targets.len() < UNROUTED_TARGETS {
            state.unrouted.targets.push(target);
            after.status = true;
        }
    }

    /// Marks the acknowledged topics without data on `arriving`'s alias
    /// scope: frames for a key the socket never asked for keep coming
    /// where they should be, so the server most likely calls them that.
    fn suspect_aliases(&self, state: &mut State<P>, arriving: &str, after: &mut After) {
        let Some(scope) = self.protocol.alias_scope(arriving) else {
            return;
        };
        let State { topics, logged, .. } = state;
        for topic in topics.values_mut() {
            if topic.release_at.is_some()
                || topic.phase != TopicPhase::Subscribed
                || topic.alias.is_some()
                || self.protocol.alias_scope(&topic.key) != Some(scope)
            {
                continue;
            }
            let pair = format!("{} -> {arriving}", topic.key);
            if logged.len() < LOGGED_LIMIT && !logged.contains(&pair) {
                log::warn!(
                    "{}: `{}` is acknowledged but gets no data while frames arrive \
                     as `{arriving}`: likely an alias; repair the symbol where the \
                     market is selected",
                    self.label,
                    topic.key
                );
                logged.insert(pair);
            }
            topic.alias = Some(arriving.to_owned());
            after.status = true;
        }
    }

    fn on_closed(&self, code: u16, reason: &str) {
        let mut after = After::default();
        {
            let mut state = self.state.borrow_mut();
            if !matches!(state.link, Link::Open { .. } | Link::Connecting { .. }) {
                return;
            }
            let message = if reason.is_empty() {
                format!("closed with code {code}")
            } else {
                format!("closed with code {code}: {reason}")
            };
            self.lost(
                &mut state,
                Failure::now("SOCKET_CLOSED", message),
                &mut after,
            );
        }
        self.finish(after);
    }

    /// Pings every interval while connection `generation` is open; replaces
    /// it once too many pings in a row go unanswered.
    fn keepalive(&self, generation: u64) -> TaskHandle {
        let me = self.me.clone();
        let (interval, timeout) = (
            self.config.keepalive_interval,
            self.config.keepalive_timeout,
        );
        TaskHandle::spawn(async move {
            loop {
                timer(interval).await;
                let Some(sent) = me.upgrade().and_then(|core| core.ping(generation)) else {
                    return;
                };
                timer(timeout).await;
                if !me
                    .upgrade()
                    .is_some_and(|core| core.check_keepalive(generation, sent))
                {
                    return;
                }
            }
        })
    }

    /// Sends the ping; when it went out.
    fn ping(&self, generation: u64) -> Option<Instant> {
        let mut state = self.state.borrow_mut();
        if state.generation != generation {
            return None;
        }
        let Link::Open { socket, .. } = &mut state.link else {
            return None;
        };
        socket
            .send(WebSocketData::Text(self.protocol.ping_frame()?))
            .ok()?;
        Some(now())
    }

    /// Whether the connection answered since the ping `sent`; false once the
    /// keepalive should stop.
    fn check_keepalive(&self, generation: u64, sent: Instant) -> bool {
        let mut after = After::default();
        let keep_going = {
            let mut state = self.state.borrow_mut();
            let state = &mut *state;
            if state.generation != generation || !matches!(state.link, Link::Open { .. }) {
                return false;
            }
            // Unrouted frames are counted silently; this is where a growing
            // count reaches the status at the latest.
            if state.unrouted.frames != state.reported_frames.get() {
                after.status = true;
            }
            if state.last_response.is_some_and(|at| at >= sent) {
                true
            } else {
                state.missed_keepalives += 1;
                after.status = true;
                let missed = state.missed_keepalives;
                if missed < self.config.max_missed_keepalives {
                    state.last_failure =
                        Some(Failure::now("KEEPALIVE_TIMEOUT", "no reply to a keepalive"));
                    true
                } else {
                    // A socket that stopped answering is replaced, not watched.
                    let message = format!("no reply to {missed} keepalives; reconnecting");
                    self.lost(
                        state,
                        Failure::now("KEEPALIVE_TIMEOUT", message),
                        &mut after,
                    );
                    false
                }
            }
        };
        self.finish(after);
        keep_going
    }

    /// Starts the task that unsubscribes topics whose grace has run out.
    fn ensure_sweeper(&self, state: &mut State<P>) {
        if state.sweeper.is_some() {
            return;
        }
        let me = self.me.clone();
        state.sweeper = Some(TaskHandle::spawn(async move {
            loop {
                let Some(at) = me.upgrade().and_then(|core| core.next_release()) else {
                    return;
                };
                timer_until(at).await;
                let Some(core) = me.upgrade() else {
                    return;
                };
                core.sweep();
            }
        }));
    }

    /// The soonest release; with none, the sweeper retires.
    fn next_release(&self) -> Option<Instant> {
        let mut state = self.state.borrow_mut();
        let next = state
            .topics
            .values()
            .filter_map(|topic| topic.release_at)
            .min();
        if next.is_none() {
            state.sweeper = None;
        }
        next
    }

    fn sweep(&self) {
        let released = {
            let mut state = self.state.borrow_mut();
            let state = &mut *state;
            let now = now();
            let due: Vec<String> = state
                .topics
                .iter()
                .filter(|(_, topic)| topic.release_at.is_some_and(|at| at <= now))
                .map(|(key, _)| key.clone())
                .collect();
            let mut released = Vec::with_capacity(due.len());
            for key in due {
                let Some(topic) = state.topics.remove(&key) else {
                    continue;
                };
                if let Link::Open { socket, .. } = &mut state.link {
                    let frame = WebSocketData::Text(self.protocol.unsubscribe_frame(&topic.topic));
                    if socket.send(frame).is_ok() {
                        self.unsubscribing(&mut state.unsubscribes, key, &topic.topic);
                    }
                }
                released.push(topic);
            }
            if state.topics.is_empty() {
                self.disconnect(state);
            }
            released
        };
        if !released.is_empty() {
            drop(released);
            self.publish();
        }
    }

    /// Notes an unsubscribe frame sent for `topic` (key `key`).
    fn unsubscribing(
        &self,
        unsubscribes: &mut HashMap<String, Unsubscribing>,
        key: String,
        topic: &P::Topic,
    ) {
        unsubscribes
            .entry(key)
            .or_insert_with(|| Unsubscribing {
                pending: 0,
                channel: self.protocol.channel(topic).to_owned(),
            })
            .pending += 1;
    }

    fn shutdown(&self) {
        let topics = {
            let mut state = self.state.borrow_mut();
            let topics = std::mem::take(&mut state.topics);
            self.disconnect(&mut state);
            state.sweeper = None;
            topics
        };
        // Outside the borrow: a subscriber's closure may own subscriptions.
        drop(topics);
        self.publish();
    }

    /// Runs the queued phase callbacks, then reports the status if it moved.
    /// A callback that panics loses that one call.
    fn finish(&self, after: After) {
        for (key, callback, phase) in after.phases {
            if let Err(panic) = contain(|| callback(phase)) {
                log::error!(
                    "{}: a subscriber of `{key}` panicked on phase {phase:?}: {panic}",
                    self.label
                );
            }
        }
        if after.status {
            self.publish();
        }
    }

    fn publish(&self) {
        let status = {
            let state = self.state.borrow();
            state.reported_frames.set(state.unrouted.frames);
            self.describe(&state)
        };
        self.status.publish(status);
    }

    fn describe(&self, state: &State<P>) -> SourceStatus {
        let wanted: Vec<&Topic<P>> = state
            .topics
            .values()
            .filter(|topic| topic.release_at.is_none())
            .collect();
        let demand = wanted.len();
        let open = matches!(state.link, Link::Open { .. });
        let aliases: Vec<Alias> = wanted
            .iter()
            .filter_map(|topic| {
                Some(Alias {
                    wanted: topic.key.to_string(),
                    arriving: topic.alias.clone()?,
                })
            })
            .collect();
        let health = if demand == 0 {
            Health::Idle
        } else if state.failures >= FAILING_AFTER
            || wanted.iter().any(|topic| topic.phase == TopicPhase::Failed)
        {
            Health::Failing
        } else if state.failures > 0
            || state.missed_keepalives > 0
            || wanted.iter().any(|topic| topic.issue.is_some())
            // Subscribed, acknowledged, no data, and evidence of why.
            || !aliases.is_empty()
        {
            Health::Degraded
        } else if !open
            || wanted.iter().any(|topic| {
                matches!(
                    topic.phase,
                    TopicPhase::Connecting | TopicPhase::Subscribing
                )
            })
        {
            Health::Pending
        } else {
            Health::Healthy
        };
        let plural = |count: usize, one: &str, many: &str| {
            format!("{count} {}", if count == 1 { one } else { many })
        };
        let summary = match &state.link {
            Link::Idle => "Not connected".to_owned(),
            Link::Connecting { .. } => "Connecting".to_owned(),
            Link::Open { .. } => {
                let mut summary = format!("Connected · {}", plural(demand, "topic", "topics"));
                if !aliases.is_empty() {
                    let aliases = plural(aliases.len(), "possible alias", "possible aliases");
                    summary.push_str(&format!(" · {aliases}"));
                } else if state.unrouted.frames > 0 {
                    summary.push_str(&format!(" · {} unrouted", state.unrouted.frames));
                }
                summary
            }
            Link::Retrying { .. } => format!("Reconnecting (attempt {})", state.attempt),
        };
        SourceStatus {
            id: self.id,
            label: self.label,
            health,
            demand,
            summary,
            last_failure: state.last_failure.clone(),
            unrouted: Unrouted {
                aliases,
                ..state.unrouted.clone()
            },
        }
    }
}

impl<P: SocketProtocol> Target for Core<P> {
    fn release(&self, key: &str, id: u64) {
        let removed = {
            let mut state = self.state.borrow_mut();
            let Some(topic) = state.topics.get_mut(key) else {
                return;
            };
            let Some(index) = topic
                .subscribers
                .iter()
                .position(|subscriber| subscriber.id == id)
            else {
                return;
            };
            let removed = topic.subscribers.remove(index);
            if topic.subscribers.is_empty() {
                topic.release_at = Some(now() + self.config.release_grace);
                self.ensure_sweeper(&mut state);
            }
            removed
        };
        // Outside the borrow: its closure may own another subscription.
        drop(removed);
        self.publish();
    }

    fn resubscribe(&self, key: &str) -> bool {
        let mut after = After::default();
        {
            let mut state = self.state.borrow_mut();
            let state = &mut *state;
            let Some(topic) = state.topics.get_mut(key) else {
                return false;
            };
            // Not connected: the next socket subscribes afresh anyway.
            if let Link::Open { socket, .. } = &mut state.link {
                let frame = WebSocketData::Text(self.protocol.unsubscribe_frame(&topic.topic));
                if socket.send(frame).is_ok() {
                    self.unsubscribing(&mut state.unsubscribes, key.to_owned(), &topic.topic);
                }
                let phase = self.send_subscribe(socket.as_mut(), &topic.topic);
                set_phase(topic, phase, &mut after);
            }
        }
        self.finish(after);
        true
    }

    fn phase(&self, key: &str) -> Option<TopicPhase> {
        self.state.borrow().topics.get(key).map(|topic| topic.phase)
    }
}
