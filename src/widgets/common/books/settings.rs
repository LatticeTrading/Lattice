//! What every widget that draws order books saves about them, under the
//! old app's names (`webGLOrderBookSettingsSchema`):
//!
//! - `primaryMarket`, `secondaryMarket`: the two books ([`Pair`]). The
//!   primary is Bybit's BTC/USDT perpetual until told otherwise and may be
//!   cleared (saved as `null`); the second has none until one is picked.
//! - `depth` (10 to 1000 levels a side, 200) and `priceGrouping` (`0` raw,
//!   `0.01`, `0.1`, `1`, `10`, `100`).
//!
//! [`Books`] holds them. A widget's settings keep one beside their own
//! members, flattened into their saved form, and say where it is
//! ([`BookSettings`]): that is all the shared top bar and settings rows
//! ([`super::bar`], [`super::panel`]) need of them.
//!
//! Each member is left out of the saved form while it holds its default,
//! and read leniently, as the old `normalizeWebGLOrderBookSettings` read
//! them: an unreadable primary market is the default one, a depth out of
//! range is clamped (numbers as text count), a grouping the widgets do not
//! offer is raw.

use std::rc::Rc;

use serde::{Serialize, Serializer};
use serde_json::{Map, Value};

use crate::{
    data::ferris::{MarketSelection, order_book},
    widgets::host::KindSettings,
};

/// The fewest and most levels a side shows, and how many until told.
pub(crate) const MIN_DEPTH: u16 = 10;
pub(crate) const MAX_DEPTH: u16 = 1_000;
const DEFAULT_DEPTH: u16 = 200;
/// The depths the top bar offers (the old `DEPTH_PRESETS`).
pub(crate) const DEPTH_PRESETS: [u16; 7] = [10, 20, 50, 100, 200, 500, 1_000];

/// Which of a widget's two books (the old `paletteRole`): the primary, or
/// the second pair tracked beside it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Pair {
    Primary,
    Secondary,
}

impl Pair {
    pub const ALL: [Self; 2] = [Self::Primary, Self::Secondary];

    /// What settings save for it.
    pub const fn id(self) -> &'static str {
        match self {
            Self::Primary => "primary",
            Self::Secondary => "secondary",
        }
    }

    /// Its name (the old app's).
    pub const fn label(self) -> &'static str {
        match self {
            Self::Primary => "Primary",
            Self::Secondary => "Pair 2",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|pair| pair.id() == id)
    }
}

impl Serialize for Pair {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.id())
    }
}

/// How prices run: down the pane beside a price column, depth growing to
/// the right; or along the bottom, depth growing up.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum BookLayout {
    #[default]
    Vertical,
    Horizontal,
}

impl BookLayout {
    pub const ALL: [Self; 2] = [Self::Vertical, Self::Horizontal];

    pub const fn id(self) -> &'static str {
        match self {
            Self::Vertical => "vertical",
            Self::Horizontal => "horizontal",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Vertical => "Vertical",
            Self::Horizontal => "Horizontal",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|layout| layout.id() == id)
    }
}

impl Serialize for BookLayout {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.id())
    }
}

/// The price step levels are merged into: bids down to it, asks up to it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Grouping {
    /// Every price as the venue quotes it.
    #[default]
    Raw,
    Hundredth,
    Tenth,
    One,
    Ten,
    Hundred,
}

impl Grouping {
    /// Every grouping, in the order the selects list them (the old
    /// `PRICE_GROUPING_OPTIONS`).
    pub const ALL: [Self; 6] = [
        Self::Raw,
        Self::Hundredth,
        Self::Tenth,
        Self::One,
        Self::Ten,
        Self::Hundred,
    ];

    /// The step, zero for raw prices.
    pub const fn step(self) -> f64 {
        match self {
            Self::Raw => 0.0,
            Self::Hundredth => 0.01,
            Self::Tenth => 0.1,
            Self::One => 1.0,
            Self::Ten => 10.0,
            Self::Hundred => 100.0,
        }
    }

    /// Its name, which is also what the selects call it.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Raw => "Raw",
            Self::Hundredth => "0.01",
            Self::Tenth => "0.1",
            Self::One => "1",
            Self::Ten => "10",
            Self::Hundred => "100",
        }
    }

    pub fn from_label(label: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|grouping| grouping.label() == label)
    }

    /// The grouping whose step `saved` is, raw for any other value.
    fn read(saved: &Value) -> Self {
        let step = numeric(saved);
        Self::ALL
            .into_iter()
            .find(|grouping| Some(grouping.step()) == step)
            .unwrap_or_default()
    }
}

impl Serialize for Grouping {
    /// Its step as the old app saved it: a whole number where it is one.
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let step = self.step();
        if step.fract() == 0.0 {
            serializer.serialize_u64(step as u64)
        } else {
            serializer.serialize_f64(step)
        }
    }
}

/// One book a widget shows: which pair it is, its market, and what tells
/// its stream apart ([`order_book::stream_key`]).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Layer {
    pub pair: Pair,
    pub market: Rc<MarketSelection>,
    pub key: String,
}

/// The books a widget draws: its two markets, the levels a side, and the
/// price grouping.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub(crate) struct Books {
    /// `None` is the default market, `Some(None)` no market (saved as
    /// `null`). Boxed: a selection is several times the size of the rest.
    #[serde(rename = "primaryMarket", skip_serializing_if = "Option::is_none")]
    primary: Option<Option<Box<MarketSelection>>>,
    #[serde(rename = "secondaryMarket", skip_serializing_if = "Option::is_none")]
    secondary: Option<Box<MarketSelection>>,
    /// `None` is the default depth.
    #[serde(skip_serializing_if = "Option::is_none")]
    depth: Option<u16>,
    /// `None` is raw prices.
    #[serde(rename = "priceGrouping", skip_serializing_if = "Option::is_none")]
    grouping: Option<Grouping>,
}

/// The old app's default market (`DEFAULT_BYBIT_BTC_USDT_MARKET`).
fn default_market() -> MarketSelection {
    MarketSelection::bybit_btc_usdt()
}

/// A saved number: a JSON number, or one written as text (the old app
/// read both with `Number`).
fn numeric(saved: &Value) -> Option<f64> {
    match saved {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
    .filter(|number: &f64| number.is_finite())
}

impl Books {
    /// The books saved among `members`, read leniently.
    pub fn read(members: &Map<String, Value>) -> Self {
        let mut books = Self::default()
            .with_market(
                Pair::Primary,
                match members.get("primaryMarket") {
                    // Cleared on purpose.
                    Some(Value::Null) => None,
                    saved => Some(
                        saved
                            .and_then(MarketSelection::from_value)
                            .unwrap_or_else(default_market),
                    ),
                },
            )
            .with_market(
                Pair::Secondary,
                members
                    .get("secondaryMarket")
                    .and_then(MarketSelection::from_value),
            );
        if let Some(depth) = members.get("depth").and_then(numeric) {
            books = books.with_depth(depth.floor().clamp(0.0, f64::from(u16::MAX)) as u16);
        }
        if let Some(saved) = members.get("priceGrouping") {
            books = books.with_grouping(Grouping::read(saved));
        }
        books
    }

    /// `pair`'s market, if it has one.
    pub fn market(&self, pair: Pair) -> Option<MarketSelection> {
        match pair {
            Pair::Primary => match &self.primary {
                None => Some(default_market()),
                Some(market) => market.as_deref().cloned(),
            },
            Pair::Secondary => self.secondary.as_deref().cloned(),
        }
    }

    /// These books with `market` as `pair`'s (`None`: with none).
    pub fn with_market(mut self, pair: Pair, market: Option<MarketSelection>) -> Self {
        match pair {
            Pair::Primary => {
                self.primary = match market {
                    Some(market) if market == default_market() => None,
                    market => Some(market.map(Box::new)),
                };
            }
            Pair::Secondary => self.secondary = market.map(Box::new),
        }
        self
    }

    /// How many levels a side shows.
    pub fn depth(&self) -> u16 {
        self.depth.unwrap_or(DEFAULT_DEPTH)
    }

    /// These books showing `depth` levels a side, within 10 to 1000.
    pub fn with_depth(mut self, depth: u16) -> Self {
        let depth = depth.clamp(MIN_DEPTH, MAX_DEPTH);
        self.depth = (depth != DEFAULT_DEPTH).then_some(depth);
        self
    }

    pub fn grouping(&self) -> Grouping {
        self.grouping.unwrap_or_default()
    }

    pub fn with_grouping(mut self, grouping: Grouping) -> Self {
        self.grouping = (grouping != Grouping::default()).then_some(grouping);
        self
    }

    /// `pair`'s market and the key of the book it streams at this depth.
    fn stream(&self, pair: Pair) -> Option<(MarketSelection, String)> {
        let market = self.market(pair)?;
        let key = order_book::stream_key(&market, u32::from(self.depth())).unwrap_or_default();
        Some((market, key))
    }

    /// Whether the second pair streams the primary's book: it is then not
    /// drawn (the old `duplicateSecondary`).
    pub fn duplicate(&self) -> bool {
        match (self.stream(Pair::Primary), self.stream(Pair::Secondary)) {
            (Some((_, primary)), Some((_, secondary))) => primary == secondary,
            _ => false,
        }
    }

    /// The books drawn: the primary, then the second pair unless it is the
    /// primary's book again.
    pub fn layers(&self) -> Vec<Layer> {
        let layer = |pair: Pair, (market, key): (MarketSelection, String)| Layer {
            pair,
            market: Rc::new(market),
            key,
        };
        let primary = self.stream(Pair::Primary);
        let secondary = self
            .stream(Pair::Secondary)
            .filter(|(_, key)| primary.as_ref().is_none_or(|(_, primary)| primary != key));
        primary
            .map(|stream| layer(Pair::Primary, stream))
            .into_iter()
            .chain(secondary.map(|stream| layer(Pair::Secondary, stream)))
            .collect()
    }
}

/// The settings of a widget that draws order books: where its [`Books`]
/// are, which is what the shared top bar and settings rows read and save.
pub(crate) trait BookSettings: KindSettings {
    /// What the test ids of its book controls start with (`order-book`).
    const TEST_ID: &'static str;

    fn books(&self) -> &Books;

    /// These settings drawing `books`.
    fn with_books(self, books: Books) -> Self;

    /// These settings with `market` as `pair`'s (`None`: with none).
    fn with_market(self, pair: Pair, market: Option<MarketSelection>) -> Self {
        let books = self.books().clone().with_market(pair, market);
        self.with_books(books)
    }

    /// These settings showing `depth` levels a side, within 10 to 1000.
    fn with_depth(self, depth: u16) -> Self {
        let books = self.books().clone().with_depth(depth);
        self.with_books(books)
    }

    fn with_grouping(self, grouping: Grouping) -> Self {
        let books = self.books().clone().with_grouping(grouping);
        self.with_books(books)
    }
}
