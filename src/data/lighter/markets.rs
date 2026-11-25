//! What Lighter trades ([`Request::Markets`](super::Request)) and what a
//! market's book says now ([`Request::BookTop`](super::Request)), as Lighter
//! writes them and as trading reads them ([`Instruments`]).
//!
//! A market is a perpetual, named by its symbol (`BTC`) and known to
//! transactions by its index, which is not its place in any list. What an
//! order needs of it beyond what trading's [`Instrument`] says is kept here
//! ([`Market`]):
//!
//! - a size and a price are sent as whole numbers of the market's smallest
//!   ones (`size_decimals`, `price_decimals`): what was typed more finely is
//!   refused in words, never rounded into another order
//!   ([`Market::size_units`], [`Market::price_units`]);
//! - an order that opens is no smaller than the market's least size
//!   (`min_base_amount`) and worth no less than its least worth
//!   (`min_quote_amount`), which Lighter would refuse after taking it
//!   ([`Market::too_small`]);
//! - the margin a position takes is a fraction of its worth, in hundredths
//!   of a percent: the least one a market allows (`min_initial_margin_fraction`)
//!   is its most leverage the other way round, and the one it starts with
//!   (`default_initial_margin_fraction`) is how an account that never set
//!   it trades it;
//! - a market trades with a position's own margin only when its margin mode
//!   says so (`market_margin_mode` of `1`), takes no orders once it is not
//!   `active`, and only ones that reduce while it is winding down
//!   (`force_reduce_only`).
//!
//! Two markets may share a symbol, one of them retired: the active ones are
//! listed first, so a symbol finds the one that trades.
//!
//! Spot markets are listed apart in the same reply and not read.

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use serde::Deserialize;

use super::state::{figure, list};
use crate::data::{
    net::ApiError,
    portfolio::{Leverage, MarginMode},
    trading::{Instrument, Instruments},
};

/// What a margin fraction is written in: hundredths of a percent, so this
/// many are the whole of a position's worth (1x).
pub(super) const WHOLE_FRACTION: u32 = 10_000;
/// The most a size comes to in a market's smallest ones.
const MOST_SIZE_UNITS: u64 = (1 << 48) - 1;

/// The reply to [`Request::Markets`](super::Request).
#[derive(Debug, Default, Deserialize)]
pub(super) struct MarketsReply {
    #[serde(default, deserialize_with = "list")]
    order_book_details: Vec<Detail>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Detail {
    symbol: String,
    market_id: u64,
    status: String,
    #[serde(deserialize_with = "figure")]
    min_base_amount: Option<f64>,
    #[serde(deserialize_with = "figure")]
    min_quote_amount: Option<f64>,
    size_decimals: u32,
    price_decimals: u32,
    default_initial_margin_fraction: u32,
    min_initial_margin_fraction: u32,
    market_config: Option<Config>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Config {
    market_margin_mode: i64,
    force_reduce_only: bool,
}

/// One market, with what an order for it takes.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Market {
    /// Its name, which orders and positions go by.
    pub symbol: String,
    /// Its index, which transactions and streams name it by.
    pub id: u64,
    /// Whether orders for it are taken.
    pub active: bool,
    /// The decimal places a size of it has at most.
    pub size_decimals: u32,
    /// The decimal places a price of it has at most.
    pub price_decimals: u32,
    /// The least an order that opens may be for.
    pub min_size: Option<f64>,
    /// The least an order that opens may be worth, in US dollars.
    pub min_worth: Option<f64>,
    /// The margin fraction an account that never set one trades it with.
    pub default_fraction: u32,
    /// The least margin fraction it allows: its most leverage.
    pub min_fraction: u32,
    /// Whether it is traded with a position's own margin only.
    pub isolated_only: bool,
    /// Whether it takes only orders that reduce a position.
    pub reduce_only: bool,
}

/// What the user is told instead of sending anything.
fn refused(message: impl AsRef<str>) -> ApiError {
    ApiError::invalid_request(message)
}

/// `value` as a whole number of steps of `decimals` places; `None` when it
/// is not one: negative, not a number, or finer than the step.
fn steps(value: f64, decimals: u32) -> Option<u64> {
    if !value.is_finite() || value < 0.0 {
        return None;
    }
    let written = format!("{value:.*}", decimals as usize);
    let read: f64 = written.parse().ok()?;
    // A figure is rarely the float it is written as: what is that near a
    // step is the step.
    if (read - value).abs() > value.abs() * 1e-12 {
        return None;
    }
    written.replace('.', "").parse().ok()
}

/// `1`, `2 decimal places`: how many places a figure has at most, in words.
fn places(decimals: u32) -> String {
    match decimals {
        1 => "1 decimal place".to_owned(),
        _ => format!("{decimals} decimal places"),
    }
}

impl Market {
    /// The most leverage it gives: its least margin fraction the other way
    /// round, as a whole number. None for a market that names none.
    pub(super) fn max_leverage(&self) -> Option<u32> {
        (self.min_fraction > 0).then(|| WHOLE_FRACTION / self.min_fraction)
    }

    /// The market as trading lists it.
    fn instrument(&self) -> Instrument {
        Instrument {
            symbol: self.symbol.clone(),
            size_decimals: self.size_decimals,
            max_leverage: self.max_leverage(),
            tradable: self.active,
            isolated_only: self.isolated_only,
        }
    }

    /// How an account that never set it trades the market: its starting
    /// fraction, with the whole account's margin unless it has none of that.
    pub(super) fn default_setting(&self) -> Option<Leverage> {
        let fraction = f64::from(self.default_fraction);
        (fraction > 0.0).then(|| Leverage {
            // To the hundredth, as an account's own setting reads.
            value: (f64::from(WHOLE_FRACTION) / fraction * 100.0).round() / 100.0,
            mode: if self.isolated_only {
                MarginMode::Isolated
            } else {
                MarginMode::Cross
            },
        })
    }

    /// `size` in the market's smallest sizes, or why it cannot be an
    /// order's.
    pub(super) fn size_units(&self, size: f64) -> Result<u64, ApiError> {
        if !size.is_finite() || size <= 0.0 {
            return Err(refused("Enter a size above zero."));
        }
        let too_fine = || {
            refused(match self.size_decimals {
                0 => format!("{} trades in whole units.", self.symbol),
                decimals => format!(
                    "A size of {} has {} at most.",
                    self.symbol,
                    places(decimals)
                ),
            })
        };
        let units = steps(size, self.size_decimals).ok_or_else(too_fine)?;
        match units {
            0 => Err(too_fine()),
            units if units > MOST_SIZE_UNITS => Err(refused(format!(
                "This is more {} than Lighter takes in one order.",
                self.symbol
            ))),
            units => Ok(units),
        }
    }

    /// `price` in the market's smallest prices, or why it cannot be an
    /// order's.
    pub(super) fn price_units(&self, price: f64) -> Result<u32, ApiError> {
        if !price.is_finite() || price <= 0.0 {
            return Err(refused("Enter a price above zero."));
        }
        let too_fine = || {
            refused(match self.price_decimals {
                0 => format!("A price of {} is a whole number.", self.symbol),
                decimals => format!(
                    "A price of {} has {} at most.",
                    self.symbol,
                    places(decimals)
                ),
            })
        };
        let units = steps(price, self.price_decimals).ok_or_else(too_fine)?;
        if units == 0 {
            return Err(too_fine());
        }
        u32::try_from(units).map_err(|_| {
            refused(format!(
                "This price is more than Lighter takes for {}.",
                self.symbol
            ))
        })
    }

    /// The price nearest `price` that an order for the market may have: to
    /// its decimal places. `None` when nothing above zero is left of it.
    pub(super) fn nearest_price(&self, price: f64) -> Option<f64> {
        if !price.is_finite() || price <= 0.0 {
            return None;
        }
        let scale = 10_f64.powi(i32::try_from(self.price_decimals).unwrap_or(0));
        let nearest = (price * scale).round() / scale;
        (nearest > 0.0).then_some(nearest)
    }

    /// The price `slippage` (a fraction) worse than `from` for an order
    /// that buys (or sells), in the market's smallest prices: the worst one
    /// an order at the market's price may trade at.
    pub(super) fn bound_units(
        &self,
        from: f64,
        buys: bool,
        slippage: f64,
    ) -> Result<u32, ApiError> {
        let slippage = slippage.clamp(0.0, 0.5);
        let bound = if buys {
            from * (1.0 + slippage)
        } else {
            from * (1.0 - slippage)
        };
        self.nearest_price(bound)
            .ok_or_else(|| refused(format!("{} has no usable price now.", self.symbol)))
            .and_then(|price| self.price_units(price))
    }

    /// Why an order that opens `size` at about `price` is too small for the
    /// market; `None` when it is not.
    pub(super) fn too_small(&self, size: f64, price: f64) -> Option<String> {
        if let Some(least) = self.min_size
            && size < least
        {
            return Some(format!(
                "The smallest {} order on Lighter is {least}.",
                self.symbol
            ));
        }
        if let Some(least) = self.min_worth
            && size * price < least
        {
            return Some(format!(
                "An order on Lighter is worth ${least} at least; this one is worth about ${:.2}.",
                size * price
            ));
        }
        None
    }
}

/// Lighter's markets: the active ones first, each in Lighter's order.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Markets(Vec<Market>);

impl From<MarketsReply> for Markets {
    fn from(reply: MarketsReply) -> Self {
        let mut markets: Vec<Market> = reply
            .order_book_details
            .into_iter()
            .map(|detail| {
                let config = detail.market_config.unwrap_or_default();
                Market {
                    symbol: detail.symbol,
                    id: detail.market_id,
                    active: detail.status == "active",
                    size_decimals: detail.size_decimals,
                    price_decimals: detail.price_decimals,
                    min_size: detail.min_base_amount.filter(|least| *least > 0.0),
                    min_worth: detail.min_quote_amount.filter(|least| *least > 0.0),
                    default_fraction: detail.default_initial_margin_fraction,
                    min_fraction: detail.min_initial_margin_fraction,
                    isolated_only: config.market_margin_mode == 1,
                    reduce_only: config.force_reduce_only,
                }
            })
            .collect();
        // Stable: each kind keeps Lighter's order.
        markets.sort_by_key(|market| !market.active);
        Self(markets)
    }
}

impl Markets {
    /// The market named `symbol`, in any case: the exact name before one
    /// that differs by case, and an active one before a retired one.
    pub(super) fn find(&self, symbol: &str) -> Option<&Market> {
        let symbol = symbol.trim();
        let exact = self.0.iter().find(|market| market.symbol == symbol);
        exact.or_else(|| {
            let same = |market: &&Market| market.symbol.eq_ignore_ascii_case(symbol);
            self.0.iter().find(same)
        })
    }

    /// The market whose index is `id`.
    pub(super) fn by_id(&self, id: u64) -> Option<&Market> {
        self.0.iter().find(|market| market.id == id)
    }

    /// The markets as trading lists them, in the same order: what a name
    /// finds there is what it finds here.
    pub(super) fn instruments(&self) -> Instruments {
        Instruments(self.0.iter().map(Market::instrument).collect())
    }
}

/// The reply to [`Request::BookTop`](super::Request): the best offer and the
/// best bid, each alone or missing.
#[derive(Debug, Default, Deserialize)]
pub(super) struct BookTop {
    #[serde(default, deserialize_with = "list")]
    asks: Vec<Level>,
    #[serde(default, deserialize_with = "list")]
    bids: Vec<Level>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Level {
    #[serde(deserialize_with = "figure")]
    price: Option<f64>,
}

impl BookTop {
    /// The middle of the best bid and the best offer, or the one there is
    /// alone; `None` for an empty book.
    pub(super) fn mid(&self) -> Option<f64> {
        let best = |levels: &[Level]| {
            let price = levels.first()?.price?;
            (price > 0.0).then_some(price)
        };
        match (best(&self.bids), best(&self.asks)) {
            (Some(bid), Some(offer)) => Some((bid + offer) / 2.0),
            (one, other) => one.or(other),
        }
    }
}

/// What the client has learned of Lighter's markets in passing, kept for
/// what cannot wait for a request: the price nearest another
/// ([`Markets`], as last read), and a market's index by its symbol, which
/// following its price takes. The indexes come from the markets when they
/// were read, and before that from the accounts read, whose every entry
/// names both.
#[derive(Default)]
pub(super) struct Directory {
    markets: RefCell<Option<Rc<Markets>>>,
    ids: RefCell<HashMap<String, u64>>,
}

impl Directory {
    /// Keeps `markets` as the markets last read.
    pub(super) fn read(&self, markets: &Rc<Markets>) {
        let mut ids = self.ids.borrow_mut();
        // The last first: a symbol two markets share stays the active one's.
        for market in markets.0.iter().rev() {
            ids.insert(market.symbol.clone(), market.id);
        }
        *self.markets.borrow_mut() = Some(markets.clone());
    }

    /// Notes that the market named `symbol` has the index `id`, unless its
    /// index is known already.
    pub(super) fn note(&self, symbol: &str, id: u64) {
        let mut ids = self.ids.borrow_mut();
        if !ids.contains_key(symbol) {
            ids.insert(symbol.to_owned(), id);
        }
    }

    /// The markets as last read, if they ever were.
    pub(super) fn markets(&self) -> Option<Rc<Markets>> {
        self.markets.borrow().clone()
    }

    /// The index of the market named `symbol`, if it was ever learned.
    pub(super) fn id(&self, symbol: &str) -> Option<u64> {
        self.ids.borrow().get(symbol).copied()
    }

    /// The symbol of the market whose index is `id`, from the markets last
    /// read.
    pub(super) fn symbol(&self, id: u64) -> Option<String> {
        let markets = self.markets.borrow();
        Some(markets.as_ref()?.by_id(id)?.symbol.clone())
    }
}

#[cfg(test)]
pub(super) mod tests {
    use serde_json::json;

    use super::*;

    /// Markets as Lighter answered on 2026-11-22, cut to what is read: BTC,
    /// a market with a position's own margin only, and a symbol two markets
    /// share, the retired one listed first.
    pub(in crate::data::lighter) fn markets() -> Markets {
        let reply: MarketsReply = serde_json::from_value(json!({
            "code": 200,
            "order_book_details": [
                { "symbol": "ASELS", "market_id": 8286, "market_type": "perp",
                  "status": "inactive", "min_base_amount": "0.1", "min_quote_amount": "10.000000",
                  "size_decimals": 1, "price_decimals": 3,
                  "default_initial_margin_fraction": 2000, "min_initial_margin_fraction": 2000,
                  "market_config": { "market_margin_mode": 0, "force_reduce_only": true } },
                { "symbol": "BTC", "market_id": 1, "market_type": "perp", "status": "active",
                  "taker_fee": "0.0000", "min_base_amount": "0.00007",
                  "min_quote_amount": "10.000000", "supported_size_decimals": 5,
                  "size_decimals": 5, "price_decimals": 1, "quote_multiplier": 1,
                  "default_initial_margin_fraction": 500, "min_initial_margin_fraction": 200,
                  "maintenance_margin_fraction": 120, "mark_price": "61125.7",
                  "last_trade_price": 61163.4,
                  "market_config": { "market_margin_mode": 0, "force_reduce_only": false,
                                     "trading_hours": "" } },
                { "symbol": "ASELS", "market_id": 10635, "status": "active",
                  "min_base_amount": "0.1", "min_quote_amount": "10.000000",
                  "size_decimals": 1, "price_decimals": 3,
                  "default_initial_margin_fraction": 3333, "min_initial_margin_fraction": 3333,
                  "market_config": { "market_margin_mode": 1, "force_reduce_only": false } },
                { "symbol": "1000PEPE", "market_id": 4, "status": "active",
                  "min_base_amount": "500", "min_quote_amount": "10.000000",
                  "size_decimals": 0, "price_decimals": 6,
                  "default_initial_margin_fraction": 1000, "min_initial_margin_fraction": 1000 },
            ],
            "spot_order_book_details": [
                { "symbol": "ETH/USDC", "market_id": 2048, "market_type": "spot" },
            ],
        }))
        .expect("the documented reply reads");
        reply.into()
    }

    #[test]
    fn markets_read_with_what_an_order_takes_the_active_ones_first() {
        let markets = markets();
        let listed: Vec<(&str, u64)> = markets
            .0
            .iter()
            .map(|market| (market.symbol.as_str(), market.id))
            .collect();
        // The spot market is not read, and the retired one comes last.
        assert_eq!(
            listed,
            [
                ("BTC", 1),
                ("ASELS", 10_635),
                ("1000PEPE", 4),
                ("ASELS", 8_286)
            ]
        );
        let btc = markets.find(" btc ").unwrap();
        assert_eq!((btc.size_decimals, btc.price_decimals), (5, 1));
        assert_eq!((btc.min_size, btc.min_worth), (Some(0.000_07), Some(10.0)));
        // 2% at the least is 50x at the most; 5% to start with is 20x.
        assert_eq!(btc.max_leverage(), Some(50));
        assert_eq!(
            btc.default_setting(),
            Some(Leverage {
                value: 20.0,
                mode: MarginMode::Cross
            })
        );
        // A symbol two markets share is the one that trades.
        let shared = markets.find("ASELS").unwrap();
        assert_eq!((shared.id, shared.active), (10_635, true));
        assert_eq!(shared.max_leverage(), Some(3));
        assert_eq!(
            shared.default_setting(),
            Some(Leverage {
                value: 3.0,
                mode: MarginMode::Isolated
            })
        );
        assert!(markets.by_id(8_286).unwrap().reduce_only);
        assert_eq!(markets.find("DOGE"), None);

        // As trading lists them: the same order, so the same finds.
        let instruments = markets.instruments();
        let asels = instruments.find("asels").unwrap();
        assert!(asels.tradable && asels.isolated_only);
        assert_eq!((asels.size_decimals, asels.max_leverage), (1, Some(3)));
        assert!(!instruments.0[3].tradable);

        // Nothing listed reads as nothing, not as a failure.
        let empty: MarketsReply = serde_json::from_value(json!({ "code": 200 })).unwrap();
        assert_eq!(Markets::from(empty), Markets::default());
    }

    #[test]
    fn a_size_and_a_price_are_whole_numbers_of_the_markets_smallest() {
        let markets = markets();
        let btc = markets.find("BTC").unwrap();
        assert_eq!(btc.size_units(0.0015), Ok(150));
        assert_eq!(btc.size_units(2.0), Ok(200_000));
        assert_eq!(btc.size_units(0.000_01), Ok(1));
        // However the float falls: 0.1 and 0.2 are not 0.3 to the last bit.
        assert_eq!(btc.size_units(0.1 + 0.2), Ok(30_000));
        assert_eq!(btc.price_units(61_125.7), Ok(611_257));
        assert_eq!(btc.price_units(61_000.0), Ok(610_000));
        // One place too many is not rounded into another order.
        assert_eq!(
            btc.size_units(0.000_001).unwrap_err().message,
            "A size of BTC has 5 decimal places at most."
        );
        assert_eq!(
            btc.price_units(61_125.75).unwrap_err().message,
            "A price of BTC has 1 decimal place at most."
        );
        let pepe = markets.find("1000PEPE").unwrap();
        assert_eq!(pepe.size_units(1_500.0), Ok(1_500));
        assert_eq!(
            pepe.size_units(1_500.5).unwrap_err().message,
            "1000PEPE trades in whole units."
        );
        assert_eq!(pepe.price_units(0.011_667), Ok(11_667));
        for nothing in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert_eq!(
                btc.size_units(nothing).unwrap_err().message,
                "Enter a size above zero."
            );
            assert_eq!(
                btc.price_units(nothing).unwrap_err().message,
                "Enter a price above zero."
            );
        }
        // More than a transaction's integers hold.
        assert_eq!(
            btc.price_units(500_000_000.0).unwrap_err().message,
            "This price is more than Lighter takes for BTC."
        );
        assert_eq!(
            btc.size_units(3e9).unwrap_err().message,
            "This is more BTC than Lighter takes in one order."
        );
    }

    #[test]
    fn a_price_is_brought_to_the_markets_places_and_bounded_from_another() {
        let markets = markets();
        let btc = markets.find("BTC").unwrap();
        assert_eq!(btc.nearest_price(61_125.74), Some(61_125.7));
        assert_eq!(btc.nearest_price(61_125.75), Some(61_125.8));
        assert_eq!(btc.nearest_price(0.01), None);
        assert_eq!(btc.nearest_price(f64::NAN), None);
        // 5% above 61,125.7 is 64,181.985: to one place.
        assert_eq!(btc.bound_units(61_125.7, true, 0.05), Ok(641_820));
        assert_eq!(btc.bound_units(61_125.7, false, 0.05), Ok(580_694));
        // A tenth past a stop at 58,000.
        assert_eq!(btc.bound_units(58_000.0, false, 0.10), Ok(522_000));
        assert_eq!(
            btc.bound_units(0.0, true, 0.05).unwrap_err().message,
            "BTC has no usable price now."
        );
    }

    #[test]
    fn an_order_that_opens_is_no_smaller_than_the_market_allows() {
        let markets = markets();
        let btc = markets.find("BTC").unwrap();
        assert_eq!(btc.too_small(0.001, 61_000.0), None);
        assert_eq!(
            btc.too_small(0.000_05, 61_000.0).as_deref(),
            Some("The smallest BTC order on Lighter is 0.00007.")
        );
        // Large enough in size, and worth too little.
        assert_eq!(
            btc.too_small(0.0001, 61_000.0).as_deref(),
            Some("An order on Lighter is worth $10 at least; this one is worth about $6.10.")
        );
    }

    #[test]
    fn the_top_of_a_book_is_the_middle_of_its_best_bid_and_offer() {
        // As Lighter answered on 2026-11-22.
        let top: BookTop = serde_json::from_value(json!({
            "code": 200, "total_asks": 1,
            "asks": [{ "order_index": 562_953_952_549_819_u64, "order_id": "562953952549819",
                       "owner_account_index": 90_364, "initial_base_amount": "0.01340",
                       "remaining_base_amount": "0.00940", "price": "60866.6" }],
            "total_bids": 1,
            "bids": [{ "order_index": 844_420_878_607_412_u64, "price": "60864.4" }],
        }))
        .unwrap();
        assert_eq!(top.mid(), Some(60_865.5));
        // One side alone, and neither.
        let one: BookTop =
            serde_json::from_value(json!({ "asks": [], "bids": [{ "price": "7.5" }] })).unwrap();
        assert_eq!(one.mid(), Some(7.5));
        let none: BookTop = serde_json::from_value(json!({ "code": 200, "asks": null })).unwrap();
        assert_eq!(none.mid(), None);
    }

    #[test]
    fn a_markets_index_is_learned_from_whatever_names_it() {
        let directory = Directory::default();
        assert_eq!(directory.id("BTC"), None);
        assert!(directory.markets().is_none());
        // An account's entry names both, before the markets are read.
        directory.note("BTC", 1);
        directory.note("ASELS", 8_286);
        assert_eq!(
            (directory.id("BTC"), directory.id("ASELS")),
            (Some(1), Some(8_286))
        );
        assert_eq!(directory.symbol(1), None);
        // The markets say which of a shared symbol's trades, and are kept.
        directory.read(&Rc::new(markets()));
        assert_eq!(directory.id("ASELS"), Some(10_635));
        assert_eq!(directory.symbol(4).as_deref(), Some("1000PEPE"));
        assert!(directory.markets().is_some());
        // What an account says after that changes nothing.
        directory.note("ASELS", 8_286);
        assert_eq!(directory.id("ASELS"), Some(10_635));
    }
}
