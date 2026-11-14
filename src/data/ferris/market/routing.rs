//! Where a selection streams and fetches: its topics and their params,
//! its route key, whether it can be streamed, and the venues' repairs of
//! request symbol aliases.

use serde_json::{Map, Value, json};

use super::{
    MarketSelection, MarketType,
    expiry::{compact_day, is_bare, is_iso_day, plain_pair},
};
use crate::data::{
    ferris::{
        Channel, FerrisTopic, Venue,
        protocol::{exchange_id, params_object},
    },
    net::ApiError,
};

/// What a REST request for a market names
/// ([`MarketSelection::rest_target`]), as Ferris reads it: the exchange id
/// lower case, the symbol trimmed, the params an object.
#[derive(Clone, Debug, PartialEq)]
pub struct RestTarget {
    pub exchange: String,
    pub symbol: String,
    pub params: Map<String, Value>,
}

impl MarketSelection {
    /// The topic of `channel` that streams this market.
    pub fn topic(&self, channel: Channel) -> Result<FerrisTopic, ApiError> {
        FerrisTopic::market(
            channel,
            &self.exchange,
            &self.routed_symbol(),
            self.request_params(),
        )
    }

    /// What a REST request for it names (its exchange, symbol and params):
    /// the streamed symbol, except on venues whose REST products are told
    /// apart by catalog id (KuCoin and Bitfinex, [`Venue::rest_by_market_id`]).
    pub fn rest_target(&self) -> Result<RestTarget, ApiError> {
        let symbol = match &self.market_id {
            Some(id) if Venue::of(&self.exchange).rest_by_market_id => id.clone(),
            _ => self.routed_symbol(),
        };
        let symbol = symbol.trim();
        if symbol.is_empty() {
            return Err(ApiError::invalid_request("a Ferris request needs a symbol"));
        }
        Ok(RestTarget {
            exchange: exchange_id(&self.exchange)?,
            symbol: symbol.to_owned(),
            params: params_object(self.request_params())?,
        })
    }

    /// The REST target as a topic of `channel` (what the identity keys).
    pub(super) fn rest_topic(&self, channel: Channel) -> Result<FerrisTopic, ApiError> {
        let RestTarget {
            exchange,
            symbol,
            params,
        } = self.rest_target()?;
        FerrisTopic::market(channel, &exchange, &symbol, Value::Object(params))
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
        // A venue that routes by type never picks a product by category.
        let category = if !Venue::of(&self.exchange).routes_by_type.is_empty() {
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

    /// Whether Ferris can stream it: a dated future of Binance, Bybit or
    /// KuCoin ([`Venue::futures_need_day`]) needs its exact expiry day in
    /// the request symbol (the old app's `isRoutableMarketSelection`); a
    /// month-code contract has none.
    pub fn is_routable(&self) -> bool {
        if self.kind != MarketType::Future || !Venue::of(&self.exchange).futures_need_day {
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

    /// The params that pick the product on venues that need them (the old
    /// app's `marketRequestParams`).
    fn request_params(&self) -> Value {
        if Venue::of(&self.exchange)
            .routes_by_type
            .contains(&self.kind)
        {
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
/// `canonicalizeRequestSymbol`), by the venue's own repair
/// ([`Venue::repair_symbol`]: [`apex_symbol`], [`kucoin_symbol`],
/// [`bitfinex_symbol`]); as requested elsewhere.
pub(super) fn canonical_symbol(
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
    Venue::of(exchange)
        .repair_symbol
        .and_then(|repair| {
            repair(&Requested {
                kind,
                requested,
                settle: settle.as_deref(),
                expiry,
                exchange_symbol,
            })
        })
        .unwrap_or_else(|| requested.to_owned())
}

/// What a venue's symbol repair reads ([`Venue::repair_symbol`]).
pub(in crate::data::ferris) struct Requested<'a> {
    kind: MarketType,
    /// The symbol asked for, as given.
    requested: &'a str,
    /// The settlement asset, upper case, when known.
    settle: Option<&'a str>,
    expiry: Option<&'a str>,
    exchange_symbol: &'a str,
}

/// KuCoin's pairs carry their settlement, its dated futures their day; a
/// month-code native id has no exact day and is kept as its own identity,
/// not routed to another contract.
pub(in crate::data::ferris) fn kucoin_symbol(input: &Requested<'_>) -> Option<String> {
    let pair = input.requested.trim().to_ascii_uppercase();
    let (base, quote) = plain_pair(&pair)?;
    if !matches!(input.kind, MarketType::Perp | MarketType::Future) {
        return None;
    }
    let settlement = input.settle.map(str::to_owned).or_else(|| match quote {
        "USD" => Some(base.to_owned()),
        "USDT" | "USDC" => Some(quote.to_owned()),
        _ => None,
    });
    match (input.kind, settlement) {
        (MarketType::Perp, Some(settlement)) => Some(format!("{pair}:{settlement}")),
        (MarketType::Future, Some(settlement)) if input.expiry.is_some_and(is_iso_day) => {
            let day = compact_day(input.expiry.unwrap_or_default());
            Some(format!("{pair}:{settlement}-{day}"))
        }
        (MarketType::Future, _) if !input.exchange_symbol.is_empty() => {
            Some(input.exchange_symbol.to_owned())
        }
        _ => None,
    }
}

/// Bitfinex perpetual pairs carry their settlement.
pub(in crate::data::ferris) fn bitfinex_symbol(input: &Requested<'_>) -> Option<String> {
    if input.kind != MarketType::Perp {
        return None;
    }
    let pair = input.requested.trim().to_ascii_uppercase();
    match pair.split_once('/') {
        Some((base, quote)) if is_bare(base) && !quote.is_empty() && !quote.contains(':') => {
            let settlement = input.settle.unwrap_or(quote);
            Some(format!("{pair}:{settlement}"))
        }
        _ => None,
    }
}

/// Apex `X/USDT` perpetuals stream as `X/USDT:USDT`.
pub(in crate::data::ferris) fn apex_symbol(input: &Requested<'_>) -> Option<String> {
    if input.kind != MarketType::Perp {
        return None;
    }
    let pair = input.requested.trim().to_ascii_uppercase();
    match plain_pair(&pair) {
        Some((_, "USDT")) => Some(format!("{pair}:USDT")),
        _ => None,
    }
}
