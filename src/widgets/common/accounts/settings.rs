//! The account a widget watches, as its settings save it: `account`, an
//! object of the account's `venue` (the exchange's id) and `address`.
//!
//! It is left out while the widget watches none, and read leniently: a
//! member that is not such an object reads as no account. An account on a
//! venue this build has no source for is kept as saved (a newer build's),
//! and the widget says it cannot be shown.

use serde::Serialize;
use serde_json::{Map, Value};

use crate::{data::portfolio::Account, widgets::host::KindSettings};

/// The account a widget watches, if it watches one.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub(crate) struct Watched {
    #[serde(skip_serializing_if = "Option::is_none")]
    account: Option<Account>,
}

impl Watched {
    /// The account saved among `members`, read leniently.
    pub fn read(members: &Map<String, Value>) -> Self {
        Self {
            account: members.get("account").and_then(Account::from_value),
        }
    }

    pub fn account(&self) -> Option<&Account> {
        self.account.as_ref()
    }

    /// Watching `account` (`None`: none).
    #[must_use]
    pub fn with_account(mut self, account: Option<Account>) -> Self {
        self.account = account;
        self
    }
}

/// The settings of a widget that shows an account: where its [`Watched`]
/// is, which is what the shared form and frame read and save. A widget's
/// settings hold one with `#[serde(flatten)]` beside their own members.
pub(crate) trait AccountSettings: KindSettings {
    /// What the test ids of its account controls start with (`positions`),
    /// and its view's test id.
    const TEST_ID: &'static str;

    fn watched(&self) -> &Watched;

    /// These settings watching `watched`.
    #[must_use]
    fn with_watched(self, watched: Watched) -> Self;

    /// The account watched, if any.
    fn account(&self) -> Option<&Account> {
        self.watched().account()
    }

    /// These settings watching `account` (`None`: none).
    #[must_use]
    fn with_account(self, account: Option<Account>) -> Self {
        let watched = self.watched().clone().with_account(account);
        self.with_watched(watched)
    }
}
