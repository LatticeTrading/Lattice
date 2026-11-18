//! The Account Summary's settings panel: the rows its account is set with
//! (what every account widget's panel starts with: [`account_panel`]), then
//! Show Balances and Hide Small Balances, the square checkboxes. The pane's gear opens it for the
//! active tab; every change is saved in the tab record at once.

use nsh_core::prelude::*;

use super::settings::AccountSummarySettings;
use crate::{
    data::Portfolio,
    ui,
    widgets::{TabContext, common::accounts::account_panel},
};

/// The settings panel of the summary's tab, below the pane's gear.
pub(super) fn panel(tab: &TabContext<AccountSummarySettings>, portfolio: &Portfolio) -> Overlay {
    let saved = tab.settings;
    let balances = {
        let tab = tab.clone();
        ui::check_box(
            "Show Balances",
            move || saved.with(AccountSummarySettings::balances),
            move || {
                let settings = saved.get_untracked();
                let shown = settings.balances();
                tab.save(&settings.with_balances(!shown));
            },
        )
    };
    let small = {
        let tab = tab.clone();
        ui::check_box(
            "Hide Small Balances",
            move || saved.with(AccountSummarySettings::small_hidden),
            move || {
                let settings = saved.get_untracked();
                let hidden = settings.small_hidden();
                tab.save(&settings.with_small_hidden(!hidden));
            },
        )
    };
    account_panel(
        tab,
        portfolio,
        "Account Summary settings",
        [
            ui::label_edge_row("Show Balances").child(balances),
            ui::label_edge_row("Hide Small Balances").child(small),
        ],
    )
}
