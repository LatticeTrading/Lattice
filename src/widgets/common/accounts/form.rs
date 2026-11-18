//! The rows a widget's account is set with ([`account_rows`]), laid out as
//! the settings panels' other rows (a name at the left edge, its control at
//! the right):
//!
//! - Exchange: the compact select, listing the exchanges that report
//!   accounts (the portfolio's sources);
//! - Address: a field for the account's address, with the exchange's hint
//!   of what one looks like while it is empty.
//!
//! Enter, leaving the field, or another exchange commits what is typed: an
//! address of the exchange is saved as its source writes it (so the field
//! then shows it that way), an empty field saves no account, and anything
//! else is left in the field with why it is no address under it. Nothing is
//! asked of the exchange to check an address: one that holds nothing shows
//! as an empty account.

use std::rc::Rc;

use nsh_base::SelectItem;
use nsh_core::{LineHeight, element::color, prelude::*};

use super::AccountSettings;
use crate::{data::Portfolio, theme, ui, widgets::TabContext};

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
        ui::compact_dropdown(
            move || {
                listed
                    .iter()
                    .map(|(venue, name, _)| SelectItem::new(*venue, *name))
                    .collect()
            },
            move || SharedString::from(venue.get()),
            move || {
                let id = venue.get();
                named(&id).map_or(id, |(_, name, _)| name.to_owned())
            },
            move |picked| {
                venue.set(picked.to_owned());
                commit();
            },
            "Exchange",
        )
    };

    let (submit, blur) = (commit.clone(), commit);
    let address = ui::text_field(draft)
        .flex_1()
        .min_w(px(0.0))
        .h(px(24.0))
        .px(px(6.0))
        .bg(theme::SURFACE)
        .border_1()
        .border_color(theme::BORDER)
        .hover(|s| s.border_color(theme::TEXT_FAINT))
        .focus(|s| s.border_color(theme::ACCENT))
        .selection_color(|| {
            color(theme::SURFACE.get()).blend(color(theme::ACCENT.opacity(0.35).get()))
        })
        .placeholder(move || {
            of(&venue.get())
                .map_or("Account address", |(.., hint)| hint)
                .to_owned()
        })
        .label("Account address")
        .test_id(&format!("{}-address", S::TEST_ID))
        .on_submit(move |_: &str| submit())
        .on_blur(move || blur());

    let said = view()
        .px(px(10.0))
        .pb(px(6.0))
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
        .child(ui::label_edge_row("Exchange").child(exchange))
        .child(ui::label_edge_row("Address").child(address))
        .child(said)
}
