//! The depth chart's settings panel, laid out as the order book's (the old
//! app's `SettingsPanel`): a row per setting, its name on the left and its
//! control on the right. The pane's gear opens it for the active tab;
//! every change is saved in the tab record at once.
//!
//! - Primary, Pair 2, Grouping and Depth: the rows every widget that
//!   draws order books has ([`books::market_row`], [`books::grouping_row`],
//!   [`books::depth_row`]).
//! - Pairs: how two books are drawn together ([`Blend`]).
//! - Depth in: what depth is measured in ([`Unit`]).
//! - Show mid price: the old schema's switch.
//! - Layout: prices along the bottom (the old Depth Chart) or down the
//!   pane (the old Vertical Depth Chart).

use nsh_base::SelectItem;
use nsh_core::prelude::*;

use super::{
    DepthChartSettings,
    settings::{Blend, Unit},
};
use crate::{
    ui::{self, Side},
    widgets::{
        TabContext,
        common::books::{self, BookLayout, Pair},
    },
};

/// The old panel's width.
const PANEL_WIDTH: Points = px(340.0);
/// What the panel calls the mid price's switch.
const MID_PRICE: &str = "Show mid price";

/// The settings panel of the depth chart's tab, below the pane's gear.
pub(super) fn panel(tab: TabContext<DepthChartSettings>) -> Overlay {
    // The pair the market picker is open for.
    let picking = signal(None::<Pair>);
    let body = view()
        .test_id("depth-chart-settings")
        .flex_col()
        .py(px(4.0))
        .child(books::market_row(tab.clone(), Pair::Primary, picking))
        .child(books::market_row(tab.clone(), Pair::Secondary, picking))
        .child(choice_row(
            tab.clone(),
            "Pairs",
            &Blend::ALL,
            Blend::id,
            Blend::label,
            DepthChartSettings::blend,
            DepthChartSettings::with_blend,
        ))
        .child(choice_row(
            tab.clone(),
            "Depth in",
            &Unit::ALL,
            Unit::id,
            Unit::label,
            DepthChartSettings::unit,
            DepthChartSettings::with_unit,
        ))
        .child(books::grouping_row(tab.clone()))
        .child(books::depth_row(tab.clone()))
        .child(mid_price_row(tab.clone()))
        .child(choice_row(
            tab.clone(),
            "Layout",
            &BookLayout::ALL,
            BookLayout::id,
            BookLayout::label,
            DepthChartSettings::layout,
            DepthChartSettings::with_layout,
        ))
        .child(books::picker(&tab, picking));
    let close = {
        let tab = tab.clone();
        move || tab.close_settings()
    };
    ui::settings_panel(
        tab.settings_anchor(),
        Side::Right,
        PANEL_WIDTH,
        "Depth Chart settings",
        close,
        body,
    )
}

/// A row named `name` whose compact select picks one of `options`: the
/// one the settings hold (`current`) is shown, and a pick is saved
/// (`with`).
fn choice_row<T: Copy + 'static>(
    tab: TabContext<DepthChartSettings>,
    name: &'static str,
    options: &'static [T],
    id: fn(T) -> &'static str,
    label: fn(T) -> &'static str,
    current: fn(&DepthChartSettings) -> T,
    with: fn(DepthChartSettings, T) -> DepthChartSettings,
) -> View {
    let saved = tab.settings;
    let select = ui::compact_dropdown(
        move || {
            options
                .iter()
                .map(|option| SelectItem::new(id(*option), label(*option)))
                .collect()
        },
        move || SharedString::from(id(saved.with(current))),
        move || label(saved.with(current)).to_owned(),
        move |picked| {
            if let Some(option) = options.iter().find(|option| id(**option) == picked) {
                tab.save(&with(saved.get_untracked(), *option));
            }
        },
        name,
    );
    ui::label_edge_row(name).child(select)
}

/// "Show mid price": its name and its checkbox.
fn mid_price_row(tab: TabContext<DepthChartSettings>) -> View {
    let saved = tab.settings;
    let check = ui::check_box(
        MID_PRICE,
        move || saved.with(DepthChartSettings::mid_price),
        move || {
            let settings = saved.get_untracked();
            let on = !settings.mid_price();
            tab.save(&settings.with_mid_price(on));
        },
    );
    ui::label_edge_row(MID_PRICE).child(check)
}
