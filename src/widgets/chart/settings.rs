//! The chart's settings, saved in its tab's record under the old app's
//! names: `market` (the market charted), `timeframe` (`tick`, `1m`, ...,
//! `1w`), `showGrid`, `showOhlcOnHover` and `linkColor` (the link color
//! whose picks the chart takes, `""` for none); and under Lattice's own:
//! `drawings` (what was drawn on the chart, on whatever market:
//! [`Drawing`]), `showDrawingTools` (whether the drawing toolbar is
//! open), `indicators` (what the chart computes from its bars and draws
//! over them: [`Indicator`]) and `fastMode` (whether its newest candle is
//! made from the market's trades: [`super::fast`]).
//!
//! Each is left out of the saved form while it holds its default (Bybit's
//! BTC/USDT perpetual, `1m`, all three shown, linked to blue, no drawing,
//! no indicator, fast mode off) and read leniently: an unreadable market or a member of
//! the wrong type reads as its default, a timeframe under one of the old
//! app's other spellings (`1`, `60`, `1T`) as the one it meant and an
//! unknown one as `1m`, a link color Lattice does not know as the default
//! one, and a drawing or an indicator that cannot be read is left out.

use serde::{Serialize, Serializer};
use serde_json::Value;

use super::{drawings::Drawing, indicators::Indicator};
use crate::{
    data::ferris::{MarketSelection, Timeframe},
    link::LinkColor,
};

/// What a chart's points are: every trade (a tick line), or candles of a
/// timeframe.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Interval {
    Tick,
    Candles(Timeframe),
}

impl Interval {
    /// Every interval, in the order the settings list them (the old
    /// app's).
    pub const ALL: [Self; 8] = [
        Self::Tick,
        Self::Candles(Timeframe::M1),
        Self::Candles(Timeframe::M5),
        Self::Candles(Timeframe::M15),
        Self::Candles(Timeframe::H1),
        Self::Candles(Timeframe::H4),
        Self::Candles(Timeframe::D1),
        Self::Candles(Timeframe::W1),
    ];

    /// The old app's default.
    const DEFAULT: Self = Self::Candles(Timeframe::M1);

    /// What settings save for it (`tick`, `1m`).
    pub fn id(self) -> &'static str {
        match self {
            Self::Tick => "tick",
            Self::Candles(timeframe) => timeframe.as_str(),
        }
    }

    /// Its name (`Tick`, `1m`).
    pub fn label(self) -> &'static str {
        match self {
            Self::Tick => "Tick",
            Self::Candles(timeframe) => timeframe.as_str(),
        }
    }

    /// The interval a saved timeframe names, as the old app's
    /// `normalizeTimeframe` read it: any case, its resolution spellings
    /// too, and `1m` for one it does not know.
    fn read(saved: &str) -> Self {
        let timeframe = match saved.trim().to_ascii_lowercase().as_str() {
            "tick" | "1t" | "t" | "1s" | "s" => return Self::Tick,
            "5" | "5m" => Timeframe::M5,
            "15" | "15m" => Timeframe::M15,
            "60" | "1h" => Timeframe::H1,
            "240" | "4h" => Timeframe::H4,
            "d" | "1d" => Timeframe::D1,
            "w" | "1w" => Timeframe::W1,
            _ => Timeframe::M1,
        };
        Self::Candles(timeframe)
    }
}

impl Serialize for Interval {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.id())
    }
}

/// One chart's settings.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub(crate) struct ChartSettings {
    /// The market charted; `None` is the default market. Boxed: a
    /// selection is several times the size of other widgets' settings.
    #[serde(skip_serializing_if = "Option::is_none")]
    market: Option<Box<MarketSelection>>,
    /// `None` is the default interval.
    #[serde(skip_serializing_if = "Option::is_none")]
    timeframe: Option<Interval>,
    /// `Some(false)` hides the grid; `None` shows it.
    #[serde(rename = "showGrid", skip_serializing_if = "Option::is_none")]
    show_grid: Option<bool>,
    /// `Some(false)` hides the hovered bar's prices; `None` shows them.
    #[serde(rename = "showOhlcOnHover", skip_serializing_if = "Option::is_none")]
    show_ohlc: Option<bool>,
    /// The link color whose picks the chart takes: `None` is the default
    /// one ([`LinkColor::DEFAULT`]), `Some(None)` no color (saved as `""`,
    /// the old app's "None").
    #[serde(
        rename = "linkColor",
        skip_serializing_if = "Option::is_none",
        serialize_with = "save_link"
    )]
    link: Option<Option<LinkColor>>,
    /// What was drawn on the chart, each on the market it names: the ones
    /// of the market charted show.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    drawings: Vec<Drawing>,
    /// `Some(false)` closes the drawing toolbar; `None` opens it.
    #[serde(rename = "showDrawingTools", skip_serializing_if = "Option::is_none")]
    show_tools: Option<bool>,
    /// What the chart computes from its bars and draws over them, in the
    /// order added.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    indicators: Vec<Indicator>,
    /// `Some(true)` makes the newest candle from the market's trades;
    /// `None` leaves it the venue's.
    #[serde(rename = "fastMode", skip_serializing_if = "Option::is_none")]
    fast: Option<bool>,
}

/// A link that is not the default one, as it is saved: its color's id, or
/// `""` for none.
fn save_link<S: Serializer>(
    link: &Option<Option<LinkColor>>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(link.flatten().map_or("", LinkColor::id))
}

impl From<Value> for ChartSettings {
    fn from(value: Value) -> Self {
        let Value::Object(members) = value else {
            return Self::default();
        };
        let shown = |name: &str| members.get(name).and_then(Value::as_bool).unwrap_or(true);
        Self::default()
            .with_market(members.get("market").and_then(MarketSelection::from_value))
            .with_interval(
                members
                    .get("timeframe")
                    .and_then(Value::as_str)
                    .map_or(Interval::DEFAULT, Interval::read),
            )
            .with_grid(shown("showGrid"))
            .with_ohlc(shown("showOhlcOnHover"))
            .with_link(match members.get("linkColor") {
                // The old app's "None".
                Some(Value::Null) => None,
                Some(Value::String(id)) if id.is_empty() => None,
                Some(Value::String(id)) => LinkColor::from_id(id).or(Some(LinkColor::DEFAULT)),
                _ => Some(LinkColor::DEFAULT),
            })
            .with_drawings(Drawing::read_all(members.get("drawings")))
            .with_tools(shown("showDrawingTools"))
            .with_indicators(Indicator::read_all(members.get("indicators")))
            .with_fast(
                members
                    .get("fastMode")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            )
    }
}

impl ChartSettings {
    /// The market charted.
    pub fn market(&self) -> MarketSelection {
        self.market
            .as_deref()
            .cloned()
            .unwrap_or_else(MarketSelection::bybit_btc_usdt)
    }

    /// Whether a market was picked (the default one is charted otherwise).
    pub fn has_market(&self) -> bool {
        self.market.is_some()
    }

    pub fn interval(&self) -> Interval {
        self.timeframe.unwrap_or(Interval::DEFAULT)
    }

    /// Whether the plot draws its grid.
    pub fn grid(&self) -> bool {
        self.show_grid.unwrap_or(true)
    }

    /// Whether the crosshair names the hovered bar's prices.
    pub fn ohlc(&self) -> bool {
        self.show_ohlc.unwrap_or(true)
    }

    /// These settings charting `market` (`None`: the default market).
    pub fn with_market(mut self, market: Option<MarketSelection>) -> Self {
        self.market = market.map(Box::new);
        self
    }

    pub fn with_interval(mut self, interval: Interval) -> Self {
        self.timeframe = (interval != Interval::DEFAULT).then_some(interval);
        self
    }

    pub fn with_grid(mut self, shown: bool) -> Self {
        self.show_grid = (!shown).then_some(false);
        self
    }

    pub fn with_ohlc(mut self, shown: bool) -> Self {
        self.show_ohlc = (!shown).then_some(false);
        self
    }

    /// The link color whose picks the chart takes, if any.
    pub fn link(&self) -> Option<LinkColor> {
        self.link.unwrap_or(Some(LinkColor::DEFAULT))
    }

    /// These settings linked to `color` (`None`: to no color).
    pub fn with_link(mut self, color: Option<LinkColor>) -> Self {
        self.link = (color != Some(LinkColor::DEFAULT)).then_some(color);
        self
    }

    /// These settings charting `market`, which the chart's link color was
    /// sent.
    pub fn with_linked_market(self, market: MarketSelection) -> Self {
        self.with_market(Some(market))
    }

    /// Every drawing saved, in the order drawn (later ones over earlier
    /// ones), whatever market each is on.
    pub fn drawings(&self) -> &[Drawing] {
        &self.drawings
    }

    pub fn with_drawings(mut self, drawings: Vec<Drawing>) -> Self {
        self.drawings = drawings;
        self
    }

    /// Whether the drawing toolbar is open.
    pub fn tools(&self) -> bool {
        self.show_tools.unwrap_or(true)
    }

    pub fn with_tools(mut self, shown: bool) -> Self {
        self.show_tools = (!shown).then_some(false);
        self
    }

    /// The chart's indicators, in the order added: the order they are
    /// listed and drawn in (later ones over earlier ones).
    pub fn indicators(&self) -> &[Indicator] {
        &self.indicators
    }

    pub fn with_indicators(mut self, indicators: Vec<Indicator>) -> Self {
        self.indicators = indicators;
        self
    }

    /// Whether the newest candle is made from the market's trades (a tick
    /// chart is its trades either way).
    pub fn fast(&self) -> bool {
        self.fast.unwrap_or(false)
    }

    pub fn with_fast(mut self, fast: bool) -> Self {
        self.fast = fast.then_some(true);
        self
    }
}
