//! Trading: placing and cancelling orders for the connected wallet's
//! account, in one shape whatever the exchange (the trading widgets' data,
//! as [`portfolio`](super::portfolio) is the account widgets').
//!
//! The parts, each of which changes for its own reasons:
//!
//! - [`model`]: what an order, an open order and an instrument are, which
//!   widgets show and no exchange's wire format leaks into;
//! - [`venue`]: what an exchange's integration implements to be traded on
//!   ([`TradingVenue`]): what it trades, an account's open orders, how an
//!   account allows trading there, and placing and cancelling, with what it
//!   may leave out;
//! - [`Trading`] (here): the venues by id, and where the connected account
//!   stands with each ([`Access`]). It is all a widget asks; it never
//!   names an exchange.
//!
//! Whose orders: the account of the connected wallet
//! ([`Wallet`](super::Wallet)), and no other. Each venue is enabled for it
//! on its own ([`Trading::enable`]), which is where the wallet is asked to
//! sign. What a venue holds afterwards (Hyperliquid: an agent's key;
//! Lighter: an API key of its own kind) it
//! also keeps on this device, so trading stays enabled between runs: when
//! an account connects, each venue that kept something for it takes it up
//! again once the exchange says it still holds ([`Access::Resuming`]), and
//! the wallet is not asked. Another account in the wallet, or none, locks
//! every venue and has each let go of what it held in memory: nothing
//! allowed by one account ever trades for another. [`Trading::disable`]
//! forgets it for good.
//!
//! An account is followed live with [`Trading::follow`]: while the hold is
//! kept, what the watches here read of it is brought as it changes by the
//! venue's stream, and what happens to it is said as it happens.
//!
//! Adding an exchange: its integration (`src/data/<exchange>.rs`)
//! implements [`TradingVenue`] from its wire shapes, as it implements
//! [`AccountSource`](super::portfolio::AccountSource) to report accounts,
//! and [`DataServices`](super::DataServices) lists it among trading's
//! venues. No widget changes: the venue shows up in their Exchange select.
//! Hyperliquid's ([`super::hyperliquid`]) is the first and Lighter's
//! ([`super::lighter`]) the second. [`paper`] (tests only) is another
//! that is no exchange, with only what a venue must have: the widgets that trade are run over it too, so that nothing above
//! a venue comes to count on what one exchange's integration does.

mod model;
#[cfg(test)]
pub(crate) mod paper;
mod venue;

use std::{cell::RefCell, rc::Rc};

use nsh_core::prelude::*;

pub use model::{
    Access, AccountEvent, CancelRequest, Exits, Instrument, Instruments, MARKET_SLIPPAGE,
    OpenOrder, OpenOrders, OrderKind, OrderRequest, Placed, Side, TimeInForce,
};
pub use venue::{Doing, Following, Hears, TradingVenue};

use super::{
    Wallet,
    net::{ApiError, QuerySpec, TaskHandle, Watch},
    portfolio::{Account, Leverage, MarginAccount, Unavailable},
};

/// The app's trading venues, and where the connected account stands with
/// each. Cloning is cheap and shares them.
#[derive(Clone)]
pub struct Trading(Rc<Inner>);

struct Inner {
    wallet: Wallet,
    /// In the order widgets offer them.
    desks: Vec<Desk>,
}

/// One venue, as the connected account stands with it.
struct Desk {
    venue: Rc<dyn TradingVenue>,
    access: Signal<Access>,
    /// The enabling, while the wallet's user is asked, or the resuming,
    /// while the venue is.
    enabling: RefCell<Option<TaskHandle>>,
}

/// An answer that is why nothing was sent.
fn refuse<T: 'static>(why: &'static str) -> Doing<T> {
    Box::pin(async move { Err(ApiError::invalid_request(why)) })
}

impl Trading {
    /// Trading on `venues` for `wallet`'s account, under the current owner
    /// (the shell). Nothing is asked of a venue until something watches it.
    pub(crate) fn new(
        wallet: Wallet,
        venues: impl IntoIterator<Item = Rc<dyn TradingVenue>>,
    ) -> Self {
        let desks = venues
            .into_iter()
            .map(|venue| Desk {
                venue,
                access: signal(Access::default()),
                enabling: RefCell::new(None),
            })
            .collect();
        let trading = Self(Rc::new(Inner { wallet, desks }));
        // Whose account it is: what one allowed is not another's.
        let owner = {
            let wallet = trading.0.wallet.clone();
            computed(move || wallet.account().map(|account| account.address))
        };
        let this = trading.clone();
        effect(move || {
            let owner = owner.get();
            untracked(|| {
                this.lock();
                if owner.is_some() {
                    this.resume();
                }
            });
        });
        trading
    }

    /// Every venue, in the order widgets offer them.
    pub fn venues(&self) -> impl Iterator<Item = &Rc<dyn TradingVenue>> {
        self.0.desks.iter().map(|desk| &desk.venue)
    }

    /// The venue whose id is `venue`.
    pub fn venue(&self, venue: &str) -> Option<&Rc<dyn TradingVenue>> {
        self.desk(venue).map(|desk| &desk.venue)
    }

    /// The connected wallet's account on `venue`, as the portfolio names
    /// one (tracked); `None` while no wallet is connected. Its address is
    /// the wallet's as the exchanges that settle on Ethereum write one
    /// (`0x` and forty hexadecimal digits, in lower case:
    /// [`evm_address`](super::portfolio::evm_address)), which is what tells
    /// a watched account to be the wallet's own.
    pub fn account(&self, venue: &str) -> Option<Account> {
        let connected = self.0.wallet.account()?;
        Some(Account {
            venue: venue.to_owned(),
            address: connected.address_text(),
        })
    }

    /// Where the connected account stands with `venue` (tracked); locked
    /// for a venue there is none of.
    pub fn access(&self, venue: &str) -> Access {
        self.desk(venue)
            .map_or_else(Access::default, |desk| desk.access.get())
    }

    /// Has the connected account allow trading on `venue`: its wallet asks
    /// its user, unless what an earlier run kept still holds. Does nothing
    /// while that is already being asked.
    pub fn enable(&self, venue: &str) {
        let Some(desk) = self.desk(venue) else {
            return;
        };
        if matches!(
            desk.access.get_untracked(),
            Access::Enabling | Access::Resuming
        ) {
            return;
        }
        let Some(signer) = self.0.wallet.signer() else {
            desk.access
                .set(Access::Failed("Connect a wallet first.".to_owned()));
            return;
        };
        desk.access.set(Access::Enabling);
        let (enabling, access) = (desk.venue.enable(signer), desk.access);
        let task = TaskHandle::spawn(async move {
            let made = match enabling.await {
                Ok(()) => Access::Ready,
                Err(error) => Access::Failed(error.message),
            };
            access.try_set(made);
        });
        *desk.enabling.borrow_mut() = Some(task);
    }

    /// Stops trading on `venue` for the connected account: what allowed
    /// it is forgotten, here and on this device. While the account is
    /// still being asked to allow it, only the asking stops: what an
    /// earlier run kept is not what was being cancelled.
    pub fn disable(&self, venue: &str) {
        let Some(desk) = self.desk(venue) else {
            return;
        };
        let asking = desk.access.get_untracked() == Access::Enabling;
        desk.close();
        if !asking && let Some(account) = self.0.wallet.connected() {
            desk.venue.disable(&account.address_text());
        }
    }

    /// Follows `account` live on its venue while the hold is kept: `hears`
    /// is told what happens to it, and what the watches below read of it is
    /// brought up to date as it changes.
    pub fn follow(
        &self,
        account: &Account,
        hears: impl Fn(&AccountEvent) + 'static,
    ) -> Result<Following, Unavailable> {
        self.venue(&account.venue)
            .ok_or(Unavailable::UnknownVenue)?
            .follow(&account.address, Rc::new(hears))
            .ok_or(Unavailable::NotReported)
    }

    /// Watches what `venue` trades.
    pub fn watch_instruments(&self, venue: &str) -> Result<Watch<Instruments>, Unavailable> {
        self.watch(venue, |venue| venue.instruments_query())
    }

    /// Watches `account`'s open orders on its venue. Every watcher of one
    /// account shares one request.
    pub fn watch_open_orders(&self, account: &Account) -> Result<Watch<OpenOrders>, Unavailable> {
        self.watch(&account.venue, |venue| {
            venue.open_orders_query(&account.address)
        })
    }

    /// Watches what `account` has to trade with on its venue: its margin
    /// account, as often as trading needs it. Every watcher of one
    /// account shares one request, the portfolio's widgets too.
    pub fn watch_margin(&self, account: &Account) -> Result<Watch<MarginAccount>, Unavailable> {
        let venue = self
            .venue(&account.venue)
            .ok_or(Unavailable::UnknownVenue)?;
        let spec = venue
            .margin_query(&account.address)
            .ok_or(Unavailable::NotReported)?;
        Ok(venue.queries().watch(spec))
    }

    /// Watches how `account` is set to trade `instrument` on its venue: its
    /// leverage and its margin mode there (`None`: the venue has none for
    /// that instrument).
    pub fn watch_leverage(
        &self,
        account: &Account,
        instrument: &str,
    ) -> Result<Watch<Option<Leverage>>, Unavailable> {
        let venue = self
            .venue(&account.venue)
            .ok_or(Unavailable::UnknownVenue)?;
        let spec = venue
            .leverage_query(&account.address, instrument)
            .ok_or(Unavailable::NotReported)?;
        Ok(venue.queries().watch(spec))
    }

    /// The price nearest `price` that a limit order for `instrument` may
    /// have on `venue`.
    pub fn nearest_price(&self, venue: &str, instrument: &Instrument, price: f64) -> Option<f64> {
        self.venue(venue)?.nearest_price(instrument, price)
    }

    /// Places `order` on `venue` for the connected account.
    pub fn place(&self, venue: &str, order: OrderRequest) -> Doing<Placed> {
        match self.acting(venue) {
            Ok((venue, address)) => venue.place(&address, order),
            Err(why) => refuse(why),
        }
    }

    /// Cancels an open order of the connected account on `venue`.
    pub fn cancel(&self, venue: &str, cancel: CancelRequest) -> Doing<()> {
        match self.acting(venue) {
            Ok((venue, address)) => venue.cancel(&address, cancel),
            Err(why) => refuse(why),
        }
    }

    /// Sets how the connected account trades `instrument` on `venue`: its
    /// leverage and its margin mode.
    pub fn set_leverage(&self, venue: &str, instrument: &str, leverage: Leverage) -> Doing<()> {
        match self.acting(venue) {
            Ok((venue, address)) => venue.set_leverage(&address, instrument, leverage),
            Err(why) => refuse(why),
        }
    }

    /// Places `exits` for the whole of the position the connected account
    /// holds in `instrument` on `venue`.
    pub fn protect(&self, venue: &str, instrument: &str, exits: Exits) -> Doing<()> {
        match self.acting(venue) {
            Ok((venue, address)) => venue.protect(&address, instrument, exits),
            Err(why) => refuse(why),
        }
    }

    /// Drops everything in flight and held (the app is going away). What
    /// the venues keep for the next run stays.
    pub(crate) fn shutdown(&self) {
        for desk in &self.0.desks {
            desk.enabling.take();
            desk.venue.release();
        }
    }

    fn desk(&self, venue: &str) -> Option<&Desk> {
        self.0.desks.iter().find(|desk| desk.venue.venue() == venue)
    }

    /// Every venue locked again, each letting go of what it held.
    fn lock(&self) {
        for desk in &self.0.desks {
            desk.close();
        }
    }

    /// Has each venue that kept something for the connected account take
    /// it up again, which asks the venue and no wallet.
    fn resume(&self) {
        let Some(account) = self.0.wallet.connected() else {
            return;
        };
        let address = account.address_text();
        for desk in &self.0.desks {
            if !desk.venue.kept(&address) {
                continue;
            }
            desk.access.set(Access::Resuming);
            let (resuming, access) = (desk.venue.resume(&address), desk.access);
            let name = desk.venue.name();
            let task = TaskHandle::spawn(async move {
                let stands = match resuming.await {
                    Ok(true) => Access::Ready,
                    // It no longer holds there: enabled again, or not.
                    Ok(false) => Access::Locked,
                    Err(error) => Access::Failed(format!(
                        "Could not ask {name} whether trading is still enabled: {}",
                        error.message
                    )),
                };
                access.try_set(stands);
            });
            *desk.enabling.borrow_mut() = Some(task);
        }
    }

    /// `venue` and the address of the account that acts on it; why there
    /// is none otherwise.
    fn acting(&self, venue: &str) -> Result<(&Rc<dyn TradingVenue>, String), &'static str> {
        let venue = self
            .venue(venue)
            .ok_or("This exchange is not available for trading in this version of Lattice.")?;
        let account = self
            .0
            .wallet
            .connected()
            .ok_or("Connect a wallet to trade.")?;
        Ok((venue, account.address_text()))
    }

    /// Watches the request `spec` makes of `venue`, in that venue's hub.
    fn watch<T: 'static>(
        &self,
        venue: &str,
        spec: impl FnOnce(&dyn TradingVenue) -> QuerySpec<T>,
    ) -> Result<Watch<T>, Unavailable> {
        let venue = self.venue(venue).ok_or(Unavailable::UnknownVenue)?;
        Ok(venue.queries().watch(spec(venue.as_ref())))
    }
}

impl Desk {
    /// Locked, with nothing held in memory and nothing being asked.
    fn close(&self) {
        self.enabling.take();
        self.venue.release();
        self.access.try_set(Access::Locked);
    }
}
