//! Lattice's widgets: what a workspace tab shows. Each builds its view
//! from its widget session (`crate::data::session`), which keeps its data
//! while the widget is in the workspace.
//!
//! This file is the registry: one line per widget, in the order the
//! picker ([`picker`]) lists them. Each widget lives in its folder, whose
//! `mod.rs` implements [`host::Widget`]; adding one is its `mod` line and
//! its registry line here, and nothing else outside its folder. From the
//! list, [`registry`] makes [`WidgetKind`] (the catalog), [`WidgetSettings`]
//! (a tab's kind and settings) and [`view`] (a tab's view, which the host
//! starts the same way for every widget). A tab saves its kind with its
//! settings; a tab saved before kinds existed is a Market Screener
//! ([`WidgetKind::UNNAMED`]).

pub(crate) mod common;
mod host;
pub(crate) mod picker;
mod registry;

// The widgets, each listed once in the registry below.
mod chart;
mod market_screener;
mod trade_stream;

pub(crate) use host::{Category, SettingsPanel, TabContext, TabHandle};

registry::registry! {
    chart::Chart,
    market_screener::MarketScreener,
    trade_stream::TradeStream,
}

impl WidgetKind {
    /// The kind of a tab whose record names none: every tab saved before
    /// kinds existed was a Market Screener, the first widget, and its tabs
    /// still save no kind.
    pub const UNNAMED: Self = Self::MarketScreener;
}
