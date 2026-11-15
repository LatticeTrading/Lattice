//! The Price Delta Monitor's settings, saved in its tab's record under the
//! old app's names: `baseMarket` (the market the others are measured
//! against) and `comparisonMarkets` (the markets compared with it).
//!
//! Both are left out of the saved form while they hold their defaults
//! (Hyperliquid's BTC/USDC perpetual, the old app's base, against Bybit's
//! and Binance's BTC/USDT perpetuals) and read leniently:
//!
//! - a base that is missing or cannot be read is the default one: the
//!   widget always has a base;
//! - a comparison that cannot be read is dropped and a repeated one kept
//!   once; an empty list is a choice and is kept; what is no list at all
//!   is the default;
//! - the base is never among the comparisons: a compared market made the
//!   base is no longer compared.
//!
//! The old app's retired forms are not read: a bare `exchange` and
//! `symbol` for the base, and without a list a comparison for every
//! enabled exchange (its `compare:<exchange>` switches), each guessed from
//! the base's coin and resolved against that exchange's catalog.
//!
//! A market is told apart by its identity ([`MarketSelection::identity`]).

use std::collections::HashSet;

use serde::Serialize;
use serde_json::Value;

use crate::data::ferris::{MarketKey, MarketSelection};

/// The base until told otherwise (the old app's `DEFAULT_BASE_MARKET`):
/// Hyperliquid's BTC/USDC perpetual.
fn default_base() -> MarketSelection {
    MarketSelection {
        exchange: "hyperliquid".into(),
        symbol: "BTC/USDC".into(),
        quote: "USDC".into(),
        request_symbol: "BTC/USDC:USDC".into(),
        exchange_symbol: "BTC/USDC:USDC".into(),
        ..MarketSelection::bybit_btc_usdt()
    }
}

/// The markets compared until told otherwise: the app's two default
/// markets, Bybit's and Binance's BTC/USDT perpetuals.
fn default_comparisons() -> Vec<MarketSelection> {
    vec![
        MarketSelection::bybit_btc_usdt(),
        MarketSelection::binance_btc_usdt(),
    ]
}

/// One Price Delta Monitor's settings.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub(crate) struct PriceDeltaSettings {
    /// `None` is the default base. Boxed: a selection is several times
    /// the size of the rest.
    #[serde(rename = "baseMarket", skip_serializing_if = "Option::is_none")]
    base: Option<Box<MarketSelection>>,
    /// The markets compared, each once and never the base, in the order
    /// they were added; `None` is the default ones.
    #[serde(rename = "comparisonMarkets", skip_serializing_if = "Option::is_none")]
    comparisons: Option<Vec<MarketSelection>>,
}

impl From<Value> for PriceDeltaSettings {
    fn from(value: Value) -> Self {
        let mut settings = Self::default();
        let Value::Object(members) = value else {
            return settings;
        };
        if let Some(base) = members
            .get("baseMarket")
            .and_then(MarketSelection::from_value)
        {
            settings = settings.with_base(base);
        }
        if let Some(Value::Array(list)) = members.get("comparisonMarkets") {
            let markets = list.iter().filter_map(MarketSelection::from_value);
            settings = settings.with_comparisons(markets.collect());
        }
        settings
    }
}

impl PriceDeltaSettings {
    /// The market the others are measured against.
    pub fn base(&self) -> MarketSelection {
        self.base.as_deref().cloned().unwrap_or_else(default_base)
    }

    /// The markets compared with the base, in the order they were added.
    pub fn comparisons(&self) -> Vec<MarketSelection> {
        self.comparisons.clone().unwrap_or_else(default_comparisons)
    }

    /// The compared market `key` names, if it is compared.
    pub fn comparison(&self, key: &MarketKey) -> Option<MarketSelection> {
        self.comparisons()
            .into_iter()
            .find(|kept| kept.identity() == *key)
    }

    /// These settings measuring against `market`, which is no longer
    /// compared if it was.
    pub fn with_base(mut self, market: MarketSelection) -> Self {
        let compared = self.comparisons();
        self.base = (market != default_base()).then(|| Box::new(market));
        self.with_comparisons(compared)
    }

    /// These settings comparing `markets`: each once, and not the base.
    fn with_comparisons(mut self, markets: Vec<MarketSelection>) -> Self {
        let mut seen = HashSet::from([self.base().identity()]);
        let kept: Vec<MarketSelection> = markets
            .into_iter()
            .filter(|market| seen.insert(market.identity()))
            .collect();
        self.comparisons = (kept != default_comparisons()).then_some(kept);
        self
    }

    /// These settings with `market` compared last; the same settings when
    /// it is the base or compared already, as the old widget left them.
    pub fn adding(self, market: MarketSelection) -> Self {
        let mut markets = self.comparisons();
        markets.push(market);
        self.with_comparisons(markets)
    }

    /// These settings with `market` in place of the compared market
    /// `replaced` names; the same settings when that one is gone, or
    /// `market` is the base or compared already.
    pub fn replacing(self, replaced: &MarketKey, market: MarketSelection) -> Self {
        let mut markets = self.comparisons();
        let keys: Vec<MarketKey> = markets.iter().map(MarketSelection::identity).collect();
        let picked = market.identity();
        let taken = picked == self.base().identity() || keys.contains(&picked);
        match keys.iter().position(|key| key == replaced) {
            Some(at) if !taken => markets[at] = market,
            _ => return self,
        }
        self.with_comparisons(markets)
    }

    /// These settings no longer comparing the market `key` names.
    pub fn without(self, key: &MarketKey) -> Self {
        let markets = self
            .comparisons()
            .into_iter()
            .filter(|kept| kept.identity() != *key)
            .collect();
        self.with_comparisons(markets)
    }
}
