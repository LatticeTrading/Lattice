//! The order book's settings, saved in its tab's record under the old
//! app's names (`webGLOrderBookSettingsSchema` and the top bar's
//! `centerTarget`):
//!
//! - its books ([`Books`], what every widget that draws order books
//!   saves): `primaryMarket`, `secondaryMarket`, `depth` and
//!   `priceGrouping`.
//! - `centerTarget`: the book the ladder follows (`"secondary"`, or `null`
//!   for none; the primary while left out).
//! - `layout` (`"horizontal"`; vertical while left out).
//! - `showAllLevels`, `showOnlyLoadedLevels`, `showMidPriceLine`,
//!   `showBidAskDivider`, `showCumulativeAmount`, `separatePairQuantities`
//!   ([`Toggle`]).
//!
//! Each is left out of the saved form while it holds its default, and read
//! leniently, as the old `normalizeWebGLOrderBookSettings` read them: a
//! switch that is not a boolean is its default, a center target it does
//! not know is none (and the books as [`Books::read`] reads them).

use serde::Serialize;
use serde_json::Value;

use crate::{
    data::ferris::MarketSelection,
    widgets::common::books::{BookLayout, BookSettings, Books, Grouping, Layer, Pair},
};

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

/// One order book's settings.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub(crate) struct OrderBookSettings {
    /// Its two markets, its depth and its grouping.
    #[serde(flatten)]
    books: Books,
    /// The book the ladder follows: `None` is the primary, `Some(None)`
    /// none (saved as `null`).
    #[serde(rename = "centerTarget", skip_serializing_if = "Option::is_none")]
    center: Option<Option<Pair>>,
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

impl From<Value> for OrderBookSettings {
    fn from(value: Value) -> Self {
        let Value::Object(members) = value else {
            return Self::default();
        };
        let mut settings = Self {
            books: Books::read(&members),
            ..Self::default()
        };
        settings.center = match members.get("centerTarget") {
            None => None,
            Some(Value::String(id)) if id == Pair::Primary.id() => None,
            Some(Value::String(id)) if id == Pair::Secondary.id() => Some(Some(Pair::Secondary)),
            // `null`, and anything the old app did not know.
            Some(_) => Some(None),
        };
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

impl BookSettings for OrderBookSettings {
    const TEST_ID: &'static str = "order-book";

    fn books(&self) -> &Books {
        &self.books
    }

    fn with_books(mut self, books: Books) -> Self {
        self.books = books;
        self
    }
}

impl OrderBookSettings {
    /// `pair`'s market, if it has one.
    pub fn market(&self, pair: Pair) -> Option<MarketSelection> {
        self.books.market(pair)
    }

    /// How many levels a side shows.
    pub fn depth(&self) -> u16 {
        self.books.depth()
    }

    pub fn grouping(&self) -> Grouping {
        self.books.grouping()
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

    /// Whether the second pair streams the primary's book: it is then not
    /// drawn (the old `duplicateSecondary`).
    pub fn duplicate(&self) -> bool {
        self.books.duplicate()
    }

    /// The books drawn: the primary, then the second pair unless it is the
    /// primary's book again.
    pub fn layers(&self) -> Vec<Layer> {
        self.books.layers()
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
