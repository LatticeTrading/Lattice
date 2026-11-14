//! One market as a widget saves it, and the Ferris topics it streams and
//! fetches on (the old app's `MarketSelection` and the routing half of
//! `marketIdentity.ts`).
//!
//! A selection is saved under the old app's member names. Its request
//! symbol is what Ferris is asked for; a few venues' saved symbols are
//! known aliases of what the server streams under (Apex `BTC/USDT` for
//! `BTC/USDT:USDT`, KuCoin and Bitfinex pairs without their settlement),
//! and are repaired here, where the market is selected, never by loosening
//! the socket's routing.
//!
//! A catalog market (`fetchMarkets`) becomes a selection through
//! [`MarketSelection::from_catalog`] (the old `normalizeMarketRow` and
//! `deriveMarketRequestSymbol`, with the corrections live Ferris called
//! for). Not ported: the old app's legacy selections
//! (`createLegacyMarketSelection`); Lattice never saved any.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{Channel, FerrisTopic, Product};
use crate::data::net::ApiError;

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
        let mut markets: Vec<Self> = Vec::new();
        for market in values.iter().filter_map(Self::from_value) {
            let identity = market.identity();
            if !markets.iter().any(|kept| kept.identity() == identity) {
                markets.push(market);
            }
        }
        markets
    }

    /// The topic of `channel` that streams this market.
    pub fn topic(&self, channel: Channel) -> Result<FerrisTopic, ApiError> {
        FerrisTopic::market(
            channel,
            &self.exchange,
            &self.routed_symbol(),
            self.request_params(),
        )
    }

    /// The topic a REST snapshot of it is asked for with (its exchange,
    /// symbol and params): the streamed one, except that KuCoin and
    /// Bitfinex REST products are told apart by catalog id.
    pub fn rest_topic(&self, channel: Channel) -> Result<FerrisTopic, ApiError> {
        let symbol = match &self.market_id {
            Some(id) if matches!(self.exchange.as_str(), "bitfinex" | "kucoin") => id.clone(),
            _ => self.routed_symbol(),
        };
        FerrisTopic::market(channel, &self.exchange, &symbol, self.request_params())
    }

    /// What tells two selections apart: the same market and the same
    /// topics are the same selection, however it was saved (the old app's
    /// `createTradeStreamMarketKey`).
    pub fn identity(&self) -> String {
        let key = |topic: Result<FerrisTopic, ApiError>| {
            topic.map(|topic| topic.key()).unwrap_or_default()
        };
        let text = |value: &Option<String>| value.clone().unwrap_or_default();
        json!([
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
        .to_string()
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

    /// The request symbol with the venue aliases repaired (the old app's
    /// `canonicalizeRequestSymbol`).
    fn routed_symbol(&self) -> String {
        canonical_symbol(
            &self.exchange,
            self.kind,
            &self.request_symbol,
            self.settle.as_deref(),
            self.expiry.as_deref(),
            &self.exchange_symbol,
        )
    }

    /// What tells catalog choices apart when they route alike (the old
    /// app's `createMarketIdentityKey`): the exchange, the repaired request
    /// symbol, the category where it picks the product, and the type.
    pub fn route_key(&self) -> String {
        let category = if matches!(self.exchange.as_str(), "bitfinex" | "kucoin") {
            String::new()
        } else {
            self.category
                .as_deref()
                .map(|category| category.trim().to_ascii_lowercase())
                .unwrap_or_default()
        };
        format!(
            "{}:{}:{category}:{}",
            self.exchange,
            self.routed_symbol().trim().to_uppercase(),
            self.kind.as_str()
        )
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

    /// Whether Ferris can stream it: a dated future of Binance, Bybit or
    /// KuCoin needs its exact expiry day in the request symbol (the old
    /// app's `isRoutableMarketSelection`); a month-code contract has none.
    pub fn is_routable(&self) -> bool {
        if self.kind != MarketType::Future
            || !matches!(self.exchange.as_str(), "binance" | "bybit" | "kucoin")
        {
            return true;
        }
        self.request_symbol
            .rsplit_once(':')
            .and_then(|(_, after)| after.split_once('-'))
            .is_some_and(|(settlement, day)| {
                !settlement.is_empty()
                    && settlement.chars().all(|c| c.is_ascii_alphanumeric())
                    && day.len() == 6
                    && day.chars().all(|c| c.is_ascii_digit())
            })
    }

    /// One market of a `fetchMarkets` catalog of `exchange` as a selection
    /// (the old app's `normalizeMarketRow` and the selection
    /// `buildDisplayMarkets` made of it), or `None` without a symbol.
    /// Whether it is active, routable and of a product on offer is the
    /// catalog's to check.
    ///
    /// Two departures from the old app, each checked against live Ferris
    /// (2026-11-13), where the old rules subscribed to aliases:
    ///
    /// - The category is the row's own (`category`), never its `info`'s:
    ///   Aster's `futures` and Extended's `perpetual` are descriptive, and
    ///   Ferris refuses them as selectors (the old app already ignored
    ///   Phoenix's and Perpl's). A row without one gets what picks its
    ///   product: `spot` for spot (Binance, Hyperliquid, Lighter and Aster
    ///   spot pairs are ambiguous without it), and Binance's and Bybit's
    ///   derivatives their family (`linear` / `inverse`; COIN-M needs
    ///   it), as the old app's legacy selections did.
    /// - See [`derive_request_symbol`] for the symbols.
    pub fn from_catalog(row: &Value, exchange: &str) -> Option<Self> {
        let record = row.as_object()?;
        let info = record.get("info").and_then(Value::as_object);
        let member = |name: &str| match record.get(name) {
            Some(value) => Some(value),
            None => info.and_then(|info| info.get(name)),
        };
        // Trimmed and not empty, the row's own or else its info's.
        let text = |name: &str| {
            let own = |value: Option<&Value>| {
                value
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|text| !text.is_empty())
                    .map(str::to_owned)
            };
            own(record.get(name)).or_else(|| own(info.and_then(|info| info.get(name))))
        };
        // An id as issued: not trimmed or recased.
        let identity = |name: &str| {
            member(name)
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty())
                .map(str::to_owned)
        };
        // Missing, null, or text. Empty text is null (Hyperliquid's main
        // dex is `""`), as a saved selection reads it.
        let metadata = |name: &str| match member(name) {
            Some(Value::Null) => Some(None),
            Some(Value::String(text)) => Some(Some(text.clone()).filter(|t| !t.is_empty())),
            _ => None,
        };

        let raw_symbol = text("symbol")?;
        let market_id = identity("marketId");
        // Catalog-issued identities come with authoritative names: their
        // case is kept rather than guessed.
        let symbol = if market_id.is_some() {
            raw_symbol.clone()
        } else {
            raw_symbol.to_uppercase()
        };
        let (derived_base, derived_quote) = match symbol.split_once('/') {
            Some((base, rest)) => (
                base.to_uppercase(),
                rest.split('/').next().unwrap_or_default().to_uppercase(),
            ),
            None => (symbol.to_uppercase(), "USD".to_owned()),
        };
        let (base, quote) = if market_id.is_some() {
            (
                identity("base").unwrap_or(derived_base),
                identity("quote").unwrap_or(derived_quote),
            )
        } else {
            (
                text("base").unwrap_or(derived_base).to_uppercase(),
                text("quote").unwrap_or(derived_quote).to_uppercase(),
            )
        };
        let exchange = text("exchange")
            .unwrap_or_else(|| exchange.trim().to_owned())
            .to_ascii_lowercase();
        let kind = match member("type").and_then(Value::as_str) {
            Some("future") => MarketType::Future,
            Some("perp" | "swap") => MarketType::Perp,
            Some("option") => MarketType::Option,
            _ => MarketType::Spot,
        };
        let exchange_symbol = text("exchangeSymbol")
            .or_else(|| text("rawSymbol"))
            .unwrap_or_else(|| raw_symbol.clone())
            .to_uppercase();
        let category = record
            .get("category")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|category| !category.is_empty())
            .map(str::to_ascii_lowercase);
        let expiry = derive_expiry(&exchange_symbol, text("expiry").as_deref());
        let settle = metadata("settle");
        let request_symbol = derive_request_symbol(&Derivation {
            exchange: &exchange,
            symbol: &symbol,
            base: &base,
            quote: &quote,
            kind,
            category: category.as_deref(),
            exchange_symbol: &exchange_symbol,
            request_symbol: text("requestSymbol").as_deref(),
            settle: settle.as_ref().map(Option::as_deref),
            expiry: expiry.as_deref(),
        });
        let settle = settle.flatten();
        let category = category.or_else(|| {
            let family = derive_family(kind, &base, &quote, settle.as_deref());
            match (kind, exchange.as_str(), family) {
                (MarketType::Spot, "kucoin" | "bitfinex", _) => None,
                (MarketType::Spot, ..) => Some("spot".to_owned()),
                (_, "binance" | "bybit", Some(family @ (Family::Linear | Family::Inverse))) => {
                    Some(family.as_str().to_owned())
                }
                _ => None,
            }
        });
        Some(Self {
            symbol: format!("{base}/{quote}"),
            base,
            quote,
            request_symbol,
            exchange_symbol,
            kind,
            category,
            expiry,
            market_id,
            exchange_market_id: identity("exchangeMarketId"),
            dex: metadata("dex").flatten(),
            contract_type: metadata("contractType").flatten(),
            settle,
            settlement_asset_id: metadata("settlementAssetId").flatten(),
            exchange,
        })
    }

    /// The params that pick the product on venues that need them (the old
    /// app's `marketRequestParams`).
    fn request_params(&self) -> Value {
        let by_type = match self.exchange.as_str() {
            "kucoin" => matches!(
                self.kind,
                MarketType::Spot | MarketType::Perp | MarketType::Future
            ),
            "bitfinex" => matches!(self.kind, MarketType::Spot | MarketType::Perp),
            _ => false,
        };
        if by_type {
            return json!({ "type": self.kind.as_str() });
        }
        match self.category.as_deref().map(str::trim) {
            Some(category) if !category.is_empty() => {
                json!({ "category": category.to_ascii_lowercase() })
            }
            _ => Value::Null,
        }
    }
}

/// The request symbol with the venue aliases repaired (the old app's
/// `canonicalizeRequestSymbol`): Apex `X/USDT` perpetuals stream as
/// `X/USDT:USDT`, KuCoin and Bitfinex pairs carry their settlement, KuCoin
/// dated futures their day.
fn canonical_symbol(
    exchange: &str,
    kind: MarketType,
    requested: &str,
    settle: Option<&str>,
    expiry: Option<&str>,
    exchange_symbol: &str,
) -> String {
    let settle = settle
        .map(str::trim)
        .filter(|settle| !settle.is_empty())
        .map(str::to_ascii_uppercase);
    match exchange.trim().to_ascii_lowercase().as_str() {
        "kucoin" => {
            let pair = requested.trim().to_ascii_uppercase();
            let Some((base, quote)) = plain_pair(&pair) else {
                return requested.to_owned();
            };
            if !matches!(kind, MarketType::Perp | MarketType::Future) {
                return requested.to_owned();
            }
            let settlement = settle.or_else(|| match quote {
                "USD" => Some(base.to_owned()),
                "USDT" | "USDC" => Some(quote.to_owned()),
                _ => None,
            });
            match (kind, settlement) {
                (MarketType::Perp, Some(settlement)) => format!("{pair}:{settlement}"),
                (MarketType::Future, Some(settlement)) if expiry.is_some_and(is_iso_day) => {
                    let day = compact_day(expiry.unwrap_or_default());
                    format!("{pair}:{settlement}-{day}")
                }
                // A month-code native id has no exact day: kept as its own
                // identity, not routed to another contract.
                (MarketType::Future, _) if !exchange_symbol.is_empty() => {
                    exchange_symbol.to_owned()
                }
                _ => requested.to_owned(),
            }
        }
        "bitfinex" if kind == MarketType::Perp => {
            let pair = requested.trim().to_ascii_uppercase();
            match pair.split_once('/') {
                Some((base, quote))
                    if is_bare(base) && !quote.is_empty() && !quote.contains(':') =>
                {
                    let settlement = settle.unwrap_or_else(|| quote.to_owned());
                    format!("{pair}:{settlement}")
                }
                _ => requested.to_owned(),
            }
        }
        "apex" if kind == MarketType::Perp => {
            let pair = requested.trim().to_ascii_uppercase();
            match plain_pair(&pair) {
                Some((_, "USDT")) => format!("{pair}:USDT"),
                _ => requested.to_owned(),
            }
        }
        _ => requested.to_owned(),
    }
}

/// What [`derive_request_symbol`] reads of a catalog market.
struct Derivation<'a> {
    exchange: &'a str,
    /// The catalog's own symbol.
    symbol: &'a str,
    base: &'a str,
    quote: &'a str,
    kind: MarketType,
    category: Option<&'a str>,
    exchange_symbol: &'a str,
    /// One the catalog gave, if any.
    request_symbol: Option<&'a str>,
    /// Missing (`None`), null (`Some(None)`) or given.
    settle: Option<Option<&'a str>>,
    expiry: Option<&'a str>,
}

/// What Ferris is asked for to stream a catalog market (the old app's
/// `deriveMarketRequestSymbol`): a catalog symbol already qualified
/// (`BTC/AUSD:AUSD`) as it is; Binance and Bybit spot `BASE/QUOTE`,
/// perpetuals `BASE/QUOTE:SETTLE` and dated futures
/// `BASE/QUOTE:SETTLE-YYMMDD` (a month-code future keeps its native id and
/// is not routable); KuCoin, Bitfinex and Apex through their alias repairs.
///
/// Two rules differ from the old app's, checked against live Ferris
/// (2026-11-13; the old ones subscribed to aliases):
///
/// - Hyperliquid spot is `BASE/QUOTE` (with the `spot` category). The old
///   `BASE/USDC:USDC` for every Hyperliquid market named the perpetual of
///   the same coin; Hyperliquid perpetuals, HIP-3 ones included
///   (`XYZ-XYZ100/USDC:USDC`), keep it.
/// - A perpetual whose catalog symbol is a plain pair and whose settlement
///   is known is `BASE/QUOTE:SETTLE` on every venue but Phoenix. Lighter's
///   `BTC/USDC` and Aster's `BTC/USDT` were acknowledged and then streamed
///   under the settled symbol; Nado's REST answers with it too. Phoenix's
///   own symbols carry no settlement (`SOL/USD`), so they stay as listed.
fn derive_request_symbol(input: &Derivation<'_>) -> String {
    let exchange = input.exchange.trim().to_ascii_lowercase();
    let settle = input
        .settle
        .flatten()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if let Some(explicit) = input
        .request_symbol
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return canonical_symbol(
            &exchange,
            input.kind,
            explicit,
            settle,
            input.expiry,
            input.exchange_symbol,
        );
    }
    let catalog = input.symbol.trim();
    if is_qualified(catalog) {
        return catalog.to_owned();
    }
    let exchange_symbol = Some(input.exchange_symbol.trim())
        .filter(|s| !s.is_empty())
        .unwrap_or(catalog);
    let base = input.base.trim().to_uppercase();
    let quote = input.quote.trim().to_uppercase();
    let pair = format!("{base}/{quote}");
    let settlement = settle.map(str::to_uppercase);

    // Extended's quote-qualified wire alias is not evidence of the market's
    // family.
    if exchange == "extended" && input.kind == MarketType::Perp {
        return format!("{pair}:{}", settlement.unwrap_or(quote));
    }
    if exchange != "binance" && exchange != "bybit" {
        if exchange == "hyperliquid" {
            return if input.kind == MarketType::Spot {
                pair
            } else if catalog.contains(':') {
                catalog.to_owned()
            } else if exchange_symbol.contains(':') {
                exchange_symbol.to_owned()
            } else {
                format!("{base}/USDC:USDC")
            };
        }
        let listed = if catalog.is_empty() {
            exchange_symbol
        } else {
            catalog
        };
        if input.kind == MarketType::Perp
            && exchange != "phoenix"
            && plain_pair(&listed.to_uppercase()).is_some()
            && let Some(settlement) = settlement
        {
            return format!("{pair}:{settlement}");
        }
        return canonical_symbol(
            &exchange,
            input.kind,
            listed,
            settle,
            input.expiry,
            exchange_symbol,
        );
    }

    if input.kind == MarketType::Spot {
        return pair;
    }
    let category = input
        .category
        .map(|category| category.trim().to_ascii_lowercase());
    // Missing settlement metadata falls back to the category; a null one is
    // unknown.
    let settle = match input.settle {
        None => match category.as_deref() {
            Some("inverse") => Some(base.clone()),
            Some("linear") => Some(quote.clone()),
            _ => None,
        },
        Some(_) => settlement,
    };
    let settlement = match derive_family(input.kind, &base, &quote, settle.as_deref()) {
        Some(Family::Inverse) => Some(base.clone()),
        Some(Family::Linear) => Some(quote.clone()),
        _ => settle,
    };
    let Some(settlement) = settlement else {
        return if input.kind == MarketType::Future || catalog.is_empty() {
            exchange_symbol.to_owned()
        } else {
            catalog.to_owned()
        };
    };
    let settled = format!("{pair}:{settlement}");
    if input.kind != MarketType::Future {
        return settled;
    }
    match derive_expiry(exchange_symbol, input.expiry) {
        Some(expiry) if is_iso_day(&expiry) => format!("{settled}-{}", compact_day(&expiry)),
        // A month-code contract has no exact day in the catalog: it keeps
        // its native id rather than routing to another contract.
        _ => exchange_symbol.to_owned(),
    }
}

/// `BASE/QUOTE:SETTLE`, optionally `-YYMMDD` (the old
/// `hasQualifiedRequestSymbol`, `/\/[^:]+:[^-]+(?:-\d{6})?$/i`).
fn is_qualified(symbol: &str) -> bool {
    let Some(slash) = symbol.find('/') else {
        return false;
    };
    let rest = &symbol[slash + 1..];
    rest.match_indices(':').any(|(colon, _)| {
        if colon == 0 {
            return false;
        }
        let tail = &rest[colon + 1..];
        match tail.split_once('-') {
            None => !tail.is_empty(),
            Some((settlement, day)) => {
                !settlement.is_empty() && day.len() == 6 && day.chars().all(|c| c.is_ascii_digit())
            }
        }
    })
}

/// A derivative's family, from its settlement asset alone (never the venue
/// or quote): settled in its base is inverse, in its quote linear.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Family {
    Linear,
    Inverse,
    Unknown,
}

impl Family {
    fn as_str(self) -> &'static str {
        match self {
            Self::Linear => "linear",
            Self::Inverse => "inverse",
            Self::Unknown => "unknown",
        }
    }
}

/// The old app's `deriveMarketFamily`: `None` for spot.
fn derive_family(
    kind: MarketType,
    base: &str,
    quote: &str,
    settle: Option<&str>,
) -> Option<Family> {
    match (kind, settle) {
        (MarketType::Spot, _) => None,
        (_, None) => Some(Family::Unknown),
        (_, Some(settle)) if settle == base => Some(Family::Inverse),
        (_, Some(settle)) if settle == quote => Some(Family::Linear),
        _ => Some(Family::Unknown),
    }
}

/// A contract's expiry, `2026-12-25` or `2026-12` (the old app's
/// `deriveMarketExpiry`): the catalog's own when it reads as a date, else
/// from the native id (`BTCUSDT_261225`, `BTCUSDT-04DEC26`, or a month code
/// `BTCUSDH27`, which has no day).
fn derive_expiry(exchange_symbol: &str, explicit: Option<&str>) -> Option<String> {
    const MONTHS: [&str; 12] = [
        "JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC",
    ];
    const CODES: &str = "FGHJKMNQUVXZ";
    let digits = |text: &str| !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit());
    if let Some(explicit) = explicit.map(str::trim).filter(|e| !e.is_empty()) {
        let year = explicit.get(..4).filter(|y| digits(y));
        let month = explicit
            .get(4..7)
            .and_then(|m| m.strip_prefix('-'))
            .filter(|m| digits(m));
        if let (Some(year), Some(month)) = (year, month) {
            let day = explicit
                .get(7..10)
                .and_then(|d| d.strip_prefix('-'))
                .filter(|d| digits(d));
            return Some(match day {
                Some(day) => format!("{year}-{month}-{day}"),
                None => format!("{year}-{month}"),
            });
        }
    }
    let native = exchange_symbol.trim().to_uppercase();
    let len = native.len();
    // `_YYMMDD`
    if let Some(compact) = native.get(len.saturating_sub(7)..)
        && let Some(compact) = compact.strip_prefix('_')
        && digits(compact)
    {
        return Some(format!(
            "20{}-{}-{}",
            &compact[..2],
            &compact[2..4],
            &compact[4..]
        ));
    }
    // `-DDMMMYY`
    if let Some(named) = native.get(len.saturating_sub(8)..)
        && let Some(named) = named.strip_prefix('-')
        && let (Some(day), Some(name), Some(year)) =
            (named.get(..2), named.get(2..5), named.get(5..))
        && digits(day)
        && digits(year)
        && let Some(month) = MONTHS.iter().position(|m| *m == name)
    {
        return Some(format!("20{year}-{:02}-{day}", month + 1));
    }
    // A month code and two digits of year: no day.
    if let Some(coded) = native.get(len.saturating_sub(3)..)
        && let (Some(code), Some(year)) = (coded.chars().next(), coded.get(1..))
        && digits(year)
        && let Some(month) = CODES.find(code)
    {
        return Some(format!("20{year}-{:02}", month + 1));
    }
    None
}

/// Text with no `/`, `:` or whitespace, and some of it.
fn is_bare(text: &str) -> bool {
    !text.is_empty()
        && !text
            .chars()
            .any(|c| c == '/' || c == ':' || c.is_whitespace())
}

/// `BASE/QUOTE` with both parts bare.
fn plain_pair(pair: &str) -> Option<(&str, &str)> {
    let (base, quote) = pair.split_once('/')?;
    (is_bare(base) && is_bare(quote)).then_some((base, quote))
}

/// `2026-12-25`.
fn is_iso_day(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(at, byte)| at == 4 || at == 7 || byte.is_ascii_digit())
}

/// `2026-12-25` as `261225`.
fn compact_day(day: &str) -> String {
    day.get(2..)
        .unwrap_or_default()
        .chars()
        .filter(|c| *c != '-')
        .collect()
}

/// The settlement after a request symbol's `:` (`BTC/USDT:USDT-261225`
/// settles in `USDT`).
fn settlement_of(request_symbol: &str) -> Option<String> {
    let (_, after) = request_symbol.split_once(':')?;
    let settlement: String = after
        .chars()
        .take_while(char::is_ascii_alphanumeric)
        .collect();
    let rest = &after[settlement.len()..];
    (!settlement.is_empty() && (rest.is_empty() || rest.starts_with('-')))
        .then(|| settlement.to_ascii_uppercase())
}

/// `2026-12-25` as `25 DEC 2026`, `2026-12` as `DEC 2026`.
fn expiry_label(expiry: &str) -> Option<String> {
    const MONTHS: [&str; 12] = [
        "JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC",
    ];
    let mut parts = expiry.split('-');
    let year = parts.next().filter(|y| y.len() == 4)?;
    let month: usize = parts.next().filter(|m| m.len() == 2)?.parse().ok()?;
    let name = MONTHS.get(month.checked_sub(1)?)?;
    match parts.next() {
        None => Some(format!("{name} {year}")),
        Some(day) if day.len() == 2 && parts.next().is_none() => {
            day.parse::<u8>().ok()?;
            Some(format!("{day} {name} {year}"))
        }
        Some(_) => None,
    }
}
