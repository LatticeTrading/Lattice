//! What trading is said in, whatever the exchange: an order to place
//! ([`OrderRequest`]), what became of it ([`Placed`]), the orders that
//! rest ([`OpenOrders`]), what a venue trades ([`Instruments`]), how far
//! the connected account has come to trading on a venue ([`Access`]), and
//! what happens to an account that is followed live ([`AccountEvent`]).
//! What an instrument is priced at now is not trading's to say: it is the
//! prices' ([`crate::data::prices`]). What closes a position for its owner
//! is its [`Exits`]: a take profit and a stop loss, on an order that opens
//! one or on one already held; what closes it at once is an order that only
//! reduces it ([`OrderRequest::closing`]). How an account is set to trade an
//! instrument (its leverage, and whose margin backs it) is the portfolio's
//! [`Leverage`](crate::data::portfolio::Leverage), read and set.
//!
//! Sizes and prices are `f64`, as the portfolio's figures are: they are
//! typed by a person, shown rounded and compared with limits. A venue
//! writes them to its own precision when it sends an order, and refuses
//! what would have to be rounded to fit ([`super::TradingVenue::place`]):
//! an order is never sent for another size or price than was asked.

use crate::data::portfolio::{Direction, Fill};

/// How far from the mid price an order sent at the market's price may
/// trade, where whoever sends it has no bound of their own: 5%, what
/// Hyperliquid's own SDK allows one.
pub const MARKET_SLIPPAGE: f64 = 0.05;

/// Which way an order trades.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Side {
    Buy,
    Sell,
}

impl Side {
    pub const ALL: [Self; 2] = [Self::Buy, Self::Sell];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Buy => "Buy",
            Self::Sell => "Sell",
        }
    }

    /// What settings and test ids call it.
    pub const fn id(self) -> &'static str {
        match self {
            Self::Buy => "buy",
            Self::Sell => "sell",
        }
    }

    /// The side that makes a position facing `direction` smaller.
    pub const fn closing(direction: Direction) -> Self {
        match direction {
            Direction::Long => Self::Sell,
            Direction::Short => Self::Buy,
        }
    }
}

/// How long a limit order stands.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum TimeInForce {
    /// It rests in the book until filled or cancelled.
    #[default]
    GoodTilCancelled,
    /// What does not fill at once is cancelled.
    ImmediateOrCancel,
    /// It only ever rests: refused if it would trade at once.
    PostOnly,
}

impl TimeInForce {
    pub const ALL: [Self; 3] = [
        Self::GoodTilCancelled,
        Self::ImmediateOrCancel,
        Self::PostOnly,
    ];

    /// The short name traders know it by.
    pub const fn label(self) -> &'static str {
        match self {
            Self::GoodTilCancelled => "GTC",
            Self::ImmediateOrCancel => "IOC",
            Self::PostOnly => "Post only",
        }
    }

    pub const fn id(self) -> &'static str {
        match self {
            Self::GoodTilCancelled => "gtc",
            Self::ImmediateOrCancel => "ioc",
            Self::PostOnly => "post-only",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|tif| tif.id() == id)
    }
}

/// At what price an order trades.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum OrderKind {
    /// At `price` or better.
    Limit {
        price: f64,
        time_in_force: TimeInForce,
    },
    /// At once, at what the book gives, and no worse than `slippage` from
    /// the mid price (a fraction: `0.05` is 5%). What that does not fill is
    /// cancelled.
    Market { slippage: f64 },
}

/// The prices at which a position is closed for its owner. Each is a
/// trigger, not a limit: once the market reaches it, the position is
/// closed at what the market gives (the venue bounds how far from the
/// trigger that may be). Either may be left out.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Exits {
    /// Where it is closed with a gain: above a long's price, below a
    /// short's.
    pub take_profit: Option<f64>,
    /// Where it is closed with a loss: below a long's price, above a
    /// short's.
    pub stop_loss: Option<f64>,
}

impl Exits {
    /// Whether it names neither.
    pub fn is_empty(&self) -> bool {
        self.take_profit.is_none() && self.stop_loss.is_none()
    }

    /// Why these are not the exits of a position that is `long` (or short)
    /// and priced at `price` now; `None` when they are: each is on its own
    /// side of the price.
    pub fn misplaced(&self, long: bool, price: f64) -> Option<&'static str> {
        let above = |exit: Option<f64>| exit.is_some_and(|exit| exit > price);
        let below = |exit: Option<f64>| exit.is_some_and(|exit| exit < price);
        let (profit, loss) = (self.take_profit, self.stop_loss);
        match long {
            true if profit.is_some() && !above(profit) => {
                Some("A long's take profit is above its price.")
            }
            true if loss.is_some() && !below(loss) => {
                Some("A long's stop loss is below its price.")
            }
            false if profit.is_some() && !below(profit) => {
                Some("A short's take profit is below its price.")
            }
            false if loss.is_some() && !above(loss) => {
                Some("A short's stop loss is above its price.")
            }
            _ => None,
        }
    }
}

/// An order to place.
#[derive(Clone, Debug, PartialEq)]
pub struct OrderRequest {
    /// The venue's name for what is traded ([`Instrument::symbol`]).
    pub instrument: String,
    pub side: Side,
    /// How much, in the instrument's units.
    pub size: f64,
    pub kind: OrderKind,
    /// Whether it may only make a position smaller.
    pub reduce_only: bool,
    /// What closes what it opens, placed with it and of its size: they
    /// wait for it to trade, and go with it if it is cancelled.
    pub exits: Exits,
}

impl OrderRequest {
    /// The order that closes `size` of a position in `instrument` facing
    /// `direction`, at the market's price: on the other side of it, and
    /// only ever making it smaller, so what is no longer held by the time
    /// it arrives is not sold short or bought back long.
    pub fn closing(instrument: &str, direction: Direction, size: f64) -> Self {
        Self {
            instrument: instrument.to_owned(),
            side: Side::closing(direction),
            size,
            kind: OrderKind::Market {
                slippage: MARKET_SLIPPAGE,
            },
            reduce_only: true,
            exits: Exits::default(),
        }
    }
}

/// What became of an order the venue took.
#[derive(Clone, Debug, PartialEq)]
pub enum Placed {
    /// It rests in the book.
    Resting { id: String },
    /// It traded: how much, and at what average price.
    Filled { id: String, size: f64, price: f64 },
    /// The venue took it and says no more yet.
    Accepted,
}

/// An order that rests, or waits for its trigger.
#[derive(Clone, Debug, PartialEq)]
pub struct OpenOrder {
    /// The venue's id of it, which cancels it.
    pub id: String,
    pub instrument: String,
    pub side: Side,
    pub price: Option<f64>,
    /// What is left of it to fill.
    pub size: Option<f64>,
    pub reduce_only: bool,
    /// What the venue calls its kind (`Limit`, `Stop Market`).
    pub kind: String,
    /// The price it waits for, for one that does.
    pub trigger_price: Option<f64>,
    /// The exits placed with it, which wait for it to trade.
    pub exits: Exits,
    /// Whether it is an exit of a whole position, whatever that comes to
    /// be worth: its size is then the position's, and none of its own.
    pub whole_position: bool,
}

/// An account's open orders, the newest first.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OpenOrders(pub Vec<OpenOrder>);

/// An open order to cancel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CancelRequest {
    pub instrument: String,
    /// [`OpenOrder::id`].
    pub id: String,
}

/// Something a venue trades.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Instrument {
    /// The venue's name for it (`BTC`), which orders name it by.
    pub symbol: String,
    /// The decimal places a size of it has at most.
    pub size_decimals: u32,
    /// The most leverage the venue gives it.
    pub max_leverage: Option<u32>,
    /// Whether orders for it are taken (`false`: delisted).
    pub tradable: bool,
    /// Whether it is traded with a position's own margin only, never the
    /// whole account's.
    pub isolated_only: bool,
}

impl Instrument {
    /// The most of it, no more than `size`, that an order may be for:
    /// `size` cut to the decimal places a size of it has. `None` when
    /// nothing above zero is left of it.
    pub fn cut_size(&self, size: f64) -> Option<f64> {
        if !size.is_finite() || size <= 0.0 {
            return None;
        }
        let scale = 10_f64.powi(i32::try_from(self.size_decimals).unwrap_or(0));
        // A size is rarely the float it is written as (`0.29 * 100` is a
        // hair under 29): what is that near a step is the step.
        let cut = (size * scale + 1e-6).floor() / scale;
        (cut > 0.0).then_some(cut)
    }
}

/// What a venue trades, in the venue's own order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Instruments(pub Vec<Instrument>);

impl Instruments {
    /// The instrument named `symbol`, in any case.
    pub fn find(&self, symbol: &str) -> Option<&Instrument> {
        let symbol = symbol.trim();
        // The exact name first: two may differ by case alone.
        let exact = self.0.iter().find(|listed| listed.symbol == symbol);
        exact.or_else(|| {
            let same = |listed: &&Instrument| listed.symbol.eq_ignore_ascii_case(symbol);
            self.0.iter().find(same)
        })
    }
}

/// Something that happened to an account followed live, as its venue
/// said it when it happened.
#[derive(Clone, Debug, PartialEq)]
pub enum AccountEvent {
    /// An order of the account's traded, in whole or in part.
    Filled(Fill),
}

/// How far the connected account has come to trading on a venue.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Access {
    /// Nothing may trade for it yet.
    #[default]
    Locked,
    /// What it allowed in an earlier run is being taken up again: the
    /// venue is asked whether it still holds. No wallet is asked.
    Resuming,
    /// It is being asked to allow it (its wallet shows what to sign).
    Enabling,
    /// Orders can be placed for it.
    Ready,
    /// It was asked and it did not come to be: why.
    Failed(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_instrument_is_found_by_its_name_in_any_case() {
        let instrument = |symbol: &str| Instrument {
            symbol: symbol.to_owned(),
            size_decimals: 2,
            max_leverage: None,
            tradable: true,
            isolated_only: false,
        };
        let listed = Instruments(vec![
            instrument("BTC"),
            instrument("kPEPE"),
            instrument("KPEPE"),
        ]);
        let found = |symbol: &str| listed.find(symbol).map(|found| found.symbol.as_str());
        assert_eq!(found(" btc "), Some("BTC"));
        // The exact name before one that differs by case.
        assert_eq!(found("KPEPE"), Some("KPEPE"));
        assert_eq!(found("kPEPE"), Some("kPEPE"));
        assert_eq!(found("kpepe"), Some("kPEPE"));
        assert_eq!(found("DOGE"), None);

        assert_eq!(
            TimeInForce::from_id("post-only"),
            Some(TimeInForce::PostOnly)
        );
        assert_eq!(TimeInForce::from_id("fok"), None);
    }

    #[test]
    fn a_position_is_closed_by_an_order_that_only_reduces_it() {
        let long = OrderRequest::closing("BTC", Direction::Long, 0.5);
        assert_eq!(
            long,
            OrderRequest {
                instrument: "BTC".to_owned(),
                side: Side::Sell,
                size: 0.5,
                kind: OrderKind::Market {
                    slippage: MARKET_SLIPPAGE
                },
                reduce_only: true,
                exits: Exits::default(),
            }
        );
        let short = OrderRequest::closing("ETH", Direction::Short, 2.0);
        assert_eq!((short.side, short.reduce_only), (Side::Buy, true));
    }

    #[test]
    fn a_size_is_cut_to_the_places_its_instrument_has() {
        let places = |size_decimals: u32| Instrument {
            symbol: "BTC".to_owned(),
            size_decimals,
            max_leverage: None,
            tradable: true,
            isolated_only: false,
        };
        // Down, never up: half of 0.0335 in four places.
        assert_eq!(places(4).cut_size(0.0335 / 2.0), Some(0.0167));
        assert_eq!(places(5).cut_size(0.01 / 2.0), Some(0.005));
        assert_eq!(places(0).cut_size(7.9), Some(7.0));
        // What already fits stays, however its float falls (0.29 times a
        // hundred is a hair under 29, and so is 0.58 halved).
        assert_eq!(places(2).cut_size(0.29), Some(0.29));
        assert_eq!(places(2).cut_size(0.58 / 2.0), Some(0.29));
        assert_eq!(places(1).cut_size(0.3), Some(0.3));
        // Written as a person would type it.
        assert_eq!(
            format!("{}", places(4).cut_size(0.01675).unwrap()),
            "0.0167"
        );
        // Nothing left, and nothing to cut.
        assert_eq!(places(0).cut_size(0.5), None);
        assert_eq!(places(5).cut_size(0.000_004), None);
        assert_eq!(places(2).cut_size(0.0), None);
        assert_eq!(places(2).cut_size(-1.0), None);
        assert_eq!(places(2).cut_size(f64::NAN), None);
    }

    #[test]
    fn exits_are_each_on_their_own_side_of_the_price() {
        let exits = |take_profit: Option<f64>, stop_loss: Option<f64>| Exits {
            take_profit,
            stop_loss,
        };
        assert!(Exits::default().is_empty());
        assert!(!exits(None, Some(1.0)).is_empty());
        // None, one or both, each where it belongs.
        for (long, placed) in [
            (true, exits(None, None)),
            (true, exits(Some(110.0), Some(90.0))),
            (true, exits(Some(110.0), None)),
            (false, exits(Some(90.0), Some(110.0))),
            (false, exits(None, Some(110.0))),
        ] {
            assert_eq!(placed.misplaced(long, 100.0), None, "{long} {placed:?}");
        }
        // On the wrong side, or at the price itself.
        assert_eq!(
            exits(Some(90.0), None).misplaced(true, 100.0),
            Some("A long's take profit is above its price.")
        );
        assert_eq!(
            exits(Some(110.0), Some(100.0)).misplaced(true, 100.0),
            Some("A long's stop loss is below its price.")
        );
        assert_eq!(
            exits(Some(110.0), None).misplaced(false, 100.0),
            Some("A short's take profit is below its price.")
        );
        assert_eq!(
            exits(None, Some(90.0)).misplaced(false, 100.0),
            Some("A short's stop loss is above its price.")
        );
    }
}
