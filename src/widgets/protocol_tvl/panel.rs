//! The Protocol TVL's settings panel, each row a compact select but the
//! last:
//!
//! - Range: the stretch of history plotted;
//! - Style: a line, or the area under it filled;
//! - Show Grid: the square checkbox.
//!
//! The pane's gear opens it for the active tab; every change is saved in
//! the tab record at once. The range's badge over the plot picks the range
//! too, and the protocol's badge the protocol.

use nsh_base::SelectItem;
use nsh_core::prelude::*;

use super::settings::{ProtocolTvlSettings, Range, Style};
use crate::{
    ui::{self, Side},
    widgets::TabContext,
};

/// The widgets' settings panels' width.
const PANEL_WIDTH: Points = px(340.0);

/// The settings panel of the widget's tab, below the pane's gear.
pub(super) fn panel(tab: &TabContext<ProtocolTvlSettings>) -> Overlay {
    let saved = tab.settings;
    let range = {
        let tab = tab.clone();
        ui::compact_dropdown(
            || {
                Range::ALL
                    .iter()
                    .map(|range| SelectItem::new(range.id(), range.label()))
                    .collect()
            },
            move || SharedString::from(saved.with(|settings| settings.range().id())),
            move || saved.with(|settings| settings.range().label().to_owned()),
            move |id| {
                if let Some(range) = Range::from_id(id) {
                    tab.save(&saved.get_untracked().with_range(range));
                }
            },
            "Range",
        )
    };
    let style = {
        let tab = tab.clone();
        ui::compact_dropdown(
            || {
                Style::ALL
                    .iter()
                    .map(|style| SelectItem::new(style.id(), style.label()))
                    .collect()
            },
            move || SharedString::from(saved.with(|settings| settings.style().id())),
            move || saved.with(|settings| settings.style().label().to_owned()),
            move |id| {
                if let Some(style) = Style::from_id(id) {
                    tab.save(&saved.get_untracked().with_style(style));
                }
            },
            "Style",
        )
    };
    let grid = {
        let tab = tab.clone();
        ui::check_box(
            "Show Grid",
            move || saved.with(ProtocolTvlSettings::grid),
            move || {
                let settings = saved.get_untracked();
                let shown = settings.grid();
                tab.save(&settings.with_grid(!shown));
            },
        )
    };
    let body = view()
        .test_id("protocol-tvl-settings")
        .flex_col()
        .py(px(4.0))
        .child(ui::label_edge_row("Range").child(range))
        .child(ui::label_edge_row("Style").child(style))
        .child(ui::label_edge_row("Show Grid").child(grid));
    let close = {
        let tab = tab.clone();
        move || tab.close_settings()
    };
    ui::settings_panel(
        tab.settings_anchor(),
        Side::Right,
        PANEL_WIDTH,
        "Protocol TVL settings",
        close,
        body,
    )
}
