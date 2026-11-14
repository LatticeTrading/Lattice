//! A catalog row (`fetchMarkets`) as a selection: its members read
//! leniently, its category, and the request symbol Ferris is asked for.

use serde_json::Value;

use super::{
    MarketSelection, MarketType,
    expiry::{compact_day, derive_expiry, is_iso_day, plain_pair},
    routing::canonical_symbol,
};
use crate::data::ferris::Venue;

impl MarketSelection {
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
            let venue = Venue::of(&exchange);
            let family = derive_family(kind, &base, &quote, settle.as_deref());
            match (kind, family) {
                (MarketType::Spot, _) if venue.routes_by_type.contains(&MarketType::Spot) => None,
                (MarketType::Spot, _) => Some("spot".to_owned()),
                (_, Some(family @ (Family::Linear | Family::Inverse))) if venue.linear_inverse => {
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
/// What differs by venue is in its [`Venue`] record: its own rule first
/// (Extended, Hyperliquid), then whether it has linear and inverse
/// families (Binance, Bybit) and whether its symbols carry a settlement.
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
    let venue = Venue::of(input.exchange);
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
            input.exchange,
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

    if let Some(symbol) = venue.derive_symbol.and_then(|derive| {
        derive(&Derived {
            kind: input.kind,
            base: &base,
            quote: &quote,
            pair: &pair,
            settlement: settlement.as_deref(),
            catalog,
            exchange_symbol,
        })
    }) {
        return symbol;
    }
    if !venue.linear_inverse {
        let listed = if catalog.is_empty() {
            exchange_symbol
        } else {
            catalog
        };
        if input.kind == MarketType::Perp
            && !venue.unsettled_symbols
            && plain_pair(&listed.to_uppercase()).is_some()
            && let Some(settlement) = settlement
        {
            return format!("{pair}:{settlement}");
        }
        return canonical_symbol(
            input.exchange,
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

/// What a venue's own request symbol rule reads of a catalog market
/// ([`Venue::derive_symbol`]).
pub(in crate::data::ferris) struct Derived<'a> {
    kind: MarketType,
    /// Upper case.
    base: &'a str,
    /// Upper case.
    quote: &'a str,
    /// `BASE/QUOTE`.
    pair: &'a str,
    /// The settlement asset, upper case, when known.
    settlement: Option<&'a str>,
    /// The catalog's own symbol, trimmed.
    catalog: &'a str,
    /// The native id, or else the catalog's symbol.
    exchange_symbol: &'a str,
}

/// Extended perpetuals are `BASE/QUOTE:SETTLE`: its quote-qualified wire
/// alias is not evidence of the market's family.
pub(in crate::data::ferris) fn extended_symbol(input: &Derived<'_>) -> Option<String> {
    (input.kind == MarketType::Perp)
        .then(|| format!("{}:{}", input.pair, input.settlement.unwrap_or(input.quote)))
}

/// Hyperliquid spot is `BASE/QUOTE`; its perpetuals, HIP-3 ones included,
/// are the catalog's or native settled symbol, else `BASE/USDC:USDC`.
pub(in crate::data::ferris) fn hyperliquid_symbol(input: &Derived<'_>) -> Option<String> {
    Some(if input.kind == MarketType::Spot {
        input.pair.to_owned()
    } else if input.catalog.contains(':') {
        input.catalog.to_owned()
    } else if input.exchange_symbol.contains(':') {
        input.exchange_symbol.to_owned()
    } else {
        format!("{}/USDC:USDC", input.base)
    })
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
