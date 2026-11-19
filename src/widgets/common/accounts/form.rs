//! The rows a widget's account is set with ([`account_rows`]), blocks that
//! meet edge to edge as the Everything Bar's and the Settings panel's do
//! (lines between them, no box in a box):
//!
//! - "Exchange", filling the row, then the dropdown as wide as what it
//!   shows at its right, listing the exchanges that report accounts (the
//!   portfolio's sources);
//! - under them, the account's address, with the exchange's hint of what
//!   one looks like while it is empty, then a Search button half the
//!   dropdown's width that commits it as Enter does;
//! - under that, why what was typed is no address, while it is not.
//!
//! Enter, Search, leaving the field, or another exchange commits what is
//! typed: an address of the exchange is saved as its source writes it (so
//! the field then shows it that way), an empty field saves no account, and
//! anything else is left in the field with why it is no address under it.
//! Nothing is asked of the exchange to check an address: one that holds
//! nothing shows as an empty account.

use std::rc::Rc;

use nsh_base::SelectItem;
use nsh_core::{LineHeight, prelude::*};

use super::AccountSettings;
use crate::{data::Portfolio, theme, ui, widgets::TabContext};

/// The narrowest the exchange's dropdown is.
const DROPDOWN_MIN_WIDTH: f32 = 120.0;
/// How wide the Search button after the address is: half the dropdown.
const SEARCH_WIDTH: f32 = DROPDOWN_MIN_WIDTH / 2.0;

/// The "Exchange" and "Address" rows of `tab`'s account, which save it in
/// the tab's settings.
pub(crate) fn account_rows<S: AccountSettings>(tab: &TabContext<S>, portfolio: &Portfolio) -> View {
    let saved = tab.settings;
    let account = computed(move || saved.with(|settings| settings.account().cloned()));
    let venues: Rc<[(&'static str, &'static str, &'static str)]> = portfolio
        .sources()
        .iter()
        .map(|source| (source.venue(), source.name(), source.address_hint()))
        .collect();
    let first = venues.first().map_or("", |(venue, ..)| *venue);

    // What the rows show: the saved account, until something else is
    // picked or typed.
    let (venue, draft) = match account.get_untracked() {
        Some(account) => (signal(account.venue), signal(account.address)),
        None => (signal(first.to_owned()), signal(String::new())),
    };
    let problem = signal(None::<&'static str>);
    effect(move || {
        if let Some(account) = account.get() {
            venue.set(account.venue);
            draft.set(account.address);
            problem.set(None);
        }
    });

    let commit: Rc<dyn Fn()> = {
        let (tab, portfolio) = (tab.clone(), portfolio.clone());
        Rc::new(move || {
            let typed = draft.get_untracked();
            let settings = saved.get_untracked();
            if typed.trim().is_empty() {
                problem.set(None);
                if settings.account().is_some() {
                    tab.save(&settings.with_account(None));
                }
                return;
            }
            match portfolio.account(&venue.get_untracked(), &typed) {
                Ok(account) => {
                    problem.set(None);
                    draft.set(account.address.clone());
                    if settings.account() != Some(&account) {
                        tab.save(&settings.with_account(Some(account)));
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
        let (listed, named, commit) = (venues.clone(), of.clone(), commit.clone());
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
                venue.set(picked.to_owned());
                commit();
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

    let (submit, blur, search) = (commit.clone(), commit.clone(), commit);
    let field = ui::text_field(draft)
        .flex_1()
        .placeholder(move || {
            of(&venue.get())
                .map_or("Account address", |(.., hint)| hint)
                .to_owned()
        })
        .label("Account address")
        .test_id(&format!("{}-address", S::TEST_ID))
        .on_submit(move |_: &str| submit())
        .on_blur(move || blur());
    let search = ui::button("Search", ui::Tone::Normal)
        .w(px(SEARCH_WIDTH))
        .flex_none()
        .justify_center()
        .border_l_1()
        .border_color(theme::BORDER)
        .label("Search account")
        .test_id(&format!("{}-search", S::TEST_ID))
        .on_press(move || search());
    let address = view()
        .h(ui::ROW_HEIGHT)
        .w_full()
        .flex_row()
        .border_t_1()
        .border_color(theme::BORDER)
        .child(field)
        .child(search);

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
        .child(exchange)
        .child(address)
        .child(said)
}
