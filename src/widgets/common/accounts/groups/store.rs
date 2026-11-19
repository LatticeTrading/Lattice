//! The app's address groups ([`AddressGroups`]): one copy for every widget,
//! which the shell restores from storage and provides, and which saves
//! itself whenever it changes.

use nsh_core::prelude::*;

use super::model::Groups;
use crate::storage;

/// The address groups, which their manager changes and every account
/// widget's rows read.
#[derive(Clone, Copy)]
pub(crate) struct AddressGroups(Signal<Groups>);

impl AddressGroups {
    /// The saved groups, provided to everything under the current owner
    /// (the shell).
    pub(crate) fn provide() -> Self {
        let groups = Self::restore();
        provide_context(groups);
        groups
    }

    /// The groups provided above; where none are (a widget on its own),
    /// the saved ones.
    pub(crate) fn get() -> Self {
        use_context::<Self>().unwrap_or_else(Self::restore)
    }

    fn restore() -> Self {
        let saved = storage::load_address_groups();
        Self(signal(saved.as_ref().map(Groups::read).unwrap_or_default()))
    }

    /// Reads the groups (tracked).
    pub(crate) fn with<R>(self, read: impl FnOnce(&Groups) -> R) -> R {
        self.0.with(read)
    }

    /// Reads the groups without tracking them.
    pub(crate) fn with_untracked<R>(self, read: impl FnOnce(&Groups) -> R) -> R {
        self.0.with_untracked(read)
    }

    /// Changes the groups and saves them; nothing happens when `change`
    /// changes nothing.
    pub(crate) fn update(self, change: impl FnOnce(&mut Groups)) {
        let mut groups = self.0.get_untracked();
        change(&mut groups);
        if self.0.with_untracked(|current| *current == groups) {
            return;
        }
        if let Err(error) = storage::save_address_groups(&groups) {
            log::warn!("Lattice: unable to save address groups: {error}");
        }
        self.0.set(groups);
    }
}
