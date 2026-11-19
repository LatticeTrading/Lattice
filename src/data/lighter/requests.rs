//! Lighter's REST requests: `GET`s under `/api/v1`, each named by its path
//! and its query. Every request here reads public data, and is one Lighter
//! answers without an auth token.
//!
//! A request costs one of the address's allowance
//! ([`super::REQUEST_LIMIT`] a minute, whatever is asked). It is spent from
//! the client's budget before the request is sent, waiting while the minute
//! is spent; Lighter refusing one all the same (`429`) spends what is left
//! of the minute and fails the request as `RATE_LIMITED`, which its query
//! retries once the budget has room.

use serde::de::DeserializeOwned;

use super::Api;
use crate::data::net::ApiError;

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
}

/// The most trades one request returns.
pub const TRADES_LIMIT: u32 = 100;

/// The code Lighter refuses an address that holds no account with.
pub(super) const ACCOUNT_NOT_FOUND: &str = "21100";

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

impl Api {
    /// Sends `request` once the budget has room for it, and reads the reply
    /// as `T`.
    pub(super) async fn get<T: DeserializeOwned>(&self, request: &Request) -> Result<T, ApiError> {
        self.budget.spend(1).await?;
        match self.http.get(&request.path()).await {
            Err(error) if error.status == Some(429) => {
                // Something else spends the address's limit too: nothing
                // more goes out until a minute has passed.
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
