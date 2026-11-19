//! The Account Summary's settings, saved in its tab's record:
//!
//! - `account`: the account it watches (what every widget that shows
//!   accounts saves: [`Watched`]), or `accounts`: the several it sums up
//!   together;
//! - `showBalances`: whether the assets the account holds show under its
//!   figures; on while left out;
//! - `hideSmallBalances`: whether the balances leave out what is worth less
//!   than a dollar; off while left out.
//!
//! Each is left out of the saved form while it holds its default, and read
//! leniently: a switch that is not a boolean holds its default.

use serde::Serialize;
use serde_json::Value;

use crate::widgets::common::accounts::{AccountSettings, Watched};

/// One Account Summary's settings.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub(crate) struct AccountSummarySettings {
    /// The accounts summed up, together.
    #[serde(flatten)]
    watched: Watched,
    /// `None` is on.
    #[serde(rename = "showBalances", skip_serializing_if = "Option::is_none")]
    balances: Option<bool>,
    /// `None` is off.
    #[serde(rename = "hideSmallBalances", skip_serializing_if = "Option::is_none")]
    hide_small: Option<bool>,
}

impl From<Value> for AccountSummarySettings {
    fn from(value: Value) -> Self {
        let Value::Object(members) = value else {
            return Self::default();
        };
        let switch = |name: &str, default: bool| {
            members
                .get(name)
                .and_then(Value::as_bool)
                .unwrap_or(default)
        };
        Self {
            watched: Watched::read(&members),
            balances: None,
            hide_small: None,
        }
        .with_balances(switch("showBalances", true))
        .with_small_hidden(switch("hideSmallBalances", false))
    }
}

impl AccountSummarySettings {
    /// Whether the account's balances show.
    pub fn balances(&self) -> bool {
        self.balances.unwrap_or(true)
    }

    /// These settings with the balances shown or not.
    #[must_use]
    pub fn with_balances(mut self, shown: bool) -> Self {
        self.balances = (!shown).then_some(false);
        self
    }

    /// Whether the balances leave out what is worth less than a dollar.
    pub fn small_hidden(&self) -> bool {
        self.hide_small.unwrap_or(false)
    }

    /// These settings with small balances hidden or not.
    #[must_use]
    pub fn with_small_hidden(mut self, hidden: bool) -> Self {
        self.hide_small = hidden.then_some(true);
        self
    }
}

impl AccountSettings for AccountSummarySettings {
    const TEST_ID: &'static str = "account-summary";

    fn watched(&self) -> &Watched {
        &self.watched
    }

    fn with_watched(mut self, watched: Watched) -> Self {
        self.watched = watched;
        self
    }
}
