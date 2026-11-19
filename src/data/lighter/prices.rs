//! What Lighter's assets are worth now
//! ([`Request::AssetDetails`](super::Request)), as Lighter writes it and as
//! the portfolio reads it ([`Prices`]).
//!
//! The reply lists every asset Lighter holds with its index price in US
//! dollars (`USDC` itself at `1`), by the name an account's assets go by.
//! An asset with no price is left out.

use serde::Deserialize;

use super::state::{figure, list};
use crate::data::portfolio::Prices;

/// The reply to [`Request::AssetDetails`](super::Request).
#[derive(Debug, Default, Deserialize)]
pub(super) struct AssetDetails {
    #[serde(default, deserialize_with = "list")]
    asset_details: Vec<AssetDetail>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct AssetDetail {
    symbol: String,
    #[serde(deserialize_with = "figure")]
    index_price: Option<f64>,
}

impl From<AssetDetails> for Prices {
    fn from(reply: AssetDetails) -> Self {
        Self(
            reply
                .asset_details
                .into_iter()
                .filter_map(|asset| {
                    let price = asset.index_price.filter(|price| *price > 0.0)?;
                    Some((asset.symbol, price))
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
    fn assets_are_priced_by_their_index_price() {
        let reply: AssetDetails = serde_json::from_value(json!({
            "code": 200,
            "asset_details": [
                { "asset_id": 3, "symbol": "USDC", "index_price": "1.000000", "decimals": 6 },
                { "asset_id": 2, "symbol": "LIT", "index_price": "2.727684" },
                { "asset_id": 10, "symbol": "AZTEC", "index_price": "0.000000" },
                { "asset_id": 11, "symbol": "NEW", "index_price": "" },
                { "asset_id": 12, "symbol": "BARE" },
            ],
        }))
        .expect("the documented reply reads");
        let prices = Prices::from(reply);
        assert_eq!(prices.of("USDC"), Some(1.0));
        assert_eq!(prices.of("LIT"), Some(2.727_684));
        // An asset with no price is not priced at nothing.
        assert_eq!(prices.0.len(), 2);

        let none: AssetDetails = serde_json::from_value(json!({ "code": 200 })).unwrap();
        assert_eq!(Prices::from(none), Prices::default());
    }
}
