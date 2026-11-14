//! The saved selection: a market's type and the selection a widget keeps,
//! read leniently from its saved form, with its identity and tags.

use std::{collections::HashSet, rc::Rc};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::expiry::{expiry_label, settlement_of};
use crate::data::{
    ferris::{Channel, FerrisTopic, Product},
    net::ApiError,
};

/// A market's product type, under the old app's names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MarketType {
    Spot,
    Future,
    Perp,
    Option,
}

impl MarketType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Spot => "spot",
            Self::Future => "future",
            Self::Perp => "perp",
            Self::Option => "option",
        }
    }

    /// The capabilities' name for it.
    pub fn product(self) -> Product {
        match self {
            Self::Spot => Product::Spot,
            Self::Future => Product::Future,
            Self::Perp => Product::Perp,
            Self::Option => Product::Option,
        }
    }

    /// `PERP`, as the market's tags show it.
    pub fn label(self) -> &'static str {
        match self {
            Self::Spot => "SPOT",
            Self::Future => "FUTURE",
            Self::Perp => "PERP",
            Self::Option => "OPTION",
        }
    }

    fn parse(name: &str) -> Option<Self> {
        match name {
            "spot" => Some(Self::Spot),
            "future" => Some(Self::Future),
            "perp" => Some(Self::Perp),
            "option" => Some(Self::Option),
            _ => None,
        }
    }
}

/// A selection's identity ([`MarketSelection::identity`]): made once per
/// market, then cheap to clone, compare and hash.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MarketKey(Rc<str>);

/// One market of one exchange, as a widget's settings keep it.
/// Saved with `Serialize`; read with [`Self::from_value`], which is lenient.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketSelection {
    /// The exchange id, lower case (`bybit`).
    pub exchange: String,
    /// The display pair (`BTC/USDT`).
    pub symbol: String,
    pub base: String,
    pub quote: String,
    /// What Ferris is asked for (`BTC/USDT:USDT`).
    pub request_symbol: String,
    /// The venue's own name for it (`BTCUSDT`).
    pub exchange_symbol: String,
    #[serde(rename = "type")]
    pub kind: MarketType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expiry: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub market_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exchange_market_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dex: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contract_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub settle: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub settlement_asset_id: Option<String>,
}

impl MarketSelection {
    /// The old app's default market (`DEFAULT_BYBIT_BTC_USDT_MARKET`):
    /// Bybit's BTC/USDT linear perpetual.
    pub fn bybit_btc_usdt() -> Self {
        Self {
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

    /// A saved selection, read leniently: the exchange, display pair,
    /// request symbol and a known type are required (the old app's
    /// `sanitizeSavedMarketSelection` cleared a selection without them);
    /// base, quote and the venue symbol fall back to the pair; optional
    /// members of the wrong type are left out.
    pub fn from_value(value: &Value) -> Option<Self> {
        let record = value.as_object()?;
        let text = |name: &str| {
            record
                .get(name)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(str::to_owned)
        };
        let exchange = text("exchange")?.to_ascii_lowercase();
        let symbol = text("symbol")?;
        let request_symbol = text("requestSymbol")?;
        let kind = MarketType::parse(record.get("type")?.as_str()?)?;
        let pair = symbol.split(':').next().unwrap_or_default();
        let (pair_base, pair_quote) = pair.split_once('/').unwrap_or((pair, "USD"));
        Some(Self {
            base: text("base").unwrap_or_else(|| pair_base.to_ascii_uppercase()),
            quote: text("quote").unwrap_or_else(|| pair_quote.to_ascii_uppercase()),
            exchange_symbol: text("exchangeSymbol").unwrap_or_else(|| symbol.clone()),
            category: text("category"),
            expiry: text("expiry"),
            market_id: text("marketId"),
            exchange_market_id: text("exchangeMarketId"),
            dex: text("dex"),
            contract_type: text("contractType"),
            settle: text("settle"),
            settlement_asset_id: text("settlementAssetId"),
            exchange,
            symbol,
            request_symbol,
            kind,
        })
    }

    /// The readable selections of a saved list, each once (by
    /// [`Self::identity`]), in order.
    pub fn read_list(values: &[Value]) -> Vec<Self> {
        Self::dedup(values.iter().filter_map(Self::from_value))
    }

    /// `markets` in order, each once (by [`Self::identity`]): a repeat is
    /// dropped and the first kept where it was. Each identity is made once.
    pub fn dedup(markets: impl IntoIterator<Item = Self>) -> Vec<Self> {
        let mut seen = HashSet::new();
        markets
            .into_iter()
            .filter(|market| seen.insert(market.identity()))
            .collect()
    }

    /// What tells two selections apart: the same market and the same
    /// topics are the same selection, however it was saved (the old app's
    /// `createTradeStreamMarketKey`). Making it builds JSON and two topics,
    /// so callers make it once per market and compare the keys.
    pub fn identity(&self) -> MarketKey {
        let key = |topic: Result<FerrisTopic, ApiError>| {
            topic.map(|topic| topic.key()).unwrap_or_default()
        };
        let text = |value: &Option<String>| value.clone().unwrap_or_default();
        let identity = json!([
            self.kind.as_str(),
            text(&self.market_id),
            text(&self.exchange_market_id),
            text(&self.contract_type),
            text(&self.settle),
            text(&self.expiry),
            text(&self.dex),
            text(&self.settlement_asset_id),
            key(self.topic(Channel::Trades)),
            key(self.rest_topic(Channel::Trades)),
        ])
        .to_string();
        MarketKey(identity.into())
    }

    /// `PERP USDT-M`: its type, and its settlement or expiry when it has
    /// one (the old app's market tags).
    pub fn tags(&self) -> String {
        match self.detail() {
            Some(detail) => format!("{} {detail}", self.kind.label()),
            None => self.kind.label().to_owned(),
        }
    }

    /// What its tags show after its type: its settlement (`USDT-M`) or
    /// expiry (`25 DEC 2026`), if any (the old app's `getMarketDetailLabel`).
    pub fn detail(&self) -> Option<String> {
        match self.kind {
            MarketType::Spot => None,
            MarketType::Future => Some(
                self.expiry
                    .as_deref()
                    .and_then(expiry_label)
                    .unwrap_or_else(|| self.exchange_symbol.clone()),
            ),
            MarketType::Perp | MarketType::Option => {
                let settlement = settlement_of(&self.request_symbol);
                match settlement.as_deref() {
                    Some("USDT") => Some("USDT-M".into()),
                    Some("USDC") => Some("USDC-M".into()),
                    Some("AUSD") => Some("AUSD-M".into()),
                    Some("USD") | None => match self.category.as_deref() {
                        Some("linear") => Some("LINEAR".into()),
                        Some("inverse") => Some("INVERSE".into()),
                        _ => None,
                    },
                    Some(_) => Some("COIN-M".into()),
                }
            }
        }
    }

    /// Whether `other` is the same market: by catalog id when both have
    /// one, else by [`Self::route_key`] (how the old symbol search marked
    /// the current selection).
    pub fn same_market(&self, other: &Self) -> bool {
        match (&self.market_id, &other.market_id) {
            (Some(mine), Some(theirs)) => self.exchange == other.exchange && mine == theirs,
            _ => self.route_key() == other.route_key(),
        }
    }
}
