//! What is drawn on the chart by hand ([`Drawing`]): a trend line, a
//! Fibonacci retracement and a measurement, each made with its tool
//! ([`Tool`]) from the toolbar beside the plot.
//!
//! A drawing is a kind ([`Kind`]) and its points ([`Anchor`]: a time and a
//! price each), so it stays on its bars through a pan, a zoom, a page of
//! older history and another interval. It belongs to the market it was
//! drawn on ([`market_key`]): the chart shows the ones of the market it
//! charts and keeps the others for when theirs is charted again. Drawings
//! are saved in the chart's settings (`drawings`), all but the kinds that
//! are not kept ([`Kind::kept`]): a measurement shows until the next press.
//!
//! Its parts:
//!
//! - [`shape`]: what a kind looks like and where it can be grabbed
//!   ([`Shape`]), one module a kind ([`trend`], [`fib`], [`measure`]), and
//!   what they draw with;
//! - [`scale`]: where a point is on the plot, and which point a place of
//!   the plot is;
//! - [`editor`]: the tool in hand and what the pointer does with it
//!   ([`Editor`]): making a drawing, picking one, moving it, removing it;
//! - [`layer`]: the canvas over the plot the drawings are painted on;
//! - [`toolbar`]: the tools, beside the plot while it is open, and the
//!   badge floating on the plot that opens it.
//!
//! A new kind is its module with its [`Shape`], its line in [`Kind`] (its
//! id, its name, how many points make one, whether it is kept) and its
//! icon ([`Tool::icon`]). The editor, the layer, the toolbar and the
//! settings take it from there.

mod editor;
mod fib;
mod layer;
mod measure;
mod scale;
mod shape;
mod toolbar;
mod trend;

use serde::{Serialize, Serializer};
use serde_json::Value;

use self::shape::Shape;
pub(crate) use self::{
    editor::Editor,
    layer::layer,
    toolbar::{toolbar, toolbar_badge},
};
#[cfg(test)]
pub(crate) use self::{
    editor::{Draft, Grip, hit, moved},
    layer::{Emphasis, Item, paint},
    scale::Scale,
    shape::Ink,
};
use crate::data::ferris::MarketSelection;

/// A point of a drawing: a time (Unix seconds, as a bar's) and a price.
/// Both are numbers ([`Anchor::at`] makes no other), so two anchors are
/// equal or they are not.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Anchor {
    time: f64,
    price: f64,
}

impl Eq for Anchor {}

impl Anchor {
    /// The point at `time` and `price`; none unless both are numbers.
    pub fn at(time: f64, price: f64) -> Option<Self> {
        (time.is_finite() && price.is_finite()).then_some(Self { time, price })
    }

    pub fn time(self) -> f64 {
        self.time
    }

    pub fn price(self) -> f64 {
        self.price
    }

    /// The point saved as `[time, price]`.
    fn read(saved: &Value) -> Option<Self> {
        let [time, price] = saved.as_array()?.as_slice() else {
            return None;
        };
        Self::at(time.as_f64()?, price.as_f64()?)
    }
}

impl Serialize for Anchor {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        (self.time, self.price).serialize(serializer)
    }
}

/// The kinds of drawing, in the order the toolbar lists their tools.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Kind {
    /// A line between two points.
    Trend,
    /// The Fibonacci retracement levels of a move between two points.
    Fib,
    /// How far apart two points are, in price, in bars and in time.
    Measure,
}

impl Kind {
    pub const ALL: [Self; 3] = [Self::Trend, Self::Fib, Self::Measure];

    /// What settings save it as, and what its tool's test id ends with.
    pub fn id(self) -> &'static str {
        match self {
            Self::Trend => "trend",
            Self::Fib => "fib",
            Self::Measure => "measure",
        }
    }

    fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.id() == id)
    }

    /// Its name, which names its tool.
    pub fn label(self) -> &'static str {
        match self {
            Self::Trend => "Trend line",
            Self::Fib => "Fib retracement",
            Self::Measure => "Measure",
        }
    }

    /// How many points make one.
    pub fn anchors(self) -> usize {
        match self {
            Self::Trend | Self::Fib | Self::Measure => 2,
        }
    }

    /// Whether one stays on the chart and is saved once made. One that is
    /// not shows until the next press on the plot.
    pub fn kept(self) -> bool {
        match self {
            Self::Trend | Self::Fib => true,
            Self::Measure => false,
        }
    }

    /// What it looks like and where it can be grabbed.
    fn shape(self) -> &'static dyn Shape {
        match self {
            Self::Trend => &trend::Trend,
            Self::Fib => &fib::Fib,
            Self::Measure => &measure::Measure,
        }
    }
}

impl Serialize for Kind {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.id())
    }
}

/// What the pointer does on the plot: the plot's own and picking drawings
/// (the cursor), or making a drawing of a kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tool {
    Cursor,
    Draw(Kind),
}

impl Tool {
    /// Every tool, in the order the toolbar lists them.
    pub const ALL: [Self; 4] = [
        Self::Cursor,
        Self::Draw(Kind::Trend),
        Self::Draw(Kind::Fib),
        Self::Draw(Kind::Measure),
    ];

    /// What its test id ends with (`chart-tool-trend`).
    pub fn id(self) -> &'static str {
        match self {
            Self::Cursor => "cursor",
            Self::Draw(kind) => kind.id(),
        }
    }

    /// Its name.
    pub fn label(self) -> &'static str {
        match self {
            Self::Cursor => "Cursor",
            Self::Draw(kind) => kind.label(),
        }
    }

    /// Its icon's SVG markup.
    pub fn icon(self) -> &'static str {
        match self {
            Self::Cursor => include_str!("../../../assets/icons/cursor.svg"),
            Self::Draw(Kind::Trend) => include_str!("../../../assets/icons/trend-line.svg"),
            Self::Draw(Kind::Fib) => include_str!("../../../assets/icons/fib-retracement.svg"),
            Self::Draw(Kind::Measure) => include_str!("../../../assets/icons/measure.svg"),
        }
    }
}

/// What tells the drawings of one market from another's: its exchange and
/// the symbol Ferris is asked for.
pub(crate) fn market_key(market: &MarketSelection) -> String {
    format!("{}:{}", market.exchange, market.request_symbol)
}

/// One drawing: its kind, the market it is on, and as many points as its
/// kind takes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct Drawing {
    #[serde(rename = "type")]
    kind: Kind,
    /// The market it was drawn on ([`market_key`]).
    market: String,
    points: Vec<Anchor>,
}

impl Drawing {
    /// A `kind` on `market` at `points`; none unless they are as many as
    /// the kind takes.
    pub fn new(kind: Kind, market: &str, points: Vec<Anchor>) -> Option<Self> {
        (points.len() == kind.anchors()).then(|| Self {
            kind,
            market: market.to_owned(),
            points,
        })
    }

    pub fn kind(&self) -> Kind {
        self.kind
    }

    /// The market it is on ([`market_key`]).
    pub fn market(&self) -> &str {
        &self.market
    }

    pub fn points(&self) -> &[Anchor] {
        &self.points
    }

    /// It with its points at `points`; none unless they are as many.
    pub fn with_points(&self, points: Vec<Anchor>) -> Option<Self> {
        Self::new(self.kind, &self.market, points)
    }

    /// The drawing saved as `saved`; none for what is no drawing: a kind
    /// Lattice does not know or does not keep, no market, or points that
    /// are not the kind's.
    fn read(saved: &Value) -> Option<Self> {
        let kind = Kind::from_id(saved.get("type")?.as_str()?).filter(|kind| kind.kept())?;
        let market = saved
            .get("market")?
            .as_str()
            .filter(|key| !key.is_empty())?;
        let points = saved.get("points")?.as_array()?;
        let points = points.iter().map(Anchor::read).collect::<Option<_>>()?;
        Self::new(kind, market, points)
    }

    /// The drawings saved as the list `saved`, leaving out what is none.
    pub fn read_all(saved: Option<&Value>) -> Vec<Self> {
        let saved = saved.and_then(Value::as_array);
        saved.map_or_else(Vec::new, |all| all.iter().filter_map(Self::read).collect())
    }
}
