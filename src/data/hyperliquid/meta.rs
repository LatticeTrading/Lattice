//! What Hyperliquid trades and at what price now: the perpetuals of its
//! own dex ([`Info::Meta`](super::Info), the "universe") and the mid price
//! of everything ([`Info::AllMids`](super::Info)), as Hyperliquid writes
//! them: the first as trading reads it ([`Instruments`]), the second for
//! Hyperliquid's own use ([`Mids`]: what a market order is priced from).
//!
//! An order names its asset by number, and a perpetual's number is its
//! place in the universe. So the [`Instruments`] read from it keep every
//! entry in Hyperliquid's order, the delisted ones too (not tradable):
//! an instrument's place there is its asset's index.
//!
//! Not read yet: the spot markets (their assets are numbered from 10,000,
//! by `spotMeta`) and the builder-deployed perpetuals dexes (from 100,000,
//! a `meta` per `dex`).

use std::collections::HashMap;

use serde::Deserialize;
use serde_json::Value;

use crate::data::{
    net::Figure,
    trading::{Instrument, Instruments},
};

/// The reply to [`Info::Meta`](super::Info).
#[derive(Debug, Deserialize)]
pub(super) struct Meta {
    #[serde(default)]
    universe: Vec<Asset>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Asset {
    name: String,
    /// The decimal places of a size.
    #[serde(default)]
    sz_decimals: u32,
    max_leverage: Option<u32>,
    #[serde(default)]
    is_delisted: bool,
    /// Traded with a position's own margin only.
    #[serde(default)]
    only_isolated: bool,
}

impl From<Meta> for Instruments {
    fn from(meta: Meta) -> Self {
        Self(
            meta.universe
                .into_iter()
                .map(|asset| Instrument {
                    symbol: asset.name,
                    size_decimals: asset.sz_decimals,
                    max_leverage: asset.max_leverage,
                    tradable: !asset.is_delisted,
                    isolated_only: asset.only_isolated,
                })
                .collect(),
        )
    }
}

/// The reply to [`Info::AllMids`](super::Info): a mid price in decimal
/// text by name. What is no price is left out, not the whole reply.
#[derive(Debug, Deserialize)]
pub(super) struct AllMids(HashMap<String, Value>);

/// The mid price of everything Hyperliquid trades, by name: what a market
/// order is priced from, asked for when one is placed.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Mids(HashMap<String, f64>);

impl Mids {
    /// The mid price of `coin`.
    pub(super) fn of(&self, coin: &str) -> Option<f64> {
        self.0.get(coin).copied()
    }
}

impl From<AllMids> for Mids {
    fn from(mids: AllMids) -> Self {
        Self(
            mids.0
                .into_iter()
                .filter_map(|(name, mid)| Some((name, Figure::of(&mid)?.0)))
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn the_universe_reads_in_its_order_with_what_was_delisted() {
        // As Hyperliquid answered on 2026-11-22, cut to four.
        let meta: Meta = serde_json::from_value(json!({
            "universe": [
                { "szDecimals": 5, "name": "BTC", "maxLeverage": 40, "marginTableId": 56 },
                { "szDecimals": 4, "name": "ETH", "maxLeverage": 25, "marginTableId": 55 },
                { "szDecimals": 1, "name": "MATIC", "maxLeverage": 20, "isDelisted": true },
                { "szDecimals": 0, "name": "kPEPE", "maxLeverage": 10, "marginTableId": 52 },
                { "szDecimals": 3, "name": "UNIBOT", "maxLeverage": 3, "marginTableId": 3,
                  "onlyIsolated": true, "isDelisted": true, "marginMode": "strictIsolated" },
            ],
            "marginTables": [],
            "collateralToken": 0,
        }))
        .unwrap();
        let instruments = Instruments::from(meta);
        // A place in the list is the asset's index: delisted ones keep
        // theirs.
        let places: Vec<&str> = instruments
            .0
            .iter()
            .map(|listed| listed.symbol.as_str())
            .collect();
        assert_eq!(places, ["BTC", "ETH", "MATIC", "kPEPE", "UNIBOT"]);
        let pepe = instruments.find("kPEPE").unwrap();
        assert_eq!((pepe.size_decimals, pepe.max_leverage), (0, Some(10)));
        assert!(!instruments.find("MATIC").unwrap().tradable);
        assert!(instruments.find("BTC").unwrap().tradable);
        // What is traded with a position's own margin only says so.
        assert!(instruments.find("UNIBOT").unwrap().isolated_only);
        assert!(!instruments.find("BTC").unwrap().isolated_only);

        // Nothing listed reads as nothing, not as a failure.
        let empty: Meta = serde_json::from_value(json!({})).unwrap();
        assert!(Instruments::from(empty).0.is_empty());
    }

    #[test]
    fn mids_read_by_name_and_skip_what_is_no_price() {
        let mids: AllMids = serde_json::from_value(json!({
            "BTC": "61012.5",
            "@107": "22.4",
            "ODD": null,
        }))
        .unwrap();
        let mids = Mids::from(mids);
        assert_eq!(mids.of("BTC"), Some(61_012.5));
        assert_eq!(mids.of("@107"), Some(22.4));
        assert_eq!(mids.of("ODD"), None);
        assert_eq!(mids.of("ETH"), None);
    }
}
