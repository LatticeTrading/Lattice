//! How an account is set to trade a perpetual
//! ([`Info::ActiveAssetData`](super::Info)), as Hyperliquid writes it and as
//! trading reads it ([`Leverage`]): its leverage there and whether what
//! backs a position is the whole account's margin (`cross`) or the
//! position's own (`isolated`).
//!
//! It is a setting, so it is answered for an account that holds none of
//! the asset too, and for one that never traded (the asset's defaults). An
//! asset Hyperliquid does not list is answered `null`, which reads as no
//! setting.
//!
//! The reply also says how much the account could buy and sell of the
//! asset now (`maxTradeSzs`, `availableToTrade`) and its mark price;
//! nothing reads those yet.

use serde::Deserialize;

use super::perps::LeverageState;
use crate::data::portfolio::Leverage;

/// The reply to [`Info::ActiveAssetData`](super::Info); `None` for an asset
/// that is not listed.
#[derive(Debug, Deserialize)]
pub(super) struct ActiveAssetData(Option<Setting>);

#[derive(Debug, Deserialize)]
struct Setting {
    leverage: Option<LeverageState>,
}

impl From<ActiveAssetData> for Option<Leverage> {
    fn from(ActiveAssetData(setting): ActiveAssetData) -> Self {
        setting?.leverage.map(Leverage::from)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::data::portfolio::MarginMode;

    #[test]
    fn a_setting_reads_as_its_leverage_and_whose_margin_backs_it() {
        // As Hyperliquid answered on 2026-11-22.
        let cross: ActiveAssetData = serde_json::from_value(json!({
            "user": "0xa62b923a112d50d03e1e096bbd53422490dac104", "coin": "BTC",
            "leverage": { "type": "cross", "value": 5 },
            "maxTradeSzs": ["3.1896", "3.51586"],
            "availableToTrade": ["38369.61216", "42294.389456"],
            "markPx": "60148.0"
        }))
        .unwrap();
        assert_eq!(
            Option::<Leverage>::from(cross),
            Some(Leverage {
                value: 5.0,
                mode: MarginMode::Cross,
            })
        );
        // A position's own margin, with what it holds.
        let isolated: ActiveAssetData = serde_json::from_value(json!({
            "coin": "ETH", "leverage": { "type": "isolated", "value": 20, "rawUsd": "-2.3" }
        }))
        .unwrap();
        assert_eq!(
            Option::<Leverage>::from(isolated),
            Some(Leverage {
                value: 20.0,
                mode: MarginMode::Isolated,
            })
        );
        // An asset that is not listed, and a reply that names no setting.
        for none in [json!(null), json!({ "coin": "BTC" })] {
            let reply: ActiveAssetData = serde_json::from_value(none).unwrap();
            assert_eq!(Option::<Leverage>::from(reply), None);
        }
    }
}
