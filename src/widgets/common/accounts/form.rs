//! The rows a widget's accounts are set with ([`account_rows`]), blocks that
//! meet edge to edge as the Everything Bar's and the Settings panel's do
//! (lines between them, no box in a box):
//!
//! - a line for each account the widget shows: its exchange's logo and
//!   name, its address in short (its tooltip has the whole of it), and ×
//!   which takes it out;
//! - "Exchange", filling the row, then the dropdown as wide as what it
//!   shows at its right, listing the exchanges that report accounts (the
//!   portfolio's sources);
//! - under them, the address of an account to add, with the exchange's hint
//!   of what one looks like while it is empty, then an Add button half the
//!   dropdown's width that commits it as Enter does;
//! - under that, why what was typed is no address, while it is not.
//!
//! Enter or Add adds what is typed, on the exchange picked, to the accounts
//! shown (once: one already shown stays as it is) and empties the field for
//! the next; the address is saved as the exchange's source writes it.
//! Anything that is no address there is left in the field with why under
//! it. Leaving the field or picking another exchange adds nothing, and an
//! empty field takes nothing away. Nothing is asked of the exchange to
//! check an address: one that holds nothing shows as an empty account.

use std::rc::Rc;

use nsh_base::SelectItem;
use nsh_core::{LineHeight, prelude::*};

use super::{AccountSettings, frame::venue_name};
use crate::{
    data::{Portfolio, portfolio::Account},
    theme, ui,
    widgets::{
        TabContext,
        common::{LogoStyle, exchange_logo},
    },
};

/// The narrowest the exchange's dropdown is.
const DROPDOWN_MIN_WIDTH: f32 = 120.0;
/// How wide the Add button after the address is: half the dropdown.
const ADD_WIDTH: f32 = DROPDOWN_MIN_WIDTH / 2.0;
/// The height of an account's line.
const LISTED_HEIGHT: f32 = 28.0;
/// How a listed account draws its exchange's logo.
const LOGO: LogoStyle = LogoStyle {
    size: 14.0,
    color: theme::TEXT,
    initials_size: 7.0,
    boxed: true,
};

/// The accounts `tab`'s widget shows, a line each, over the "Exchange" and
/// "Address" rows that add to them, saved in the tab's settings.
pub(crate) fn account_rows<S: AccountSettings>(tab: &TabContext<S>, portfolio: &Portfolio) -> View {
    let saved = tab.settings;
    let venues: Rc<[(&'static str, &'static str, &'static str)]> = portfolio
        .sources()
        .iter()
        .map(|source| (source.venue(), source.name(), source.address_hint()))
        .collect();
    let first = venues.first().map_or("", |(venue, ..)| *venue);

    // The account to add: its exchange, and its address as it is typed.
    let (venue, draft) = (signal(first.to_owned()), signal(String::new()));
    let problem = signal(None::<&'static str>);

    let commit: Rc<dyn Fn()> = {
        let (tab, portfolio) = (tab.clone(), portfolio.clone());
        Rc::new(move || {
            let typed = draft.get_untracked();
            let settings = saved.get_untracked();
            if typed.trim().is_empty() {
                problem.set(None);
                return;
            }
            match portfolio.account(&venue.get_untracked(), &typed) {
                Ok(account) => {
                    problem.set(None);
                    draft.set(String::new());
                    if !settings.accounts().contains(&account) {
                        let mut accounts = settings.accounts().to_vec();
                        accounts.push(account);
                        tab.save(&settings.with_accounts(accounts));
                    }
                }
                Err(error) => problem.set(Some(error.message())),
            }
        })
    };

    let of = {
        let venues = venues.clone();
        move |id: &str| venues.iter().find(|(venue, ..)| *venue == id).copied()
    };
    let exchange = {
        let (listed, named) = (venues.clone(), of.clone());
        let trigger = view()
            .h_full()
            .flex_row()
            .items_center()
            .justify_between()
            .gap(px(8.0))
            .px(px(10.0))
            .text_color(theme::TEXT)
            .hover(|style| style.bg(theme::SURFACE))
            .active(|style| style.bg(theme::BORDER))
            .child(
                text(move || {
                    let id = venue.get();
                    named(&id).map_or(id, |(_, name, _)| name.to_owned())
                })
                .number_of_lines(1),
            )
            .child(text("▾").text_color(theme::TEXT_FAINT));
        ui::select_from(
            move || {
                listed
                    .iter()
                    .map(|(venue, name, _)| SelectItem::new(*venue, *name))
                    .collect()
            },
            move || SharedString::from(venue.get()),
            move |picked| {
                // What was typed may be an address there.
                venue.set(picked.to_owned());
                problem.set(None);
            },
            "Exchange",
            trigger,
        )
        .flex_none()
        .min_w(px(DROPDOWN_MIN_WIDTH))
        .h_full()
        .border_l_1()
        .border_color(theme::BORDER)
    };
    let exchange = view()
        .h(ui::ROW_HEIGHT)
        .w_full()
        .flex_row()
        .items_center()
        .text_size(px(11.0))
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .px(px(10.0))
                .text_color(theme::TEXT_MUTED)
                .child(text("Exchange").number_of_lines(1)),
        )
        .child(exchange);

    let (submit, add) = (commit.clone(), commit);
    let field = ui::text_field(draft)
        .flex_1()
        .placeholder(move || {
            of(&venue.get())
                .map_or("Account address", |(.., hint)| hint)
                .to_owned()
        })
        .label("Account address")
        .test_id(&format!("{}-address", S::TEST_ID))
        .on_submit(move |_: &str| submit());
    let add = ui::button("Add", ui::Tone::Normal)
        .w(px(ADD_WIDTH))
        .flex_none()
        .justify_center()
        .border_l_1()
        .border_color(theme::BORDER)
        .label("Add account")
        .test_id(&format!("{}-add", S::TEST_ID))
        .on_press(move || add());
    let shown = {
        let (tab, portfolio) = (tab.clone(), portfolio.clone());
        let accounts = computed(move || saved.with(|settings| settings.accounts().to_vec()));
        dynamic(move || {
            let accounts = accounts.get();
            (!accounts.is_empty()).then(|| {
                let lines = accounts
                    .into_iter()
                    .enumerate()
                    .map(|(at, account)| listed(&tab, &portfolio, account, at));
                view()
                    .test_id(&format!("{}-accounts", S::TEST_ID))
                    .w_full()
                    .flex_col()
                    .children(lines.collect::<Vec<_>>())
            })
        })
    };

    let address = view()
        .h(ui::ROW_HEIGHT)
        .w_full()
        .flex_row()
        .border_t_1()
        .border_color(theme::BORDER)
        .child(field)
        .child(add);

    let said = view()
        .px(px(10.0))
        .py(px(5.0))
        .border_t_1()
        .border_l(px(3.0))
        .border_color(theme::BORDER)
        .border_l_color(theme::WARNING)
        .bg(theme::WARNING.opacity(0.1))
        .text_size(px(11.0))
        .text_color(theme::WARNING)
        .role(AccessibilityRole::Text)
        .test_id(&format!("{}-address-problem", S::TEST_ID))
        .style_when(move || problem.get().is_none(), |s| s.hidden())
        .child(
            text(move || problem.get().unwrap_or_default().to_owned())
                .line_height(LineHeight::Multiple(1.5)),
        );

    view()
        .w_full()
        .flex_col()
        .child(shown)
        .child(exchange)
        .child(address)
        .child(said)
}

/// The line of `account`, the `at`th of those `tab`'s widget shows: its
/// exchange's logo and name, its address in short (its tooltip has the
/// whole of it), and × which takes it out of them.
fn listed<S: AccountSettings>(
    tab: &TabContext<S>,
    portfolio: &Portfolio,
    account: Account,
    at: usize,
) -> View {
    let (saved, tab) = (tab.settings, tab.clone());
    let exchange = venue_name(portfolio, &account.venue);
    let (short, whole) = (account.short_address(), account.address.clone());
    let logo = exchange_logo(&account.venue, LOGO);
    let remove = ui::button("×", ui::Tone::Quiet)
        .flex_none()
        .label(format!("Remove {exchange} {short}"))
        .test_id(&format!("{}-remove-{at}", S::TEST_ID))
        .on_press(move || {
            let settings = saved.get_untracked();
            let left: Vec<Account> = settings
                .accounts()
                .iter()
                .filter(|kept| **kept != account)
                .cloned()
                .collect();
            tab.save(&settings.with_accounts(left));
        });
    view()
        .h(px(LISTED_HEIGHT))
        .w_full()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .pl(px(10.0))
        .border_b_1()
        .border_color(theme::BORDER)
        .text_size(px(11.0))
        .child(logo)
        .child(
            text(exchange)
                .flex_none()
                .number_of_lines(1)
                .text_color(theme::TEXT),
        )
        .child(
            text(short)
                .flex_1()
                .min_w(px(0.0))
                .truncate()
                .tabular_nums()
                .text_color(theme::TEXT_MUTED)
                .tooltip(ui::hint(move || whole.clone())),
        )
        .child(remove)
}
