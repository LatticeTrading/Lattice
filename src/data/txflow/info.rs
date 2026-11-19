//! The info endpoint: `POST /info` with a JSON body whose `type` names what
//! is asked. Every request here reads public data.
//!
//! A request costs one of the address's allowance
//! ([`super::REQUEST_LIMIT`] a second, whatever is asked). It is spent from
//! the client's budget before the request is sent, waiting while the second
//! is spent; TxFlow refusing one all the same (`429`) spends what is left of
//! the second and fails the request as `RATE_LIMITED`, which its query
//! retries once the budget has room.
//!
//! Replies come wrapped (`{ "success": true, "code": 0, "data": … }`), but
//! for the history's ([`Info::UserOverview`]), which is the reply itself.

use std::{rc::Rc, time::Duration};

use serde::{Deserialize, Serialize, de::DeserializeOwned};

use super::{Api, TxFlow};
use crate::data::net::{ApiError, QuerySpec, Subject, canonical_json};

/// The endpoint every [`Info`] request is posted to.
const PATH: &str = "/info";

/// The most fills one request returns.
pub const FILLS_LIMIT: u32 = 2_000;

/// A stretch the history is asked over.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
pub enum OverviewPeriod {
    #[serde(rename = "24H")]
    Day,
    #[serde(rename = "7D")]
    Week,
    #[serde(rename = "30D")]
    Month,
}

impl OverviewPeriod {
    /// Every period, shortest first.
    pub const ALL: [Self; 3] = [Self::Day, Self::Week, Self::Month];

    /// How long it is, in milliseconds.
    pub const fn span_ms(self) -> f64 {
        const DAY: f64 = 86_400_000.0;
        match self {
            Self::Day => DAY,
            Self::Week => 7.0 * DAY,
            Self::Month => 30.0 * DAY,
        }
    }
}

/// One request of the info endpoint, which serializes as its body.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Info {
    /// A user's perpetuals sub-account: its margin summary and open
    /// positions.
    #[serde(rename_all = "camelCase")]
    ClearinghouseState {
        user: String,
        /// `perp`: the perpetuals sub-account, not the Outcome one.
        account_type: &'static str,
    },
    /// What a user made and traded over `period`, and its equity now. Its
    /// keys are snake case, as TxFlow takes them.
    UserOverview {
        user_address: String,
        period: OverviewPeriod,
        /// The sub-accounts counted; the perpetuals one alone when left
        /// out.
        #[serde(skip_serializing_if = "Option::is_none")]
        account_type: Option<Vec<&'static str>>,
    },
    /// What a user pays to trade: its rates, the fee schedule and its
    /// daily volume. The rates are those of `asset`'s kind of instrument
    /// (an instrument's index, as text).
    UserFees { user: String, asset: String },
    /// The vaults a user has a share of. TxFlow spells its type in lower
    /// case.
    #[serde(rename = "uservaultequities")]
    UserVaultEquities { user: String },
    /// A user's latest perpetuals fills, newest first: `limit` of them,
    /// which TxFlow caps at [`FILLS_LIMIT`].
    UserFills { user: String, limit: u32 },
}

impl Info {
    /// Whether the reply comes wrapped in `{ "success", "code", "data" }`.
    fn wrapped(&self) -> bool {
        !matches!(self, Self::UserOverview { .. })
    }

    /// What tells the request apart in the query hub: the endpoint and its
    /// body, members in one order.
    pub fn key(&self) -> String {
        let body = serde_json::to_value(self).unwrap_or_default();
        format!("info {}", canonical_json(&body))
    }
}

/// A wrapped reply: what was asked for is its `data`.
#[derive(Deserialize)]
struct Wrapped<T> {
    data: T,
}

impl Api {
    /// Posts `request` once the budget has room for it, and reads the reply
    /// as `T`.
    pub(super) async fn info<T: DeserializeOwned>(&self, request: &Info) -> Result<T, ApiError> {
        self.budget.spend(1).await?;
        let reply = if request.wrapped() {
            self.http
                .post::<Wrapped<T>>(PATH, request)
                .await
                .map(|wrapped| wrapped.data)
        } else {
            self.http.post::<T>(PATH, request).await
        };
        match reply {
            Err(error) if error.status == Some(429) => {
                // Something else spends the address's limit too: nothing
                // more goes out until a second has passed.
                self.budget.exhaust();
                Err(ApiError::api(
                    Some(429),
                    "RATE_LIMITED",
                    "TxFlow's request limit for this connection was reached; \
                     requests resume within moments",
                ))
            }
            reply => reply,
        }
    }
}

impl TxFlow {
    /// A shared `request`, its reply read as `R` and turned into the value
    /// by `decode`, refetched every `refresh` while watched and named
    /// `subject` in the connection panel.
    pub(super) fn info_query<R, T>(
        &self,
        request: Info,
        refresh: Duration,
        subject: Subject,
        decode: impl Fn(R) -> T + 'static,
    ) -> QuerySpec<T>
    where
        R: DeserializeOwned + 'static,
        T: 'static,
    {
        let api = self.api.clone();
        let key = request.key();
        let decode = Rc::new(decode);
        QuerySpec::new(key, move || {
            let (api, request, decode) = (api.clone(), request.clone(), decode.clone());
            async move { Ok(decode(api.info::<R>(&request).await?)) }
        })
        .refresh_every(refresh)
        .subject(subject)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const USER: &str = "0x0123456789abcdef0123456789abcdef01234567";

    #[test]
    fn requests_serialize_as_their_bodies() {
        let state = Info::ClearinghouseState {
            user: USER.to_owned(),
            account_type: "perp",
        };
        assert_eq!(
            serde_json::to_value(&state).unwrap(),
            json!({ "type": "clearinghouseState", "user": USER, "accountType": "perp" })
        );
        assert!(state.wrapped());

        // The history's keys are snake case, and its reply is not wrapped.
        let perps = Info::UserOverview {
            user_address: USER.to_owned(),
            period: OverviewPeriod::Week,
            account_type: None,
        };
        assert_eq!(
            serde_json::to_value(&perps).unwrap(),
            json!({ "type": "userOverview", "user_address": USER, "period": "7D" })
        );
        assert!(!perps.wrapped());
        let total = Info::UserOverview {
            user_address: USER.to_owned(),
            period: OverviewPeriod::Day,
            account_type: Some(vec!["perps", "outcome"]),
        };
        assert_eq!(
            serde_json::to_value(&total).unwrap(),
            json!({
                "type": "userOverview", "user_address": USER, "period": "24H",
                "account_type": ["perps", "outcome"]
            })
        );
        assert_ne!(perps.key(), total.key());

        for (request, body) in [
            (
                Info::UserFees {
                    user: USER.to_owned(),
                    asset: "1".to_owned(),
                },
                json!({ "type": "userFees", "user": USER, "asset": "1" }),
            ),
            (
                Info::UserVaultEquities {
                    user: USER.to_owned(),
                },
                json!({ "type": "uservaultequities", "user": USER }),
            ),
            (
                Info::UserFills {
                    user: USER.to_owned(),
                    limit: FILLS_LIMIT,
                },
                json!({ "type": "userFills", "user": USER, "limit": 2000 }),
            ),
        ] {
            assert_eq!(serde_json::to_value(&request).unwrap(), body);
        }
    }

    #[test]
    fn a_key_is_the_body_whatever_its_order() {
        let request = Info::UserFills {
            user: USER.to_owned(),
            limit: 10,
        };
        assert_eq!(
            request.key(),
            format!(r#"info {{"limit":10,"type":"userFills","user":"{USER}"}}"#)
        );
    }
}
