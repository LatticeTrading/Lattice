//! The Protocol TVL's settings, saved in its tab's record:
//!
//! - `protocol`: the DefiLlama slug of the protocol charted (`aave-v3`, or
//!   a parent's: `aave`); Hyperliquid's while left out;
//! - `range`: the stretch of its history plotted (`"30d"`, `"90d"`,
//!   `"all"`; a year while left out);
//! - `style`: `"area"` for the area under the line filled; the line alone
//!   while left out. Either is green above what the range began at and red
//!   below it;
//! - `showGrid`: the plot's grid, on while left out.
//!
//! Each is left out of the saved form while it holds its default, and read
//! leniently: a choice it does not know is the default, a switch that is
//! not a boolean is on, a slug that is not text is the default protocol.

use serde::{Serialize, Serializer};
use serde_json::Value;

use crate::data::ferris::defillama::normalized;

/// The protocol charted until another is picked.
pub(crate) const DEFAULT_PROTOCOL: &str = "hyperliquid";
/// The longest slug kept: DefiLlama's run to about forty characters.
const MAX_SLUG: usize = 96;

/// The stretch of a protocol's history plotted, counted back from its
/// latest point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Range {
    Month,
    Quarter,
    Year,
    All,
}

impl Range {
    /// Every range, in the order the controls list them.
    pub const ALL: [Self; 4] = [Self::Month, Self::Quarter, Self::Year, Self::All];
    /// The one plotted until told otherwise.
    pub const DEFAULT: Self = Self::Year;

    /// What settings save for it.
    pub fn id(self) -> &'static str {
        match self {
            Self::Month => "30d",
            Self::Quarter => "90d",
            Self::Year => "1y",
            Self::All => "all",
        }
    }

    /// What the controls call it.
    pub fn label(self) -> &'static str {
        match self {
            Self::Month => "30D",
            Self::Quarter => "90D",
            Self::Year => "1Y",
            Self::All => "All",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|range| range.id() == id)
    }

    /// The days it reaches back; `None` for all of the history.
    pub fn days(self) -> Option<f64> {
        match self {
            Self::Month => Some(30.0),
            Self::Quarter => Some(90.0),
            Self::Year => Some(365.0),
            Self::All => None,
        }
    }
}

/// How the series is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Style {
    /// A line, green above what the range began at and red below it.
    Line,
    /// That line with the area between it and where it began filled.
    Area,
}

impl Style {
    pub const ALL: [Self; 2] = [Self::Line, Self::Area];
    pub const DEFAULT: Self = Self::Line;

    pub fn id(self) -> &'static str {
        match self {
            Self::Line => "line",
            Self::Area => "area",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Line => "Line",
            Self::Area => "Area",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|style| style.id() == id)
    }
}

fn save_range<S: Serializer>(range: &Option<Range>, serializer: S) -> Result<S::Ok, S::Error> {
    match range {
        Some(range) => serializer.serialize_str(range.id()),
        None => serializer.serialize_none(),
    }
}

fn save_style<S: Serializer>(style: &Option<Style>, serializer: S) -> Result<S::Ok, S::Error> {
    match style {
        Some(style) => serializer.serialize_str(style.id()),
        None => serializer.serialize_none(),
    }
}

/// One Protocol TVL's settings.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub(crate) struct ProtocolTvlSettings {
    /// `None` is [`DEFAULT_PROTOCOL`].
    #[serde(skip_serializing_if = "Option::is_none")]
    protocol: Option<String>,
    /// `None` is a year.
    #[serde(skip_serializing_if = "Option::is_none", serialize_with = "save_range")]
    range: Option<Range>,
    /// `None` is a line.
    #[serde(skip_serializing_if = "Option::is_none", serialize_with = "save_style")]
    style: Option<Style>,
    /// `Some(false)` hides the grid; `None` shows it.
    #[serde(rename = "showGrid", skip_serializing_if = "Option::is_none")]
    show_grid: Option<bool>,
}

impl From<Value> for ProtocolTvlSettings {
    fn from(value: Value) -> Self {
        let text = |name: &str| value.get(name).and_then(Value::as_str);
        Self::default()
            .with_protocol(text("protocol").unwrap_or(DEFAULT_PROTOCOL))
            .with_range(
                text("range")
                    .and_then(Range::from_id)
                    .unwrap_or(Range::DEFAULT),
            )
            .with_style(
                text("style")
                    .and_then(Style::from_id)
                    .unwrap_or(Style::DEFAULT),
            )
            .with_grid(
                value
                    .get("showGrid")
                    .and_then(Value::as_bool)
                    .unwrap_or(true),
            )
    }
}

impl ProtocolTvlSettings {
    /// The DefiLlama slug of the protocol charted.
    pub fn protocol(&self) -> &str {
        self.protocol.as_deref().unwrap_or(DEFAULT_PROTOCOL)
    }

    /// These settings charting the protocol `slug` names; one that names
    /// none (empty, or longer than any slug) is the default protocol.
    #[must_use]
    pub fn with_protocol(mut self, slug: &str) -> Self {
        let slug = normalized(slug);
        let named = !slug.is_empty() && slug.chars().count() <= MAX_SLUG;
        self.protocol = (named && slug != DEFAULT_PROTOCOL).then_some(slug);
        self
    }

    /// The stretch of history plotted.
    pub fn range(&self) -> Range {
        self.range.unwrap_or(Range::DEFAULT)
    }

    #[must_use]
    pub fn with_range(mut self, range: Range) -> Self {
        self.range = (range != Range::DEFAULT).then_some(range);
        self
    }

    /// How the series is drawn.
    pub fn style(&self) -> Style {
        self.style.unwrap_or(Style::DEFAULT)
    }

    #[must_use]
    pub fn with_style(mut self, style: Style) -> Self {
        self.style = (style != Style::DEFAULT).then_some(style);
        self
    }

    /// Whether the plot draws its grid.
    pub fn grid(&self) -> bool {
        self.show_grid.unwrap_or(true)
    }

    #[must_use]
    pub fn with_grid(mut self, shown: bool) -> Self {
        self.show_grid = (!shown).then_some(false);
        self
    }
}
