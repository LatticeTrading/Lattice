//! `GET /v1/capabilities`: what each exchange supports.
//!
//! Every exchange lists its operations (`fetchMarkets`, `fetchOhlcv`, ...)
//! and realtime channels (`realtime.trades`, ...) with a state and the
//! products they cover. Missing metadata is `Unknown`, never support: an
//! older or failed discovery must not be read as proof either way. Unknown
//! states and products are kept as `Unknown` / `Other`, and a feature this
//! client cannot read is skipped, so a newer backend does not break the
//! whole document.

use std::{collections::BTreeMap, rc::Rc, time::Duration};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{Channel, Ferris, Operation};
use crate::data::net::{ApiError, QuerySpec, Watch};

/// The whole capabilities document.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct Capabilities {
    pub exchanges: Vec<ExchangeCapabilities>,
}

/// One exchange's capabilities.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ExchangeCapabilities {
    /// The exchange id, lower case (`binance`).
    pub exchange: String,
    #[serde(default)]
    pub features: Features,
    /// The statistics contract (scopes, fields, cadence, WS limits), kept
    /// raw until the statistics service types it.
    #[serde(default)]
    pub market_stats: Value,
    #[serde(default)]
    pub funding_rate_history: Value,
}

/// Operations and realtime channels, by name.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(from = "BTreeMap<String, Value>")]
pub struct Features {
    /// REST operations by feature name ([`Operation::feature`]).
    pub snapshot: BTreeMap<String, Feature>,
    /// Realtime channels by name (`trades`, `orderbook`, `ohlcv`,
    /// `marketstats`).
    pub realtime: BTreeMap<String, Feature>,
}

impl From<BTreeMap<String, Value>> for Features {
    fn from(mut raw: BTreeMap<String, Value>) -> Self {
        let realtime = match raw.remove("realtime") {
            Some(Value::Object(channels)) => features(channels.into_iter()),
            _ => BTreeMap::new(),
        };
        Self {
            snapshot: features(raw.into_iter()),
            realtime,
        }
    }
}

/// The entries that read as features; one that does not is skipped.
fn features(entries: impl Iterator<Item = (String, Value)>) -> BTreeMap<String, Feature> {
    entries
        .filter_map(|(name, value)| match serde_json::from_value(value) {
            Ok(feature) => Some((name, feature)),
            Err(error) => {
                log::debug!("Ferris capabilities: skipping feature `{name}`: {error}");
                None
            }
        })
        .collect()
}

/// One operation's or channel's support.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Feature {
    pub state: CapabilityState,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub supported_products: Vec<Product>,
    /// `catalog`, `allMarkets`, `selectedMarkets` or `marketSpecific`.
    #[serde(default)]
    pub market_scope: Option<String>,
    /// How a realtime channel is served upstream.
    #[serde(default)]
    pub mode: Option<DeliveryMode>,
    /// A polled channel's cadence.
    #[serde(default)]
    pub poll_interval_ms: Option<u64>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub enum CapabilityState {
    Supported,
    /// Supported for the listed products only.
    Partial,
    Unsupported,
    NotApplicable,
    #[serde(other)]
    Unknown,
}

/// A market's product type.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Product {
    Perp,
    Future,
    Spot,
    Option,
    #[serde(other)]
    Other,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum DeliveryMode {
    Polling,
    Websocket,
    Hybrid,
    #[serde(other)]
    Other,
}

/// Whether something may be offered.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Availability {
    Supported,
    Unsupported,
    /// No metadata says either way: keep what already works, do not offer
    /// what does not exist yet.
    Unknown,
}

impl Feature {
    /// Its availability, for `product` when given.
    pub fn availability(&self, product: Option<Product>) -> Availability {
        match self.state {
            CapabilityState::Unsupported | CapabilityState::NotApplicable => {
                Availability::Unsupported
            }
            _ if product.is_some_and(|product| !self.supported_products.contains(&product)) => {
                Availability::Unsupported
            }
            CapabilityState::Supported | CapabilityState::Partial => Availability::Supported,
            CapabilityState::Unknown => Availability::Unknown,
        }
    }
}

impl Capabilities {
    /// The exchange `id` (any case).
    pub fn exchange(&self, id: &str) -> Option<&ExchangeCapabilities> {
        let id = id.trim();
        self.exchanges
            .iter()
            .find(|exchange| exchange.exchange.eq_ignore_ascii_case(id))
    }

    /// Whether `exchange` serves `operation` (for `product` when given).
    pub fn snapshot(
        &self,
        exchange: &str,
        operation: Operation,
        product: Option<Product>,
    ) -> Availability {
        self.exchange(exchange)
            .and_then(|exchange| exchange.features.snapshot.get(operation.feature()))
            .map_or(Availability::Unknown, |feature| {
                feature.availability(product)
            })
    }

    /// Whether `exchange` streams `channel` (for `product` when given).
    pub fn realtime(
        &self,
        exchange: &str,
        channel: Channel,
        product: Option<Product>,
    ) -> Availability {
        self.exchange(exchange)
            .and_then(|exchange| exchange.features.realtime.get(channel.as_str()))
            .map_or(Availability::Unknown, |feature| {
                feature.availability(product)
            })
    }
}

/// What a feature needs of a market's exchange: an operation, a realtime
/// channel, or both (the old app's `FerrisCapabilityRequirement`). The
/// Trade Stream needs realtime `trades` only; its REST history is an
/// optional head start.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Requirement {
    pub snapshot: Option<Operation>,
    pub realtime: Option<Channel>,
}

impl Requirement {
    /// Realtime `channel` only.
    pub fn realtime(channel: Channel) -> Self {
        Self {
            snapshot: None,
            realtime: Some(channel),
        }
    }

    /// REST `operation` only.
    pub fn snapshot(operation: Operation) -> Self {
        Self {
            snapshot: Some(operation),
            realtime: None,
        }
    }

    /// Whether markets of `product` on `exchange` meet it (the old
    /// `getCapabilityRequirementAvailability`): an explicit denial of either
    /// part wins over unknown metadata, which wins over support. Callers
    /// that must stay backward compatible exclude only `Unsupported`.
    pub fn availability(
        &self,
        capabilities: &Capabilities,
        exchange: &str,
        product: Product,
    ) -> Availability {
        let snapshot = self.snapshot.map_or(Availability::Supported, |operation| {
            capabilities.snapshot(exchange, operation, Some(product))
        });
        let realtime = self.realtime.map_or(Availability::Supported, |channel| {
            capabilities.realtime(exchange, channel, Some(product))
        });
        match (snapshot, realtime) {
            (Availability::Unsupported, _) | (_, Availability::Unsupported) => {
                Availability::Unsupported
            }
            (Availability::Unknown, _) | (_, Availability::Unknown) => Availability::Unknown,
            _ => Availability::Supported,
        }
    }
}

/// How often capabilities are refetched while something watches them.
const REFRESH: Duration = Duration::from_secs(5 * 60);

impl Ferris {
    /// The capabilities request: shared, cached and refreshed every five
    /// minutes while watched.
    pub fn capabilities_query(&self) -> QuerySpec<Capabilities> {
        let http = self.http.clone();
        QuerySpec::new("ferris:capabilities", move || {
            let http = http.clone();
            async move { http.get::<Capabilities>("/v1/capabilities").await }
        })
        .refresh_every(REFRESH)
    }

    /// The capabilities: the cached copy while fresh, else fetched.
    pub async fn capabilities(&self) -> Result<Rc<Capabilities>, ApiError> {
        self.queries.get(self.capabilities_query()).await
    }

    /// Watches the capabilities (refreshed while watched).
    pub fn watch_capabilities(&self) -> Watch<Capabilities> {
        self.queries.watch(self.capabilities_query())
    }
}
