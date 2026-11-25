//! Lighter's own API, asked by the app itself.
//!
//! As with Hyperliquid ([`super::hyperliquid`]), what one address holds on
//! Lighter is asked of Lighter, from the user's machine, so the requests
//! count against the user's own rate limit there and not against one
//! backend's for everybody. What the portfolio reads is public: Lighter
//! answers those requests by an address or an account's index alone.
//!
//! Trading is the other half ([`trading`], which is all the app's trading
//! knows of Lighter: [`TradingVenue`](super::trading::TradingVenue)).
//! Lighter's accounts do not sign as Ethereum's do: an account registers
//! API keys of Lighter's own kind, and those sign its transactions
//! ([`crypto`], the signature scheme, in Rust: Lighter publishes it in Go
//! alone). The connected wallet registers one key of the app's, once
//! ([`key`]); the app keeps it (in memory, and on this device between
//! runs) and signs every order, cancel and leverage change with it
//! ([`tx`]), which are posted ([`requests`]). The key can trade, and
//! cannot send the account's funds anywhere but to the wallet that owns it
//! (Lighter pays a withdrawal a key asks for only there). It also signs the
//! auth tokens that read what Lighter tells only an account itself: its
//! open orders ([`orders`]).
//!
//! An account is followed live ([`live`]) over Lighter's WebSocket
//! ([`stream`], one shared socket, `lighter.socket`): its trades arrive as
//! they happen, a change of its positions has its accounts asked for again
//! at once, and, once trading is enabled, its orders arrive as they change.
//! What a market is priced at (its mark, and the middle of its best bid
//! and offer) is followed over the same socket ([`pricing`]), for
//! whatever values positions between two reports of their account.
//!
//! - Requests are `GET`s under `/api/v1` ([`Request`], in [`requests`]),
//!   and one `POST` of a signed transaction.
//! - The rate limit: Lighter allows an IP address that sends no token
//!   [`REQUEST_LIMIT`] requests in any minute, whatever they ask. Every
//!   request spends one from a [`RateBudget`] first and waits while the
//!   minute is spent ([`LighterConfig::requests_per_minute`]); a `429` all
//!   the same spends what is left, so nothing more goes out for a minute.
//! - Requests are shared and polled through one [`Queries`] hub, which
//!   reports to the status board as `lighter.queries`.
//! - An account is an L1 (Ethereum) address, which holds several accounts
//!   on Lighter, each known by its index: its main one and its
//!   sub-accounts. A report of the address is of all of them together
//!   ([`accounts`]).
//! - What it reports of an address: its margin trading and the assets it
//!   holds ([`state`], both from the one request that lists its accounts),
//!   its history ([`performance`], from Lighter's profit and loss chart)
//!   and its latest trades ([`fills`]); and, for every address, what the
//!   assets are worth ([`prices`]). [`accounts`] gives them to the app in
//!   the portfolio's shapes
//!   ([`AccountSource`](super::portfolio::AccountSource)), which is all the
//!   widgets know of Lighter.
//! - What it does not: what an address pays to trade, has staked and holds
//!   of pools. Lighter answers those with an auth token only, or not in
//!   dollars.
//! - What trading reads besides: what is traded and how finely
//!   ([`markets`]), an account's open orders ([`orders`]), and how it is
//!   set to trade a market (its entry for it among its accounts' positions,
//!   [`state`]).
//!
//! Adding a report: its request in [`Request`], a module for its reply's
//! shape, and its query in [`accounts`], made of every account of the
//! address and combined as the portfolio combines accounts.

mod accounts;
mod crypto;
mod fills;
#[cfg(test)]
mod flow;
mod key;
mod live;
mod markets;
mod orders;
mod performance;
mod prices;
mod pricing;
mod requests;
mod state;
mod stream;
mod trading;
mod tx;

use std::{cell::RefCell, collections::HashMap, fmt, rc::Rc, sync::Arc, time::Duration};

use nsh_core::websocket::WebSocketTransport;
pub use requests::{Request, Resolution};
use serde::Deserialize;
pub use tx::Network;

use super::net::{
    ApiError, FeedHubs, HttpClient, HttpOptions, Queries, RateBudget, SharedSocket, SocketConfig,
    SourceDetails, StatusBoard, WeakQueries, platform_transport,
};

/// The production API (Lighter's mainnet).
pub const PRODUCTION_HTTP: &str = "https://mainnet.zklighter.elliot.ai";
/// The requests Lighter allows one IP address a minute while it sends no
/// auth token (a standard account's limit, which is not weighted).
pub const REQUEST_LIMIT: u32 = 60;
/// The window Lighter counts requests in.
const REQUEST_WINDOW: Duration = Duration::from_secs(60);

/// Where Lighter is, how long to wait for it and how much to ask of it.
#[derive(Clone)]
pub struct LighterConfig {
    /// The HTTP origin.
    pub http_base: String,
    /// The WebSocket's URL; `None` is the HTTP origin's own (`/stream`
    /// there, as Lighter's are), so another API is never streamed from
    /// production's.
    pub ws_url: Option<String>,
    /// The deadline for one request (headers and body).
    pub request_timeout: Duration,
    /// The requests the app makes a minute at most. Half of
    /// [`REQUEST_LIMIT`] by default: the limit is the address's, which
    /// Lighter's own site and anything else the user runs spend from too.
    pub requests_per_minute: u32,
    /// A stand-in for the platform's WebSocket stack (a test's).
    pub(crate) transport: Option<Arc<dyn WebSocketTransport>>,
}

impl fmt::Debug for LighterConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let transport = if self.transport.is_some() {
            "a stand-in"
        } else {
            "the platform's"
        };
        formatter
            .debug_struct("LighterConfig")
            .field("http_base", &self.http_base)
            .field("ws_url", &self.ws_url)
            .field("request_timeout", &self.request_timeout)
            .field("requests_per_minute", &self.requests_per_minute)
            .field("transport", &transport)
            .finish()
    }
}

impl LighterConfig {
    pub fn production() -> Self {
        Self {
            http_base: PRODUCTION_HTTP.to_owned(),
            ws_url: None,
            request_timeout: Duration::from_secs(15),
            requests_per_minute: REQUEST_LIMIT / 2,
            transport: None,
        }
    }

    /// Production, unless the build set `LATTICE_LIGHTER_HTTP` (the
    /// testnet's API, a local stand-in) or `LATTICE_LIGHTER_WS` (a
    /// WebSocket that is not at the HTTP origin's `/stream`). Read at
    /// compile time, so it works in the web build too.
    pub fn from_build_env() -> Self {
        let mut config = Self::production();
        if let Some(http) = option_env!("LATTICE_LIGHTER_HTTP") {
            http.clone_into(&mut config.http_base);
        }
        if let Some(ws) = option_env!("LATTICE_LIGHTER_WS") {
            config.ws_url = Some(ws.to_owned());
        }
        config
    }

    /// The WebSocket's URL: the one set, else the HTTP origin's `/stream`
    /// (`https://mainnet.zklighter.elliot.ai` is streamed at
    /// `wss://mainnet.zklighter.elliot.ai/stream`), opened as a reading
    /// socket (`readonly=true`): only subscriptions go over it, and Lighter
    /// serves a reading socket where it serves no other.
    fn socket_url(&self) -> Result<String, ApiError> {
        let invalid =
            |why: &str| ApiError::invalid_request(format!("invalid Lighter WebSocket URL: {why}"));
        let ws = match &self.ws_url {
            Some(ws) => ws.clone(),
            None => {
                let origin = self.http_base.trim_end_matches('/');
                let rest = origin
                    .strip_prefix("http")
                    .ok_or_else(|| invalid("the API is not at an http(s) origin"))?;
                format!("ws{rest}/stream")
            }
        };
        let mut url = url::Url::parse(&ws).map_err(|error| invalid(&error.to_string()))?;
        if !matches!(url.scheme(), "ws" | "wss") {
            return Err(invalid("it must be ws: or wss:"));
        }
        if !url.query_pairs().any(|(name, _)| name == "readonly") {
            url.query_pairs_mut().append_pair("readonly", "true");
        }
        Ok(url.into())
    }
}

impl Default for LighterConfig {
    fn default() -> Self {
        Self::from_build_env()
    }
}

/// The Lighter client. Cloning is cheap: every clone shares one HTTP
/// client, one budget, one query hub and one socket.
///
/// Each service module owns its part of it (`impl Lighter` in
/// `accounts.rs`), so a new report adds no field here.
#[derive(Clone)]
pub struct Lighter {
    api: Api,
    queries: Queries,
    network: Network,
    /// The API key that trades for the connected account, once it
    /// registered one ([`key`]).
    keys: Rc<key::Keys>,
    /// The WebSocket, which opens when an account or a price is followed
    /// live.
    socket: SharedSocket<stream::Protocol>,
    /// The accounts followed live ([`live`]) and the prices ([`pricing`]).
    feeds: FeedHubs,
    /// What is kept between a request and what reads it later.
    kept: Rc<Kept>,
}

/// What sends the requests: the HTTP client, and the budget each spends
/// from. Kept apart from the query hub so that a query's fetch holds this
/// and not the hub that holds the query.
#[derive(Clone)]
struct Api {
    http: HttpClient,
    budget: RateBudget,
    /// What the replies say of Lighter's markets in passing.
    directory: Rc<markets::Directory>,
}

/// The client as a request's own fetch holds it: with the query hub held
/// weakly, since the hub holds the fetch.
#[derive(Clone)]
struct WeakLighter {
    queries: WeakQueries,
    /// The client's other parts, none of which holds the hub.
    api: Api,
    network: Network,
    keys: Rc<key::Keys>,
    socket: SharedSocket<stream::Protocol>,
    feeds: FeedHubs,
    kept: Rc<Kept>,
}

impl WeakLighter {
    /// The client, while its hub is there to ask.
    fn upgrade(&self) -> Result<Lighter, ApiError> {
        let queries = self
            .queries
            .upgrade()
            .ok_or_else(|| ApiError::network("the data services shut down"))?;
        Ok(Lighter {
            api: self.api.clone(),
            queries,
            network: self.network,
            keys: self.keys.clone(),
            socket: self.socket.clone(),
            feeds: self.feeds.clone(),
            kept: self.kept.clone(),
        })
    }
}

/// What the client keeps of its own between a request and what reads it
/// later.
#[derive(Default)]
struct Kept {
    /// Each address's open orders as Lighter last said them, which the
    /// stream's changes are applied to ([`orders`]).
    orders: RefCell<HashMap<String, orders::Active>>,
    /// The asking again that waits a moment, by address ([`accounts`]).
    refreshes: RefCell<HashMap<String, accounts::Refreshing>>,
}

impl Lighter {
    /// A client for `config` that reports to `status` as `lighter.queries`
    /// and `lighter.socket`. Nothing is asked until something watches an
    /// account, and nothing connects until one is followed live.
    pub fn new(config: &LighterConfig, status: StatusBoard) -> Result<Self, ApiError> {
        let http = HttpClient::new(
            &config.http_base,
            HttpOptions {
                timeout: config.request_timeout,
                decode_error,
                ..HttpOptions::default()
            },
        )?;
        let keys = Rc::<key::Keys>::default();
        let socket = SharedSocket::with_transport(
            stream::Protocol { keys: keys.clone() },
            SocketConfig::new(config.socket_url()?),
            config.transport.clone().unwrap_or_else(platform_transport),
            status,
            "lighter.socket",
            "Lighter realtime",
        );
        Ok(Self {
            api: Api {
                http,
                budget: RateBudget::new(config.requests_per_minute, REQUEST_WINDOW),
                directory: Rc::default(),
            },
            queries: Queries::new(status, "lighter.queries", "Lighter requests"),
            network: Network::of(&config.http_base),
            keys,
            socket,
            feeds: FeedHubs::default(),
            kept: Rc::default(),
        })
    }

    /// Which Lighter the API is: what a transaction signed for it names.
    pub fn network(&self) -> Network {
        self.network
    }

    /// The client as a request's own fetch holds it.
    fn downgrade(&self) -> WeakLighter {
        WeakLighter {
            queries: self.queries.downgrade(),
            api: self.api.clone(),
            network: self.network,
            keys: self.keys.clone(),
            socket: self.socket.clone(),
            feeds: self.feeds.clone(),
            kept: self.kept.clone(),
        }
    }

    /// The shared query hub, which every request made here is watched in.
    pub fn queries(&self) -> &Queries {
        &self.queries
    }

    /// The budget every request spends from.
    pub fn budget(&self) -> &RateBudget {
        &self.api.budget
    }

    /// Its sources in full, as they stand now: the socket with every
    /// topic followed, and the query hub with every watched request, each
    /// with who wants it.
    pub fn sources(&self) -> Vec<SourceDetails> {
        vec![self.socket.details(), self.queries.details()]
    }

    /// Closes the socket and drops every query and the API key held (the
    /// app is going away). What is kept of the key for the next run stays.
    pub(crate) fn shutdown(&self) {
        self.keys.forget();
        self.kept.refreshes.borrow_mut().clear();
        self.kept.orders.borrow_mut().clear();
        self.socket.shutdown();
        self.queries.shutdown();
    }
}

/// Reads Lighter's error replies: a JSON object of its own `code` (`21100`)
/// and a `message` saying what is wrong (`account not found`), under an
/// error status. Anything else (a proxy's page, a reply that is no error)
/// is left to say itself.
fn decode_error(status: u16, body: &[u8]) -> Option<(String, String)> {
    #[derive(Deserialize)]
    struct Refusal {
        code: i64,
        message: String,
    }
    if status < 400 {
        return None;
    }
    let refusal: Refusal = serde_json::from_slice(body).ok()?;
    let message = refusal.message.trim();
    (!message.is_empty()).then(|| (refusal.code.to_string(), message.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_replies_read_as_their_code_and_message() {
        assert_eq!(
            decode_error(400, br#"{"code":21100,"message":"account not found"}"#),
            Some(("21100".to_owned(), "account not found".to_owned()))
        );
        // Nothing to read, and a reply that is no error.
        for body in [&b""[..], b"null", b"<html>Bad gateway</html>", b"{}"] {
            assert_eq!(decode_error(502, body), None);
        }
        assert_eq!(decode_error(200, br#"{"code":200,"message":"fine"}"#), None);
    }

    #[test]
    fn the_socket_is_a_reading_one_at_the_apis_own_origin_unless_told_otherwise() {
        let at = |http: &str| LighterConfig {
            http_base: http.to_owned(),
            ..LighterConfig::production()
        };
        assert_eq!(
            LighterConfig::production().socket_url().as_deref(),
            Ok("wss://mainnet.zklighter.elliot.ai/stream?readonly=true")
        );
        // Another API is never streamed from production's.
        assert_eq!(
            at("http://127.0.0.1:3001/").socket_url().as_deref(),
            Ok("ws://127.0.0.1:3001/stream?readonly=true")
        );
        let elsewhere = |ws: &str| LighterConfig {
            ws_url: Some(ws.to_owned()),
            ..at("https://lighter.test")
        };
        assert_eq!(
            elsewhere("wss://stream.example/lighter")
                .socket_url()
                .as_deref(),
            Ok("wss://stream.example/lighter?readonly=true")
        );
        // One that says what kind it is stays as it says.
        assert_eq!(
            elsewhere("wss://stream.example/s?readonly=false")
                .socket_url()
                .as_deref(),
            Ok("wss://stream.example/s?readonly=false")
        );
        for invalid in [
            elsewhere("https://stream.example"),
            at("ftp://lighter.test"),
        ] {
            assert!(invalid.socket_url().is_err(), "{invalid:?}");
        }
    }
}
