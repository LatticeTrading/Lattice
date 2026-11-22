//! What an exchange's integration implements to be traded on
//! ([`TradingVenue`]), as [`AccountSource`](crate::data::portfolio::AccountSource)
//! is what it implements to report accounts.
//!
//! Reading is shared requests, as everywhere ([`QuerySpec`]s in the
//! venue's own hub: one request however many widgets watch). Acting is a
//! future the venue makes ([`Doing`]): it resolves once the venue has
//! answered, with what the venue said.
//!
//! How an account allows trading differs by exchange, and is the venue's
//! own business behind [`TradingVenue::enable`]: Hyperliquid has the
//! wallet approve a key the app keeps, another has it sign a session or
//! register an API key. What the app holds for it afterwards (the key, the
//! session) stays inside the venue, which also keeps it for the next run
//! (the app's storage, on this device):
//!
//! - [`TradingVenue::resume`] takes it up again when the account connects,
//!   once the exchange says it still holds, without asking the wallet;
//! - [`TradingVenue::release`] lets go of it in memory when the account is
//!   no longer the connected one, and leaves what is kept;
//! - [`TradingVenue::disable`] forgets it for good.
//!
//! An account can be followed live ([`TradingVenue::follow`]): while the
//! hold is kept, the venue's stream of the account brings what the
//! requests above read (its open orders, its margin account, its trades)
//! as it changes, into the same hub, so whatever watches them shows it at
//! once; and says what happened as it happens ([`AccountEvent`]). The
//! requests go on being polled underneath: a stream that stops is caught up
//! with within a poll.
//!
//! What a venue need not have, each said by a `None` and handled by
//! whoever asks: a margin report for trading
//! ([`TradingVenue::margin_query`]), a leverage setting
//! ([`TradingVenue::leverage_query`]), a stream of accounts
//! ([`TradingVenue::follow`]), anything to sign or to keep. What an
//! instrument is priced at is not a venue's to say at all: it is the
//! prices' ([`crate::data::prices`]).
//!
//! What a venue shares with its exchange's other faces (its
//! [`AccountSource`](crate::data::portfolio::AccountSource), its
//! [`PriceSource`](crate::data::prices::PriceSource)), since widgets pass
//! them from one to the other:
//!
//! - its id ([`TradingVenue::venue`]);
//! - an instrument's name: what a position of an account is held in
//!   ([`Position::instrument`](crate::data::portfolio::Position)) is what an
//!   order for it names ([`Instrument::symbol`]) and what its price is
//!   followed by;
//! - an account's address, written as the account source writes it. Every
//!   method here that takes an `address` is given the connected wallet's so
//!   ([`Trading::account`](super::Trading::account)).
//!
//! What a venue needs of an instrument beyond [`Instrument`] (its number
//! on the exchange, its tick) is its own to keep; nothing above it counts
//! on more than a name finds ([`Instruments::find`]).
//!
//! The layers above are run over a venue that has only what one must
//! ([`paper`](super::paper), in tests), which is what shows they know no
//! exchange.

use std::{any::Any, future::Future, pin::Pin, rc::Rc};

use super::model::{
    AccountEvent, CancelRequest, Exits, Instrument, Instruments, OpenOrders, OrderRequest, Placed,
};
use crate::data::{
    net::{ApiError, Queries, QuerySpec},
    portfolio::{Leverage, MarginAccount},
    wallet::WalletSigner,
};

/// Something a venue is doing: the venue's answer, once it has one.
/// Dropping it abandons the wait, not what was already sent.
pub type Doing<T> = Pin<Box<dyn Future<Output = Result<T, ApiError>>>>;

/// What hears a followed account's events. It runs in the stream's own
/// callback: it writes state and returns.
pub type Hears = Rc<dyn Fn(&AccountEvent)>;

/// A hold on an account being followed live ([`TradingVenue::follow`]).
/// Dropping it lets go; the last hold of an account ends its stream.
#[must_use = "dropping it stops following the account"]
pub struct Following(#[allow(dead_code, reason = "held for what dropping it ends")] Box<dyn Any>);

impl Following {
    /// A hold that lasts as long as `held` does.
    pub fn new(held: impl Any) -> Self {
        Self(Box::new(held))
    }
}

/// An exchange the connected account can trade on.
pub trait TradingVenue {
    /// The venue's id: the exchange id its name and logo are filed under,
    /// and its [`AccountSource`](crate::data::portfolio::AccountSource)'s.
    fn venue(&self) -> &'static str;

    /// What people call it.
    fn name(&self) -> &'static str;

    /// What enabling trading asks of the user and what it allows, said
    /// before they are asked.
    fn enabling_note(&self) -> &'static str;

    /// The hub its requests are watched in.
    fn queries(&self) -> &Queries;

    /// What it trades.
    fn instruments_query(&self) -> QuerySpec<Instruments>;

    /// The open orders of the account at `address`.
    fn open_orders_query(&self, address: &str) -> QuerySpec<OpenOrders>;

    /// What the account at `address` has to trade with: its margin
    /// account, in the portfolio's shape and by the portfolio's own
    /// request (every widget watching the account shares it), asked for as
    /// often as trading needs it. `None` for a venue that reports none.
    fn margin_query(&self, address: &str) -> Option<QuerySpec<MarginAccount>>;

    /// How the account at `address` is set to trade `instrument`, whether
    /// it holds any or not: its leverage, and whether what backs a position
    /// is the whole account's margin or the position's own. `Some(None)`
    /// for an instrument the venue has no setting for (it does not list
    /// it); `None` for a venue with no such setting at all.
    fn leverage_query(
        &self,
        address: &str,
        instrument: &str,
    ) -> Option<QuerySpec<Option<Leverage>>>;

    /// The price nearest `price` that a limit order for `instrument` may
    /// have here (its tick, its precision): what a price read off the
    /// market is turned into before it is offered as an order's. `None`
    /// when there is none (nothing above zero).
    fn nearest_price(&self, instrument: &Instrument, price: f64) -> Option<f64>;

    /// Follows the account at `address` live while the hold is kept:
    /// `hears` is told what happens to it, and what the requests above
    /// read of it is brought up to date as it changes. `None` for a venue
    /// with no stream of accounts, whose requests are polled alone, and
    /// for one that streams an account only to what the account allowed,
    /// until trading is enabled for it: whoever follows asks again when
    /// that changes.
    fn follow(&self, address: &str, hears: Hears) -> Option<Following>;

    /// Has `signer`'s account allow the app to trade for it, which its
    /// wallet asks its user about, and keeps what allows it: held for this
    /// run, and saved on this device for the next. What an earlier run
    /// kept is taken up instead when it still holds, and the wallet is not
    /// asked.
    fn enable(&self, signer: WalletSigner) -> Doing<()>;

    /// Whether something is kept for the account at `address` from an
    /// earlier run that [`Self::resume`] could take up.
    fn kept(&self, address: &str) -> bool;

    /// Takes up what the account at `address` allowed in an earlier run,
    /// once the venue says it still holds there; no wallet is asked.
    /// `Ok(false)` when it no longer does, which forgets it; an error when
    /// the venue could not be asked, which keeps it for another try.
    fn resume(&self, address: &str) -> Doing<bool>;

    /// Lets go of what is held in memory (the account is no longer the
    /// connected one, or the app is going away). What is kept for another
    /// run stays.
    fn release(&self);

    /// Forgets what allows trading for the account at `address`, held and
    /// kept. The venue may still list it; nothing here can use it any
    /// more.
    fn disable(&self, address: &str);

    /// Places `order` for the account at `address`, with its exits if it
    /// names any.
    fn place(&self, address: &str, order: OrderRequest) -> Doing<Placed>;

    /// Sets how the account at `address` trades `instrument`
    /// ([`Self::leverage_query`]). What the venue cannot take (more than
    /// the instrument's most, a mode it is not traded in) is refused in
    /// words before anything is sent; what it will not change while a
    /// position is open, it says itself.
    fn set_leverage(&self, address: &str, instrument: &str, leverage: Leverage) -> Doing<()>;

    /// Places `exits` for the whole of the position the account at
    /// `address` holds in `instrument`: they follow the position's size as
    /// it changes, and go when it is closed. Each is an open order of the
    /// account's from then on, cancelled as one. Refused when it holds no
    /// such position.
    fn protect(&self, address: &str, instrument: &str, exits: Exits) -> Doing<()>;

    /// Cancels an open order of the account at `address`.
    fn cancel(&self, address: &str, cancel: CancelRequest) -> Doing<()>;
}
