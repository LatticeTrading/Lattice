//! Market catalogs (`POST /v1/fetchMarkets`): every market an exchange
//! lists, one shared request per catalog scope (the old app's
//! `FerrisMarketsService` and `MarketSymbolService`).
//!
//! A scope is one request profile of one exchange (its venue's
//! [`catalog_profiles`](super::Venue::catalog_profiles)). Most exchanges
//! list every product in one reply (`{"params":{}}`); a few only list some
//! products unless asked for others by `category` (checked live
//! 2026-11-13):
//!
//! - Binance: `{}` is USDT-M perpetuals and futures; COIN-M is
//!   `{"category":"inverse"}`, spot `{"category":"spot"}`.
//! - Aster: `{}` is perpetuals; spot is `{"category":"spot"}`.
//! - Bybit and KuCoin list spot, perpetuals and futures with `{}`.
//!
//! Options are not asked for (Binance and Bybit list them under
//! `{"category":"option"}`, about 3,900): the catalog names an option only
//! by its pair (`BTC/USDT`), not its strike or expiry, so the picker could
//! not tell them apart, and the old app never offered them.
//!
//! [`catalog_scopes`] picks the scopes the capabilities' `fetchMarkets`
//! feature supports, for the products a caller can use;
//! [`super::Ferris::catalog_query`] is a scope's shared request (key
//! `ferris:fetchMarkets:` plus its canonical body), refreshed every minute
//! while watched (the old cache went stale after 60s). A reply becomes a
//! [`Catalog`]: its active, routable markets as selections
//! ([`MarketSelection::from_catalog`]), each once.

use std::{collections::HashSet, rc::Rc, time::Duration};

use serde_json::{Map, Value, json};

use super::{Availability, Capabilities, MarketSelection, MarketType, Operation, Product, Venue};
use crate::data::net::{ApiError, canonical_json};

/// How long a catalog is fresh, and how often a watched one is refetched.
pub(super) const REFRESH: Duration = Duration::from_secs(60);

/// One catalog request: an exchange and the params of one of its listing
/// profiles, with the products it lists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogScope {
    /// The exchange id, lower case.
    pub exchange: String,
    pub params: Map<String, Value>,
    /// The products asked for from this scope (those it lists that the
    /// exchange supports and the caller wants).
    pub products: Vec<Product>,
    /// The query key: the endpoint and the canonical body.
    pub key: String,
}

impl CatalogScope {
    fn new(exchange: &str, params: Map<String, Value>, products: Vec<Product>) -> Self {
        let mut scope = Self {
            exchange: exchange.to_owned(),
            params,
            products,
            key: String::new(),
        };
        scope.key = format!("ferris:fetchMarkets:{}", canonical_json(&scope.body()));
        scope
    }

    /// The request's body.
    pub(super) fn body(&self) -> Value {
        json!({ "exchange": self.exchange, "params": self.params })
    }

    /// Its deadline ([`Venue::catalog_timeout`]).
    pub(super) fn timeout(&self) -> Duration {
        Venue::of(&self.exchange).catalog_timeout
    }
}

/// The catalog scopes to ask for: of every exchange whose `fetchMarkets`
/// is supported (only `exchange` when given), each profile listing a
/// product the feature supports and `wanted(exchange, product)` accepts.
pub fn catalog_scopes(
    capabilities: &Capabilities,
    exchange: Option<&str>,
    wanted: impl Fn(&str, Product) -> bool,
) -> Vec<CatalogScope> {
    let mut scopes = Vec::new();
    for listed in &capabilities.exchanges {
        let id = listed.exchange.trim().to_ascii_lowercase();
        if exchange.is_some_and(|only| !only.trim().eq_ignore_ascii_case(&id)) {
            continue;
        }
        let Some(feature) = listed
            .features
            .snapshot
            .get(Operation::FetchMarkets.feature())
        else {
            continue;
        };
        if feature.availability(None) != Availability::Supported {
            continue;
        }
        for (category, products) in Venue::of(&id).catalog_profiles {
            let products: Vec<Product> = products
                .iter()
                .copied()
                .filter(|product| {
                    feature.supported_products.contains(product) && wanted(&id, *product)
                })
                .collect();
            if products.is_empty() {
                continue;
            }
            let mut params = Map::new();
            if let Some(category) = category {
                params.insert("category".into(), Value::from(*category));
            }
            scopes.push(CatalogScope::new(&id, params, products));
        }
    }
    scopes
}

/// One scope's markets, as the reply listed them.
#[derive(Debug, Default)]
pub struct Catalog {
    pub markets: Vec<Rc<CatalogMarket>>,
}

/// A market a picker can offer: its selection and what finding and
/// showing it needs, worked out once per reply (the old app's
/// `DisplayMarket`).
#[derive(Debug)]
pub struct CatalogMarket {
    pub selection: MarketSelection,
    /// What tells it from the other choices: the catalog id for
    /// Hyperliquid (deployers can share a route), else where it routes
    /// ([`MarketSelection::route_key`]).
    pub key: String,
    /// `PERP USDT-M` ([`MarketSelection::tags`]).
    pub tags: String,
    /// `USDT-M` ([`MarketSelection::detail`]).
    pub detail: Option<String>,
    /// The pair, lower case.
    lower: String,
    /// The pair as search compares it ([`normalize_search`]).
    search: String,
    /// Where it sorts: the old app's `compareDisplayMarkets` (base, quote,
    /// type, detail, venue symbol), each ignoring case.
    order: [String; 5],
}

impl CatalogMarket {
    fn new(selection: MarketSelection) -> Self {
        let key = match &selection.market_id {
            Some(id) if Venue::of(&selection.exchange).catalog_keys_by_id => {
                format!("{}:{id}", selection.exchange)
            }
            _ => selection.route_key(),
        };
        let lower = selection.symbol.to_lowercase();
        let tags = selection.tags();
        let order = [
            selection.base.to_lowercase(),
            selection.quote.to_lowercase(),
            selection.kind.as_str().to_owned(),
            tags.to_lowercase(),
            selection.exchange_symbol.to_lowercase(),
        ];
        Self {
            key,
            detail: selection.detail(),
            tags,
            search: normalize_search(&lower),
            lower,
            order,
            selection,
        }
    }
}

/// A `fetchMarkets` reply for `exchange`: its markets array (the reply's
/// `markets`, or `data`, or the reply itself), every row read leniently.
/// Kept: active markets that can be streamed (dated futures with an exact
/// day), not options, each once. A reply with no markets array fails.
pub(super) fn parse(payload: &Value, exchange: &str) -> Result<Catalog, ApiError> {
    let rows = match payload {
        Value::Array(rows) => rows,
        Value::Object(reply) => match reply.get("markets").or_else(|| reply.get("data")) {
            Some(Value::Array(rows)) => rows,
            _ => {
                return Err(ApiError::invalid_response(
                    "the market catalog reply has no markets",
                ));
            }
        },
        _ => {
            return Err(ApiError::invalid_response(
                "the market catalog reply is not an object",
            ));
        }
    };
    let markets = rows
        .iter()
        .filter(|row| row.get("active") != Some(&Value::Bool(false)))
        .filter_map(|row| MarketSelection::from_catalog(row, exchange))
        .filter(|market| market.kind != MarketType::Option && market.is_routable())
        .map(|market| Rc::new(CatalogMarket::new(market)))
        .collect();
    Ok(Catalog {
        markets: merge([markets].iter()),
    })
}

/// The markets of `catalogs` (one exchange's scopes) as one list, each
/// once, in the old app's order: base, quote, type, detail, venue symbol.
pub fn merge<'a>(
    catalogs: impl Iterator<Item = &'a Vec<Rc<CatalogMarket>>>,
) -> Vec<Rc<CatalogMarket>> {
    let mut seen = HashSet::new();
    let mut markets: Vec<Rc<CatalogMarket>> = catalogs
        .flatten()
        .filter(|market| seen.insert(market.key.clone()))
        .cloned()
        .collect();
    markets.sort_by(|a, b| a.order.cmp(&b.order).then_with(|| a.key.cmp(&b.key)));
    markets
}

/// Text as symbol search compares it: lower case letters and digits only,
/// so `btcusd` finds `BTC/USD` (the old `normalizeSymbolSearchValue`).
pub fn normalize_search(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        .collect()
}

/// What was typed into a symbol search, ready to score markets with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SymbolQuery {
    raw: String,
    normalized: String,
}

impl SymbolQuery {
    pub fn new(typed: &str) -> Self {
        let raw = typed.trim().to_lowercase();
        Self {
            normalized: normalize_search(&raw),
            raw,
        }
    }

    /// Nothing typed: every market matches.
    pub fn is_empty(&self) -> bool {
        self.raw.is_empty()
    }

    /// How well `market` matches, higher is better, 0 not at all (the old
    /// `getDisplayMarketSearchScore`): the whole pair, then the pair as
    /// typed, then its start, then anywhere in it.
    pub fn score(&self, market: &CatalogMarket) -> u8 {
        if self.raw.is_empty() {
            return 1;
        }
        if self.normalized.is_empty() || !market.search.contains(&self.normalized) {
            return 0;
        }
        if market.search == self.normalized {
            120
        } else if market.lower == self.raw {
            118
        } else if market.search.starts_with(&self.normalized) {
            105
        } else if market.lower.starts_with(&self.raw) {
            100
        } else {
            85
        }
    }
}
