//! What every widget that shows an account has around its own content
//! ([`account_frame`]), top to bottom:
//!
//! - the account's strip: its exchange's logo and name, and its address in
//!   short (its tooltip has the whole of it). Pressing it opens the
//!   widget's settings, where the account is changed. A widget that shows
//!   the account itself ([`super::account_badge`], over its plot) has its
//!   frame without the strip ([`bare_account_frame`]);
//! - what is to be said of the report the widget shows ([`Standing`]): that
//!   it is loading, in muted text; that it could not be asked for or read,
//!   or has not been brought up to date, as a warning. The figures of a
//!   report that is merely not up to date stay;
//! - the widget's content;
//! - instead of all three while no account is set: the rows that set one
//!   ([`account_rows`]), one bordered piece in the middle.

use nsh_core::{LineHeight, prelude::*};

use super::{AccountSettings, Standing, account_rows};
use crate::{
    data::{
        Portfolio,
        portfolio::{Account, Unavailable},
    },
    exchanges, theme, ui,
    widgets::{
        TabContext,
        common::{LogoStyle, bound_exchange_logo},
    },
};

/// The strip's height, with the line under it.
const STRIP_HEIGHT: f32 = 29.0;
/// How the strip draws its exchange's logo.
const LOGO: LogoStyle = LogoStyle {
    size: 14.0,
    color: theme::TEXT,
    initials_size: 7.0,
    boxed: true,
};
/// The widest the rows are while no account is set.
const FORM_WIDTH: f32 = 360.0;

/// `body` (the widget's content, which fills what is left) in the frame of
/// a widget showing `what` (`positions`: the words its notes use) of its
/// tab's account, whose report stands as `standing` says.
pub(crate) fn account_frame<S: AccountSettings>(
    tab: &TabContext<S>,
    portfolio: &Portfolio,
    standing: Memo<Standing>,
    what: &'static str,
    body: View,
) -> View {
    frame(tab, portfolio, standing, what, body, true)
}

/// [`account_frame`] without the account's strip, for a widget that shows
/// its account itself ([`super::account_badge`]), which leaves its content
/// the room.
pub(crate) fn bare_account_frame<S: AccountSettings>(
    tab: &TabContext<S>,
    portfolio: &Portfolio,
    standing: Memo<Standing>,
    what: &'static str,
    body: View,
) -> View {
    frame(tab, portfolio, standing, what, body, false)
}

/// The frame, with the account's strip when `with_strip` says so.
fn frame<S: AccountSettings>(
    tab: &TabContext<S>,
    portfolio: &Portfolio,
    standing: Memo<Standing>,
    what: &'static str,
    body: View,
    with_strip: bool,
) -> View {
    let saved = tab.settings;
    let account = computed(move || saved.with(|settings| settings.account().cloned()));
    let unset = computed(move || account.with(Option::is_none));
    let venue = venue_of(portfolio, account);
    let ask = {
        let (tab, portfolio) = (tab.clone(), portfolio.clone());
        dynamic(move || unset.get().then(|| ask(&tab, &portfolio)))
    };
    view()
        .test_id(S::TEST_ID)
        .flex_1()
        .min_h(px(0.0))
        .min_w(px(0.0))
        .flex_col()
        .overflow_hidden()
        .bg(theme::BACKGROUND)
        .text_color(theme::TEXT)
        .text_size(px(12.0))
        .children(
            with_strip.then(|| {
                strip(tab, account, venue).style_when(move || unset.get(), |s| s.hidden())
            }),
        )
        .child(note(venue, standing, what))
        .child(
            view()
                .flex_1()
                .min_h(px(0.0))
                .min_w(px(0.0))
                .flex_col()
                .style_when(move || unset.get(), |s| s.hidden())
                .child(body),
        )
        .child(ask)
}

/// What `account`'s exchange is called (tracked): its source's name, else
/// the name Lattice knows its id by; nothing with no account.
pub(super) fn venue_of(portfolio: &Portfolio, account: Memo<Option<Account>>) -> Memo<String> {
    let portfolio = portfolio.clone();
    computed(move || {
        account.with(|account| {
            account.as_ref().map_or_else(String::new, |account| {
                portfolio.source(&account.venue).map_or_else(
                    || exchanges::display_name(&account.venue).to_owned(),
                    |source| source.name().to_owned(),
                )
            })
        })
    })
}

/// What a widget says of a report of its tab's account besides the one its
/// frame speaks for (a second report it shows), as the frame would: a strip
/// saying how `what` stands, nothing while all is well.
pub(crate) fn report_note<S: AccountSettings>(
    tab: &TabContext<S>,
    portfolio: &Portfolio,
    standing: Memo<Standing>,
    what: &'static str,
) -> View {
    let saved = tab.settings;
    let account = computed(move || saved.with(|settings| settings.account().cloned()));
    note(venue_of(portfolio, account), standing, what)
}

/// What the frame says of a report standing as `standing`, and whether it
/// warns; nothing while all is well. `venue` is the account's exchange and
/// `what` what the widget shows of it.
fn said(standing: &Standing, venue: &str, what: &str) -> Option<(bool, String)> {
    Some(match standing {
        Standing::Unset | Standing::Ready => return None,
        Standing::Loading => (false, format!("Loading {what}…")),
        Standing::Unavailable(Unavailable::UnknownVenue) => (
            true,
            "This account's exchange is not available in this version of Lattice.".to_owned(),
        ),
        Standing::Unavailable(Unavailable::NotReported) => {
            (true, format!("{venue} does not report {what}."))
        }
        Standing::Failed(why) => (true, format!("Could not load {what}: {why}")),
        Standing::Stale(why) => (true, format!("Not up to date: {why}")),
    })
}

/// The account's strip, which opens the widget's settings.
fn strip<S: AccountSettings>(
    tab: &TabContext<S>,
    account: Memo<Option<Account>>,
    venue: Memo<String>,
) -> View {
    let of = move |read: fn(&Account) -> String| {
        account.with(|account| account.as_ref().map(read).unwrap_or_default())
    };
    let (logo, initials) = bound_exchange_logo(
        move || account.with(|account| account.as_ref().map(|account| account.venue.clone())),
        LOGO,
    );
    let tab = tab.clone();
    view()
        .h(px(STRIP_HEIGHT))
        .flex_none()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .px(px(10.0))
        .bg(theme::SURFACE)
        .border_b_1()
        .border_color(theme::BORDER)
        .text_size(px(11.0))
        .role(AccessibilityRole::Button)
        .label(move || {
            format!(
                "Change account: {} {}",
                venue.get(),
                of(Account::short_address)
            )
        })
        .test_id(&format!("{}-account", S::TEST_ID))
        .tooltip(ui::hint(move || of(|account| account.address.clone())))
        .hover(|s| s.bg(theme::PANEL))
        .active(|s| s.bg(theme::BORDER))
        .on_press(move || tab.open_settings())
        .child(logo)
        .child(initials)
        .child(
            text(move || venue.get())
                .flex_none()
                .number_of_lines(1)
                .text_color(theme::TEXT),
        )
        .child(
            text(move || of(Account::short_address))
                .min_w(px(0.0))
                .truncate()
                .tabular_nums()
                .text_color(theme::TEXT_MUTED),
        )
}

/// A strip saying how `what` of an account on `venue` stands
/// ([`said`]), in small text with a line under it, as a warning (the
/// warning color, on a tint of it, behind a thick left edge) when it is
/// one; hidden while there is nothing to say.
fn note(venue: Memo<String>, standing: Memo<Standing>, what: &'static str) -> View {
    let notice = computed(move || said(&standing.get(), &venue.get(), what));
    let warns = move || notice.with(|notice| matches!(notice, Some((true, _))));
    view()
        .flex_none()
        .px(px(10.0))
        .py(px(5.0))
        .border_b_1()
        .border_color(theme::BORDER)
        .text_size(px(11.0))
        .text_color(theme::TEXT_MUTED)
        .role(AccessibilityRole::Text)
        .style_when(move || notice.with(Option::is_none), |s| s.hidden())
        .style_when(warns, |s| {
            s.border_l(px(3.0))
                .border_l_color(theme::WARNING)
                .bg(theme::WARNING.opacity(0.1))
                .text_color(theme::WARNING)
        })
        .child(
            text(move || {
                notice.with(|notice| {
                    notice
                        .as_ref()
                        .map_or_else(String::new, |(_, said)| said.clone())
                })
            })
            .line_height(LineHeight::Multiple(1.5)),
        )
}

/// What a widget with no account shows: the rows that set one, one
/// bordered piece in the middle of it.
fn ask<S: AccountSettings>(tab: &TabContext<S>, portfolio: &Portfolio) -> View {
    view()
        .test_id(&format!("{}-no-account", S::TEST_ID))
        .flex_1()
        .min_h(px(0.0))
        .min_w(px(0.0))
        .flex()
        .items_center()
        .justify_center()
        .p(px(16.0))
        .child(
            account_rows(tab, portfolio)
                .max_w(px(FORM_WIDTH))
                .bg(theme::PANEL)
                .border_1()
                .border_color(theme::BORDER),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_frame_says_what_is_wrong_and_nothing_while_all_is_well() {
        let said = |standing: Standing| said(&standing, "Hyperliquid", "positions");
        assert_eq!(said(Standing::Unset), None);
        assert_eq!(said(Standing::Ready), None);
        assert_eq!(
            said(Standing::Loading),
            Some((false, "Loading positions…".to_owned()))
        );
        assert_eq!(
            said(Standing::Failed("offline".to_owned())),
            Some((true, "Could not load positions: offline".to_owned()))
        );
        assert_eq!(
            said(Standing::Stale("offline".to_owned())),
            Some((true, "Not up to date: offline".to_owned()))
        );
        assert_eq!(
            said(Standing::Unavailable(Unavailable::NotReported)),
            Some((true, "Hyperliquid does not report positions.".to_owned()))
        );
        let unknown = said(Standing::Unavailable(Unavailable::UnknownVenue)).unwrap();
        assert!(unknown.0 && unknown.1.contains("not available"));
    }
}
