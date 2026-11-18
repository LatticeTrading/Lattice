//! The Account Performance's settings panel: the rows its account is set
//! with (what every account widget's panel starts with:
//! [`account_panel`]), then its own, each a compact select but the last:
//!
//! - Period: the stretch of history plotted;
//! - Plot: the account's value, or its profit and loss;
//! - Of: the whole account, or its perpetuals alone;
//! - Style: a line, or the area under it filled;
//! - Show Grid: the square checkbox.
//!
//! The pane's gear opens it for the active tab; every change is saved in
//! the tab record at once. The period's badge over the plot picks the
//! period too.

use nsh_core::prelude::*;

use super::{
    settings::{AccountPerformanceSettings, Metric, Style},
    view::choice_select,
};
use crate::{
    data::{
        Portfolio,
        portfolio::{Period, Scope},
    },
    ui,
    widgets::{TabContext, common::accounts::account_panel},
};

/// The settings panel of the widget's tab, below the pane's gear.
pub(super) fn panel(
    tab: &TabContext<AccountPerformanceSettings>,
    portfolio: &Portfolio,
) -> Overlay {
    let saved = tab.settings;
    let grid = {
        let tab = tab.clone();
        ui::check_box(
            "Show Grid",
            move || saved.with(AccountPerformanceSettings::grid),
            move || {
                let settings = saved.get_untracked();
                let shown = settings.grid();
                tab.save(&settings.with_grid(!shown));
            },
        )
    };
    let rows = [
        ui::label_edge_row("Period").child(choice_select::<Period>(
            tab,
            "Period",
            AccountPerformanceSettings::period,
            AccountPerformanceSettings::with_period,
        )),
        ui::label_edge_row("Plot").child(choice_select::<Metric>(
            tab,
            "Plot",
            AccountPerformanceSettings::metric,
            AccountPerformanceSettings::with_metric,
        )),
        ui::label_edge_row("Of").child(choice_select::<Scope>(
            tab,
            "Of",
            AccountPerformanceSettings::scope,
            AccountPerformanceSettings::with_scope,
        )),
        ui::label_edge_row("Style").child(choice_select::<Style>(
            tab,
            "Style",
            AccountPerformanceSettings::style,
            AccountPerformanceSettings::with_style,
        )),
        ui::label_edge_row("Show Grid").child(grid),
    ];
    account_panel(tab, portfolio, "Account Performance settings", rows)
}
