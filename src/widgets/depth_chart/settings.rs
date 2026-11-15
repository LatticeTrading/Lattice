//! The depth chart's settings, saved in its tab's record:
//!
//! - its books ([`Books`], what every widget that draws order books
//!   saves): `primaryMarket`, `secondaryMarket`, `depth` and
//!   `priceGrouping`. The old depth chart had one market, saved as
//!   `market`: it is read as the primary while no `primaryMarket` is saved.
//! - `pairMode`: how two books are drawn together ([`Blend`]: `"stacked"`
//!   or `"combined"`; over one another while left out).
//! - `depthUnit`: what depth is measured in ([`Unit`]: `"value"`, the
//!   quote asset; the base asset's amount while left out).
//! - `showMidPriceLine`: the old switch, on while left out.
//! - `layout`: `"vertical"` for prices down the pane (the old Vertical
//!   Depth Chart); along the bottom while left out.
//!
//! Each is left out of the saved form while it holds its default, and read
//! leniently: a mode, unit or layout it does not know is the default, a
//! switch that is not a boolean is on (and the books as [`Books::read`]
//! reads them).

use serde::{Serialize, Serializer};
use serde_json::Value;

use crate::{
    data::ferris::MarketSelection,
    widgets::common::books::{BookLayout, BookSettings, Books, Layer, Pair},
};

/// How two books are drawn together.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Blend {
    /// Each book's depth from the baseline, the second over the first.
    #[default]
    Overlaid,
    /// The second book's depth on top of the first's: the outline is
    /// their sum, each band a book's share of it.
    Stacked,
    /// Their sum, as one book's.
    Combined,
}

impl Blend {
    /// Every mode, in the order the selects list them.
    pub const ALL: [Self; 3] = [Self::Overlaid, Self::Stacked, Self::Combined];

    /// What settings save for it.
    pub const fn id(self) -> &'static str {
        match self {
            Self::Overlaid => "overlay",
            Self::Stacked => "stacked",
            Self::Combined => "combined",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Overlaid => "Overlaid",
            Self::Stacked => "Stacked",
            Self::Combined => "Combined",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|blend| blend.id() == id)
    }
}

impl Serialize for Blend {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.id())
    }
}

/// What depth is measured in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Unit {
    /// What rests, in the base asset (the venue's unit).
    #[default]
    Amount,
    /// What it is worth, in the quote asset: price times amount.
    Value,
}

impl Unit {
    pub const ALL: [Self; 2] = [Self::Amount, Self::Value];

    /// What settings save for it.
    pub const fn id(self) -> &'static str {
        match self {
            Self::Amount => "amount",
            Self::Value => "value",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Amount => "Amount",
            Self::Value => "Value",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|unit| unit.id() == id)
    }

    /// The asset of `market` it counts in.
    pub fn asset(self, market: &MarketSelection) -> &str {
        match self {
            Self::Amount => &market.base,
            Self::Value => &market.quote,
        }
    }
}

impl Serialize for Unit {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.id())
    }
}

/// One depth chart's settings.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub(crate) struct DepthChartSettings {
    /// Its two markets, its depth and its grouping.
    #[serde(flatten)]
    books: Books,
    /// `None` is over one another.
    #[serde(rename = "pairMode", skip_serializing_if = "Option::is_none")]
    blend: Option<Blend>,
    /// `None` is the base asset's amount.
    #[serde(rename = "depthUnit", skip_serializing_if = "Option::is_none")]
    unit: Option<Unit>,
    /// `None` is on.
    #[serde(rename = "showMidPriceLine", skip_serializing_if = "Option::is_none")]
    mid_price: Option<bool>,
    /// `None` is horizontal: prices along the bottom.
    #[serde(skip_serializing_if = "Option::is_none")]
    layout: Option<BookLayout>,
}

impl From<Value> for DepthChartSettings {
    fn from(value: Value) -> Self {
        let Value::Object(members) = value else {
            return Self::default();
        };
        let mut books = Books::read(&members);
        // The old depth chart's one market.
        if !members.contains_key("primaryMarket")
            && let Some(market) = members.get("market").and_then(MarketSelection::from_value)
        {
            books = books.with_market(Pair::Primary, Some(market));
        }
        let named = |name: &str| members.get(name).and_then(Value::as_str);
        let mid_price = members.get("showMidPriceLine").and_then(Value::as_bool);
        let layout = named("layout").and_then(BookLayout::from_id);
        Self::default()
            .with_books(books)
            .with_blend(
                named("pairMode")
                    .and_then(Blend::from_id)
                    .unwrap_or_default(),
            )
            .with_unit(
                named("depthUnit")
                    .and_then(Unit::from_id)
                    .unwrap_or_default(),
            )
            .with_mid_price(mid_price.unwrap_or(true))
            .with_layout(layout.unwrap_or(BookLayout::Horizontal))
    }
}

impl BookSettings for DepthChartSettings {
    const TEST_ID: &'static str = "depth-chart";

    fn books(&self) -> &Books {
        &self.books
    }

    fn with_books(mut self, books: Books) -> Self {
        self.books = books;
        self
    }
}

impl DepthChartSettings {
    /// How two books are drawn together.
    pub fn blend(&self) -> Blend {
        self.blend.unwrap_or_default()
    }

    pub fn with_blend(mut self, blend: Blend) -> Self {
        self.blend = (blend != Blend::default()).then_some(blend);
        self
    }

    /// What depth is measured in.
    pub fn unit(&self) -> Unit {
        self.unit.unwrap_or_default()
    }

    pub fn with_unit(mut self, unit: Unit) -> Self {
        self.unit = (unit != Unit::default()).then_some(unit);
        self
    }

    /// Whether a line marks the mid price.
    pub fn mid_price(&self) -> bool {
        self.mid_price.unwrap_or(true)
    }

    pub fn with_mid_price(mut self, on: bool) -> Self {
        self.mid_price = (!on).then_some(false);
        self
    }

    /// How prices run: along the bottom until told otherwise.
    pub fn layout(&self) -> BookLayout {
        self.layout.unwrap_or(BookLayout::Horizontal)
    }

    pub fn with_layout(mut self, layout: BookLayout) -> Self {
        self.layout = (layout != BookLayout::Horizontal).then_some(layout);
        self
    }

    /// The books drawn: the primary, then the second pair unless it is the
    /// primary's book again.
    pub fn layers(&self) -> Vec<Layer> {
        self.books.layers()
    }

    /// What the plot says above itself about the books it draws: that the
    /// second pair repeats the primary, or that two books are measured in
    /// different assets, which no depth of both can be read in.
    pub fn warning(&self) -> Option<&'static str> {
        if self.books.duplicate() {
            return Some("Pair 2 duplicates Primary and is not rendered.");
        }
        let primary = self.books.market(Pair::Primary)?;
        let secondary = self.books.market(Pair::Secondary)?;
        let unit = self.unit();
        if unit.asset(&primary) == unit.asset(&secondary) {
            return None;
        }
        Some(match unit {
            Unit::Amount => "Pairs have different base assets; amounts are not normalized.",
            Unit::Value => "Pairs have different quote assets; values are not normalized.",
        })
    }
}
