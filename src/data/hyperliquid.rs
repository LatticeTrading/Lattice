//! Hyperliquid's own API, asked by the app itself.
//!
//! Ferris carries every exchange's public market data; what one address
//! holds on Hyperliquid is asked of Hyperliquid, from the user's machine,
//! so the requests count against the user's own rate limit there (it is
//! per IP address) and not against one backend's for everybody. Everything
//! read is public: an address is all Hyperliquid reports an account by, and
//! no key or signature is held or sent.
//!
//! What a connected wallet signs for Hyperliquid is made in [`signing`]:
//! the typed data of its user-signed actions, of which only the connection
//! test is used today (the wallet, `data::wallet`, signs it and nothing is
//! sent).
//!
//! - One endpoint: a JSON `POST /info` whose body's `type` names what is
//!   asked ([`Info`], in [`info`], with what each request weighs).
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
//!
//! Adding a request: its variant and weight in [`Info`], a module for its
//! reply's shape, and a query made with `Hyperliquid::info_query` (which
//! spends the weight before each fetch).

mod accounts;
mod fees;
mod fills;
mod info;
mod performance;
mod perps;
mod prices;
pub mod signing;
mod spot;
mod staking;

use std::time::Duration;

pub use info::Info;
pub use signing::Network;

use super::net::{
    ApiError, HttpClient, HttpOptions, Queries, RateBudget, SourceDetails, StatusBoard,
};

/// The production API.
pub const PRODUCTION_HTTP: &str = "https://api.hyperliquid.xyz";
/// The weight Hyperliquid allows one IP address a minute over REST.
pub const WEIGHT_LIMIT: u32 = 1200;
/// The window Hyperliquid counts weight in.
const WEIGHT_WINDOW: Duration = Duration::from_secs(60);

/// Where Hyperliquid is, how long to wait for it and how much to ask of it.
#[derive(Clone, Debug)]
pub struct HyperliquidConfig {
    /// The HTTP origin.
    pub http_base: String,
    /// The deadline for one request (headers and body).
    pub request_timeout: Duration,
    /// The weight the app spends a minute at most. Half of
    /// [`WEIGHT_LIMIT`] by default: the limit is the address's, which
    /// Hyperliquid's own site and anything else the user runs spend from
    /// too.
    pub weight_per_minute: u32,
}

impl HyperliquidConfig {
    pub fn production() -> Self {
        Self {
            http_base: PRODUCTION_HTTP.to_owned(),
            request_timeout: Duration::from_secs(15),
            weight_per_minute: WEIGHT_LIMIT / 2,
        }
    }

    /// Production, unless the build set `LATTICE_HYPERLIQUID_HTTP` (the
    /// testnet's API, a local stand-in). Read at compile time, so it works
    /// in the web build too.
    pub fn from_build_env() -> Self {
        let mut config = Self::production();
        if let Some(http) = option_env!("LATTICE_HYPERLIQUID_HTTP") {
            http.clone_into(&mut config.http_base);
        }
        config
    }
}

impl Default for HyperliquidConfig {
    fn default() -> Self {
        Self::from_build_env()
    }
}

/// The Hyperliquid client. Cloning is cheap: every clone shares one HTTP
/// client, one budget and one query hub.
///
/// Each service module owns its part of it (`impl Hyperliquid` in
/// `info.rs`, `accounts.rs`), so a new request adds no field here.
#[derive(Clone)]
pub struct Hyperliquid {
    api: Api,
    queries: Queries,
    network: Network,
}

/// What sends the requests: the HTTP client, and the budget each spends
/// from. Kept apart from the query hub so that a query's fetch holds this
/// and not the hub that holds the query.
#[derive(Clone)]
struct Api {
    http: HttpClient,
    budget: RateBudget,
}

impl Hyperliquid {
    /// A client for `config` that reports to `status` as
    /// `hyperliquid.queries`. Nothing is asked until something watches an
    /// account.
    pub fn new(config: &HyperliquidConfig, status: StatusBoard) -> Result<Self, ApiError> {
        let http = HttpClient::new(
            &config.http_base,
            HttpOptions {
                timeout: config.request_timeout,
                decode_error,
                ..HttpOptions::default()
            },
        )?;
        Ok(Self {
            api: Api {
                http,
                budget: RateBudget::new(config.weight_per_minute, WEIGHT_WINDOW),
            },
            queries: Queries::new(status, "hyperliquid.queries", "Hyperliquid requests"),
            network: Network::of(&config.http_base),
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

    /// Its sources in full, as they stand now: the query hub with every
    /// watched request and who wants it.
    pub fn sources(&self) -> Vec<SourceDetails> {
        vec![self.queries.details()]
    }

    /// Drops every query (the app is going away).
    pub(crate) fn shutdown(&self) {
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
