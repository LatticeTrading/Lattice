//! Lattice's widgets: what a workspace tab shows. Each builds its view
//! from its widget session (`crate::data::session`), which keeps its data
//! while the widget is in the workspace.
//!
//! [`WidgetKind`] is the catalog the widget picker ([`picker`]) offers. The
//! Market Screener is the only widget so far, so every tab is one; tabs do
//! not save their kind yet (see the development guide's widget notes).

pub(crate) mod market_screener;
pub(crate) mod picker;

/// A widget a tab can be: its name, what it is for, and where the picker
/// files it (the old app's `widgetRegistry` and picker catalog).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WidgetKind {
    MarketScreener,
}

impl WidgetKind {
    /// Every widget, in the order the picker lists them.
    pub const ALL: &[Self] = &[Self::MarketScreener];

    /// The old app's widget id, which test ids use.
    pub const fn id(self) -> &'static str {
        match self {
            Self::MarketScreener => "market-screener",
        }
    }

    /// Its name, which a new tab of it is titled with.
    pub const fn title(self) -> &'static str {
        match self {
            Self::MarketScreener => "Market Screener",
        }
    }

    /// One sentence on what it is for, which the picker shows.
    pub const fn description(self) -> &'static str {
        match self {
            Self::MarketScreener => "Scan and compare derivative markets in a customizable table.",
        }
    }

    pub const fn category(self) -> Category {
        match self {
            Self::MarketScreener => Category::Market,
        }
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
