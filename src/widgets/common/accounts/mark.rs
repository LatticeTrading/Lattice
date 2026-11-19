//! Whose a row is ([`Mark`]), in a widget that lists what several accounts
//! hold side by side (their positions, their balances, their trades): the
//! account's exchange's logo at the start of the row, whose tooltip names
//! the account ([`mark_view`]). A widget showing one account marks nothing:
//! its strip says whose everything is.

use std::rc::Rc;

use nsh_core::prelude::*;

use super::frame::venue_name;
use crate::{
    data::{Portfolio, portfolio::Account},
    theme, ui,
    widgets::common::{LogoStyle, exchange_logo},
};

/// How a mark draws its exchange's logo.
const LOGO: LogoStyle = LogoStyle {
    size: 13.0,
    color: theme::TEXT,
    initials_size: 6.5,
    boxed: true,
};

/// The account a row is of.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Mark {
    /// Its exchange's id, which its logo is filed under.
    pub venue: String,
    /// What it is called: its exchange and its address in short
    /// (`Lighter 0x1234…cdef`).
    pub named: String,
    /// What tells its rows from another account's.
    key: String,
}

impl Mark {
    /// The mark of `account`.
    pub fn of(portfolio: &Portfolio, account: &Account) -> Rc<Self> {
        Rc::new(Self {
            venue: account.venue.clone(),
            named: format!(
                "{} {}",
                venue_name(portfolio, &account.venue),
                account.short_address()
            ),
            key: format!("{}\n{}", account.venue, account.address),
        })
    }

    /// The mark of `account` when the widget shows `several` accounts;
    /// none when it shows one.
    pub fn when(several: bool, portfolio: &Portfolio, account: &Account) -> Option<Rc<Self>> {
        several.then(|| Self::of(portfolio, account))
    }

    /// The key of a row that `own` tells apart among its account's: that,
    /// after the account when the row is marked, so two accounts' rows of
    /// one name are two rows.
    pub fn key(mark: Option<&Rc<Self>>, own: &str) -> Rc<str> {
        match mark {
            Some(mark) => format!("{}\n{own}", mark.key).into(),
            None => own.into(),
        }
    }
}

/// `mark`'s logo, which names its account for assistive tech and in its
/// tooltip.
pub(crate) fn mark_view(mark: &Rc<Mark>) -> View {
    let (named, hinted) = (mark.named.clone(), mark.clone());
    exchange_logo(&mark.venue, LOGO)
        .role(AccessibilityRole::Image)
        .label(named)
        .tooltip(ui::hint(move || hinted.named.clone()))
}
