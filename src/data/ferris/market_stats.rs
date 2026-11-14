//! Market statistics: `POST /v1/fetchMarketStats` returns every market of
//! one exchange scope with its prices, funding, 24h volume and open
//! interest, each field with its own state and timestamps.
//!
//! - What an exchange serves is in its capabilities' `marketStats`
//!   ([`ExchangeCapabilities::market_stats`]): the fields each product
//!   supports, the poll cadence and how long a value stays fresh.
//! - A *partition* is one complete request profile of an exchange (Bybit's
//!   linear perpetuals, its inverse futures, ...): the backend will list
//!   them in `marketStats.partitions`; until it does, they come from the
//!   venue's [`stats_profiles`](super::Venue::stats_profiles), ported from
//!   the old app's `marketPartitions.ts`.
//! - [`stats_scopes`] turns the capabilities into the scopes a consumer
//!   polls, and [`super::Ferris::market_stats_query`] into one shared query
//!   per scope, so every widget asking for a scope shares one request.
//!
//! Decimal values (prices, funding rates) stay the exact strings the API
//! sends ([`Decimal`]); volumes and open interest are JSON numbers.

use std::{collections::HashMap, rc::Rc, time::Duration};

use serde::{Deserialize, Deserializer};
use serde_json::{Map, Value};

use super::{Capabilities, CapabilityState, ExchangeCapabilities, Product, Venue};
use crate::data::net::canonical_json;

/// A statistics field, as the API names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StatsFieldName {
    Funding,
    IndexPrice,
    LastPrice,
    LastSettledFunding,
    MarkPrice,
    OpenInterest,
    Volume24h,
}

impl StatsFieldName {
    /// Every field, in the API's (alphabetical) order.
    pub const ALL: [Self; 7] = [
        Self::Funding,
        Self::IndexPrice,
        Self::LastPrice,
        Self::LastSettledFunding,
        Self::MarkPrice,
        Self::OpenInterest,
        Self::Volume24h,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Funding => "funding",
            Self::IndexPrice => "indexPrice",
            Self::LastPrice => "lastPrice",
            Self::LastSettledFunding => "lastSettledFunding",
            Self::MarkPrice => "markPrice",
            Self::OpenInterest => "openInterest",
            Self::Volume24h => "volume24h",
        }
    }
}

impl Product {
    /// The API's name for it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Perp => "perp",
            Self::Future => "future",
            Self::Spot => "spot",
            Self::Option => "option",
            Self::Other => "other",
        }
    }
}

// Capabilities.

/// An exchange's statistics contract (`marketStats` when supported).
#[derive(Clone, Debug, PartialEq)]
pub struct StatsCapability {
    /// The primary scope.
    pub scope: StatsScope,
    /// Every scope the exchange can serve; the primary one when not listed.
    pub supported_scopes: Vec<StatsScope>,
    /// The backend's own request profiles, once it sends them.
    pub partitions: Option<Vec<Partition>>,
    /// How often the backend refreshes the statistics.
    pub poll_interval: Duration,
    /// How long after its receipt a value counts as stale, in milliseconds.
    pub stale_after_ms: f64,
    /// Field support by product name, then field name.
    fields: HashMap<String, HashMap<String, FieldSupport>>,
}

/// Whether a product's field is served.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct FieldSupport {
    pub state: CapabilityState,
    #[serde(default)]
    pub reason: Option<String>,
}

/// An exchange and the request parameters of one of its scopes.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct StatsScope {
    pub exchange: String,
    #[serde(default)]
    pub params: Map<String, Value>,
}

/// `marketStats` as sent, before it is checked.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawCapability {
    state: CapabilityState,
    scope: StatsScope,
    #[serde(default)]
    supported_scopes: Option<Vec<StatsScope>>,
    #[serde(default)]
    partitions: Option<Vec<Partition>>,
    #[serde(default)]
    fields: HashMap<String, HashMap<String, FieldSupport>>,
    poll_interval_ms: u64,
    stale_after_ms: u64,
}

impl ExchangeCapabilities {
    /// Its statistics contract, when the exchange serves statistics.
    /// `None` when it does not, or when the contract cannot be read
    /// (logged): a scope with no usable cadence is not offered.
    pub fn market_stats(&self) -> Option<StatsCapability> {
        let state = self.market_stats.get("state").and_then(Value::as_str);
        if state != Some("supported") {
            return None;
        }
        let raw = match RawCapability::deserialize(&self.market_stats) {
            Ok(raw) => raw,
            Err(error) => {
                log::warn!(
                    "Ferris capabilities: unreadable marketStats for `{}`: {error}",
                    self.exchange
                );
                return None;
            }
        };
        if raw.state != CapabilityState::Supported
            || raw.poll_interval_ms == 0
            || raw.stale_after_ms == 0
        {
            log::warn!(
                "Ferris capabilities: `{}` statistics need positive poll and stale intervals",
                self.exchange
            );
            return None;
        }
        let supported_scopes = raw
            .supported_scopes
            .filter(|scopes| !scopes.is_empty())
            .unwrap_or_else(|| vec![raw.scope.clone()]);
        Some(StatsCapability {
            scope: raw.scope,
            supported_scopes,
            partitions: raw.partitions,
            poll_interval: Duration::from_millis(raw.poll_interval_ms),
            stale_after_ms: raw.stale_after_ms as f64,
            fields: raw.fields,
        })
    }
}

impl StatsCapability {
    /// Whether `product`'s `field` is served (`None`: not described).
    pub fn field(&self, product: Product, field: StatsFieldName) -> Option<&FieldSupport> {
        self.fields.get(product.as_str())?.get(field.as_str())
    }

    /// The fields of `wanted` that `exchange` serves for every market of a
    /// `product` scope at once.
    pub fn bulk_fields(
        &self,
        exchange: &str,
        product: Product,
        wanted: &[StatsFieldName],
    ) -> Vec<StatsFieldName> {
        wanted
            .iter()
            .copied()
            .filter(|&field| {
                self.field(product, field)
                    .is_some_and(|support| support.state == CapabilityState::Supported)
                    && !bulk_unsupported(exchange, field)
            })
            .collect()
    }
}

/// Whether `exchange` advertises `field` but serves it only for selected
/// markets ([`Venue::stats_bulk_unsupported`]: Binance's open interest).
pub fn bulk_unsupported(exchange: &str, field: StatsFieldName) -> bool {
    Venue::of(exchange).stats_bulk_unsupported.contains(&field)
}

// Partitions.

/// One complete request profile of an exchange: its own identity, label,
/// product and statistics parameters (`None`: it has no statistics).
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Partition {
    pub id: String,
    pub label: String,
    pub product: Product,
    #[serde(default)]
    pub stats_params: Option<Map<String, Value>>,
}

/// `exchange`'s partitions: the backend's when it lists them, else the
/// venue's fallback profiles once per advertised scope dimension (scope parameters
/// other than `category` and `type`, which the profiles already say).
fn partitions(exchange: &str, capability: &StatsCapability) -> Vec<Partition> {
    if let Some(partitions) = &capability.partitions {
        return partitions.clone();
    }
    let mut dimensions: Vec<(String, Map<String, Value>)> = Vec::new();
    for scope in &capability.supported_scopes {
        let params: Map<String, Value> = scope
            .params
            .iter()
            .filter(|(key, _)| *key != "category" && *key != "type")
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        let key = canonical_json(&Value::Object(params.clone()));
        if !dimensions.iter().any(|(seen, _)| *seen == key) {
            dimensions.push((key, params));
        }
    }
    if dimensions.is_empty() {
        dimensions.push(("{}".to_owned(), Map::new()));
    }
    let mut result = Vec::new();
    for &(id, label, product, params) in Venue::of(exchange).stats_profiles {
        for (key, dimension) in &dimensions {
            let mut stats: Map<String, Value> = params
                .iter()
                .map(|&(name, value)| (name.to_owned(), Value::from(value)))
                .collect();
            stats.extend(dimension.clone());
            let mut names: Vec<&String> = dimension.keys().collect();
            names.sort();
            let suffix = names
                .iter()
                .map(|name| format!("{name}={}", dimension[*name]))
                .collect::<Vec<_>>()
                .join(", ");
            result.push(Partition {
                id: if key == "{}" {
                    format!("{exchange}:{id}")
                } else {
                    format!("{exchange}:{id}:{key}")
                },
                label: if suffix.is_empty() {
                    label.to_owned()
                } else {
                    format!("{label} · {suffix}")
                },
                product,
                stats_params: Some(stats),
            });
        }
    }
    result
}

/// One partition to poll: what to ask for and how to read the answer.
#[derive(Debug, PartialEq)]
pub struct MarketStatsScope {
    /// The exchange id (`bybit`).
    pub exchange: String,
    pub partition: Partition,
    /// The exchange's statistics contract.
    pub capability: Rc<StatsCapability>,
    /// The fields requested: those wanted that the scope serves in bulk.
    pub fields: Vec<StatsFieldName>,
    /// The request's parameters.
    pub params: Map<String, Value>,
    /// The request's identity, shared by everyone polling this scope.
    pub key: String,
}

/// Every scope of `capabilities` with statistics for one of `products`,
/// asking for those of `fields` it serves, in the capabilities' exchange
/// order. A partition with no statistics or none of the fields is left
/// out, and so is one listed twice (the first wins).
pub fn stats_scopes(
    capabilities: &Capabilities,
    products: &[Product],
    fields: &[StatsFieldName],
) -> Vec<MarketStatsScope> {
    let mut scopes: Vec<MarketStatsScope> = Vec::new();
    for entry in &capabilities.exchanges {
        let Some(capability) = entry.market_stats() else {
            continue;
        };
        let exchange = entry.exchange.trim().to_ascii_lowercase();
        let capability = Rc::new(capability);
        for partition in partitions(&exchange, &capability) {
            let Some(params) = partition.stats_params.clone() else {
                continue;
            };
            if !products.contains(&partition.product)
                || scopes
                    .iter()
                    .any(|scope| scope.partition.id == partition.id)
            {
                continue;
            }
            let fields = capability.bulk_fields(&exchange, partition.product, fields);
            if fields.is_empty() {
                continue;
            }
            let params = request_params(&exchange, params);
            let key = format!(
                "ferris:fetchMarketStats:{}",
                canonical_json(&request_body(&exchange, &params, &fields))
            );
            scopes.push(MarketStatsScope {
                exchange: exchange.clone(),
                partition,
                capability: capability.clone(),
                fields,
                params,
                key,
            });
        }
    }
    scopes
}

/// The params sent for a scope: its own, or its venue's default when it
/// has none ([`Venue::stats_default_params`]: Bybit's linear category).
fn request_params(exchange: &str, params: Map<String, Value>) -> Map<String, Value> {
    let defaults = Venue::of(exchange).stats_default_params;
    if params.is_empty() && !defaults.is_empty() {
        return defaults
            .iter()
            .map(|&(name, value)| (name.to_owned(), Value::from(value)))
            .collect();
    }
    params
}

pub(super) fn request_body(
    exchange: &str,
    params: &Map<String, Value>,
    fields: &[StatsFieldName],
) -> Value {
    serde_json::json!({
        "exchange": exchange,
        "params": params,
        "fields": fields.iter().map(|field| field.as_str()).collect::<Vec<_>>(),
    })
}

/// How long a statistics request may take: whole-exchange scopes of many
/// fields are slow on the largest venues (old app measurements: 20-30s).
/// Some venues' typed scopes are large whatever they ask for, and some
/// allow a large request longer ([`Venue::stats_typed_scopes_large`],
/// [`Venue::stats_large_timeout`]: Hyperliquid).
pub(super) fn request_timeout(
    exchange: &str,
    params: &Map<String, Value>,
    fields: usize,
) -> Duration {
    let venue = Venue::of(exchange);
    let large = fields >= 5 || (venue.stats_typed_scopes_large && params.contains_key("type"));
    if large {
        venue.stats_large_timeout
    } else {
        Duration::from_secs(15)
    }
}

// Snapshots.

/// One scope's statistics.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatsSnapshot {
    /// When the backend made it (Unix milliseconds).
    pub timestamp: f64,
    pub markets: Vec<StatsMarket>,
    pub coverage: Coverage,
}

/// How complete a snapshot is.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Coverage {
    #[serde(default)]
    pub expected_markets: Option<f64>,
    pub returned_markets: f64,
    pub enumeration_complete: bool,
    #[serde(default)]
    pub source_failures: Vec<SourceFailure>,
}

/// A source the backend could not read for this snapshot.
#[derive(Debug, Deserialize)]
pub struct SourceFailure {
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub message: String,
}

/// One market and its statistics.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatsMarket {
    pub exchange: String,
    pub symbol: String,
    pub base: String,
    pub quote: String,
    /// The market's identity across Ferris (opaque).
    pub market_id: String,
    #[serde(rename = "type")]
    pub product: Product,
    pub active: bool,
    #[serde(default)]
    pub category: Option<String>,
    /// The settlement asset (`USDT`; the base for inverse contracts).
    #[serde(default)]
    pub settle: Option<String>,
    #[serde(default)]
    pub fields: StatsFields,
}

/// A market's fields; one the request did not ask for is absent.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatsFields {
    #[serde(default)]
    pub funding: Option<StatsField<Funding>>,
    #[serde(default)]
    pub index_price: Option<StatsField<Price>>,
    #[serde(default)]
    pub last_price: Option<StatsField<Price>>,
    #[serde(default)]
    pub last_settled_funding: Option<StatsField<Funding>>,
    #[serde(default)]
    pub mark_price: Option<StatsField<Price>>,
    #[serde(default)]
    pub open_interest: Option<StatsField<OpenInterest>>,
    #[serde(default)]
    pub volume24h: Option<StatsField<Volume>>,
}

impl StatsFields {
    /// A price field.
    pub fn price(&self, field: StatsFieldName) -> Option<&StatsField<Price>> {
        match field {
            StatsFieldName::IndexPrice => self.index_price.as_ref(),
            StatsFieldName::LastPrice => self.last_price.as_ref(),
            StatsFieldName::MarkPrice => self.mark_price.as_ref(),
            _ => None,
        }
    }

    /// A funding field.
    pub fn funding(&self, field: StatsFieldName) -> Option<&StatsField<Funding>> {
        match field {
            StatsFieldName::Funding => self.funding.as_ref(),
            StatsFieldName::LastSettledFunding => self.last_settled_funding.as_ref(),
            _ => None,
        }
    }

    /// Any field's envelope, without its value.
    pub fn meta(&self, field: StatsFieldName) -> Option<FieldMeta<'_>> {
        match field {
            StatsFieldName::Funding | StatsFieldName::LastSettledFunding => {
                self.funding(field).map(StatsField::meta)
            }
            StatsFieldName::IndexPrice | StatsFieldName::LastPrice | StatsFieldName::MarkPrice => {
                self.price(field).map(StatsField::meta)
            }
            StatsFieldName::OpenInterest => self.open_interest.as_ref().map(StatsField::meta),
            StatsFieldName::Volume24h => self.volume24h.as_ref().map(StatsField::meta),
        }
    }
}

/// What every field says besides its value.
#[derive(Clone, Copy, Debug)]
pub struct FieldMeta<'a> {
    pub state: FieldState,
    pub received_timestamp: Option<f64>,
    pub reason: Option<&'a str>,
}

/// One field of a market.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatsField<T> {
    pub state: FieldState,
    pub value: Option<T>,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    /// When the exchange produced it (Unix milliseconds), if it says.
    #[serde(default)]
    pub exchange_timestamp: Option<f64>,
    /// When the backend received it (Unix milliseconds).
    #[serde(default)]
    pub received_timestamp: Option<f64>,
}

impl<T> StatsField<T> {
    /// What it says besides its value.
    pub fn meta(&self) -> FieldMeta<'_> {
        FieldMeta {
            state: self.state,
            received_timestamp: self.received_timestamp,
            reason: self.reason.as_deref(),
        }
    }

    /// Its value, when the field is available (or stale).
    pub fn usable(&self) -> Option<&T> {
        matches!(self.state, FieldState::Available | FieldState::Stale)
            .then_some(self.value.as_ref())
            .flatten()
    }
}

/// A field's state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FieldState {
    Available,
    Stale,
    Unavailable,
    Unsupported,
    NotApplicable,
    #[serde(other)]
    Unknown,
}

/// A funding rate and its period equivalents (percent).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Funding {
    pub rate: Decimal,
    #[serde(default)]
    pub rate_unit: Option<String>,
    #[serde(default)]
    pub equivalents: Option<FundingEquivalents>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub payment_interval_ms: Option<f64>,
}

/// A funding rate restated per period, in percent (API-supplied; the
/// annual rate is simple, not compounded).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FundingEquivalents {
    #[serde(default)]
    pub one_hour_percent: Option<Decimal>,
    #[serde(default)]
    pub eight_hour_percent: Option<Decimal>,
    #[serde(default)]
    pub one_day_percent: Option<Decimal>,
    #[serde(default)]
    pub annualized_percent: Option<Decimal>,
}

/// A price of the base asset in the quote asset.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Price {
    pub amount: Decimal,
    pub base_asset: String,
    pub quote_asset: String,
}

/// 24h volume in the base and the quote denomination.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Volume {
    #[serde(default)]
    pub base_volume: Option<f64>,
    #[serde(default)]
    pub quote_volume: Option<f64>,
}

/// Open interest as the venue reports it: an amount (base or contracts)
/// and/or a value (quote).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenInterest {
    #[serde(default)]
    pub open_interest_amount: Option<f64>,
    #[serde(default)]
    pub open_interest_value: Option<f64>,
}

/// A decimal kept exactly as sent (`"0.00005"`). A number is accepted too
/// and kept as Rust writes it, which never uses an exponent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decimal(pub String);

impl Decimal {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for Decimal {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        match Value::deserialize(deserializer)? {
            Value::String(text) => Ok(Self(text)),
            Value::Number(number) => Ok(Self(
                number
                    .as_f64()
                    .map_or_else(|| number.to_string(), |n| n.to_string()),
            )),
            other => Err(serde::de::Error::custom(format!(
                "expected a decimal, found {other}"
            ))),
        }
    }
}
