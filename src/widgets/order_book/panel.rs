//! The order book's settings panel, laid out as the old app's
//! `SettingsPanel` drew its schema (`webGLOrderBookSettingsSchema`): a row
//! per setting, its name on the left and its control on the right, as the
//! Chart's and the Trade Stream's. The pane's gear opens it for the active
//! tab; every change is saved in the tab record at once.
//!
//! - Primary, Pair 2, Grouping and Depth: the rows every widget that
//!   draws order books has ([`books::market_row`], [`books::grouping_row`],
//!   [`books::depth_row`]).
//! - The six switches ([`Toggle`]): the old square checkboxes.
//! - Layout: the old compact select (Vertical, Horizontal).
//!
//! The book the ladder follows is not here: it was the top bar's alone in
//! the old app too.

use nsh_base::SelectItem;
use nsh_core::prelude::*;

use super::{OrderBookSettings, settings::Toggle};
use crate::{
    ui::{self, Side},
    widgets::{
        TabContext,
        common::books::{self, BookLayout, Pair},
    },
};

/// The old panel's width.
const PANEL_WIDTH: Points = px(340.0);

/// The settings panel of the order book's tab, below the pane's gear.
pub(super) fn panel(tab: TabContext<OrderBookSettings>) -> Overlay {
    // The pair the market picker is open for.
    let picking = signal(None::<Pair>);
    let body = view()
        .test_id("order-book-settings")
        .flex_col()
        .py(px(4.0))
        .child(books::market_row(tab.clone(), Pair::Primary, picking))
        .child(books::market_row(tab.clone(), Pair::Secondary, picking))
        .child(books::grouping_row(tab.clone()))
        .child(books::depth_row(tab.clone()))
        .children(
            Toggle::ALL
                .into_iter()
                .map(|toggle| toggle_row(tab.clone(), toggle)),
        )
        .child(layout_row(tab.clone()))
        .child(books::picker(&tab, picking));
    let close = {
        let tab = tab.clone();
        move || tab.close_settings()
    };
    ui::settings_panel(
        tab.settings_anchor(),
        Side::Right,
        PANEL_WIDTH,
        "Order Book settings",
        close,
        body,
    )
}

/// One switch: its name and its checkbox.
fn toggle_row(tab: TabContext<OrderBookSettings>, toggle: Toggle) -> View {
    let saved = tab.settings;
    let check = ui::check_box(
        toggle.label(),
        move || saved.with(|settings| settings.shows(toggle)),
        move || {
            let settings = saved.get_untracked();
            let on = !settings.shows(toggle);
            tab.save(&settings.with_toggle(toggle, on));
        },
    );
    ui::label_edge_row(toggle.label()).child(check)
}

/// "Layout": prices down the pane, or along its bottom.
fn layout_row(tab: TabContext<OrderBookSettings>) -> View {
    let saved = tab.settings;
    let select = ui::compact_dropdown(
        || {
            BookLayout::ALL
                .iter()
                .map(|layout| SelectItem::new(layout.id(), layout.label()))
                .collect()
        },
        move || SharedString::from(saved.with(|settings| settings.layout().id())),
        move || saved.with(|settings| settings.layout().label().to_owned()),
        move |id| {
            if let Some(layout) = BookLayout::from_id(id) {
                tab.save(&saved.get_untracked().with_layout(layout));
            }
        },
        "Layout",
    );
    ui::label_edge_row("Layout").child(select)
}
