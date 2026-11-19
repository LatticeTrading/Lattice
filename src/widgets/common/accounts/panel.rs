//! The settings panel of a widget that shows an account
//! ([`account_panel`]): the rows its account is set with
//! ([`account_rows`]), then the widget's own, as the other widgets' panels
//! are laid out (a name at the left edge, its control at the right). The
//! pane's gear opens it for the active tab, and so does the account's strip
//! ([`super::frame`]); every change is saved in the tab record at once.

use nsh_core::prelude::*;

use super::{AccountSettings, account_rows};
use crate::{
    data::Portfolio,
    theme,
    ui::{self, Side},
    widgets::TabContext,
};

/// Wide enough for an address in its field beside the row's name.
const PANEL_WIDTH: Points = px(400.0);

/// The settings panel of `tab`'s widget, titled `title`, below the pane's
/// gear: its account's rows, then `rows`.
pub(crate) fn account_panel<S: AccountSettings>(
    tab: &TabContext<S>,
    portfolio: &Portfolio,
    title: &str,
    rows: impl IntoIterator<Item = View>,
) -> Overlay {
    let body = view()
        .test_id(&format!("{}-settings", S::TEST_ID))
        .flex_col()
        .child(
            account_rows(tab, portfolio)
                .border_b_1()
                .border_color(theme::BORDER),
        )
        .child(view().flex_col().py(px(4.0)).children(rows));
    let close = {
        let tab = tab.clone();
        move || tab.close_settings()
    };
    ui::settings_panel(
        tab.settings_anchor(),
        Side::Right,
        PANEL_WIDTH,
        title,
        close,
        body,
    )
}
