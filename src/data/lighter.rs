//! Lighter's own API, asked by the app itself.
//!
//! As with Hyperliquid ([`super::hyperliquid`]), what one address holds on
//! Lighter is asked of Lighter, from the user's machine, so the requests
//! count against the user's own rate limit there and not against one
//! backend's for everybody. Everything read is public: Lighter answers these
//! requests by an address or an account's index alone. A request it answers
//! only with an auth token (which takes the account's key to make) is never
//! made, and no key, token or signature is held or sent.
//!
//! - Requests are `GET`s under `/api/v1` ([`Request`], in [`requests`]).
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
//!
//! Adding a report: its request in [`Request`], a module for its reply's
//! shape, and its query in [`accounts`], made of every account of the
//! address and combined as the portfolio combines accounts.

mod accounts;
mod fills;
mod performance;
mod prices;
mod requests;
mod state;

use std::time::Duration;

pub use requests::{Request, Resolution};
use serde::Deserialize;

use super::net::{
    ApiError, HttpClient, HttpOptions, Queries, RateBudget, SourceDetails, StatusBoard,
};

/// The production API (Lighter's mainnet).
pub const PRODUCTION_HTTP: &str = "https://mainnet.zklighter.elliot.ai";
/// The requests Lighter allows one IP address a minute while it sends no
/// auth token (a standard account's limit, which is not weighted).
pub const REQUEST_LIMIT: u32 = 60;
/// The window Lighter counts requests in.
const REQUEST_WINDOW: Duration = Duration::from_secs(60);

/// Where Lighter is, how long to wait for it and how much to ask of it.
#[derive(Clone, Debug)]
pub struct LighterConfig {
    /// The HTTP origin.
    pub http_base: String,
    /// The deadline for one request (headers and body).
    pub request_timeout: Duration,
    /// The requests the app makes a minute at most. Half of
    /// [`REQUEST_LIMIT`] by default: the limit is the address's, which
    /// Lighter's own site and anything else the user runs spend from too.
    pub requests_per_minute: u32,
}

impl LighterConfig {
    pub fn production() -> Self {
        Self {
            http_base: PRODUCTION_HTTP.to_owned(),
            request_timeout: Duration::from_secs(15),
            requests_per_minute: REQUEST_LIMIT / 2,
        }
    }

    /// Production, unless the build set `LATTICE_LIGHTER_HTTP` (the
    /// testnet's API, a local stand-in). Read at compile time, so it works
    /// in the web build too.
    pub fn from_build_env() -> Self {
        let mut config = Self::production();
        if let Some(http) = option_env!("LATTICE_LIGHTER_HTTP") {
            http.clone_into(&mut config.http_base);
        }
        config
    }
}

impl Default for LighterConfig {
    fn default() -> Self {
        Self::from_build_env()
    }
}

/// The Lighter client. Cloning is cheap: every clone shares one HTTP
/// client, one budget and one query hub.
///
/// Each service module owns its part of it (`impl Lighter` in
/// `accounts.rs`), so a new report adds no field here.
#[derive(Clone)]
pub struct Lighter {
    api: Api,
    queries: Queries,
}

/// What sends the requests: the HTTP client, and the budget each spends
/// from. Kept apart from the query hub so that a query's fetch holds this
/// and not the hub that holds the query.
#[derive(Clone)]
struct Api {
    http: HttpClient,
    budget: RateBudget,
}

impl Lighter {
    /// A client for `config` that reports to `status` as `lighter.queries`.
    /// Nothing is asked until something watches an account.
    pub fn new(config: &LighterConfig, status: StatusBoard) -> Result<Self, ApiError> {
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
                budget: RateBudget::new(config.requests_per_minute, REQUEST_WINDOW),
            },
            queries: Queries::new(status, "lighter.queries", "Lighter requests"),
        })
    }

    /// The shared query hub, which every request made here is watched in.
    pub fn queries(&self) -> &Queries {
        &self.queries
    }

    /// The budget every request spends from.
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
}
