//! The badges a widget that shows accounts floats over its plot (the
//! Account Performance's): small, bordered and see-through until the
//! pointer is over them, as the Chart's are ([`menu_badge`]), and the
//! accounts' own ([`account_badge`]): a logo for each of their exchanges,
//! then the address in short of one account or how many several are (its
//! tooltip has each in full), which opens a panel below it with the rows
//! that list them, add to them and take from them ([`account_rows`]). The
//! panel stays open while they change, until its close button.
//!
//! What a badge says of its accounts is what the frame's strip says too
//! ([`super::frame`]): [`watched`], [`counted`], [`described`], [`spelled`]
//! and [`logos`].

use nsh_core::prelude::*;

use super::{AccountSettings, account_rows, frame::venue_name};
use crate::{
    data::{Portfolio, portfolio::Account},
    theme,
    ui::{self, Side},
    widgets::{
        TabContext,
        common::{LogoStyle, exchange_logo},
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
/// What the accounts' panel is titled.
const TITLE: &str = "Accounts";
/// How far apart the logos of several exchanges are.
const LOGO_GAP: f32 = 3.0;

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

/// The accounts `tab`'s settings name (tracked), each with what its
/// exchange is called.
pub(super) fn watched<S: AccountSettings>(
    tab: &TabContext<S>,
    portfolio: &Portfolio,
) -> Memo<Vec<(String, Account)>> {
    let (saved, portfolio) = (tab.settings, portfolio.clone());
    computed(move || {
        saved.with(|settings| {
            settings
                .accounts()
                .iter()
                .map(|account| (venue_name(&portfolio, &account.venue), account.clone()))
                .collect::<Vec<_>>()
        })
    })
}

/// A logo for each exchange `accounts` are on, side by side.
pub(super) fn logos(accounts: Memo<Vec<(String, Account)>>) -> impl IntoElement {
    let exchanges = computed(move || accounts.with(|accounts| exchanges_of(accounts)));
    dynamic(move || {
        let logos: Vec<View> = exchanges
            .get()
            .iter()
            .map(|(id, _)| exchange_logo(id, LOGO))
            .collect();
        Some(
            view()
                .flex_none()
                .flex_row()
                .items_center()
                .gap(px(LOGO_GAP))
                .children(logos),
        )
    })
}

/// What the exchanges `accounts` are on are called, each once.
pub(super) fn exchange_names(accounts: &[(String, Account)]) -> String {
    let names: Vec<String> = exchanges_of(accounts)
        .into_iter()
        .map(|(_, name)| name)
        .collect();
    names.join(", ")
}

/// What the badge says of `accounts`, each with what its exchange is
/// called: the address in short of one, how many of several.
pub(super) fn counted(accounts: &[(String, Account)]) -> String {
    match accounts {
        [] => String::new(),
        [(_, account)] => account.short_address(),
        several => format!("{} accounts", several.len()),
    }
}

/// What the badge is named for assistive tech: what it changes, and whose.
pub(super) fn described(accounts: &[(String, Account)]) -> String {
    let each: Vec<String> = accounts
        .iter()
        .map(|(exchange, account)| format!("{exchange} {}", account.short_address()))
        .collect();
    let what = if accounts.len() > 1 {
        "Change accounts"
    } else {
        "Change account"
    };
    format!("{what}: {}", each.join(", "))
}

/// What the badge's tooltip reads: the whole address of one account, and
/// of several each on a line after its exchange.
pub(super) fn spelled(accounts: &[(String, Account)]) -> String {
    match accounts {
        [(_, account)] => account.address.clone(),
        several => several
            .iter()
            .map(|(exchange, account)| format!("{exchange} {}", account.address))
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

/// The exchanges of `accounts` (each its id and what it is called), each
/// once, in their order.
fn exchanges_of(accounts: &[(String, Account)]) -> Vec<(String, String)> {
    let mut exchanges: Vec<(String, String)> = Vec::new();
    for (name, account) in accounts {
        if !exchanges.iter().any(|(id, _)| *id == account.venue) {
            exchanges.push((account.venue.clone(), name.clone()));
        }
    }
    exchanges
}

/// The badge of `tab`'s accounts: their exchanges' logos, then the address
/// in short of one or how many several are, which opens the panel that
/// changes them.
pub(crate) fn account_badge<S: AccountSettings>(
    tab: &TabContext<S>,
    portfolio: &Portfolio,
) -> View {
    let accounts = watched(tab, portfolio);
    let names = move || accounts.with(|accounts| exchange_names(accounts));
    let open = signal(false);
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
        .label(names)
        .tooltip(ui::hint(names))
        .child(logos(accounts));
    let address = menu_badge(open)
        .element_ref(&anchor)
        .label(move || accounts.with(|accounts| described(accounts)))
        .test_id(&format!("{}-account", S::TEST_ID))
        .tooltip(ui::hint(move || {
            accounts.with(|accounts| spelled(accounts))
        }))
        .child(
            text(move || accounts.with(|accounts| counted(accounts)))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_badge_says_one_account_by_its_address_and_several_by_their_number() {
        let account = |exchange: &str, venue: &str, address: &str| {
            (
                exchange.to_owned(),
                Account {
                    venue: venue.to_owned(),
                    address: address.to_owned(),
                },
            )
        };
        const ADDRESS: &str = "0x0123456789abcdef0123456789abcdef01234567";
        const OTHER: &str = "0x00000000000000000000000000000000000000aa";
        let one = [account("Hyperliquid", "hyperliquid", ADDRESS)];
        assert_eq!(counted(&one), "0x0123…4567");
        assert_eq!(described(&one), "Change account: Hyperliquid 0x0123…4567");
        assert_eq!(spelled(&one), ADDRESS);

        // The same address on two exchanges, and another on the first.
        let several = [
            one[0].clone(),
            account("Lighter", "lighterxyz", ADDRESS),
            account("Hyperliquid", "hyperliquid", OTHER),
        ];
        assert_eq!(counted(&several), "3 accounts");
        assert_eq!(
            described(&several),
            "Change accounts: Hyperliquid 0x0123…4567, Lighter 0x0123…4567, \
             Hyperliquid 0x0000…00aa"
        );
        assert_eq!(spelled(&several).lines().count(), 3);
        assert!(spelled(&several).contains(&format!("Lighter {ADDRESS}")));
        // A logo an exchange, however many accounts are on it.
        assert_eq!(
            exchanges_of(&several),
            [
                ("hyperliquid".to_owned(), "Hyperliquid".to_owned()),
                ("lighterxyz".to_owned(), "Lighter".to_owned()),
            ]
        );
        assert_eq!(exchange_names(&several), "Hyperliquid, Lighter");
        assert_eq!(counted(&[]), "");
    }
}
