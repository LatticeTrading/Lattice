//! Lattice's widgets: what a workspace tab shows. Each builds its view
//! from its widget session (`crate::data::session`), which keeps its data
//! while the widget is in the workspace.
//!
//! [`WidgetKind`] is the catalog the widget picker ([`picker`]) offers. A
//! tab saves its kind with its settings ([`WidgetSettings`], one type per
//! kind); a tab saved before kinds existed is a Market Screener.

pub(crate) mod market_screener;
pub(crate) mod picker;
pub(crate) mod trade_stream;

use serde_json::Value;

use self::{market_screener::ScreenerSettings, trade_stream::TradeStreamSettings};

/// A widget a tab can be: its name, what it is for, and where the picker
/// files it (the old app's `widgetRegistry` and picker catalog).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WidgetKind {
    MarketScreener,
    TradeStream,
}

impl WidgetKind {
    /// Every widget, in the order the picker lists them (the old app's
    /// registry order).
    pub const ALL: &[Self] = &[Self::MarketScreener, Self::TradeStream];

    /// The old app's widget id, which tabs save and test ids use.
    pub const fn id(self) -> &'static str {
        match self {
            Self::MarketScreener => "market-screener",
            Self::TradeStream => "trade-stream",
        }
    }

    /// The widget a saved id names.
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|kind| kind.id() == id)
    }

    /// Its name, which a new tab of it is titled with.
    pub const fn title(self) -> &'static str {
        match self {
            Self::MarketScreener => "Market Screener",
            Self::TradeStream => "Trade Stream",
        }
    }

    /// One sentence on what it is for, which the picker shows.
    pub const fn description(self) -> &'static str {
        match self {
            Self::MarketScreener => "Scan and compare derivative markets in a customizable table.",
            Self::TradeStream => "Watch recent buys and sells across your selected markets.",
        }
    }

    pub const fn category(self) -> Category {
        match self {
            Self::MarketScreener => Category::Market,
            Self::TradeStream => Category::OrderFlow,
        }
    }
}

/// A tab's widget and its settings: the kind is the variant, so a tab can
/// never hold another kind's settings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum WidgetSettings {
    MarketScreener(ScreenerSettings),
    TradeStream(TradeStreamSettings),
}

impl WidgetSettings {
    /// A new widget of `kind`, with its default settings.
    pub fn new(kind: WidgetKind) -> Self {
        match kind {
            WidgetKind::MarketScreener => Self::MarketScreener(ScreenerSettings::default()),
            WidgetKind::TradeStream => Self::TradeStream(TradeStreamSettings::default()),
        }
    }

    pub fn kind(&self) -> WidgetKind {
        match self {
            Self::MarketScreener(_) => WidgetKind::MarketScreener,
            Self::TradeStream(_) => WidgetKind::TradeStream,
        }
    }

    /// Whether they are the kind's defaults (the tab saves none).
    pub fn is_default(&self) -> bool {
        match self {
            Self::MarketScreener(settings) => settings.is_default(),
            Self::TradeStream(settings) => settings.is_default(),
        }
    }

    /// `kind`'s settings read from a saved member, leniently (a damaged
    /// member reads as defaults).
    pub fn read(kind: WidgetKind, saved: Value) -> Self {
        match kind {
            WidgetKind::MarketScreener => Self::MarketScreener(saved.into()),
            WidgetKind::TradeStream => Self::TradeStream(saved.into()),
        }
    }

    /// The saved member.
    pub fn to_value(&self) -> Value {
        let value = match self {
            Self::MarketScreener(settings) => serde_json::to_value(settings),
            Self::TradeStream(settings) => serde_json::to_value(settings),
        };
        value.unwrap_or(Value::Null)
    }
}

/// One kind's settings type, which its widget's view is given
/// (`layout::TabContext<S>`).
pub(crate) trait KindSettings: Clone + Default + PartialEq + 'static {
    /// Its settings in `settings`, when they are its kind's.
    fn of(settings: &WidgetSettings) -> Option<&Self>;
    fn wrap(self) -> WidgetSettings;
}

impl KindSettings for ScreenerSettings {
    fn of(settings: &WidgetSettings) -> Option<&Self> {
        match settings {
            WidgetSettings::MarketScreener(settings) => Some(settings),
            _ => None,
        }
    }

    fn wrap(self) -> WidgetSettings {
        WidgetSettings::MarketScreener(self)
    }
}

impl KindSettings for TradeStreamSettings {
    fn of(settings: &WidgetSettings) -> Option<&Self> {
        match settings {
            WidgetSettings::TradeStream(settings) => Some(settings),
            _ => None,
        }
    }

    fn wrap(self) -> WidgetSettings {
        WidgetSettings::TradeStream(self)
    }
}

/// The picker's groups of widgets, in the order its strip shows them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Category {
    Charts,
    Market,
    OrderFlow,
    Trading,
    Portfolio,
    Tools,
}

impl Category {
    pub const ALL: &[Self] = &[
        Self::Charts,
        Self::Market,
        Self::OrderFlow,
        Self::Trading,
        Self::Portfolio,
        Self::Tools,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Charts => "Charts",
            Self::Market => "Market",
            Self::OrderFlow => "Order Flow",
            Self::Trading => "Trading",
            Self::Portfolio => "Portfolio",
            Self::Tools => "Tools",
        }
    }

    /// Whether any widget is filed under it; the picker leaves out those
    /// that would list nothing.
    pub fn has_widgets(self) -> bool {
        WidgetKind::ALL.iter().any(|kind| kind.category() == self)
    }
}
