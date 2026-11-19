//! What the widgets that show accounts share (the Positions, the Account
//! Summary, the Trade History and the Account Performance), so that it
//! exists once. They read the portfolio ([`crate::data::portfolio`]), never
//! an exchange: which exchanges there are comes from its sources.
//!
//! Each shows any number of accounts together, on one exchange or on
//! several: what they hold listed side by side, what they are worth and
//! made added up.
//!
//! - [`settings`]: the accounts a widget watches as its settings save them
//!   ([`Watched`]), and how a widget's settings hold them
//!   ([`AccountSettings`]);
//! - [`report`]: a report of each of those accounts kept in step with the
//!   settings ([`follow_all`], [`Reports`]), and where they stand as one
//!   meanwhile ([`Standing`]);
//! - [`figures`]: how their figures are written and colored ([`shown`],
//!   [`Tone`]);
//! - [`form`]: the rows the accounts are set with ([`account_rows`]: a line
//!   for each, then the exchange and the address of one to add), in a
//!   settings panel and in a widget that has none yet;
//! - [`frame`]: what every such widget has around its own content
//!   ([`account_frame`]): the accounts' strip, what could not be read, and
//!   the form while no account is set;
//! - [`badge`]: the badges a widget floats over its plot, as the Chart's
//!   ([`menu_badge`]), and the accounts' ([`account_badge`]: their
//!   exchanges' logos and whose they are, which opens the rows that change
//!   them);
//! - [`mark`]: whose a row is, in a widget that lists what several accounts
//!   hold ([`Mark`], [`mark_view`]);
//! - [`panel`]: its settings panel ([`account_panel`]): the accounts'
//!   rows, then the widget's own;
//! - [`groups`]: the lists of addresses the user keeps
//!   ([`AddressGroups`]), and the manager the rows open to fill them and
//!   to give a widget one whole.
//!
//! A new account widget holds a [`Watched`] in its settings, follows the
//! reports it shows in its session ([`follow_all`]), shows them together
//! (added up as [`crate::data::portfolio`]'s `combine` adds accounts up, or
//! listed with a [`Mark`] each) and lays its content in the frame.

mod badge;
mod figures;
mod form;
mod frame;
mod groups;
mod mark;
mod panel;
mod report;
mod settings;
#[cfg(test)]
pub(crate) mod testing;

pub(crate) use self::{
    badge::{account_badge, menu_badge},
    figures::{NONE, Tone, shown},
    form::account_rows,
    frame::{account_frame, bare_account_frame, report_note},
    groups::AddressGroups,
    mark::{Mark, mark_view},
    panel::account_panel,
    report::{Reports, Standing, follow_all},
    settings::{AccountSettings, Watched},
};
