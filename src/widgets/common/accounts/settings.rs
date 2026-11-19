//! The accounts a widget watches, as its settings save them:
//!
//! - `account`, an object of the account's `venue` (the exchange's id) and
//!   `address`, while it watches one;
//! - `accounts`, a list of such objects, while it watches several, which
//!   it shows together, on one exchange or on several.
//!
//! Both are left out while the widget watches none, and read leniently: a
//! member that is not such an object reads as no account, and a list keeps
//! the accounts it can read, each once. An account on a venue this build
//! has no source for is kept as saved (a newer build's), and the widget
//! says it cannot be shown.

use serde::{Serialize, Serializer, ser::SerializeMap};
use serde_json::{Map, Value};

use crate::{data::portfolio::Account, widgets::host::KindSettings};

/// The accounts a widget watches, in the order they were added: none, one,
/// or the several it shows together.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Watched {
    accounts: Vec<Account>,
}

impl Serialize for Watched {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut members = serializer.serialize_map(None)?;
        match &self.accounts[..] {
            [] => {}
            [account] => members.serialize_entry("account", account)?,
            accounts => members.serialize_entry("accounts", accounts)?,
        }
        members.end()
    }
}

impl Watched {
    /// The accounts saved among `members`, read leniently: the list, or
    /// without one the single account.
    pub fn read(members: &Map<String, Value>) -> Self {
        let listed = members
            .get("accounts")
            .and_then(Value::as_array)
            .map(|accounts| accounts.iter().filter_map(Account::from_value))
            .map(|accounts| Self::default().with_accounts(accounts))
            .filter(|watched| !watched.accounts.is_empty());
        listed.unwrap_or_else(|| {
            Self::default().with_account(members.get("account").and_then(Account::from_value))
        })
    }

    /// Every account watched, in the order they were added.
    pub fn accounts(&self) -> &[Account] {
        &self.accounts
    }

    /// Watching `account` alone (`None`: none).
    #[must_use]
    pub fn with_account(mut self, account: Option<Account>) -> Self {
        self.accounts = account.into_iter().collect();
        self
    }

    /// Watching `accounts`, each once.
    #[must_use]
    pub fn with_accounts(mut self, accounts: impl IntoIterator<Item = Account>) -> Self {
        self.accounts.clear();
        for account in accounts {
            if !self.accounts.contains(&account) {
                self.accounts.push(account);
            }
        }
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

    /// Every account watched.
    fn accounts(&self) -> &[Account] {
        self.watched().accounts()
    }

    /// These settings watching `accounts`, each once.
    #[must_use]
    fn with_accounts(self, accounts: impl IntoIterator<Item = Account>) -> Self {
        let watched = self.watched().clone().with_accounts(accounts);
        self.with_watched(watched)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn account(venue: &str, address: &str) -> Account {
        Account {
            venue: venue.to_owned(),
            address: address.to_owned(),
        }
    }

    fn read(saved: Value) -> Watched {
        let Value::Object(members) = saved else {
            panic!("settings are an object");
        };
        Watched::read(&members)
    }

    #[test]
    fn one_account_saves_as_itself_and_several_as_a_list() {
        let (first, second) = (
            account("hyperliquid", "0x01"),
            account("lighterxyz", "0x01"),
        );
        let none = Watched::default();
        assert_eq!(serde_json::to_value(&none).unwrap(), json!({}));
        assert!(none.accounts().is_empty());

        let one = none.clone().with_account(Some(first.clone()));
        let saved = serde_json::to_value(&one).unwrap();
        assert_eq!(saved, json!({ "account": first }));
        assert_eq!(read(saved), one);

        // The same address on another exchange is another account; one
        // listed twice is watched once.
        let both = none
            .clone()
            .with_accounts([first.clone(), second.clone(), first.clone()]);
        assert_eq!(both.accounts(), [first.clone(), second.clone()]);
        let saved = serde_json::to_value(&both).unwrap();
        assert_eq!(saved, json!({ "accounts": [first, second] }));
        assert_eq!(read(saved), both);
        // Down to one again, it saves as one; one alone replaces them all.
        assert_eq!(
            both.clone().with_accounts([second.clone()]),
            none.clone().with_account(Some(second.clone()))
        );
        assert_eq!(both.with_account(None), none);
    }

    #[test]
    fn saved_accounts_read_leniently() {
        let kept = account("hyperliquid", "0x01");
        // A list keeps what it can read of itself.
        let damaged = read(json!({
            "accounts": [null, { "venue": "hyperliquid", "address": "0x01" }, { "venue": "" }, 7],
        }));
        assert_eq!(damaged.accounts(), std::slice::from_ref(&kept));
        // One with nothing to read falls back on the single account.
        for saved in [
            json!({ "accounts": [], "account": kept }),
            json!({ "accounts": "0x01", "account": kept }),
            json!({ "accounts": [null], "account": kept }),
        ] {
            assert_eq!(read(saved).accounts(), std::slice::from_ref(&kept));
        }
        assert_eq!(read(json!({ "account": "0x01" })), Watched::default());
        assert_eq!(read(json!({})), Watched::default());
    }
}
