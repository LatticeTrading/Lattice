//! Lighter's REST requests: `GET`s under `/api/v1`, each named by its path
//! and its query, and the one `POST` that sends a signed transaction
//! ([`Api::send`]).
//!
//! What is read is public, and answered without an auth token, but for an
//! account's open orders ([`Request::ActiveOrders`]), which Lighter tells
//! only to a token one of the account's own API keys signed
//! ([`Api::get_as`]).
//!
//! A request costs one of the address's allowance
//! ([`super::REQUEST_LIMIT`] a minute, whatever is asked). It is spent from
//! the client's budget before the request is sent, waiting while the minute
//! is spent; Lighter refusing one all the same (`429`) spends what is left
//! of the minute and fails the request as `RATE_LIMITED`, which its query
//! retries once the budget has room. What an order waits on is never held
//! back so ([`Api::get_now`], [`Api::send`]): it is counted against the
//! minute and sent at once, so it is the polls that make room for it.

use serde::{Deserialize, de::DeserializeOwned};

use super::{Api, tx::Signed};
use crate::data::net::{ApiError, Request as HttpRequest};

/// What every request's path starts with.
const API: &str = "/api/v1";

/// How far apart the samples of a history are.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resolution {
    /// An hour, which Lighter keeps for the last ten days.
    Hour,
    /// A day, which it keeps since 2025.
    Day,
}

impl Resolution {
    /// What Lighter calls it.
    pub const fn id(self) -> &'static str {
        match self {
            Self::Hour => "1h",
            Self::Day => "1d",
        }
    }

    /// How long one is, in seconds.
    pub const fn seconds(self) -> u64 {
        match self {
            Self::Hour => 3_600,
            Self::Day => 86_400,
        }
    }
}

/// One request of the REST API.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    /// Every account an L1 address holds (its main one, its sub-accounts
    /// and the pools it runs), each with its index, its standing, its
    /// positions and its assets. An address with none is refused as
    /// [`ACCOUNT_NOT_FOUND`].
    Accounts { l1_address: String },
    /// What an account had made, taken in and paid out by each sample
    /// between two times (Unix seconds), `resolution` apart, with what it
    /// traded between each two.
    Pnl {
        account_index: u64,
        resolution: Resolution,
        start: u64,
        end: u64,
    },
    /// An account's latest trades, newest first: `limit` of them, which
    /// Lighter caps at [`TRADES_LIMIT`].
    Trades { account_index: u64, limit: u32 },
    /// Every asset Lighter holds, with its index price.
    AssetDetails,
    /// Every market: what it trades, how fine its sizes and prices are, and
    /// what margin it asks.
    Markets,
    /// The best bid and the best offer of a market's book.
    BookTop { market_id: u64 },
    /// The nonce the next transaction of an account's API key takes.
    NextNonce {
        account_index: u64,
        api_key_index: u8,
    },
    /// The public keys of an account's API keys: the one at an index, or
    /// all of them (`None`). An index with no key is refused as
    /// [`API_KEY_NOT_FOUND`].
    ApiKeys {
        account_index: u64,
        api_key_index: Option<u8>,
    },
    /// An account's open orders, in every market. Answered only with an
    /// auth token of the account's own.
    ActiveOrders { account_index: u64 },
}

/// The most trades one request returns.
pub const TRADES_LIMIT: u32 = 100;

/// The code Lighter refuses an address that holds no account with.
pub(super) const ACCOUNT_NOT_FOUND: &str = "21100";
/// The code Lighter refuses an API key index that holds no key with.
pub(super) const API_KEY_NOT_FOUND: &str = "21109";
/// What asks for every API key of an account, in an index's place.
const EVERY_API_KEY: u8 = 255;
/// Where a signed transaction is posted.
const SEND: &str = "/api/v1/sendTx";

impl Request {
    /// The request's path and query, from `/`.
    pub fn path(&self) -> String {
        match self {
            Self::Accounts { l1_address } => {
                format!("{API}/account?by=l1_address&value={l1_address}")
            }
            Self::Trades {
                account_index,
                limit,
            } => format!(
                "{API}/trades?sort_by=timestamp&account_index={account_index}&limit={}",
                (*limit).clamp(1, TRADES_LIMIT)
            ),
            Self::AssetDetails => format!("{API}/assetDetails"),
            Self::Markets => format!("{API}/orderBookDetails"),
            Self::BookTop { market_id } => {
                format!("{API}/orderBookOrders?market_id={market_id}&limit=1")
            }
            Self::NextNonce {
                account_index,
                api_key_index,
            } => format!(
                "{API}/nextNonce?account_index={account_index}&api_key_index={api_key_index}"
            ),
            Self::ApiKeys {
                account_index,
                api_key_index,
            } => format!(
                "{API}/apikeys?account_index={account_index}&api_key_index={}",
                api_key_index.unwrap_or(EVERY_API_KEY)
            ),
            Self::ActiveOrders { account_index } => {
                format!("{API}/accountActiveOrders?account_index={account_index}")
            }
            Self::Pnl {
                account_index,
                resolution,
                start,
                end,
            } => {
                // Lighter takes the times in milliseconds, and how many
                // samples that is as a hint.
                let samples = (end.saturating_sub(*start) / resolution.seconds()).max(1);
                format!(
                    "{API}/pnl?by=index&value={account_index}&resolution={}\
                     &start_timestamp={}&end_timestamp={}&count_back={samples}",
                    resolution.id(),
                    start.saturating_mul(1000),
                    end.saturating_mul(1000),
                )
            }
        }
    }
}

/// What Lighter answers a transaction it took with. It takes one before it
/// runs it: what the transaction then does (an order resting, trading, or
/// being refused for its margin) shows in the account, not here.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Sent {
    code: i64,
    message: String,
}

impl Api {
    /// Sends `request` once the budget has room for it, and reads the reply
    /// as `T`.
    pub(super) async fn get<T: DeserializeOwned>(&self, request: &Request) -> Result<T, ApiError> {
        self.budget.spend(1).await?;
        self.limited(self.http.get(&request.path()).await)
    }

    /// [`Self::get`] sent at once, the budget having room for it or not:
    /// for what an order waits on (its key's nonce, the price it is bounded
    /// from).
    pub(super) async fn get_now<T: DeserializeOwned>(
        &self,
        request: &Request,
    ) -> Result<T, ApiError> {
        self.budget.charge(1);
        self.limited(self.http.get(&request.path()).await)
    }

    /// [`Self::get`] with the auth token `token`: for what Lighter tells
    /// only an account's own key.
    pub(super) async fn get_as<T: DeserializeOwned>(
        &self,
        request: &Request,
        token: &str,
    ) -> Result<T, ApiError> {
        self.budget.spend(1).await?;
        let authorized = HttpRequest::get(request.path()).header("authorization", token);
        self.limited(self.http.send(authorized).await)
    }

    /// Posts the signed transaction `signed`, at once. `Ok` is Lighter
    /// having taken it, which is before it runs it.
    pub(super) async fn send(&self, signed: &Signed) -> Result<(), ApiError> {
        self.budget.charge(1);
        let kind = signed.kind.to_string();
        let form = [
            ("tx_type", kind.as_str()),
            ("tx_info", signed.info.as_str()),
        ];
        let sent: Sent = self.limited(self.http.send(HttpRequest::post_form(SEND, form)).await)?;
        // A refusal comes under an error status; one under a success would
        // still say so in its code.
        if sent.code == 200 || sent.code == 0 {
            Ok(())
        } else {
            Err(ApiError::api(
                Some(200),
                sent.code.to_string(),
                sent.message,
            ))
        }
    }

    /// `reply` as it came, unless Lighter refused the request for the
    /// address's limit (a `429`): something else spends that limit too, so
    /// what is left of the minute is spent here and the request fails as
    /// `RATE_LIMITED`.
    fn limited<T>(&self, reply: Result<T, ApiError>) -> Result<T, ApiError> {
        match reply {
            Err(error) if error.status == Some(429) => {
                self.budget.exhaust();
                Err(ApiError::api(
                    Some(429),
                    "RATE_LIMITED",
                    "Lighter's request limit for this connection was reached; \
                     requests resume within a minute",
                ))
            }
            reply => reply,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_are_their_paths_and_queries() {
        let accounts = Request::Accounts {
            l1_address: "0x0123456789abcdef0123456789abcdef01234567".to_owned(),
        };
        assert_eq!(
            accounts.path(),
            "/api/v1/account?by=l1_address&value=0x0123456789abcdef0123456789abcdef01234567"
        );
        // No more trades than Lighter returns at once.
        let trades = |limit| Request::Trades {
            account_index: 2_180,
            limit,
        };
        assert_eq!(
            trades(100).path(),
            "/api/v1/trades?sort_by=timestamp&account_index=2180&limit=100"
        );
        assert!(trades(5_000).path().ends_with("limit=100"));
        assert_eq!(Request::AssetDetails.path(), "/api/v1/assetDetails");
        assert_eq!(Request::Markets.path(), "/api/v1/orderBookDetails");
        assert_eq!(
            Request::BookTop { market_id: 1 }.path(),
            "/api/v1/orderBookOrders?market_id=1&limit=1"
        );

        // An API key is asked for by its account's index and its own; all of
        // an account's by the index that is none's.
        let key = |api_key_index| Request::ApiKeys {
            account_index: 2_180,
            api_key_index,
        };
        assert_eq!(
            key(Some(253)).path(),
            "/api/v1/apikeys?account_index=2180&api_key_index=253"
        );
        assert!(key(None).path().ends_with("api_key_index=255"));
        let nonce = Request::NextNonce {
            account_index: 2_180,
            api_key_index: 253,
        };
        assert_eq!(
            nonce.path(),
            "/api/v1/nextNonce?account_index=2180&api_key_index=253"
        );
        let orders = Request::ActiveOrders {
            account_index: 2_180,
        };
        assert_eq!(
            orders.path(),
            "/api/v1/accountActiveOrders?account_index=2180"
        );

        // A week of hours: the times in milliseconds, and how many hours.
        let week = Request::Pnl {
            account_index: 281_474_976_689_919,
            resolution: Resolution::Hour,
            start: 1_794_449_724,
            end: 1_794_449_724 + 7 * 86_400,
        };
        assert_eq!(
            week.path(),
            "/api/v1/pnl?by=index&value=281474976689919&resolution=1h\
             &start_timestamp=1794449724000&end_timestamp=1795054524000&count_back=168"
        );
        let days = Request::Pnl {
            account_index: 7,
            resolution: Resolution::Day,
            start: 1_735_689_600,
            end: 1_735_689_600 + 86_400 / 2,
        };
        assert!(days.path().contains("resolution=1d"));
        assert!(days.path().ends_with("count_back=1"));
    }
}
