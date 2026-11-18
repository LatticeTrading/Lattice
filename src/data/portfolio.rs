//! Accounts: what an address holds and has made on an exchange, in one
//! shape whatever the exchange (the app's portfolio widgets' data).
//!
//! Ferris carries public market data; an account's is asked of its exchange
//! directly, from the user's machine, so each exchange's rate limit is the
//! user's own there rather than one backend's for everybody. Only public
//! data is read: an address is all an exchange that publishes its accounts
//! needs, and no key or signature is held or sent.
//!
//! The parts, each of which changes for its own reasons:
//!
//! - [`model`]: what an account is said to hold and have made
//!   ([`MarginAccount`] with its [`Position`]s, [`Balances`],
//!   [`Performance`]), which widgets show and no exchange's wire format
//!   leaks into;
//! - [`account`]: which account ([`Account`]: an address on a venue), as
//!   widgets save it;
//! - [`source`]: what an exchange's integration implements to report its
//!   accounts ([`AccountSource`]): how its addresses read, and one shared
//!   request per report;
//! - [`Portfolio`] (here): the sources by venue, which is all a widget
//!   asks. It never names an exchange.
//!
//! Adding an exchange: its integration (`src/data/<exchange>.rs`, built
//! from the [`net`](super::net) parts with its own
//! [`RateBudget`](super::net::RateBudget)) implements [`AccountSource`]
//! from its wire shapes, and [`DataServices`](super::DataServices) lists it
//! among the portfolio's sources. No widget changes: the venue shows up in
//! their settings, and a report it does not make reads as not reported.
//! Hyperliquid's ([`super::hyperliquid`]) is the first.

mod account;
mod model;
mod source;

use std::rc::Rc;

pub use account::{short_address, Account, AddressError};
pub use model::{
    Balance, Balances, Direction, Fees, Fill, Fills, Leverage, MarginAccount, MarginMode,
    Performance, Period, Point, Position, Prices, Scope, Staking, TradeSide, VaultShare, Vaults,
    Window,
};
pub use source::AccountSource;

use super::net::{QuerySpec, Watch};

/// Why a report of an account cannot be watched.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unavailable {
    /// No source reports the account's venue (a save from a build that
    /// knew more exchanges).
    UnknownVenue,
    /// The venue does not make this report.
    NotReported,
}

/// The app's account sources, by venue. Cloning is cheap and shares them.
#[derive(Clone)]
pub struct Portfolio(Rc<[Rc<dyn AccountSource>]>);

impl Portfolio {
    /// The portfolio of `sources`, in the order widgets offer their venues.
    pub fn new(sources: impl IntoIterator<Item = Rc<dyn AccountSource>>) -> Self {
        Self(sources.into_iter().collect())
    }

    /// Every source, in the order widgets offer their venues.
    pub fn sources(&self) -> &[Rc<dyn AccountSource>] {
        &self.0
    }

    /// The source of `venue`'s accounts.
    pub fn source(&self, venue: &str) -> Option<&Rc<dyn AccountSource>> {
        self.0.iter().find(|source| source.venue() == venue)
    }

    /// The account `input` names on `venue`, its address as the venue's
    /// source writes it; why not, when `input` is no address there.
    pub fn account(&self, venue: &str, input: &str) -> Result<Account, AddressError> {
        let source = self.source(venue).ok_or(AddressError::UNKNOWN_VENUE)?;
        Ok(Account {
            venue: source.venue().to_owned(),
            address: source.parse_address(input)?,
        })
    }

    /// Watches `account`'s margin trading: its standing and its open
    /// positions. Every watcher of one account shares one request.
    pub fn watch_margin(&self, account: &Account) -> Result<Watch<MarginAccount>, Unavailable> {
        self.watch(account, |source, address| source.margin_query(address))
    }

    /// Watches the assets `account` holds.
    pub fn watch_balances(&self, account: &Account) -> Result<Watch<Balances>, Unavailable> {
        self.watch(account, |source, address| source.balances_query(address))
    }

    /// Watches what `account` was worth and made over time.
    pub fn watch_performance(&self, account: &Account) -> Result<Watch<Performance>, Unavailable> {
        self.watch(account, |source, address| source.performance_query(address))
    }

    /// Watches what the assets of `account`'s venue are worth: one request
    /// for the venue, however many accounts watch it.
    pub fn watch_prices(&self, account: &Account) -> Result<Watch<Prices>, Unavailable> {
        self.watch(account, |source, _| source.prices_query())
    }

    /// Watches what `account` has staked.
    pub fn watch_staking(&self, account: &Account) -> Result<Watch<Staking>, Unavailable> {
        self.watch(account, |source, address| source.staking_query(address))
    }

    /// Watches the vaults `account` has a share of.
    pub fn watch_vaults(&self, account: &Account) -> Result<Watch<Vaults>, Unavailable> {
        self.watch(account, |source, address| source.vaults_query(address))
    }

    /// Watches what `account` pays to trade.
    pub fn watch_fees(&self, account: &Account) -> Result<Watch<Fees>, Unavailable> {
        self.watch(account, |source, address| source.fees_query(address))
    }

    /// Watches `account`'s latest trades.
    pub fn watch_fills(&self, account: &Account) -> Result<Watch<Fills>, Unavailable> {
        self.watch(account, |source, address| source.fills_query(address))
    }

    /// Watches the request `spec` makes of `account`'s source, in that
    /// source's hub.
    fn watch<T: 'static>(
        &self,
        account: &Account,
        spec: impl FnOnce(&dyn AccountSource, &str) -> Option<QuerySpec<T>>,
    ) -> Result<Watch<T>, Unavailable> {
        let source = self
            .source(&account.venue)
            .ok_or(Unavailable::UnknownVenue)?;
        let spec = spec(source.as_ref(), &account.address).ok_or(Unavailable::NotReported)?;
        Ok(source.queries().watch(spec))
    }
}
