//! The Funding Rates' settings panel, laid out as the old app's
//! `SettingsPanel` drew its schema (`fundingRatesSettingsSchema`): a row
//! per switch, its name on the left and its checkbox on the right, as the
//! other widgets' panels. The pane's gear opens it for the active tab;
//! every change is saved in the tab record at once.
//!
//! The markets are not here: the table adds, changes and removes them, as
//! the old widget's did.

use nsh_core::prelude::*;

use super::settings::{FundingRatesSettings, Toggle};
use crate::{
    ui::{self, Side},
    widgets::TabContext,
};

/// The old panel's width.
const PANEL_WIDTH: Points = px(340.0);

/// The settings panel of the widget's tab, below the pane's gear.
pub(super) fn panel(tab: TabContext<FundingRatesSettings>) -> Overlay {
    let body = view()
        .test_id("funding-rates-settings")
        .flex_col()
        .py(px(4.0))
        .children(
            Toggle::ALL
                .into_iter()
                .map(|toggle| toggle_row(tab.clone(), toggle)),
        );
    let close = {
        let tab = tab.clone();
        move || tab.close_settings()
    };
    ui::settings_panel(
        tab.settings_anchor(),
        Side::Right,
        PANEL_WIDTH,
        "Funding Rates settings",
        close,
        body,
    )
}

/// One switch: its name and its checkbox.
fn toggle_row(tab: TabContext<FundingRatesSettings>, toggle: Toggle) -> View {
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
