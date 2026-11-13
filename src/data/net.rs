//! Networking that every integration shares and none owns.
//!
//! An integration (Ferris today; an exchange or news API later) is built
//! from four parts, each usable on its own:
//!
//! - [`HttpClient`]: one API origin, JSON in and out, a deadline on every
//!   request and one [`ApiError`] model. It runs on NSH's `fetch`, so it is
//!   the platform's network stack on desktop and the browser's on the web.
//! - [`SharedSocket`]: one physical WebSocket for the API, multiplexed by
//!   topic and reference counted. The first subscriber to a topic subscribes
//!   on the server, later ones share it, the last one leaving unsubscribes
//!   (after a short grace, so a re-render does not churn), and the socket
//!   closes when no topic is left. It reconnects with capped backoff,
//!   resubscribes everything on reconnect, and replaces a socket that stops
//!   answering its keepalive. The API's wire format is a [`SocketProtocol`].
//! - [`Queries`]: shared, keyed requests. However many consumers watch a
//!   key, one request is in flight; a watched key is refetched on its
//!   interval and the value is kept for the next consumer.
//! - [`StatusBoard`]: each socket and query hub reports its health there,
//!   which is what the connection indicator shows.
//!
//! Everything runs on the engine thread. A long-lived task (a socket's
//! driver, a poll loop) is owned by its service, never by whichever widget
//! happened to ask first, so a widget closing cannot end a stream another
//! widget still uses. Consumer code the plumbing calls (callbacks, fetches,
//! watcher signals) runs contained: its panic costs that consumer that one
//! call, not the shared socket or poll loop (desktop builds; on the web a
//! panic aborts the module, see `guard.rs`).

mod error;
mod guard;
mod http;
mod json;
mod query;
mod socket;
mod status;
mod task;

pub use error::{ApiError, ErrorKind};
pub use http::{ErrorDecoder, HttpClient, HttpOptions, Request};
pub use json::canonical_json;
pub use query::{Queries, QuerySpec, QueryState, Watch};
pub use socket::{
    Inbound, Route, SharedSocket, SocketConfig, SocketProtocol, Subscription, TopicPhase,
    platform_transport,
};
pub use status::{Alias, Failure, Health, SourceStatus, StatusBoard, Unrouted};
