//! The Trade Stream's settings, saved in its tab's record under the old
//! app's names: `markets` (the markets watched) and `maxTrades` (how many
//! prints show, 10-200).
//!
//! Both are left out of the saved form while they hold their defaults
//! (Bybit BTC/USDT perpetual; 50), and read leniently: an unreadable
//! market is dropped, a repeated one kept once, a count out of range
//! clamped, a member of the wrong type read as its default. An explicit
//! empty list is kept: the stream then asks for a market.

use serde::Serialize;
use serde_json::Value;

use crate::data::ferris::{MarketSelection, MarketType};

/// The fewest and most prints the stream shows, and its default.
pub(super) const MIN_TRADES: u16 = 10;
pub(super) const MAX_TRADES: u16 = 200;
const DEFAULT_TRADES: u16 = 50;

/// The old app's default Trade Stream market: Bybit's BTC/USDT linear
/// perpetual.
fn default_market() -> MarketSelection {
    MarketSelection {
        exchange: "bybit".into(),
        symbol: "BTC/USDT".into(),
        base: "BTC".into(),
        quote: "USDT".into(),
        request_symbol: "BTC/USDT:USDT".into(),
        exchange_symbol: "BTCUSDT".into(),
        kind: MarketType::Perp,
        category: Some("linear".into()),
        expiry: None,
        market_id: None,
        exchange_market_id: None,
        dex: None,
        contract_type: None,
        settle: None,
        settlement_asset_id: None,
    }
}

/// One Trade Stream's settings.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub(crate) struct TradeStreamSettings {
    /// The markets watched, each once; `None` is the default market.
    #[serde(skip_serializing_if = "Option::is_none")]
    markets: Option<Vec<MarketSelection>>,
    /// How many prints show; `None` is the default.
    #[serde(rename = "maxTrades", skip_serializing_if = "Option::is_none")]
    max_trades: Option<u16>,
}

impl From<Value> for TradeStreamSettings {
    fn from(value: Value) -> Self {
        let mut settings = Self::default();
        let Value::Object(members) = value else {
            return settings;
        };
        if let Some(Value::Array(list)) = members.get("markets") {
            let markets = MarketSelection::read_list(list);
            // An empty list is a choice; one naming no readable market
            // reads as the default.
            if list.is_empty() || !markets.is_empty() {
                settings = settings.with_markets(markets);
            }
        }
        if let Some(count) = members.get("maxTrades").and_then(Value::as_f64)
            && count.is_finite()
        {
            settings =
                settings.with_max_trades(count.floor().clamp(0.0, f64::from(u16::MAX)) as u16);
        }
        settings
    }
}

impl TradeStreamSettings {
    /// The markets watched, in order.
    pub fn markets(&self) -> Vec<MarketSelection> {
        self.markets
            .clone()
            .unwrap_or_else(|| vec![default_market()])
    }

    /// How many prints show.
    pub fn max_trades(&self) -> usize {
        usize::from(self.max_trades.unwrap_or(DEFAULT_TRADES))
    }

    /// These settings watching `markets` (each once).
    pub fn with_markets(mut self, markets: Vec<MarketSelection>) -> Self {
        let kept = MarketSelection::dedup(markets);
        self.markets = (kept != [default_market()]).then_some(kept);
        self
    }

    /// These settings with `market` watched last, unless it already is.
    pub fn adding(self, market: MarketSelection) -> Self {
        let mut markets = self.markets();
        markets.push(market);
        self.with_markets(markets)
    }

    /// These settings with `market` in place of `replaced` (added last
    /// when `replaced` is not watched). A market already watched is kept
    /// once, where it was.
    pub fn replacing(self, replaced: &MarketSelection, market: MarketSelection) -> Self {
        let identity = replaced.identity();
        let mut markets = self.markets();
        match markets.iter().position(|kept| kept.identity() == identity) {
            Some(at) => markets[at] = market,
            None => markets.push(market),
        }
        self.with_markets(markets)
    }

    /// These settings no longer watching `market` (the last one too).
    pub fn without(self, market: &MarketSelection) -> Self {
        let identity = market.identity();
        let markets = self
            .markets()
            .into_iter()
            .filter(|kept| kept.identity() != identity)
            .collect();
        self.with_markets(markets)
    }

    /// These settings showing `count` prints, within 10-200.
    pub fn with_max_trades(mut self, count: u16) -> Self {
        let count = count.clamp(MIN_TRADES, MAX_TRADES);
        self.max_trades = (count != DEFAULT_TRADES).then_some(count);
        self
    }
}
