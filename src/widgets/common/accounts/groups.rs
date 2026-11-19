//! Address groups: named lists of addresses the user keeps, each address
//! with the exchanges it is watched on, so that a widget is given a whole
//! list at once instead of an address at a time ("High value traders":
//! eight addresses, one watched on Lighter alone, the others on Lighter and
//! Hyperliquid).
//!
//! - [`model`]: the groups as they are kept, changed and saved
//!   ([`Groups`](model::Groups)), and the accounts a group comes to;
//! - [`store`]: the app's one copy of them ([`AddressGroups`]), which the
//!   shell provides and which saves itself as it changes;
//! - [`exchanges`]: the exchanges an address is watched on, as a button of
//!   their logos that opens their checklist;
//! - [`manager`]: the modal that lists, makes, renames and removes groups,
//!   fills them, and hands one to the widget it was opened from
//!   ([`group_manager`], [`Use`]).
//!
//! The groups belong to the app, not to a widget: a widget that uses one
//! takes its accounts as they are then into its own settings, and does not
//! follow the group afterwards.

mod exchanges;
mod manager;
mod model;
mod store;
#[cfg(test)]
mod tests;

pub(crate) use self::{
    manager::{Use, group_manager},
    store::AddressGroups,
};
