//! Prices: what an instrument is priced at on an exchange now, followed
//! live, in one shape whatever the exchange and wherever the price comes
//! from (what values a position between two reports of its account, and
//! what a ticket shows beside the market it trades).
//!
//! A price is neither an account's nor trading's: an account that is only
//! watched is priced, and so is a market nobody holds. So it has a source
//! of its own ([`PriceSource`]), listed by the exchange it prices
//! ([`Prices`]). The source is whatever streams the exchange's prices
//! best: the exchange's own integration (Hyperliquid's WebSocket), or
//! another's stream of the same markets. An exchange nothing prices is
//! simply not followed: what shows is then what its reports say.
//!
//! An instrument is named as its exchange names it, which is the name its
//! positions go by ([`Position::instrument`](super::portfolio::Position))
//! and its orders ([`Instrument::symbol`](super::trading::Instrument)).
//!
//! Adding a source: it implements [`PriceSource`], and
//! [`DataServices`](super::DataServices) lists it among the prices'. No
//! widget changes.

use std::{any::Any, rc::Rc};

/// Which of an instrument's prices is followed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum PriceKind {
    /// The fastest price its exchange streams for it: the middle of its
    /// best bid and its best offer, each time either changes (several times
    /// a second for a busy market), and its mark while its book has
    /// neither. What closing a position now would come to, near enough.
    #[default]
    Live,
    /// The price its exchange marks it at: what the exchange itself values
    /// a position at, margins it by and liquidates it on. Exchanges say it
    /// at their own pace (Hyperliquid: about once a second).
    Mark,
}

impl PriceKind {
    pub const ALL: [Self; 2] = [Self::Live, Self::Mark];

    /// What settings call it.
    pub const fn id(self) -> &'static str {
        match self {
            Self::Live => "live",
            Self::Mark => "mark",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.id() == id)
    }
}

/// What hears an instrument's price followed live: the price, each time
/// its source says another, and `None` once the source's stream of it has
/// stopped, when the last price heard is no longer the price. It runs in
/// the stream's own callback: it writes state and returns.
pub type HearsPrice = Rc<dyn Fn(Option<f64>)>;

/// A hold on an instrument's price followed live ([`Prices::follow`]).
/// Dropping it lets go; the last hold of an instrument ends its stream.
#[must_use = "dropping it stops following the price"]
pub struct PriceFeed(#[allow(dead_code, reason = "held for what dropping it ends")] Box<dyn Any>);

impl PriceFeed {
    /// A hold that lasts as long as `held` does.
    pub fn new(held: impl Any) -> Self {
        Self(Box::new(held))
    }
}

/// Something that streams the prices of one exchange's instruments.
pub trait PriceSource {
    /// The exchange it prices, by its id: the one its accounts' source
    /// ([`AccountSource`](super::portfolio::AccountSource)) and its
    /// trading venue ([`TradingVenue`](super::trading::TradingVenue)) go
    /// by.
    fn venue(&self) -> &'static str;

    /// Follows `instrument`'s price live while the hold is kept, the one
    /// `kind` names. `hears` is told each price as it is said, and at once
    /// the one last heard where another hold follows the instrument
    /// already; `None` once the stream of it stopped. A source with only
    /// one kind of price gives that one for either. `None` for an
    /// instrument it cannot price at all.
    fn follow_price(
        &self,
        instrument: &str,
        kind: PriceKind,
        hears: HearsPrice,
    ) -> Option<PriceFeed>;
}

/// The app's price sources, by exchange. Cloning is cheap and shares them.
#[derive(Clone)]
pub struct Prices(Rc<[Rc<dyn PriceSource>]>);

impl Prices {
    /// The prices `sources` stream. Nothing connects until something
    /// follows a price.
    pub fn new(sources: impl IntoIterator<Item = Rc<dyn PriceSource>>) -> Self {
        Self(sources.into_iter().collect())
    }

    /// Follows `instrument`'s price on `venue` live while the hold is kept,
    /// the one `kind` names: `hears` is told each price as it is said, and
    /// `None` once the stream of it has stopped. Every hold of one
    /// instrument shares one stream. `None` for an exchange nothing prices.
    pub fn follow(
        &self,
        venue: &str,
        instrument: &str,
        kind: PriceKind,
        hears: impl Fn(Option<f64>) + 'static,
    ) -> Option<PriceFeed> {
        let source = self.0.iter().find(|source| source.venue() == venue)?;
        source.follow_price(instrument, kind, Rc::new(hears))
    }
}
