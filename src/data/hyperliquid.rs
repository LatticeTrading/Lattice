//! Hyperliquid's own API, asked by the app itself.
//!
//! Ferris carries every exchange's public market data; what one address
//! holds on Hyperliquid is asked of Hyperliquid, from the user's machine,
//! so the requests count against the user's own rate limit there (it is
//! per IP address) and not against one backend's for everybody. Everything
//! read is public: an address is all Hyperliquid reports an account by, and
//! reading needs no key or signature.
//!
//! Trading is the other half ([`trading`], which is all the app's trading
//! knows of Hyperliquid: [`TradingVenue`](super::trading::TradingVenue)).
//! The connected wallet approves an agent once ([`signing`], [`agent`]),
//! whose key the app keeps (in memory, and on this device between runs)
//! and signs every order and cancel with; they are posted to the second
//! endpoint ([`exchange`]). The key can trade and cannot withdraw.
//!
//! An account that trades is followed live ([`live`]) over Hyperliquid's
//! WebSocket ([`stream`], one shared socket, `hyperliquid.socket`): its
//! open orders, its perpetuals account and its fills arrive as they change
//! and are given to the requests that read them, so every widget watching
//! the account shows them at once and the polls are only what catches up
//! after a gap. What an asset is priced at (its mark, or the faster
//! middle of its best bid and offer) is followed over the same socket
//! ([`pricing`]), for whatever values positions between two reports of
//! their account.
//!
//! - Reading is one endpoint: a JSON `POST /info` whose body's `type`
//!   names what is asked ([`Info`], in [`info`], with what each request
//!   weighs). Acting is the other: `POST /exchange` with a signed action.
//! - The rate limit: Hyperliquid allows an address [`WEIGHT_LIMIT`] of
//!   weight a minute over REST. Every request spends its weight from one
//!   [`RateBudget`] first and waits while the minute is spent
//!   ([`HyperliquidConfig::weight_per_minute`]); a `429` all the same
//!   spends what is left, so nothing more goes out for a minute.
//! - Requests are shared and polled through one [`Queries`] hub, which
//!   reports to the status board as `hyperliquid.queries`.
//! - What it reports of an account, each from one request: its perpetuals
//!   ([`perps`]), its token balances ([`spot`]), its history
//!   ([`performance`]), what it has staked and in vaults ([`staking`]) and
//!   what it pays to trade ([`fees`]), its latest trades ([`fills`]); and,
//!   for every account, what the
//!   tokens are worth ([`prices`]). [`accounts`] gives them to the app in the
//!   portfolio's shapes ([`AccountSource`](super::portfolio::AccountSource)),
//!   which is all the widgets know of Hyperliquid.
//! - What trading reads besides: what is traded and at what price
//!   ([`meta`]), an account's open orders ([`orders`]), and how it is set
//!   to trade an asset ([`leverage`]).
//!
//! Adding a request: its variant and weight in [`Info`], a module for its
//! reply's shape, and a query made with `Hyperliquid::info_query` (which
//! spends the weight before each fetch).

mod accounts;
mod agent;
mod exchange;
mod fees;
mod fills;
mod info;
mod leverage;
mod live;
mod meta;
mod orders;
mod pack;
mod performance;
mod perps;
mod prices;
mod pricing;
pub mod signing;
mod spot;
mod staking;
mod stream;
mod trading;

use std::{fmt, rc::Rc, sync::Arc, time::Duration};

use nsh_core::websocket::WebSocketTransport;

pub use info::Info;
pub use signing::Network;

use super::net::{
    ApiError, FeedHubs, HttpClient, HttpOptions, Queries, RateBudget, SharedSocket, SocketConfig,
    SourceDetails, StatusBoard, platform_transport,
};

/// The production API.
pub const PRODUCTION_HTTP: &str = "https://api.hyperliquid.xyz";
/// The weight Hyperliquid allows one IP address a minute over REST.
pub const WEIGHT_LIMIT: u32 = 1200;
/// The window Hyperliquid counts weight in.
const WEIGHT_WINDOW: Duration = Duration::from_secs(60);

/// Where Hyperliquid is, how long to wait for it and how much to ask of it.
#[derive(Clone)]
pub struct HyperliquidConfig {
    /// The HTTP origin.
    pub http_base: String,
    /// The WebSocket's URL; `None` is the HTTP origin's own (`/ws` there,
    /// as Hyperliquid's are), so another API is never streamed from
    /// production's.
    pub ws_url: Option<String>,
    /// The deadline for one request (headers and body).
    pub request_timeout: Duration,
    /// The weight the app spends a minute at most. Half of
    /// [`WEIGHT_LIMIT`] by default: the limit is the address's, which
    /// Hyperliquid's own site and anything else the user runs spend from
    /// too.
    pub weight_per_minute: u32,
    /// A stand-in for the platform's WebSocket stack (a test's).
    pub(crate) transport: Option<Arc<dyn WebSocketTransport>>,
}

impl fmt::Debug for HyperliquidConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let transport = if self.transport.is_some() {
            "a stand-in"
        } else {
            "the platform's"
        };
        formatter
            .debug_struct("HyperliquidConfig")
            .field("http_base", &self.http_base)
            .field("ws_url", &self.ws_url)
            .field("request_timeout", &self.request_timeout)
            .field("weight_per_minute", &self.weight_per_minute)
            .field("transport", &transport)
            .finish()
    }
}

impl HyperliquidConfig {
    pub fn production() -> Self {
        Self {
            http_base: PRODUCTION_HTTP.to_owned(),
            ws_url: None,
            request_timeout: Duration::from_secs(15),
            weight_per_minute: WEIGHT_LIMIT / 2,
            transport: None,
        }
    }

    /// Production, unless the build set `LATTICE_HYPERLIQUID_HTTP` (the
    /// testnet's API, a local stand-in) or `LATTICE_HYPERLIQUID_WS` (a
    /// WebSocket that is not at the HTTP origin's `/ws`). Read at compile
    /// time, so it works in the web build too.
    pub fn from_build_env() -> Self {
        let mut config = Self::production();
        if let Some(http) = option_env!("LATTICE_HYPERLIQUID_HTTP") {
            http.clone_into(&mut config.http_base);
        }
        if let Some(ws) = option_env!("LATTICE_HYPERLIQUID_WS") {
            config.ws_url = Some(ws.to_owned());
        }
        config
    }

    /// The WebSocket's URL: the one set, else the HTTP origin's `/ws`
    /// (`https://api.hyperliquid.xyz` is streamed at
    /// `wss://api.hyperliquid.xyz/ws`).
    fn socket_url(&self) -> Result<String, ApiError> {
        let invalid = |why: &str| {
            ApiError::invalid_request(format!("invalid Hyperliquid WebSocket URL: {why}"))
        };
        let ws = match &self.ws_url {
            Some(ws) => ws.clone(),
            None => {
                let origin = self.http_base.trim_end_matches('/');
                let rest = origin
                    .strip_prefix("http")
                    .ok_or_else(|| invalid("the API is not at an http(s) origin"))?;
                format!("ws{rest}/ws")
            }
        };
        let url = url::Url::parse(&ws).map_err(|error| invalid(&error.to_string()))?;
        if !matches!(url.scheme(), "ws" | "wss") {
            return Err(invalid("it must be ws: or wss:"));
        }
        Ok(url.into())
    }
}

impl Default for HyperliquidConfig {
    fn default() -> Self {
        Self::from_build_env()
    }
}

/// The Hyperliquid client. Cloning is cheap: every clone shares one HTTP
/// client, one budget, one query hub and one socket.
///
/// Each service module owns its part of it (`impl Hyperliquid` in
/// `info.rs`, `accounts.rs`), so a new request adds no field here.
#[derive(Clone)]
pub struct Hyperliquid {
    api: Api,
    queries: Queries,
    network: Network,
    /// The agent that trades for the connected account, once it approved
    /// one ([`agent`]).
    agents: Rc<agent::Agents>,
    /// The WebSocket, which opens when an account or a price is followed
    /// live.
    socket: SharedSocket<stream::Protocol>,
    /// The accounts followed live ([`live`]) and the prices ([`pricing`]).
    feeds: FeedHubs,
}

/// What sends the requests: the HTTP client, and the budget each spends
/// from. Kept apart from the query hub so that a query's fetch holds this
/// and not the hub that holds the query.
#[derive(Clone)]
struct Api {
    http: HttpClient,
    budget: RateBudget,
}

impl Api {
    /// `reply` as it came, unless Hyperliquid refused the request for the
    /// address's limit (a `429`): something else spends that limit too, so
    /// what is left of the minute is spent here and the request fails as
    /// `RATE_LIMITED`. Every request goes through it, whichever endpoint.
    fn limited<T>(&self, reply: Result<T, ApiError>) -> Result<T, ApiError> {
        match reply {
            Err(error) if error.status == Some(429) => {
                self.budget.exhaust();
                Err(ApiError::api(
                    Some(429),
                    "RATE_LIMITED",
                    "Hyperliquid's request limit for this connection was reached; \
                     requests resume within a minute",
                ))
            }
            reply => reply,
        }
    }
}

impl Hyperliquid {
    /// A client for `config` that reports to `status` as
    /// `hyperliquid.queries` and `hyperliquid.socket`. Nothing is asked
    /// until something watches an account, and nothing connects until one
    /// is followed live.
    pub fn new(config: &HyperliquidConfig, status: StatusBoard) -> Result<Self, ApiError> {
        let http = HttpClient::new(
            &config.http_base,
            HttpOptions {
                timeout: config.request_timeout,
                decode_error,
                ..HttpOptions::default()
            },
        )?;
        let socket = SharedSocket::with_transport(
            stream::Protocol,
            SocketConfig::new(config.socket_url()?),
            config.transport.clone().unwrap_or_else(platform_transport),
            status,
            "hyperliquid.socket",
            "Hyperliquid realtime",
        );
        Ok(Self {
            api: Api {
                http,
                budget: RateBudget::new(config.weight_per_minute, WEIGHT_WINDOW),
            },
            queries: Queries::new(status, "hyperliquid.queries", "Hyperliquid requests"),
            network: Network::of(&config.http_base),
            agents: Rc::default(),
            socket,
            feeds: FeedHubs::default(),
        })
    }

    /// Which Hyperliquid the API is: what a message signed for it names.
    pub fn network(&self) -> Network {
        self.network
    }

    /// The shared query hub, which every request made here is watched in.
    pub fn queries(&self) -> &Queries {
        &self.queries
    }

    /// The budget every request spends its weight from.
    pub fn budget(&self) -> &RateBudget {
        &self.api.budget
    }

    /// Its sources in full, as they stand now: the socket with every
    /// topic followed, and the query hub with every watched request, each
    /// with who wants it.
    pub fn sources(&self) -> Vec<SourceDetails> {
        vec![self.socket.details(), self.queries.details()]
    }

    /// Closes the socket and drops every query and the agent's key held
    /// (the app is going away). What is kept of the agent for the next run
    /// stays.
    pub(crate) fn shutdown(&self) {
        self.agents.forget();
        self.socket.shutdown();
        self.queries.shutdown();
    }
}

/// Reads Hyperliquid's error replies: an error status with what is wrong
/// in plain text (`Failed to deserialize the JSON body into the target
/// type`). A status with nothing to read (an empty body, `null`, a proxy's
/// page) is left to say itself.
fn decode_error(status: u16, body: &[u8]) -> Option<(String, String)> {
    if status < 400 {
        return None;
    }
    let text = std::str::from_utf8(body).ok()?.trim();
    if text.is_empty() || text == "null" || text.starts_with('<') {
        return None;
    }
    Some((format!("HTTP_{status}"), text.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_socket_is_at_the_apis_own_origin_unless_told_otherwise() {
        let at = |http: &str| HyperliquidConfig {
            http_base: http.to_owned(),
            ..HyperliquidConfig::production()
        };
        assert_eq!(
            HyperliquidConfig::production().socket_url().as_deref(),
            Ok("wss://api.hyperliquid.xyz/ws")
        );
        assert_eq!(
            at("https://api.hyperliquid-testnet.xyz/")
                .socket_url()
                .as_deref(),
            Ok("wss://api.hyperliquid-testnet.xyz/ws")
        );
        // Another API is never streamed from production's.
        assert_eq!(
            at("http://127.0.0.1:3001").socket_url().as_deref(),
            Ok("ws://127.0.0.1:3001/ws")
        );
        let elsewhere = HyperliquidConfig {
            ws_url: Some("wss://stream.example/hl".to_owned()),
            ..at("https://hyperliquid.test")
        };
        assert_eq!(
            elsewhere.socket_url().as_deref(),
            Ok("wss://stream.example/hl")
        );
        for invalid in [
            HyperliquidConfig {
                ws_url: Some("https://stream.example".to_owned()),
                ..HyperliquidConfig::production()
            },
            at("ftp://hyperliquid.test"),
        ] {
            assert!(invalid.socket_url().is_err(), "{invalid:?}");
        }
    }

    #[test]
    fn error_replies_read_as_their_text() {
        assert_eq!(
            decode_error(
                422,
                b"Failed to deserialize the JSON body into the target type\n"
            ),
            Some((
                "HTTP_422".to_owned(),
                "Failed to deserialize the JSON body into the target type".to_owned()
            ))
        );
        // Nothing to read, and a reply that is no error.
        for body in [&b""[..], b"null", b"<html>Bad gateway</html>"] {
            assert_eq!(decode_error(502, body), None);
        }
        assert_eq!(decode_error(200, b"unexpected"), None);
    }
}
