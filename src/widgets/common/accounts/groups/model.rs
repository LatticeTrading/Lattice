//! Address groups as they are kept and saved: an object whose `groups` is a
//! list of groups, each its `name` and its `addresses`, each of those an
//! `address` and the `venues` (exchange ids) it is watched on.
//!
//! Read leniently: a group with no name, or a name an earlier one has, is
//! left out; so is an address that is not text or names no venue; an
//! address listed twice is listed once, watched on the venues of both. A
//! venue this build has no source for is kept as saved (a newer build's).
//!
//! A group's name is its identity: no two groups share one, whatever their
//! case.

use serde::Serialize;
use serde_json::Value;

use crate::data::{Portfolio, portfolio::Account};

/// The longest a group's name is, in characters.
pub(crate) const NAME_LENGTH: usize = 40;

/// One address of a group, and the exchanges it is watched on.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct GroupAddress {
    /// The address, as the first of its venues' sources wrote it.
    pub address: String,
    /// The ids of the venues it is watched on, in the order they were
    /// checked: never none.
    pub venues: Vec<String>,
}

impl GroupAddress {
    /// The address a saved member names; `None` unless it names an address
    /// and a venue.
    fn from_value(value: &Value) -> Option<Self> {
        let address = value.get("address")?.as_str()?.trim();
        let mut venues: Vec<String> = Vec::new();
        for venue in value.get("venues")?.as_array()? {
            let Some(venue) = venue.as_str().map(str::trim) else {
                continue;
            };
            if !venue.is_empty() && !venues.iter().any(|listed| listed == venue) {
                venues.push(venue.to_owned());
            }
        }
        (!address.is_empty() && !venues.is_empty()).then(|| Self {
            address: address.to_owned(),
            venues,
        })
    }

    /// The accounts the address is on its venues, each written as its
    /// venue's source writes it. One on a venue `portfolio` has no source
    /// for is kept as saved, as a widget's settings keep it.
    pub fn accounts(&self, portfolio: &Portfolio) -> Vec<Account> {
        let account = |venue: &String| match portfolio.source(venue) {
            Some(_) => portfolio.account(venue, &self.address).ok(),
            None => Some(Account {
                venue: venue.clone(),
                address: self.address.clone(),
            }),
        };
        self.venues.iter().filter_map(account).collect()
    }
}

/// A named list of addresses.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct AddressGroup {
    pub name: String,
    /// Its addresses, in the order they were added, each once.
    pub addresses: Vec<GroupAddress>,
}

impl AddressGroup {
    /// The group a saved member names; `None` without a name.
    fn from_value(value: &Value) -> Option<Self> {
        let name = clean_name(value.get("name")?.as_str()?)?;
        let mut group = Self {
            name,
            addresses: Vec::new(),
        };
        let saved = value.get("addresses").and_then(Value::as_array);
        for address in saved.into_iter().flatten() {
            if let Some(address) = GroupAddress::from_value(address) {
                group.put(address);
            }
        }
        Some(group)
    }

    /// Every account of the group, each once, in the order of its
    /// addresses: what a widget that uses the group watches.
    pub fn accounts(&self, portfolio: &Portfolio) -> Vec<Account> {
        let mut accounts: Vec<Account> = Vec::new();
        for account in self
            .addresses
            .iter()
            .flat_map(|address| address.accounts(portfolio))
        {
            if !accounts.contains(&account) {
                accounts.push(account);
            }
        }
        accounts
    }

    fn address_mut(&mut self, address: &str) -> Option<&mut GroupAddress> {
        self.addresses
            .iter_mut()
            .find(|listed| listed.address == address)
    }

    /// Lists `address`; one already listed is watched on its venues too.
    fn put(&mut self, address: GroupAddress) {
        match self.address_mut(&address.address) {
            Some(listed) => {
                for venue in address.venues {
                    if !listed.venues.contains(&venue) {
                        listed.venues.push(venue);
                    }
                }
            }
            None => self.addresses.push(address),
        }
    }
}

/// A name as a group has it: without the space around it, [`NAME_LENGTH`]
/// characters at most. `None` when nothing is left.
fn clean_name(name: &str) -> Option<String> {
    let name: String = name.trim().chars().take(NAME_LENGTH).collect();
    let name = name.trim_end();
    (!name.is_empty()).then(|| name.to_owned())
}

/// Whether two names are one group's: the same whatever their case.
fn same_name(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

/// Every address group, in the order they were made.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub(crate) struct Groups {
    groups: Vec<AddressGroup>,
}

impl Groups {
    /// The groups `saved` holds, read leniently.
    pub fn read(saved: &Value) -> Self {
        let mut read = Self::default();
        let groups = saved.get("groups").and_then(Value::as_array);
        for group in groups.into_iter().flatten() {
            if let Some(group) = AddressGroup::from_value(group)
                && read.named(&group.name).is_none()
            {
                read.groups.push(group);
            }
        }
        read
    }

    /// Every group, in the order they were made.
    pub fn all(&self) -> &[AddressGroup] {
        &self.groups
    }

    /// The group called `name`, whatever its case.
    pub fn named(&self, name: &str) -> Option<&AddressGroup> {
        self.groups
            .iter()
            .find(|group| same_name(&group.name, name))
    }

    fn named_mut(&mut self, name: &str) -> Option<&mut AddressGroup> {
        self.groups
            .iter_mut()
            .find(|group| same_name(&group.name, name))
    }

    /// The venues `address` of the group `name` is watched on; none when
    /// either is gone.
    pub fn venues_of(&self, name: &str, address: &str) -> Vec<String> {
        self.named(name)
            .and_then(|group| {
                group
                    .addresses
                    .iter()
                    .find(|listed| listed.address == address)
            })
            .map_or_else(Vec::new, |listed| listed.venues.clone())
    }

    /// Makes an empty group called `name` unless one is, and says what the
    /// group of that name is called; `None` when `name` is no name.
    pub fn add(&mut self, name: &str) -> Option<String> {
        let name = clean_name(name)?;
        if let Some(group) = self.named(&name) {
            return Some(group.name.clone());
        }
        self.groups.push(AddressGroup {
            name: name.clone(),
            addresses: Vec::new(),
        });
        Some(name)
    }

    /// Takes the group `name` out, and says what to show after it: the
    /// group that took its place, else the one before it.
    pub fn remove(&mut self, name: &str) -> Option<String> {
        let at = self
            .groups
            .iter()
            .position(|group| same_name(&group.name, name))?;
        self.groups.remove(at);
        self.groups
            .get(at)
            .or_else(|| self.groups.last())
            .map(|group| group.name.clone())
    }

    /// Calls the group `name` `renamed`, and says what it is called now;
    /// `None` when `renamed` is no name, another group's, or the group is
    /// gone.
    pub fn rename(&mut self, name: &str, renamed: &str) -> Option<String> {
        let renamed = clean_name(renamed)?;
        let taken = self
            .named(&renamed)
            .is_some_and(|other| !same_name(&other.name, name));
        if taken {
            return None;
        }
        let group = self.named_mut(name)?;
        group.name.clone_from(&renamed);
        Some(renamed)
    }

    /// Lists `address` in the group `name`; one already listed there is
    /// watched on its venues too.
    pub fn put(&mut self, name: &str, address: GroupAddress) {
        if address.venues.is_empty() {
            return;
        }
        if let Some(group) = self.named_mut(name) {
            group.put(address);
        }
    }

    /// Takes `address` out of the group `name`.
    pub fn take(&mut self, name: &str, address: &str) {
        if let Some(group) = self.named_mut(name) {
            group.addresses.retain(|listed| listed.address != address);
        }
    }

    /// Watches `address` of the group `name` on `venue` too (`on`), or no
    /// longer; the last venue of an address stays, since one watched
    /// nowhere is no account.
    pub fn watch(&mut self, name: &str, address: &str, venue: &str, on: bool) {
        let Some(listed) = self
            .named_mut(name)
            .and_then(|group| group.address_mut(address))
        else {
            return;
        };
        let at = listed.venues.iter().position(|watched| watched == venue);
        match (on, at) {
            (true, None) => listed.venues.push(venue.to_owned()),
            (false, Some(at)) if listed.venues.len() > 1 => {
                listed.venues.remove(at);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const FIRST: &str = "0x0123456789abcdef0123456789abcdef01234567";
    const SECOND: &str = "0x00000000000000000000000000000000000000aa";

    fn address(address: &str, venues: &[&str]) -> GroupAddress {
        GroupAddress {
            address: address.to_owned(),
            venues: venues.iter().map(|venue| (*venue).to_owned()).collect(),
        }
    }

    #[test]
    fn groups_are_made_named_once_renamed_and_removed() {
        let mut groups = Groups::default();
        assert_eq!(
            serde_json::to_value(&groups).unwrap(),
            json!({ "groups": [] })
        );

        // A name is kept without the space around it; no name, no group.
        assert_eq!(
            groups.add("  High value traders "),
            Some("High value traders".to_owned())
        );
        assert_eq!(groups.add("   "), None);
        // One of the same name, whatever its case, is the group there is.
        assert_eq!(
            groups.add("HIGH VALUE TRADERS"),
            Some("High value traders".to_owned())
        );
        assert_eq!(groups.add("Friends"), Some("Friends".to_owned()));
        assert_eq!(groups.add("Funds"), Some("Funds".to_owned()));
        let names = |groups: &Groups| -> Vec<String> {
            groups
                .all()
                .iter()
                .map(|group| group.name.clone())
                .collect()
        };
        assert_eq!(names(&groups), ["High value traders", "Friends", "Funds"]);
        // A long one is cut.
        assert_eq!(
            Groups::default()
                .add(&"x".repeat(NAME_LENGTH + 9))
                .unwrap()
                .len(),
            NAME_LENGTH
        );

        // Renamed, but never to another's name; its own in another case is
        // its own.
        assert_eq!(groups.rename("friends", "Funds"), None);
        assert_eq!(groups.rename("friends", " "), None);
        assert_eq!(groups.rename("gone", "Others"), None);
        assert_eq!(
            groups.rename("friends", "FRIENDS"),
            Some("FRIENDS".to_owned())
        );
        assert_eq!(groups.rename("FRIENDS", "Desk"), Some("Desk".to_owned()));
        assert_eq!(names(&groups), ["High value traders", "Desk", "Funds"]);

        // Removed, the one that took its place shows, else the one before.
        assert_eq!(groups.remove("desk"), Some("Funds".to_owned()));
        assert_eq!(
            groups.remove("Funds"),
            Some("High value traders".to_owned())
        );
        assert_eq!(groups.remove("Funds"), None);
        assert_eq!(groups.remove("High value traders"), None);
        assert!(groups.all().is_empty());
    }

    #[test]
    fn addresses_are_listed_once_and_watched_on_one_venue_at_least() {
        let mut groups = Groups::default();
        groups.add("Desk");
        groups.put("Desk", address(FIRST, &["lighterxyz"]));
        groups.put("Desk", address(SECOND, &["hyperliquid", "lighterxyz"]));
        // Nothing is watched nowhere, or in a group there is not.
        groups.put("Desk", address("0x01", &[]));
        groups.put("Gone", address(FIRST, &["hyperliquid"]));
        assert_eq!(groups.all().len(), 1);
        assert_eq!(groups.all()[0].addresses.len(), 2);

        // Listed again, it is watched on the venues of both.
        groups.put("desk", address(FIRST, &["hyperliquid", "lighterxyz"]));
        assert_eq!(
            groups.venues_of("Desk", FIRST),
            ["lighterxyz", "hyperliquid"]
        );
        assert_eq!(groups.all()[0].addresses.len(), 2);

        // A venue goes and comes; the last one stays.
        groups.watch("Desk", FIRST, "lighterxyz", false);
        assert_eq!(groups.venues_of("Desk", FIRST), ["hyperliquid"]);
        groups.watch("Desk", FIRST, "hyperliquid", false);
        assert_eq!(groups.venues_of("Desk", FIRST), ["hyperliquid"]);
        groups.watch("Desk", FIRST, "hyperliquid", true);
        groups.watch("Desk", FIRST, "lighterxyz", true);
        assert_eq!(
            groups.venues_of("Desk", FIRST),
            ["hyperliquid", "lighterxyz"]
        );
        assert!(groups.venues_of("Desk", "0x02").is_empty());
        assert!(groups.venues_of("Gone", FIRST).is_empty());

        groups.take("Desk", SECOND);
        assert_eq!(
            groups.all()[0].addresses,
            [address(FIRST, &["hyperliquid", "lighterxyz"])]
        );
    }

    #[test]
    fn a_group_comes_to_an_account_for_each_venue_of_each_address() {
        // No source reports these venues: the accounts are kept as saved,
        // each once, in the order of the addresses.
        let portfolio = Portfolio::new(std::iter::empty());
        let account = |venue: &str, address: &str| Account {
            venue: venue.to_owned(),
            address: address.to_owned(),
        };
        let mut groups = Groups::default();
        groups.add("Desk");
        groups.put("Desk", address(FIRST, &["lighterxyz"]));
        groups.put("Desk", address(SECOND, &["hyperliquid", "lighterxyz"]));
        let group = groups.named("desk").unwrap();
        assert_eq!(
            group.accounts(&portfolio),
            [
                account("lighterxyz", FIRST),
                account("hyperliquid", SECOND),
                account("lighterxyz", SECOND),
            ]
        );
        assert_eq!(
            group.addresses[1].accounts(&portfolio),
            [
                account("hyperliquid", SECOND),
                account("lighterxyz", SECOND)
            ]
        );
    }

    #[test]
    fn groups_save_as_they_read_and_read_leniently() {
        let mut groups = Groups::default();
        groups.add("Desk");
        groups.add("Empty");
        groups.put("Desk", address(FIRST, &["hyperliquid", "lighterxyz"]));
        groups.put("Desk", address(SECOND, &["lighterxyz"]));
        let saved = serde_json::to_value(&groups).unwrap();
        assert_eq!(
            saved,
            json!({ "groups": [
                { "name": "Desk", "addresses": [
                    { "address": FIRST, "venues": ["hyperliquid", "lighterxyz"] },
                    { "address": SECOND, "venues": ["lighterxyz"] },
                ] },
                { "name": "Empty", "addresses": [] },
            ] })
        );
        assert_eq!(Groups::read(&saved), groups);

        // What can be read of a damaged save is kept: a venue of a newer
        // build as saved, an address listed twice once.
        let damaged = Groups::read(&json!({ "groups": [
            null,
            { "name": "  ", "addresses": [] },
            { "name": " Desk ", "addresses": [
                7,
                { "address": FIRST },
                { "address": FIRST, "venues": [] },
                { "address": FIRST, "venues": ["newer", "", 3, "newer"] },
                { "address": FIRST, "venues": ["hyperliquid"] },
                { "address": " ", "venues": ["hyperliquid"] },
            ] },
            { "name": "DESK", "addresses": [{ "address": SECOND, "venues": ["hyperliquid"] }] },
            { "name": "Bare" },
        ] }));
        let mut kept = Groups::default();
        kept.add("Desk");
        kept.put("Desk", address(FIRST, &["newer", "hyperliquid"]));
        kept.add("Bare");
        assert_eq!(damaged, kept);
        for nothing in [
            json!(null),
            json!([]),
            json!({ "groups": "Desk" }),
            json!({}),
        ] {
            assert_eq!(Groups::read(&nothing), Groups::default(), "{nothing}");
        }
    }
}
