//! The badges a widget that shows an account floats over its plot (the
//! Account Performance's): small, bordered and see-through until the
//! pointer is over them, as the Chart's are ([`menu_badge`]), and the
//! account's own ([`account_badge`]): its exchange's logo, then its address
//! in short (its tooltip has the whole of it), which opens a panel below it
//! with the rows that change the account ([`account_rows`]). The panel
//! closes once another account is saved, and with its close button.

use nsh_core::prelude::*;

use super::{AccountSettings, account_rows, frame::venue_of};
use crate::{
    data::{Portfolio, portfolio::Account},
    theme,
    ui::{self, Side},
    widgets::{
        TabContext,
        common::{LogoStyle, bound_exchange_logo},
    },
};

/// A badge's height, its text size and the room either side of what it
/// says (the Chart's).
const HEIGHT: f32 = 16.0;
const TEXT_SIZE: f32 = 10.0;
const PADDING: f32 = 6.0;
/// How opaque a badge is until the pointer is over it.
const OPACITY: f32 = 0.6;
/// How the account's badge draws its exchange's logo.
const LOGO: LogoStyle = LogoStyle {
    size: 14.0,
    color: theme::TEXT,
    initials_size: 7.0,
    boxed: true,
};
/// Wide enough for an address in its field beside the row's name.
const PANEL_WIDTH: Points = px(360.0);
/// What the account's panel is titled.
const TITLE: &str = "Account";

/// A badge as wide as what it says: a button that opens and closes what
/// `open` says is open (its menu or panel), lit while it is. Its caller
/// names it and gives it what it says.
pub(crate) fn menu_badge(open: Signal<bool>) -> View {
    view()
        .h(px(HEIGHT))
        .flex_none()
        .flex_row()
        .items_center()
        .px(px(PADDING))
        .bg(theme::SURFACE.opacity(OPACITY))
        .border_1()
        .border_color(theme::BORDER)
        .text_size(px(TEXT_SIZE))
        .text_color(theme::TEXT_MUTED)
        .role(AccessibilityRole::Button)
        .accessibility_state(move || AccessibilityState {
            expanded: Some(open.get()),
            has_popup: true,
            ..Default::default()
        })
        .hover(|s| {
            s.bg(theme::SURFACE)
                .border_color(theme::TEXT_FAINT)
                .text_color(theme::TEXT)
        })
        .active(|s| s.bg(theme::BORDER))
        .style_when(
            move || open.get(),
            |s| {
                s.bg(theme::SURFACE)
                    .border_color(theme::ACCENT)
                    .text_color(theme::ACCENT)
            },
        )
        .on_press(move || open.update(|open| *open = !*open))
}

/// The badge of `tab`'s account: its exchange's logo, then its address in
/// short, which opens the panel that changes it.
pub(crate) fn account_badge<S: AccountSettings>(
    tab: &TabContext<S>,
    portfolio: &Portfolio,
) -> View {
    let saved = tab.settings;
    let account = computed(move || saved.with(|settings| settings.account().cloned()));
    let venue = venue_of(portfolio, account);
    let of = move |read: fn(&Account) -> String| {
        account.with(|account| account.as_ref().map(read).unwrap_or_default())
    };
    let (logo, initials) = bound_exchange_logo(
        move || account.with(|account| account.as_ref().map(|account| account.venue.clone())),
        LOGO,
    );
    let open = signal(false);
    // Another account saved: the panel has done its work.
    effect(move || {
        account.with(|_| ());
        open.set(false);
    });
    let anchor = element_ref();
    let panel = {
        let (tab, portfolio) = (tab.clone(), portfolio.clone());
        dynamic(move || {
            open.get().then(|| {
                ui::settings_panel(
                    &anchor,
                    Side::Left,
                    PANEL_WIDTH,
                    TITLE,
                    move || open.set(false),
                    view()
                        .test_id(&format!("{}-account-panel", S::TEST_ID))
                        .flex_col()
                        .py(px(4.0))
                        .child(account_rows(&tab, &portfolio)),
                )
            })
        })
    };
    let exchange = view()
        .flex_none()
        .flex_row()
        .items_center()
        .role(AccessibilityRole::Image)
        .label(move || venue.get())
        .tooltip(ui::hint(move || venue.get()))
        .child(logo)
        .child(initials);
    let address = menu_badge(open)
        .element_ref(&anchor)
        .label(move || {
            format!(
                "Change account: {} {}",
                venue.get(),
                of(Account::short_address)
            )
        })
        .test_id(&format!("{}-account", S::TEST_ID))
        .tooltip(ui::hint(move || of(|account| account.address.clone())))
        .child(
            text(move || of(Account::short_address))
                .number_of_lines(1)
                .tabular_nums(),
        )
        .child(panel);
    view()
        .flex_none()
        .flex_row()
        .items_center()
        .gap(px(6.0))
        .child(exchange)
        .child(address)
}
