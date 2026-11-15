//! The order book's settings, saved in its tab's record under the old
//! app's names (`webGLOrderBookSettingsSchema` and the top bar's
//! `centerTarget`):
//!
//! - `primaryMarket`, `secondaryMarket`: the two books ([`Pair`]). The
//!   primary is Bybit's BTC/USDT perpetual until told otherwise and may be
//!   cleared (saved as `null`); the second has none until one is picked.
//! - `centerTarget`: the book the ladder follows (`"secondary"`, or `null`
//!   for none; the primary while left out).
//! - `depth` (10 to 1000 levels a side, 200), `priceGrouping` (`0` raw,
//!   `0.01`, `0.1`, `1`, `10`, `100`), `layout` (`"horizontal"`; vertical
//!   while left out).
//! - `showAllLevels`, `showOnlyLoadedLevels`, `showMidPriceLine`,
//!   `showBidAskDivider`, `showCumulativeAmount`, `separatePairQuantities`
//!   ([`Toggle`]).
//!
//! Each is left out of the saved form while it holds its default, and read
//! leniently, as the old `normalizeWebGLOrderBookSettings` read them: an
//! unreadable primary market is the default one, a depth out of range is
//! clamped (numbers as text count), a grouping the widget does not offer
//! is raw, a switch that is not a boolean is its default, a center target
//! it does not know is none.

use std::rc::Rc;

use serde::{Serialize, Serializer};
use serde_json::Value;

use crate::data::ferris::{MarketSelection, order_book};

/// The fewest and most levels a side shows, and how many until told.
pub(super) const MIN_DEPTH: u16 = 10;
pub(super) const MAX_DEPTH: u16 = 1_000;
const DEFAULT_DEPTH: u16 = 200;
/// The depths the top bar offers (the old `DEPTH_PRESETS`).
pub(super) const DEPTH_PRESETS: [u16; 7] = [10, 20, 50, 100, 200, 500, 1_000];

/// Which of the widget's two books (the old `paletteRole`): the primary,
/// or the second pair tracked beside it.
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

/// How the ladder runs: prices down the pane beside a price column, depth
/// growing to the right; or prices along the bottom, depth growing up.
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

/// One of the widget's switches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Toggle {
    /// A row for every price step, traded at or not, instead of only the
    /// prices that hold orders.
    AllLevels,
    /// Depth bars stop at the last level loaded instead of running on to
    /// the plot's edge.
    OnlyLoaded,
    /// A line at each book's mid price.
    MidPrice,
    /// A dashed line between each book's bids and asks.
    Divider,
    /// Each row's running amount beside its total.
    Cumulative,
    /// Each pair's quantities in a lane of their own instead of summed.
    Separate,
}

impl Toggle {
    /// Every switch, in the order the settings panel lists them (the old
    /// schema's).
    pub const ALL: [Self; 6] = [
        Self::AllLevels,
        Self::OnlyLoaded,
        Self::MidPrice,
        Self::Divider,
        Self::Cumulative,
        Self::Separate,
    ];

    /// What the settings panel calls it (the old schema's labels).
    pub const fn label(self) -> &'static str {
        match self {
            Self::AllLevels => "Show all levels",
            Self::OnlyLoaded => "Show only loaded depth bars",
            Self::MidPrice => "Show mid price",
            Self::Divider => "Show bid/ask divider",
            Self::Cumulative => "Show Cumulative Amount",
            Self::Separate => "Separate Pair Quantity Lanes",
        }
    }

    /// What settings save it under.
    const fn name(self) -> &'static str {
        match self {
            Self::AllLevels => "showAllLevels",
            Self::OnlyLoaded => "showOnlyLoadedLevels",
            Self::MidPrice => "showMidPriceLine",
            Self::Divider => "showBidAskDivider",
            Self::Cumulative => "showCumulativeAmount",
            Self::Separate => "separatePairQuantities",
        }
    }

    /// Whether it is on until told otherwise: only the mid price is.
    const fn default(self) -> bool {
        matches!(self, Self::MidPrice)
    }
}

/// One book the widget shows: which pair it is, its market, and what tells
/// its stream apart ([`order_book::stream_key`]).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Layer {
    pub pair: Pair,
    pub market: Rc<MarketSelection>,
    pub key: String,
}

/// One order book's settings.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub(crate) struct OrderBookSettings {
    /// `None` is the default market, `Some(None)` no market (saved as
    /// `null`). Boxed: a selection is several times the size of the rest.
    #[serde(rename = "primaryMarket", skip_serializing_if = "Option::is_none")]
    primary: Option<Option<Box<MarketSelection>>>,
    #[serde(rename = "secondaryMarket", skip_serializing_if = "Option::is_none")]
    secondary: Option<Box<MarketSelection>>,
    /// The book the ladder follows: `None` is the primary, `Some(None)`
    /// none (saved as `null`).
    #[serde(rename = "centerTarget", skip_serializing_if = "Option::is_none")]
    center: Option<Option<Pair>>,
    /// `None` is the default depth.
    #[serde(skip_serializing_if = "Option::is_none")]
    depth: Option<u16>,
    /// `None` is raw prices.
    #[serde(rename = "priceGrouping", skip_serializing_if = "Option::is_none")]
    grouping: Option<Grouping>,
    /// Each switch: `None` is its default.
    #[serde(rename = "showAllLevels", skip_serializing_if = "Option::is_none")]
    all_levels: Option<bool>,
    #[serde(
        rename = "showOnlyLoadedLevels",
        skip_serializing_if = "Option::is_none"
    )]
    only_loaded: Option<bool>,
    #[serde(rename = "showMidPriceLine", skip_serializing_if = "Option::is_none")]
    mid_price: Option<bool>,
    #[serde(rename = "showBidAskDivider", skip_serializing_if = "Option::is_none")]
    divider: Option<bool>,
    #[serde(
        rename = "showCumulativeAmount",
        skip_serializing_if = "Option::is_none"
    )]
    cumulative: Option<bool>,
    #[serde(
        rename = "separatePairQuantities",
        skip_serializing_if = "Option::is_none"
    )]
    separate: Option<bool>,
    /// `None` is vertical.
    #[serde(skip_serializing_if = "Option::is_none")]
    layout: Option<BookLayout>,
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

impl From<Value> for OrderBookSettings {
    fn from(value: Value) -> Self {
        let Value::Object(members) = value else {
            return Self::default();
        };
        let mut settings = Self::default()
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
        settings.center = match members.get("centerTarget") {
            None => None,
            Some(Value::String(id)) if id == Pair::Primary.id() => None,
            Some(Value::String(id)) if id == Pair::Secondary.id() => Some(Some(Pair::Secondary)),
            // `null`, and anything the old app did not know.
            Some(_) => Some(None),
        };
        if let Some(depth) = members.get("depth").and_then(numeric) {
            settings = settings.with_depth(depth.floor().clamp(0.0, f64::from(u16::MAX)) as u16);
        }
        if let Some(saved) = members.get("priceGrouping") {
            settings = settings.with_grouping(Grouping::read(saved));
        }
        for toggle in Toggle::ALL {
            let on = members
                .get(toggle.name())
                .and_then(Value::as_bool)
                .unwrap_or(toggle.default());
            settings = settings.with_toggle(toggle, on);
        }
        if members.get("layout").and_then(Value::as_str) == Some(BookLayout::Horizontal.id()) {
            settings = settings.with_layout(BookLayout::Horizontal);
        }
        settings
    }
}

impl OrderBookSettings {
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

    /// These settings with `market` as `pair`'s (`None`: with none).
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

    /// These settings showing `depth` levels a side, within 10 to 1000.
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

    pub fn layout(&self) -> BookLayout {
        self.layout.unwrap_or_default()
    }

    pub fn with_layout(mut self, layout: BookLayout) -> Self {
        self.layout = (layout != BookLayout::default()).then_some(layout);
        self
    }

    fn switch(&self, toggle: Toggle) -> Option<bool> {
        match toggle {
            Toggle::AllLevels => self.all_levels,
            Toggle::OnlyLoaded => self.only_loaded,
            Toggle::MidPrice => self.mid_price,
            Toggle::Divider => self.divider,
            Toggle::Cumulative => self.cumulative,
            Toggle::Separate => self.separate,
        }
    }

    /// Whether `toggle` is on.
    pub fn shows(&self, toggle: Toggle) -> bool {
        self.switch(toggle).unwrap_or(toggle.default())
    }

    /// These settings with `toggle` on or off.
    pub fn with_toggle(mut self, toggle: Toggle, on: bool) -> Self {
        let saved = (on != toggle.default()).then_some(on);
        match toggle {
            Toggle::AllLevels => self.all_levels = saved,
            Toggle::OnlyLoaded => self.only_loaded = saved,
            Toggle::MidPrice => self.mid_price = saved,
            Toggle::Divider => self.divider = saved,
            Toggle::Cumulative => self.cumulative = saved,
            Toggle::Separate => self.separate = saved,
        }
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

    /// The book the ladder follows: the saved one while it is drawn, else
    /// none (the old `centerTarget`, which a removed or repeated pair
    /// cleared).
    pub fn center(&self) -> Option<Pair> {
        let wanted = self.center.unwrap_or(Some(Pair::Primary))?;
        let drawn = match wanted {
            Pair::Primary => self.market(Pair::Primary).is_some(),
            Pair::Secondary => self.market(Pair::Secondary).is_some() && !self.duplicate(),
        };
        drawn.then_some(wanted)
    }

    /// These settings following `target` (`None`: no book).
    pub fn with_center(mut self, target: Option<Pair>) -> Self {
        self.center = (target != Some(Pair::Primary)).then_some(target);
        self
    }

    /// What the plot says above its rows about the books it draws (the old
    /// widget's warnings): that the second pair repeats the primary, or
    /// that two books at raw prices are not normalized.
    pub fn warning(&self) -> Option<&'static str> {
        if self.duplicate() {
            return Some("Pair 2 duplicates Primary and is not rendered.");
        }
        let two = self.market(Pair::Primary).is_some() && self.market(Pair::Secondary).is_some();
        if !(two && self.grouping() == Grouping::Raw) {
            return None;
        }
        Some(if self.shows(Toggle::Separate) {
            "Raw prices; pair quantities are separate and not normalized."
        } else {
            "Raw prices; aligned quantities are combined without normalization."
        })
    }
}
