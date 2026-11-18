//! The info endpoint: `POST /info` with a JSON body whose `type` names
//! what is asked. Every request here reads public data.
//!
//! A request has a weight, which Hyperliquid counts against the address's
//! limit ([`super::WEIGHT_LIMIT`] a minute): 2 for the two account states,
//! 20 for every other documented request. A request spends its weight from
//! the client's budget before it is sent, waiting while the minute is
//! spent; Hyperliquid refusing one all the same (`429`) spends what is left
//! of the minute and fails the request as `RATE_LIMITED`, which its query
//! retries once the budget has room.

use std::{rc::Rc, time::Duration};

use serde::{Serialize, de::DeserializeOwned};

use super::{Api, Hyperliquid};
use crate::data::net::{ApiError, QuerySpec, Subject, canonical_json};

/// The endpoint every [`Info`] request is posted to.
const PATH: &str = "/info";

/// One request of the info endpoint, which serializes as its body.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Info {
    /// A user's perpetuals account: its margin summary and open positions.
    ClearinghouseState {
        user: String,
        /// The perpetuals dex asked about; the first one (Hyperliquid's
        /// own) when left out.
        #[serde(skip_serializing_if = "Option::is_none")]
        dex: Option<String>,
    },
    /// A user's token balances.
    SpotClearinghouseState { user: String },
    /// What a user's account was worth and made over a day, a week, a
    /// month and all time, for the whole account and for its perpetuals.
    Portfolio { user: String },
    /// The spot markets and their tokens, with each market's prices now.
    SpotMetaAndAssetCtxs,
    /// What a user has staked.
    DelegatorSummary { user: String },
    /// The vaults a user has a share of.
    UserVaultEquities { user: String },
    /// What a user pays to trade: its rates, the fee schedule and its
    /// daily volume.
    UserFees { user: String },
    /// A user's latest fills, newest first (2000 at most).
    UserFills {
        user: String,
        /// Whether the fills one order made at one time come as one.
        #[serde(rename = "aggregateByTime")]
        aggregate_by_time: bool,
    },
}

impl Info {
    /// What the request costs of the address's limit, before any weight
    /// Hyperliquid adds per item returned (none of these has one).
    pub fn weight(&self) -> u32 {
        match self {
            Self::ClearinghouseState { .. } | Self::SpotClearinghouseState { .. } => 2,
            Self::Portfolio { .. }
            | Self::SpotMetaAndAssetCtxs
            | Self::DelegatorSummary { .. }
            | Self::UserVaultEquities { .. }
            | Self::UserFees { .. }
            | Self::UserFills { .. } => 20,
        }
    }

    /// What Hyperliquid adds to [`Self::weight`] for a reply of `items`
    /// items: 1 for every 20 a list request returns, nothing for the rest.
    pub fn item_weight(&self, items: usize) -> u32 {
        match self {
            Self::UserFills { .. } => u32::try_from(items / 20).unwrap_or(u32::MAX),
            _ => 0,
        }
    }

    /// What tells the request apart in the query hub: the endpoint and its
    /// body, members in one order.
    pub fn key(&self) -> String {
        let body = serde_json::to_value(self).unwrap_or_default();
        format!("info {}", canonical_json(&body))
    }
}

impl Api {
    /// Posts `request` once the budget has room for its weight, and reads
    /// the reply as `T`.
    pub(super) async fn info<T: DeserializeOwned>(&self, request: &Info) -> Result<T, ApiError> {
        self.budget.spend(request.weight()).await?;
        match self.http.post(PATH, request).await {
            Err(error) if error.status == Some(429) => {
                // Something else spends the address's limit too: nothing
                // more goes out until a minute has passed.
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
    /// A shared `request`, its reply read as `R` and turned into the value
    /// by `decode`, refetched every `refresh` while watched and named
    /// `subject` in the connection panel: what every polled service's
    /// query is.
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
        self.info_list_query(request, refresh, subject, |_: &R| 0, decode)
    }

    /// [`Self::info_query`] for a request Hyperliquid also charges per item
    /// returned ([`Info::item_weight`]): `items` counts a reply's, and what
    /// they weigh is charged to the budget once it has arrived.
    pub(super) fn info_list_query<R, T>(
        &self,
        request: Info,
        refresh: Duration,
        subject: Subject,
        items: impl Fn(&R) -> usize + 'static,
        decode: impl Fn(R) -> T + 'static,
    ) -> QuerySpec<T>
    where
        R: DeserializeOwned + 'static,
        T: 'static,
    {
        let api = self.api.clone();
        let key = request.key();
        let (items, decode) = (Rc::new(items), Rc::new(decode));
        QuerySpec::new(key, move || {
            let (api, request) = (api.clone(), request.clone());
            let (items, decode) = (items.clone(), decode.clone());
            async move {
                let reply = api.info::<R>(&request).await?;
                api.budget.charge(request.item_weight(items(&reply)));
                Ok(decode(reply))
            }
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
    fn requests_serialize_as_their_bodies_and_weigh_what_the_api_charges() {
        let state = Info::ClearinghouseState {
            user: USER.to_owned(),
            dex: None,
        };
        assert_eq!(
            serde_json::to_value(&state).unwrap(),
            json!({ "type": "clearinghouseState", "user": USER })
        );
        assert_eq!(state.weight(), 2);
        assert_eq!(
            state.key(),
            format!(r#"info {{"type":"clearinghouseState","user":"{USER}"}}"#)
        );

        // Another dex is another request.
        let dex = Info::ClearinghouseState {
            user: USER.to_owned(),
            dex: Some("xyz".to_owned()),
        };
        assert_eq!(
            serde_json::to_value(&dex).unwrap(),
            json!({ "type": "clearinghouseState", "user": USER, "dex": "xyz" })
        );
        assert_ne!(dex.key(), state.key());

        let spot = Info::SpotClearinghouseState {
            user: USER.to_owned(),
        };
        assert_eq!(
            serde_json::to_value(&spot).unwrap(),
            json!({ "type": "spotClearinghouseState", "user": USER })
        );
        assert_eq!(spot.weight(), 2);

        let portfolio = Info::Portfolio {
            user: USER.to_owned(),
        };
        assert_eq!(
            serde_json::to_value(&portfolio).unwrap(),
            json!({ "type": "portfolio", "user": USER })
        );
        assert_eq!(portfolio.weight(), 20);
    }
}
