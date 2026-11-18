//! What an exchange's integration implements to report its accounts.

use super::{
    AddressError, Balances, Fees, Fills, MarginAccount, Performance, Prices, Staking, Vaults,
};
use crate::data::net::{Queries, QuerySpec};

/// One venue's accounts: how its addresses read, and the shared request
/// behind each report.
///
/// A report is a [`QuerySpec`] in the source's own hub, so however many
/// widgets watch one account there is one request for it, refetched on the
/// interval the source picks for it (what its rate limit affords), and the
/// connection panel lists it under the source's heading. The spec's fetch
/// asks the venue and turns its reply into the neutral shape
/// ([`super::model`]); whether that takes one request or several is the
/// source's business. `None` is a report the venue does not make.
pub trait AccountSource {
    /// The venue's id (`hyperliquid`), which accounts are saved under and
    /// its name and logo are filed under.
    fn venue(&self) -> &'static str;

    /// The venue's name for people.
    fn name(&self) -> &'static str;

    /// What an address of the venue looks like, shown in an empty address
    /// field (`0x… address`).
    fn address_hint(&self) -> &'static str;

    /// What was typed as an address of the venue, written the one way
    /// every request and key uses (so two spellings share one request);
    /// why not, when it is none.
    fn parse_address(&self, input: &str) -> Result<String, AddressError>;

    /// The hub its reports are watched in.
    fn queries(&self) -> &Queries;

    /// `address`'s margin trading: its standing and its open positions.
    fn margin_query(&self, address: &str) -> Option<QuerySpec<MarginAccount>>;

    /// The assets `address` holds.
    fn balances_query(&self, address: &str) -> Option<QuerySpec<Balances>>;

    /// What `address` was worth and made over time.
    fn performance_query(&self, address: &str) -> Option<QuerySpec<Performance>>;

    /// What the venue's assets are worth now: one request for the venue,
    /// not per address.
    fn prices_query(&self) -> Option<QuerySpec<Prices>> {
        None
    }

    /// What `address` has staked.
    fn staking_query(&self, _address: &str) -> Option<QuerySpec<Staking>> {
        None
    }

    /// The vaults `address` has a share of.
    fn vaults_query(&self, _address: &str) -> Option<QuerySpec<Vaults>> {
        None
    }

    /// What `address` pays to trade.
    fn fees_query(&self, _address: &str) -> Option<QuerySpec<Fees>> {
        None
    }

    /// `address`'s latest trades.
    fn fills_query(&self, _address: &str) -> Option<QuerySpec<Fills>> {
        None
    }
}
