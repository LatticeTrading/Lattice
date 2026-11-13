//! Ferris: Lattice's market data backend, used like any external API.
//!
//! Ferris serves public market data from many exchanges through one HTTP
//! API and one multiplexed WebSocket; the app never connects to exchanges
//! for public data itself. No credentials are sent.
//!
//! - REST: `GET /v1/capabilities` and JSON `POST /v1/<operation>`
//!   ([`Operation`]). Errors come back as `{"error":{"code","message"}}`
//!   (or the same members at the top level) with the HTTP status.
//! - WebSocket: `/v1/ws`, frames described in [`protocol`].
//! - Capabilities say, per exchange, which operations, channels and
//!   products are available; check them before offering a feature
//!   ([`Capabilities::snapshot`], [`Capabilities::realtime`]).
//! - Market statistics ([`market_stats`]): every market of a scope with
//!   prices, funding, volume and open interest, polled through one shared
//!   query per scope ([`Ferris::market_stats_query`]).
//!
//! Ferris descends from a CCXT-compatible client, so its vocabulary
//! (`fetchMarkets`, unified market/ticker/trade/book/candle shapes) is
//! CCXT's; the shapes are typed by the services that use them.

mod capabilities;
pub mod market_stats;
mod protocol;

use std::{rc::Rc, time::Duration};

use serde::{Serialize, de::DeserializeOwned};

pub use capabilities::{
    Availability, Capabilities, CapabilityState, DeliveryMode, ExchangeCapabilities, Feature,
    Features, Product,
};
pub use market_stats::{MarketStatsScope, StatsFieldName, StatsSnapshot, stats_scopes};
pub use protocol::{Channel, FerrisProtocol, FerrisPush, FerrisTopic};

use super::net::{
    ApiError, HttpClient, HttpOptions, Queries, QuerySpec, Request, SharedSocket, SocketConfig,
    StatusBoard, Subscription, TopicPhase, Watch,
};

/// The production API.
pub const PRODUCTION_HTTP: &str = "https://api.latticeterminal.com";
/// The production WebSocket.
pub const PRODUCTION_WS: &str = "wss://api.latticeterminal.com/v1/ws";

/// How often capabilities are refetched while something watches them.
const CAPABILITIES_REFRESH: Duration = Duration::from_secs(5 * 60);

/// Where Ferris is and how long to wait for it.
#[derive(Clone, Debug)]
pub struct FerrisConfig {
    /// The HTTP origin.
    pub http_base: String,
    /// The WebSocket URL.
    pub ws_url: String,
    /// The default deadline for one request (headers and body).
    pub request_timeout: Duration,
}

impl FerrisConfig {
    pub fn production() -> Self {
        Self {
            http_base: PRODUCTION_HTTP.to_owned(),
            ws_url: PRODUCTION_WS.to_owned(),
            request_timeout: Duration::from_secs(15),
        }
    }

    /// Production, unless the build set `LATTICE_FERRIS_HTTP` and/or
    /// `LATTICE_FERRIS_WS` (a local or staging backend). They are read at
    /// compile time, so they work in the web build too.
    pub fn from_build_env() -> Self {
        let mut config = Self::production();
        if let Some(http) = option_env!("LATTICE_FERRIS_HTTP") {
            http.clone_into(&mut config.http_base);
        }
        if let Some(ws) = option_env!("LATTICE_FERRIS_WS") {
            ws.clone_into(&mut config.ws_url);
        }
        config
    }
}

impl Default for FerrisConfig {
    fn default() -> Self {
        Self::from_build_env()
    }
}

/// A Ferris REST operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Operation {
    FetchMarkets,
    FetchMarketStats,
    FetchOrderBook,
    FetchTrades,
    FetchOhlcv,
}

impl Operation {
    /// Its endpoint.
    pub fn path(self) -> &'static str {
        match self {
            Self::FetchMarkets => "/v1/fetchMarkets",
            Self::FetchMarketStats => "/v1/fetchMarketStats",
            Self::FetchOrderBook => "/v1/fetchOrderBook",
            Self::FetchTrades => "/v1/fetchTrades",
            Self::FetchOhlcv => "/v1/fetchOHLCV",
        }
    }

    /// Its name in an exchange's capability features.
    pub fn feature(self) -> &'static str {
        match self {
            Self::FetchMarkets => "fetchMarkets",
            Self::FetchMarketStats => "fetchMarketStats",
            Self::FetchOrderBook => "fetchOrderBook",
            Self::FetchTrades => "fetchTrades",
            Self::FetchOhlcv => "fetchOhlcv",
        }
    }
}

/// The Ferris client. Cloning is cheap: every clone shares one HTTP
/// client, one socket and one query hub.
#[derive(Clone)]
pub struct Ferris {
    config: Rc<FerrisConfig>,
    http: HttpClient,
    socket: SharedSocket<FerrisProtocol>,
    queries: Queries,
}

impl Ferris {
    /// A client for `config` that reports to `status` as `ferris.socket`
    /// and `ferris.queries`. Nothing connects until something asks for data.
    pub fn new(config: FerrisConfig, status: StatusBoard) -> Result<Self, ApiError> {
        let http = HttpClient::new(
            &config.http_base,
            HttpOptions {
                timeout: config.request_timeout,
                decode_error,
                ..HttpOptions::default()
            },
        )?;
        let ws = url::Url::parse(&config.ws_url).map_err(|error| {
            ApiError::invalid_request(format!("invalid Ferris WebSocket URL: {error}"))
        })?;
        if !matches!(ws.scheme(), "ws" | "wss") {
            return Err(ApiError::invalid_request(
                "the Ferris WebSocket URL must be ws: or wss:",
            ));
        }
        let socket = SharedSocket::new(
            FerrisProtocol,
            SocketConfig::new(ws.as_str()),
            status,
            "ferris.socket",
            "Ferris realtime",
        );
        let queries = Queries::new(status, "ferris.queries", "Ferris requests");
        Ok(Self {
            config: Rc::new(config),
            http,
            socket,
            queries,
        })
    }

    pub fn config(&self) -> &FerrisConfig {
        &self.config
    }

    /// The HTTP client, for an endpoint without an [`Operation`].
    pub fn http(&self) -> &HttpClient {
        &self.http
    }

    /// The shared query hub. A service that polls Ferris (statistics,
    /// catalogs) makes its [`QuerySpec`]s here, so every widget asking for
    /// the same request shares one.
    pub fn queries(&self) -> &Queries {
        &self.queries
    }

    /// `POST`s `body` to `operation` and decodes the reply, within the
    /// configured deadline.
    pub async fn request<T: DeserializeOwned>(
        &self,
        operation: Operation,
        body: &(impl Serialize + ?Sized),
    ) -> Result<T, ApiError> {
        self.http
            .send(Request::post_json(operation.path(), body)?)
            .await
    }

    /// [`Self::request`] with its own deadline (large statistics scopes are
    /// slower than the default allows).
    pub async fn request_within<T: DeserializeOwned>(
        &self,
        operation: Operation,
        body: &(impl Serialize + ?Sized),
        timeout: Duration,
    ) -> Result<T, ApiError> {
        let request = Request::post_json(operation.path(), body)?.timeout(timeout);
        self.http.send(request).await
    }

    /// The capabilities request: shared, cached and refreshed every five
    /// minutes while watched.
    pub fn capabilities_query(&self) -> QuerySpec<Capabilities> {
        let http = self.http.clone();
        QuerySpec::new("ferris:capabilities", move || {
            let http = http.clone();
            async move { http.get::<Capabilities>("/v1/capabilities").await }
        })
        .refresh_every(CAPABILITIES_REFRESH)
    }

    /// The capabilities: the cached copy while fresh, else fetched.
    pub async fn capabilities(&self) -> Result<Rc<Capabilities>, ApiError> {
        self.queries.get(self.capabilities_query()).await
    }

    /// Watches the capabilities (refreshed while watched).
    pub fn watch_capabilities(&self) -> Watch<Capabilities> {
        self.queries.watch(self.capabilities_query())
    }

    /// The statistics of `scope`: one shared request per scope, refetched on
    /// the scope's poll interval while watched.
    pub fn market_stats_query(&self, scope: &MarketStatsScope) -> QuerySpec<StatsSnapshot> {
        let http = self.http.clone();
        let body = market_stats::request_body(&scope.exchange, &scope.params, &scope.fields);
        let timeout =
            market_stats::request_timeout(&scope.exchange, &scope.params, scope.fields.len());
        QuerySpec::new(scope.key.clone(), move || {
            let http = http.clone();
            let request = Request::post_json(Operation::FetchMarketStats.path(), &body)
                .map(|request| request.timeout(timeout));
            async move { http.send::<StatsSnapshot>(request?).await }
        })
        .refresh_every(scope.capability.poll_interval)
    }

    /// Subscribes to a live topic on the shared socket.
    pub fn subscribe(
        &self,
        topic: FerrisTopic,
        on_message: impl Fn(&FerrisPush) + 'static,
    ) -> Subscription {
        self.socket.subscribe(topic, on_message)
    }

    /// [`Self::subscribe`], also told the topic's phase as it changes.
    pub fn subscribe_with_phase(
        &self,
        topic: FerrisTopic,
        on_message: impl Fn(&FerrisPush) + 'static,
        on_phase: impl Fn(TopicPhase) + 'static,
    ) -> Subscription {
        self.socket
            .subscribe_with_phase(topic, on_message, on_phase)
    }

    /// Closes the socket and drops every query (the app is going away).
    pub(crate) fn shutdown(&self) {
        self.socket.shutdown();
        self.queries.shutdown();
    }
}

/// Reads Ferris's error replies: `{"error":{"code","message"}}`, or `code`
/// and `message` at the top level.
fn decode_error(_status: u16, body: &[u8]) -> Option<(String, String)> {
    let reply: serde_json::Value = serde_json::from_slice(body).ok()?;
    let error = reply
        .get("error")
        .filter(|error| error.is_object())
        .unwrap_or(&reply);
    let code = error
        .get("code")
        .and_then(|code| code.as_str())
        .filter(|code| !code.trim().is_empty())?;
    let message = error
        .get("message")
        .and_then(|message| message.as_str())
        .unwrap_or("Ferris reported an error");
    Some((code.to_owned(), message.to_owned()))
}
