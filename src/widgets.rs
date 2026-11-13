//! Lattice's widgets: what a workspace tab shows. Each builds its view
//! from its widget session (`crate::data::session`), which keeps its data
//! while the widget is in the workspace.
//!
//! The Market Screener is the only widget so far, so every tab is one.

pub(crate) mod market_screener;
