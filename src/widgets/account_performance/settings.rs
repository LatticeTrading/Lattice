//! The Account Performance's settings, saved in its tab's record:
//!
//! - `account`: the account it watches (what every widget that shows an
//!   account saves: [`Watched`]), or `accounts`: the several it plots
//!   together, on one exchange or on several;
//! - `period`: the stretch of history plotted (`"day"`, `"week"`,
//!   `"allTime"`; a month while left out);
//! - `scope`: `"perps"` for the account's perpetuals alone; the whole
//!   account while left out;
//! - `metric`: `"pnl"` for what the account made; what it was worth while
//!   left out;
//! - `style`: `"area"` for the area under the line filled; the line
//!   alone while left out. Either is green above its baseline and red
//!   below it;
//! - `showGrid`: the plot's grid, on while left out.
//!
//! Each is left out of the saved form while it holds its default, and read
//! leniently: a choice it does not know is the default, a switch that is
//! not a boolean is on.

use serde::{Serialize, Serializer};
use serde_json::{Map, Value};

use crate::{
    data::portfolio::{Period, Scope},
    widgets::common::accounts::{AccountSettings, Watched},
};

/// One of a few things the settings pick between: what they save it as,
/// and what the controls call it.
pub(crate) trait Choice: Copy + PartialEq + 'static {
    /// Every choice, in the order the controls list them.
    const ALL: &'static [Self];
    /// The one picked until told otherwise.
    const DEFAULT: Self;

    /// What settings save for it.
    fn id(self) -> &'static str;

    /// What the controls call it.
    fn label(self) -> &'static str;

    fn from_id(id: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|choice| choice.id() == id)
    }
}

impl Choice for Period {
    const ALL: &'static [Self] = &Period::ALL;
    const DEFAULT: Self = Self::Month;

    fn id(self) -> &'static str {
        match self {
            Self::Day => "day",
            Self::Week => "week",
            Self::Month => "month",
            Self::AllTime => "allTime",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Day => "24H",
            Self::Week => "7D",
            Self::Month => "30D",
            Self::AllTime => "All",
        }
    }
}

impl Choice for Scope {
    const ALL: &'static [Self] = &Scope::ALL;
    const DEFAULT: Self = Self::Total;

    fn id(self) -> &'static str {
        match self {
            Self::Total => "total",
            Self::Perps => "perps",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Total => "Whole Account",
            Self::Perps => "Perpetuals",
        }
    }
}

/// What of the account is plotted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Metric {
    /// What it was worth.
    Value,
    /// What it had made since the period began.
    Pnl,
}

impl Choice for Metric {
    const ALL: &'static [Self] = &[Self::Value, Self::Pnl];
    const DEFAULT: Self = Self::Value;

    fn id(self) -> &'static str {
        match self {
            Self::Value => "value",
            Self::Pnl => "pnl",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Value => "Account Value",
            Self::Pnl => "PnL",
        }
    }
}

/// How the series is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Style {
    /// A line, green above its baseline and red below it.
    Line,
    /// That line with the area between it and the baseline filled.
    Area,
}

impl Choice for Style {
    const ALL: &'static [Self] = &[Self::Line, Self::Area];
    const DEFAULT: Self = Self::Line;

    fn id(self) -> &'static str {
        match self {
            Self::Line => "line",
            Self::Area => "area",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Line => "Line",
            Self::Area => "Area",
        }
    }
}

/// Saves a choice as its id (a choice left out is not saved at all).
fn save<C: Choice, S: Serializer>(choice: &Option<C>, serializer: S) -> Result<S::Ok, S::Error> {
    match choice {
        Some(choice) => serializer.serialize_str(choice.id()),
        None => serializer.serialize_none(),
    }
}

/// The choice saved as `name` among `members`; the default for anything
/// else.
fn read<C: Choice>(members: &Map<String, Value>, name: &str) -> C {
    members
        .get(name)
        .and_then(Value::as_str)
        .and_then(C::from_id)
        .unwrap_or(C::DEFAULT)
}

/// `choice` as it is kept: `None` for the default.
fn kept<C: Choice>(choice: C) -> Option<C> {
    (choice != C::DEFAULT).then_some(choice)
}

/// One Account Performance's settings.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub(crate) struct AccountPerformanceSettings {
    /// The accounts charted, together.
    #[serde(flatten)]
    watched: Watched,
    /// `None` is a month.
    #[serde(skip_serializing_if = "Option::is_none", serialize_with = "save")]
    period: Option<Period>,
    /// `None` is the whole account.
    #[serde(skip_serializing_if = "Option::is_none", serialize_with = "save")]
    scope: Option<Scope>,
    /// `None` is the account's value.
    #[serde(skip_serializing_if = "Option::is_none", serialize_with = "save")]
    metric: Option<Metric>,
    /// `None` is a line.
    #[serde(skip_serializing_if = "Option::is_none", serialize_with = "save")]
    style: Option<Style>,
    /// `Some(false)` hides the grid; `None` shows it.
    #[serde(rename = "showGrid", skip_serializing_if = "Option::is_none")]
    show_grid: Option<bool>,
}

impl From<Value> for AccountPerformanceSettings {
    fn from(value: Value) -> Self {
        let Value::Object(members) = value else {
            return Self::default();
        };
        Self {
            watched: Watched::read(&members),
            ..Self::default()
        }
        .with_period(read(&members, "period"))
        .with_scope(read(&members, "scope"))
        .with_metric(read(&members, "metric"))
        .with_style(read(&members, "style"))
        .with_grid(
            members
                .get("showGrid")
                .and_then(Value::as_bool)
                .unwrap_or(true),
        )
    }
}

impl AccountPerformanceSettings {
    /// The stretch of history plotted.
    pub fn period(&self) -> Period {
        self.period.unwrap_or(Period::DEFAULT)
    }

    #[must_use]
    pub fn with_period(mut self, period: Period) -> Self {
        self.period = kept(period);
        self
    }

    /// The part of the account plotted.
    pub fn scope(&self) -> Scope {
        self.scope.unwrap_or(Scope::DEFAULT)
    }

    #[must_use]
    pub fn with_scope(mut self, scope: Scope) -> Self {
        self.scope = kept(scope);
        self
    }

    /// What of it is plotted.
    pub fn metric(&self) -> Metric {
        self.metric.unwrap_or(Metric::DEFAULT)
    }

    #[must_use]
    pub fn with_metric(mut self, metric: Metric) -> Self {
        self.metric = kept(metric);
        self
    }

    /// How the series is drawn.
    pub fn style(&self) -> Style {
        self.style.unwrap_or(Style::DEFAULT)
    }

    #[must_use]
    pub fn with_style(mut self, style: Style) -> Self {
        self.style = kept(style);
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

impl AccountSettings for AccountPerformanceSettings {
    const TEST_ID: &'static str = "account-performance";

    fn watched(&self) -> &Watched {
        &self.watched
    }

    fn with_watched(mut self, watched: Watched) -> Self {
        self.watched = watched;
        self
    }
}
