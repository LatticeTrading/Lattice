//! TxFlow's own API, asked by the app itself.
//!
//! As with Hyperliquid ([`super::hyperliquid`]) and Lighter
//! ([`super::lighter`]), what one address holds on TxFlow is asked of TxFlow,
//! from the user's machine, so the requests count against the user's own
//! rate limit there (it is per IP address) and not against one backend's for
//! everybody. Everything read is public: TxFlow's reads take an address
//! alone, and no key or signature is held or sent.
//!
//! - One endpoint: a JSON `POST /info` whose body's `type` names what is
//!   asked ([`Info`], in [`info`]), sent with the API version its replies
//!   are documented in ([`API_VERSION`]). Most replies come wrapped as
//!   `{ "success": true, "code": 0, "data": … }`; the history's does not.
//! - The rate limit: TxFlow allows an IP address [`REQUEST_LIMIT`] info
//!   requests a second, whatever they ask. Every request spends one from a
//!   [`RateBudget`] first and waits while the second is spent
//!   ([`TxFlowConfig::requests_per_second`]); a `429` all the same spends
//!   what is left, so nothing more goes out for a second.
//! - Requests are shared and polled through one [`Queries`] hub, which
//!   reports to the status board as `txflow.queries`.
//! - An address holds three sub-accounts on TxFlow: spot, perpetuals and
//!   Outcome (prediction markets), each margined apart.
//! - What it reports of an address: its perpetuals sub-account ([`perps`]),
//!   its history over a day, a week and a month ([`performance`]), its
//!   vault shares and what it pays to trade ([`fees`], [`vaults`]) and its
//!   latest perpetuals trades ([`fills`]). [`accounts`] gives them to the
//!   app in the portfolio's shapes
//!   ([`AccountSource`](super::portfolio::AccountSource)), which is all the
//!   widgets know of TxFlow.
//! - What it does not: token balances (TxFlow documents no read of the
//!   spot sub-account, and the perpetuals sub-account's tokens are its
//!   margin, already its equity), Outcome positions, staking, and a
//!   history over all time or as a chart (TxFlow reports a period's figures,
//!   not samples of it).
//!
//! Adding a report: its request in [`Info`], a module for its reply's
//! shape, and its query in [`accounts`].

mod accounts;
mod fees;
mod fills;
mod info;
mod performance;
mod perps;
mod vaults;

use std::time::Duration;

pub use info::{Info, OverviewPeriod};
use serde_json::Value;

use super::net::{
    ApiError, HttpClient, HttpOptions, Queries, RateBudget, SourceDetails, StatusBoard,
};

/// The production API (TxFlow's mainnet).
pub const PRODUCTION_HTTP: &str = "https://rpc.txflow.com";
/// The info requests TxFlow allows one IP address a second.
pub const REQUEST_LIMIT: u32 = 50;
/// The window TxFlow counts requests in.
const REQUEST_WINDOW: Duration = Duration::from_secs(1);
/// The API version every request asks for (`x-api-version`): the one the
/// replies read here are documented in.
pub const API_VERSION: &str = "2026-08-16";

/// Where TxFlow is, how long to wait for it and how much to ask of it.
#[derive(Clone, Debug)]
pub struct TxFlowConfig {
    /// The HTTP origin.
    pub http_base: String,
    /// The deadline for one request (headers and body).
    pub request_timeout: Duration,
    /// The requests the app makes a second at most. Half of
    /// [`REQUEST_LIMIT`] by default: the limit is the address's, which
    /// TxFlow's own site and anything else the user runs spend from too.
    pub requests_per_second: u32,
}

impl TxFlowConfig {
    pub fn production() -> Self {
        Self {
            http_base: PRODUCTION_HTTP.to_owned(),
            request_timeout: Duration::from_secs(15),
            requests_per_second: REQUEST_LIMIT / 2,
        }
    }

    /// Production, unless the build set `LATTICE_TXFLOW_HTTP` (the
    /// testnet's API, `https://rpc.txflow-testnet.xyz`, or a local
    /// stand-in). Read at compile time, so it works in the web build too.
    pub fn from_build_env() -> Self {
        let mut config = Self::production();
        if let Some(http) = option_env!("LATTICE_TXFLOW_HTTP") {
            http.clone_into(&mut config.http_base);
        }
        config
    }
}

impl Default for TxFlowConfig {
    fn default() -> Self {
        Self::from_build_env()
    }
}

/// The TxFlow client. Cloning is cheap: every clone shares one HTTP
/// client, one budget and one query hub.
///
/// Each service module owns its part of it (`impl TxFlow` in `info.rs`,
/// `accounts.rs`), so a new report adds no field here.
#[derive(Clone)]
pub struct TxFlow {
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

impl TxFlow {
    /// A client for `config` that reports to `status` as `txflow.queries`.
    /// Nothing is asked until something watches an account.
    pub fn new(config: &TxFlowConfig, status: StatusBoard) -> Result<Self, ApiError> {
        let http = HttpClient::new(
            &config.http_base,
            HttpOptions {
                timeout: config.request_timeout,
                headers: vec![("x-api-version".to_owned(), API_VERSION.to_owned())],
                decode_error,
            },
        )?;
        Ok(Self {
            api: Api {
                http,
                budget: RateBudget::new(config.requests_per_second.max(1), REQUEST_WINDOW),
            },
            queries: Queries::new(status, "txflow.queries", "TxFlow requests"),
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

/// Reads TxFlow's error replies: a JSON object of its own `code` (`10000`)
/// and a `message` saying what is wrong (`invalid user address: …`), under
/// an error status or with `success: false`. The history's replies say it
/// in `msg`, and a few requests in the node's own shape (`"status": "err"`
/// with a `reason`). A reply that says it succeeded, or one with nothing to
/// read (a proxy's page), is left to say itself.
fn decode_error(status: u16, body: &[u8]) -> Option<(String, String)> {
    let reply: Value = serde_json::from_slice(body).ok()?;
    let reply = reply.as_object()?;
    let succeeded = reply.get("success").and_then(Value::as_bool) == Some(true)
        || reply.get("status").and_then(Value::as_str) == Some("ok");
    let code = reply.get("code").and_then(Value::as_i64);
    if succeeded || (status < 400 && code == Some(0)) {
        return None;
    }
    let message = ["message", "msg", "reason", "response"]
        .into_iter()
        .filter_map(|name| reply.get(name).and_then(Value::as_str))
        .map(str::trim)
        .find(|message| !message.is_empty())?;
    let code = code.map_or_else(|| format!("HTTP_{status}"), |code| code.to_string());
    Some((code, message.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_replies_read_as_their_code_and_message() {
        assert_eq!(
            decode_error(
                400,
                br#"{"success":false,"code":10000,"message":"invalid user address: nope"}"#
            ),
            Some(("10000".to_owned(), "invalid user address: nope".to_owned()))
        );
        // Refused with a success status, and in the node's own shape.
        assert_eq!(
            decode_error(
                200,
                br#"{"success":false,"code":10000,"message":"rate limit exceeded"}"#
            ),
            Some(("10000".to_owned(), "rate limit exceeded".to_owned()))
        );
        assert_eq!(
            decode_error(500, br#"{"status":"err","reason":"internal server error"}"#),
            Some(("HTTP_500".to_owned(), "internal server error".to_owned()))
        );
        // Nothing to read, and replies that are no error.
        for body in [&b""[..], b"null", b"<html>Bad gateway</html>", b"{}"] {
            assert_eq!(decode_error(502, body), None);
        }
        assert_eq!(
            decode_error(200, br#"{"success":true,"code":0,"message":"fine"}"#),
            None
        );
        assert_eq!(decode_error(200, br#"{"code":0,"msg":"","pnl":"x"}"#), None);
    }
}
