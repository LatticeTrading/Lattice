//! The Price Delta Monitor's session ([`Monitor`]): the book leases its
//! markets need, and the base's price and the rows its view shows (the
//! widget's data, described in the parent module).

use std::rc::Rc;

use nsh_core::prelude::*;

use super::settings::PriceDeltaSettings;
use crate::{
    data::{
        SessionScope,
        ferris::{Ferris, MarketKey, MarketSelection, OrderBook, order_book},
    },
    widgets::common::{Shared, books::BookFeeds},
};

/// The levels a side a price is read from: the top of the book.
const DEPTH: usize = 1;
/// What a cell with nothing to show reads (the old widget's).
pub(super) const NONE: &str = "--";

/// A market's price, or why it has none.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Price {
    /// Its book has not come yet.
    Waiting,
    /// The server refused its book's topic, and no book came.
    Unavailable,
    At(f64),
}

impl Price {
    /// The price `book` gives, of a feed that `failed` or did not.
    fn of(book: Option<&OrderBook>, failed: bool) -> Self {
        let price = book
            .and_then(OrderBook::mid_price)
            .filter(|price| price.is_finite());
        match price {
            Some(price) => Self::At(price),
            None if failed => Self::Unavailable,
            None => Self::Waiting,
        }
    }

    pub fn value(self) -> Option<f64> {
        match self {
            Self::At(price) => Some(price),
            Self::Waiting | Self::Unavailable => None,
        }
    }
}

/// Which way a difference reads: what colors it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Tone {
    Above,
    Below,
    Level,
}

/// How far a market's price is from the base's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Delta {
    /// The market's price less the base's.
    pub difference: f64,
    /// That as a percentage of the base's price; none of a base at zero.
    pub percent: Option<f64>,
}

impl Delta {
    /// `price` against `base`; none while either is unknown.
    fn of(base: Price, price: Price) -> Option<Self> {
        let (base, price) = (base.value()?, price.value()?);
        let difference = price - base;
        Some(Self {
            difference,
            percent: (base != 0.0).then(|| difference / base * 100.0),
        })
    }
}

/// One compared market's row.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Row {
    pub key: MarketKey,
    pub market: Rc<MarketSelection>,
    pub price: Price,
    /// None while its price or the base's is unknown.
    pub delta: Option<Delta>,
}

impl Row {
    /// Its price cell (the old widget's words).
    pub fn price_text(&self) -> String {
        match self.price {
            Price::At(price) => price_text(price),
            Price::Waiting => "Waiting...".to_owned(),
            Price::Unavailable => NONE.to_owned(),
        }
    }

    /// `+12.5000`: its difference, signed, to four decimals.
    pub fn difference_text(&self) -> String {
        self.delta.map_or_else(
            || NONE.to_owned(),
            |delta| format!("{:+.4}", delta.difference),
        )
    }

    /// `+0.012%`: its difference as a percentage, signed, to three
    /// decimals.
    pub fn percent_text(&self) -> String {
        self.delta
            .and_then(|delta| delta.percent)
            .map_or_else(|| NONE.to_owned(), |percent| format!("{percent:+.3}%"))
    }

    /// Which way its difference reads; none while it has none.
    pub fn tone(&self) -> Option<Tone> {
        let difference = self.delta?.difference;
        Some(if difference > 0.0 {
            Tone::Above
        } else if difference < 0.0 {
            Tone::Below
        } else {
            Tone::Level
        })
    }
}

/// A price as the old widget wrote it (its `formatPrice`): thousands
/// apart, at least two decimals, and at most two from a thousand up, three
/// from a hundred, four from one and six below it.
pub(super) fn price_text(price: f64) -> String {
    let places = match price.abs() {
        size if size >= 1_000.0 => 2,
        size if size >= 100.0 => 3,
        size if size >= 1.0 => 4,
        _ => 6,
    };
    let fixed = format!("{price:.places$}");
    let (whole, fraction) = fixed.split_once('.').unwrap_or((&fixed, ""));
    let fraction = &fraction[..fraction.trim_end_matches('0').len().max(2)];
    let (sign, digits) = match whole.strip_prefix('-') {
        Some(digits) => ("-", digits),
        None => ("", whole),
    };
    let mut grouped = String::with_capacity(fixed.len() + digits.len() / 3);
    for (at, digit) in digits.chars().enumerate() {
        if at > 0 && (digits.len() - at) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    format!("{sign}{grouped}.{fraction}")
}

/// A market the widget reads, with what its row and its book's stream are
/// known by: each made once per change of the settings.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Watched {
    key: MarketKey,
    market: Rc<MarketSelection>,
    /// What tells its book's stream apart ([`order_book::stream_key`]).
    stream: String,
}

impl Watched {
    pub fn of(market: MarketSelection) -> Self {
        Self {
            key: market.identity(),
            stream: order_book::stream_key(&market, DEPTH as u32).unwrap_or_default(),
            market: Rc::new(market),
        }
    }
}

/// The markets the settings read: the base's first, as the feeds are
/// followed and their books taken.
fn watched(settings: &PriceDeltaSettings) -> Vec<Watched> {
    std::iter::once(settings.base())
        .chain(settings.comparisons())
        .map(Watched::of)
        .collect()
}

/// The base's price and the compared markets' rows, from what the feeds of
/// `watched` hold: `books` and `failed` in its order, the base's first.
pub(super) fn quote(
    watched: &[Watched],
    books: &[Option<Rc<OrderBook>>],
    failed: &[bool],
) -> (Price, Vec<Row>) {
    let price = |at: usize| {
        Price::of(
            books.get(at).and_then(Option::as_deref),
            failed.get(at).copied().unwrap_or(false),
        )
    };
    let base = price(0);
    let rows = watched
        .iter()
        .enumerate()
        .skip(1)
        .map(|(at, watched)| {
            let price = price(at);
            Row {
                key: watched.key.clone(),
                market: watched.market.clone(),
                price,
                delta: Delta::of(base, price),
            }
        })
        .collect();
    (base, rows)
}

/// A Price Delta Monitor's session state: what its view shows (its leases
/// live in the effects `new` starts).
pub(crate) struct Monitor {
    /// The base's price, updated while the widget is visible.
    pub(super) base: Signal<Price>,
    /// The compared markets' rows, in the settings' order, updated while
    /// the widget is visible.
    pub(super) rows: Signal<Shared<Vec<Row>>>,
}

impl Monitor {
    /// A monitor following `settings`, the session's copy of the tab's.
    pub(super) fn new(
        scope: SessionScope,
        ferris: Ferris,
        settings: Signal<PriceDeltaSettings>,
    ) -> Self {
        let base = signal(Price::Waiting);
        let rows = signal(Shared::new(Vec::new()));
        // A lease per market; dirty when one brought something (or the
        // settings changed) since the rows were made.
        let feeds = BookFeeds::new(ferris);
        let watched = computed(move || settings.with(|settings| Rc::new(watched(settings))));
        {
            let feeds = feeds.clone();
            effect(move || {
                let next = watched.get();
                untracked(|| {
                    let books = next
                        .iter()
                        .map(|watched| (watched.stream.clone(), &*watched.market));
                    feeds.follow_markets(books, DEPTH);
                });
                feeds.mark();
            });
        }
        scope.while_visible(move || {
            let refresh: Rc<dyn Fn()> = {
                let feeds = feeds.clone();
                Rc::new(move || {
                    let watched = watched.get_untracked();
                    let (books, failed) = feeds.take();
                    let (price, next) = quote(&watched, &books, &failed);
                    // Only what changed: a book that moved without moving
                    // its mid price sets nothing.
                    batch(|| {
                        if base.get_untracked() != price {
                            base.try_set(price);
                        }
                        if rows.with_untracked(|rows| *rows.0 != next) {
                            rows.try_set(Shared::new(next));
                        }
                    });
                })
            };
            // New books and settings: at most once a frame.
            feeds.each_frame(refresh);
        });

        Self { base, rows }
    }
}
