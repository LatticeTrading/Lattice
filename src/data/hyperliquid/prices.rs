//! What Hyperliquid's tokens are worth
//! ([`Info::SpotMetaAndAssetCtxs`](super::Info)), as the portfolio reads it
//! ([`Prices`]).
//!
//! The reply is a pair: the spot meta (the tokens, each with its index, and
//! the markets, each naming its base and quote tokens by index) and a
//! context per market, which names its market (`PURR/USDC`, `@107`) and
//! has its mark and mid prices. A token is priced by its market against
//! USDC, at the mark price (the mid when there is no mark); one with no
//! such market is left out. USDC is `1.0`.

use std::collections::HashMap;

use serde::Deserialize;

use crate::data::{net::Figure, portfolio::Prices};

/// The token every price is in.
const QUOTE: &str = "USDC";

/// The reply to [`Info::SpotMetaAndAssetCtxs`](super::Info).
#[derive(Debug, Deserialize)]
pub(super) struct SpotMetaAndAssetCtxs(SpotMeta, Vec<MarketContext>);

#[derive(Debug, Deserialize)]
struct SpotMeta {
    #[serde(default)]
    tokens: Vec<Token>,
    #[serde(default)]
    universe: Vec<Market>,
}

#[derive(Debug, Deserialize)]
struct Token {
    name: String,
    index: u32,
}

#[derive(Debug, Deserialize)]
struct Market {
    name: String,
    /// The base token's index, then the quote's.
    tokens: Vec<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MarketContext {
    coin: String,
    mark_px: Option<Figure>,
    mid_px: Option<Figure>,
}

impl From<SpotMetaAndAssetCtxs> for Prices {
    fn from(SpotMetaAndAssetCtxs(meta, contexts): SpotMetaAndAssetCtxs) -> Self {
        let names: HashMap<u32, &str> = meta
            .tokens
            .iter()
            .map(|token| (token.index, token.name.as_str()))
            .collect();
        let Some(quote) = meta
            .tokens
            .iter()
            .find(|token| token.name == QUOTE)
            .map(|token| token.index)
        else {
            return Self::default();
        };
        let priced: HashMap<&str, f64> = contexts
            .iter()
            .filter_map(|context| {
                let Figure(price) = context.mark_px.or(context.mid_px)?;
                (price > 0.0).then_some((context.coin.as_str(), price))
            })
            .collect();
        let mut prices = HashMap::from([(QUOTE.to_owned(), 1.0)]);
        for market in &meta.universe {
            let [base, of] = market.tokens[..] else {
                continue;
            };
            let (Some(name), Some(price)) = (names.get(&base), priced.get(market.name.as_str()))
            else {
                continue;
            };
            if of == quote {
                // The first market of a token is its own; later ones are
                // other listings of it.
                prices.entry((*name).to_owned()).or_insert(*price);
            }
        }
        Self(prices)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn tokens_are_priced_by_their_market_against_usdc() {
        let reply: SpotMetaAndAssetCtxs = serde_json::from_value(json!([
            {
                "tokens": [
                    { "name": "USDC", "index": 0, "szDecimals": 8 },
                    { "name": "PURR", "index": 1 },
                    { "name": "HYPE", "index": 150 },
                    { "name": "USDT0", "index": 268 },
                    { "name": "LONE", "index": 300 }
                ],
                "universe": [
                    { "tokens": [1, 0], "name": "PURR/USDC", "index": 0, "isCanonical": true },
                    { "tokens": [150, 0], "name": "@107", "index": 107 },
                    { "tokens": [150, 268], "name": "@207", "index": 207 },
                    { "tokens": [268, 0], "name": "@166", "index": 166 },
                    { "tokens": [300, 268], "name": "@400", "index": 400 }
                ]
            },
            [
                { "coin": "PURR/USDC", "markPx": "0.089344", "midPx": "0.0892685" },
                { "coin": "@107", "markPx": "68.853", "midPx": "68.8565" },
                { "coin": "@207", "markPx": "69.066", "midPx": null },
                { "coin": "@166", "markPx": null, "midPx": "0.99915" },
                { "coin": "@400", "markPx": "2.0" }
            ]
        ]))
        .expect("the documented reply reads");
        let prices = Prices::from(reply);
        assert_eq!(prices.of("USDC"), Some(1.0));
        assert_eq!(prices.of("PURR"), Some(0.089_344));
        // Against USDC, not another stablecoin; the mid without a mark.
        assert_eq!(prices.of("HYPE"), Some(68.853));
        assert_eq!(prices.of("USDT0"), Some(0.999_15));
        // No market against USDC: not priced.
        assert_eq!(prices.of("LONE"), None);
        assert_eq!(prices.0.len(), 4);
    }
}
