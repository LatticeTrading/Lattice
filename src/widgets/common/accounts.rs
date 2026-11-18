//! What the widgets that show an account share (the Positions, the Account
//! Summary and the Account Performance), so that it exists once. They read
//! the portfolio ([`crate::data::portfolio`]), never an exchange: which
//! exchanges there are comes from its sources.
//!
//! - [`settings`]: the account a widget watches as its settings save it
//!   ([`Watched`]), and how a widget's settings hold it
//!   ([`AccountSettings`]);
//! - [`report`]: one report of that account kept in step with the settings
//!   ([`follow`], a [`Report`]), and where it stands meanwhile
//!   ([`Standing`]);
//! - [`figures`]: how their figures are written and colored ([`shown`],
//!   [`Tone`]);
//! - [`form`]: the rows an account is set with ([`account_rows`]: its
//!   exchange and its address), in a settings panel and in a widget that
//!   has none yet;
//! - [`frame`]: what every such widget has around its own content
//!   ([`account_frame`]): the account's strip, what could not be read, and
//!   the form while no account is set;
//! - [`badge`]: the badges a widget floats over its plot, as the Chart's
//!   ([`menu_badge`]), and the account's ([`account_badge`]: its
//!   exchange's logo and its address, which opens the rows that change it);
//! - [`panel`]: its settings panel ([`account_panel`]): the account's
//!   rows, then the widget's own.
//!
//! A new account widget holds a [`Watched`] in its settings, follows the
//! reports it shows in its session and lays its content in the frame.

mod badge;
mod figures;
mod form;
mod frame;
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
    panel::account_panel,
    report::{Report, Standing, follow},
    settings::{AccountSettings, Watched},
};
