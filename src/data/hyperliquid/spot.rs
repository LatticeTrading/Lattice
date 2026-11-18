//! A user's token balances ([`Info::SpotClearinghouseState`](super::Info)),
//! as Hyperliquid writes them and as the portfolio reads them
//! ([`Balances`]). Under Hyperliquid's unified account these are also what
//! backs the user's perpetuals.
//!
//! A token is named as Hyperliquid names it (`USDC`, `PURR`); one the user
//! holds none of is left out. `entryNtl` is what the holding cost, in USDC.

use serde::Deserialize;

use crate::data::{
    net::Figure,
    portfolio::{Balance, Balances},
};

/// The reply to [`Info::SpotClearinghouseState`](super::Info).
#[derive(Debug, Deserialize)]
pub(super) struct SpotClearinghouseState {
    #[serde(default)]
    balances: Vec<SpotBalance>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SpotBalance {
    coin: String,
    total: Figure,
    /// What open orders hold of it.
    hold: Option<Figure>,
    entry_ntl: Option<Figure>,
}

impl From<SpotClearinghouseState> for Balances {
    fn from(state: SpotClearinghouseState) -> Self {
        Self(
            state
                .balances
                .into_iter()
                .filter(|balance| balance.total.0 != 0.0)
                .map(|balance| Balance {
                    asset: balance.coin,
                    total: balance.total.0,
                    held: balance.hold.map_or(0.0, |Figure(held)| held),
                    entry_value: balance.entry_ntl.map(|Figure(value)| value),
                })
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn balances_read_as_the_tokens_held() {
        // The documented reply, with a token the user holds none of.
        let state: SpotClearinghouseState = serde_json::from_value(json!({
            "balances": [
                { "coin": "USDC", "token": 0, "hold": "4.5", "total": "14.625485",
                  "entryNtl": "0.0" },
                { "coin": "PURR", "token": 1, "hold": "0.0", "total": "2000.0",
                  "entryNtl": "1234.56" },
                { "coin": "HYPE", "token": 150, "hold": "0.0", "total": "0.0",
                  "entryNtl": "0.0" },
                { "coin": "+1230", "total": "25.0" }
            ]
        }))
        .expect("the documented reply reads");
        let Balances(balances) = state.into();
        let assets: Vec<&str> = balances
            .iter()
            .map(|balance| balance.asset.as_str())
            .collect();
        assert_eq!(assets, ["USDC", "PURR", "+1230"]);
        assert_eq!((balances[0].total, balances[0].held), (14.625_485, 4.5));
        assert!((balances[0].available() - 10.125_485).abs() < 1e-9);
        assert_eq!(balances[1].entry_value, Some(1234.56));
        // What Hyperliquid leaves out is nothing held and no cost known.
        assert_eq!((balances[2].held, balances[2].entry_value), (0.0, None));

        let empty: SpotClearinghouseState = serde_json::from_value(json!({})).unwrap();
        assert_eq!(Balances::from(empty), Balances::default());
    }
}
