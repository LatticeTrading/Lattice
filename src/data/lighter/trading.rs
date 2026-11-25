//! Lighter as trading asks for it ([`TradingVenue`]): its perpetuals,
//! traded for the connected address's main account by an API key the
//! account registers.
//!
//! - Enabling ([`TradingVenue::enable`]): the app finds the address's main
//!   account, makes a key ([`super::key`]) for an index of it nothing is
//!   registered at, and has it registered: the key signs the transaction
//!   that says so and the wallet signs a line of text naming the key
//!   ([`register_message`]), which is read back (a contract wallet's is not
//!   the account's, and Lighter would refuse it). The transaction is
//!   posted and the key kept: saved on this device at once, and held once
//!   Lighter lists it, which takes it a moment. Other keys of the account
//!   (Lighter's own site's, a bot's) are left as they are.
//! - Resuming ([`TradingVenue::resume`]): a key kept from an earlier run is
//!   taken up for its address once Lighter lists it at its index; one it no
//!   longer lists there is discarded, and the account enables trading
//!   again. Enabling tries this first, so the wallet is asked only when it
//!   must be.
//! - Disabling forgets the key and remembers its index, which enabling
//!   again registers the next key at: the account does not fill up with
//!   keys nothing holds. Lighter goes on listing the forgotten one until
//!   then; nothing can sign with it.
//! - An order ([`TradingVenue::place`]) names its market by its index and
//!   is written in the market's smallest size and price
//!   ([`super::markets`]). What was typed more finely is refused here, in
//!   words, and never rounded into another order; so is one that opens and
//!   is smaller than the market allows. An order at the market's price is
//!   bounded: it trades no worse than the slippage away from the middle of
//!   the book as it stands, and what that does not fill is cancelled. One
//!   that rests does so for [`ORDER_LIFE`], the longest Lighter keeps one.
//! - Exits ([`Exits`]): a take profit and a stop loss are orders that wait
//!   for a price and then close at the market's, no worse than
//!   [`EXIT_SLIPPAGE`] past it, on the other side of what they close, only
//!   reducing and with no size of their own: they are of the position
//!   whatever it comes to. Placed with an order they are grouped with it:
//!   they wait for it to trade, go if it is cancelled, and each cancels the
//!   other. Placed for a position held ([`TradingVenue::protect`]) they are
//!   a group of their own, or one order. An exit on the wrong side of the
//!   price it is measured from would trigger at once: it is refused here,
//!   in words.
//! - Leverage and margin mode ([`TradingVenue::set_leverage`]): one
//!   transaction sets both for a market, as the fraction of a position's
//!   worth its margin is (20x is 5%, written `500`). Any leverage from 1 to
//!   the market's most is one; a market traded with isolated margin only is
//!   not set to cross. The setting is read from the account's entry for the
//!   market, which it has whether it holds any or not, and from the
//!   market's own start for one it never traded.
//! - Lighter takes a transaction before it runs it: the answer says it was
//!   taken, not what became of it ([`Placed::Accepted`]). What it changed is
//!   asked for a moment later (the account's open orders and its accounts),
//!   and told as it happens to an account followed live
//!   ([`TradingVenue::follow`], [`super::live`]).
//! - The open orders are Lighter's to tell only to the account's own key:
//!   until trading is enabled they read as none.
//!
//! Not done: the address's sub-accounts (their positions are reported with
//! the main account's, and an order for one is the main account's), spot,
//! orders that wait for a price and are no exit, exits that rest at a limit
//! once triggered, adding margin to an isolated position, and cancelling
//! everything at once.

use std::{rc::Rc, time::Duration};

use alloy::{primitives::hex, signers::Signer};
use nsh_core::{prelude::*, time::unix_ms};
use serde::Deserialize;

use super::{
    Lighter, Request,
    accounts::{VENUE, parse_address},
    key::{self, ApiKey},
    markets::{BookTop, Market, Markets, MarketsReply, WHOLE_FRACTION},
    orders::{Active, ActiveOrdersReply},
    requests::API_KEY_NOT_FOUND,
    state::list,
    tx::{Grouping, Order, OrderType, Standing, Transaction, register_message},
};
use crate::data::{
    net::{ApiError, ErrorKind, Queries, QuerySpec, Subject},
    portfolio::{AccountSource, Direction, Leverage, MarginAccount, MarginMode, short_address},
    trading::{
        CancelRequest, Doing, Exits, Following, Hears, Instrument, Instruments, OpenOrders,
        OrderKind, OrderRequest, Placed, Side, TimeInForce, TradingVenue,
    },
    wallet::{WalletSigner, describe_signing},
};

/// How often the markets are asked for again while watched: listings change
/// by the day.
const MARKETS_REFRESH: Duration = Duration::from_secs(10 * 60);
/// How often an account's open orders are asked for again while watched;
/// what the app itself places or cancels is asked for a moment after.
const ORDERS_REFRESH: Duration = Duration::from_secs(30);
/// How often an address's accounts are asked for again while a trading
/// widget watches its margin: what it has to trade with moves with the
/// market, where the portfolio's widgets are content with minutes.
const MARGIN_REFRESH: Duration = Duration::from_secs(15);
/// How often an account's setting for a market is read again while watched:
/// it changes when the account changes it, here (read a moment after) or
/// elsewhere.
const LEVERAGE_REFRESH: Duration = Duration::from_secs(60);
/// How old a position the exits placed for it may be measured against.
const POSITION_FRESH: Duration = Duration::from_secs(2);
/// How far past its trigger an exit may trade once triggered: 10%, what
/// trading's exits are bounded by on every exchange here.
const EXIT_SLIPPAGE: f64 = 0.10;
/// How long Lighter may take a transaction for, in milliseconds: five
/// minutes. Lighter takes none good for more than ten, by its own clock:
/// half of that leaves this machine's clock as long to be off either way.
const TRANSACTION_LIFE: u64 = 5 * 60 * 1000;
/// How long an order rests or waits, in milliseconds: 28 days, what
/// Lighter's own site gives one (it keeps none past 30).
const ORDER_LIFE: u64 = 28 * 24 * 60 * 60 * 1000;
/// How often, and how many times, Lighter is asked whether it lists a key
/// just registered: it takes a transaction before it runs it.
const LISTED_EVERY: Duration = Duration::from_secs(1);
const LISTED_TRIES: u32 = 12;

/// What the user is told instead of sending anything.
fn refused(message: impl AsRef<str>) -> ApiError {
    ApiError::invalid_request(message)
}

/// Now, in whole Unix milliseconds.
fn now_ms() -> u64 {
    // Whole milliseconds since 1970 fit a `u64` for good.
    #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let now = unix_ms().max(0.0) as u64;
    now
}

/// The key of the request for `address`'s open orders.
pub(super) fn orders_key(address: &str) -> String {
    format!("open orders {address}")
}

/// The key of the request for how `address` is set to trade `instrument`.
fn leverage_key(address: &str, instrument: &str) -> String {
    format!("leverage {address} {instrument}")
}

/// `address` as the address it is, written as every request of it is.
fn owner(address: &str) -> Result<String, ApiError> {
    parse_address(address).map_err(|_| refused("This account's address is not a Lighter address."))
}

/// What the connection panel calls a request of trading's.
fn subject(name: &str, detail: &str) -> Subject {
    Subject {
        exchange: VENUE.to_owned(),
        name: name.to_owned(),
        detail: detail.to_owned(),
    }
}

/// The market `symbol` names among `markets`, when orders for it are taken.
fn listed<'a>(markets: &'a Markets, symbol: &str) -> Result<&'a Market, ApiError> {
    let market = markets
        .find(symbol)
        .ok_or_else(|| refused(format!("Lighter lists no {symbol} perpetual.")))?;
    if !market.active {
        return Err(refused(format!(
            "{} is not trading on Lighter now.",
            market.symbol
        )));
    }
    Ok(market)
}

/// The margin fraction that is `leverage` for `market`, or why it cannot be
/// one.
fn margin_fraction(leverage: f64, market: &Market) -> Result<u16, ApiError> {
    let most = market.max_leverage();
    let out_of_range = || {
        refused(match most {
            Some(most) => format!("Leverage of {} is from 1 to {most}.", market.symbol),
            None => format!("Leverage of {} is 1 or more.", market.symbol),
        })
    };
    if !leverage.is_finite() || leverage < 1.0 {
        return Err(out_of_range());
    }
    // Down to a whole fraction, as Lighter's own site writes one: never a
    // hair more leverage than was asked for.
    let fraction = (f64::from(WHOLE_FRACTION) / leverage).floor();
    if fraction < f64::from(market.min_fraction.max(1)) {
        return Err(out_of_range());
    }
    // From the least a market allows to the whole, checked above.
    #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok(fraction as u16)
}

/// The order that is the exit `kind` of a whole position in `market`: it
/// waits for `trigger` and then buys (`buys`) or sells at the market's
/// price, no worse than the slippage past its trigger, until `expires_at`.
fn exit_order(
    kind: OrderType,
    trigger: f64,
    buys: bool,
    market: &Market,
    expires_at: u64,
) -> Result<Order, ApiError> {
    Ok(Order {
        market: market.id,
        client_index: 0,
        // None of its own: the position's, whatever that comes to.
        base_amount: 0,
        price: market.bound_units(trigger, buys, EXIT_SLIPPAGE)?,
        ask: !buys,
        kind,
        standing: Standing::ImmediateOrCancel,
        reduce_only: true,
        trigger_price: market.price_units(trigger)?,
        expires_at,
    })
}

/// The orders `exits` are, each buying (`buys`) or selling the whole of a
/// position in `market`: the take profit, then the stop loss.
fn exit_orders(
    exits: Exits,
    buys: bool,
    market: &Market,
    expires_at: u64,
) -> Result<Vec<Order>, ApiError> {
    [
        (OrderType::TakeProfit, exits.take_profit),
        (OrderType::StopLoss, exits.stop_loss),
    ]
    .into_iter()
    .filter_map(|(kind, trigger)| Some((kind, trigger?)))
    .map(|(kind, trigger)| exit_order(kind, trigger, buys, market, expires_at))
    .collect()
}

/// The transaction that places `order` with `exits` waiting for it: the
/// order alone, or grouped with one exit or with both.
fn placing(order: Order, exits: Vec<Order>) -> Transaction {
    let grouping = match exits.len() {
        0 => return Transaction::CreateOrder(order),
        1 => Grouping::WithExit,
        _ => Grouping::WithExits,
    };
    let mut orders = vec![order];
    orders.extend(exits);
    Transaction::CreateGroupedOrders { grouping, orders }
}

/// The transaction that places `exits` for a position held: the one there
/// is, or both, each cancelling the other.
fn protecting(mut exits: Vec<Order>) -> Option<Transaction> {
    match exits.len() {
        0 => None,
        1 => exits.pop().map(Transaction::CreateOrder),
        _ => Some(Transaction::CreateGroupedOrders {
            grouping: Grouping::Exits,
            orders: exits,
        }),
    }
}

/// The reply to [`Request::NextNonce`].
#[derive(Debug, Deserialize)]
struct NextNonce {
    nonce: u64,
}

/// The reply to [`Request::ApiKeys`].
#[derive(Debug, Default, Deserialize)]
struct ApiKeys {
    #[serde(default, deserialize_with = "list")]
    api_keys: Vec<ListedKey>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ListedKey {
    api_key_index: u8,
    public_key: String,
}

impl Lighter {
    /// The request for what Lighter trades, which is also kept for what
    /// cannot wait for it ([`Directory`](super::markets::Directory)).
    fn markets_query(&self) -> QuerySpec<Markets> {
        let api = self.api.clone();
        QuerySpec::new("markets", move || {
            let api = api.clone();
            async move {
                let reply: MarketsReply = api.get(&Request::Markets).await?;
                let markets = Markets::from(reply);
                api.directory.read(&Rc::new(markets.clone()));
                Ok(markets)
            }
        })
        .refresh_every(MARKETS_REFRESH)
        .subject(subject("Perpetuals", "Markets"))
    }

    /// What Lighter trades, no older than a listing may be.
    pub(super) async fn markets(&self) -> Result<Rc<Markets>, ApiError> {
        self.queries.get(self.markets_query()).await
    }

    /// The API keys Lighter lists for `account`: the index and the public
    /// key of each.
    async fn listed_keys(&self, account: u64) -> Result<Vec<ListedKey>, ApiError> {
        let request = Request::ApiKeys {
            account_index: account,
            api_key_index: None,
        };
        match self.api.get_now::<ApiKeys>(&request).await {
            Ok(listed) => Ok(listed.api_keys),
            Err(error) if error.code == API_KEY_NOT_FOUND => Ok(Vec::new()),
            Err(error) => Err(error),
        }
    }

    /// Whether Lighter lists `key` at its index of its account.
    async fn lists(&self, key: &ApiKey) -> Result<bool, ApiError> {
        let request = Request::ApiKeys {
            account_index: key.account,
            api_key_index: Some(key.index),
        };
        match self.api.get_now::<ApiKeys>(&request).await {
            Ok(listed) => Ok(listed
                .api_keys
                .iter()
                .any(|listed| listed.api_key_index == key.index && key.is(&listed.public_key))),
            Err(error) if error.code == API_KEY_NOT_FOUND => Ok(false),
            Err(error) => Err(error),
        }
    }

    /// Holds `key` as the one that trades for `owner`, and asks for the
    /// orders only it can read.
    fn take_up(&self, owner: &str, key: ApiKey) {
        self.keys.hold(owner, key);
        self.queries.refresh(&orders_key(owner));
    }

    /// Takes up the key kept for `owner` from an earlier run, once Lighter
    /// lists it at its index. `Ok(false)` when none is kept, or the one
    /// kept is not the one listed there any more, which discards it.
    async fn resume_key(&self, owner: &str) -> Result<bool, ApiError> {
        let Some(key) = key::kept(self.network, owner) else {
            return Ok(false);
        };
        if self.lists(&key).await? {
            self.take_up(owner, key);
            Ok(true)
        } else {
            // Another key was registered at its index, or none ever was:
            // this one signs for nothing.
            key::discard(self.network, owner);
            Ok(false)
        }
    }

    /// The index a new key of `owner`'s account `account` is registered
    /// at: the one a key of the app's was at before, while that key or
    /// nothing is listed there; else one nothing is listed at.
    async fn index_for(&self, owner: &str, account: u64) -> Result<u8, ApiError> {
        let listed = self.listed_keys(account).await?;
        if let Some((index, public)) = key::slot(self.network, owner, account) {
            let there = listed.iter().find(|listed| listed.api_key_index == index);
            if there.is_none_or(|there| there.public_key.eq_ignore_ascii_case(&public)) {
                return Ok(index);
            }
        }
        let taken: Vec<u8> = listed.iter().map(|listed| listed.api_key_index).collect();
        key::free_index(&taken).ok_or_else(|| {
            refused(
                "Every API key index of this Lighter account is taken. Remove one on Lighter \
                 and enable trading again.",
            )
        })
    }

    /// Signs `transaction` as `address`'s key and posts it, after the
    /// transactions before it. `Ok` is Lighter having taken it.
    async fn act(&self, address: &str, transaction: Transaction) -> Result<(), ApiError> {
        let not_enabled = || refused("Enable trading on Lighter for this account first.");
        let _turn = self.keys.turn().await;
        let (account, key_index) = self.keys.signer(address).ok_or_else(not_enabled)?;
        let nonce = match self.keys.nonce(address) {
            Some(nonce) => nonce,
            None => {
                let request = Request::NextNonce {
                    account_index: account,
                    api_key_index: key_index,
                };
                self.api.get_now::<NextNonce>(&request).await?.nonce
            }
        };
        let expires_at = now_ms() + TRANSACTION_LIFE;
        let signed = self
            .keys
            .sign(address, self.network, &transaction, nonce, expires_at)
            .ok_or_else(not_enabled)?;
        let sent = self.api.send(&signed).await;
        // What Lighter took is counted; whatever it did not take, or did
        // not answer, has it asked what the next one is.
        self.keys
            .set_nonce(address, sent.is_ok().then_some(nonce + 1));
        sent
    }

    /// Asks again, a moment from now, for what a transaction of `address`'s
    /// changed: its open orders and its accounts, and `also`.
    fn refresh_after(&self, address: &str, also: Option<String>) {
        let mut keys = vec![orders_key(address)];
        keys.extend(also);
        self.refresh_soon(address, &keys);
    }
}

impl TradingVenue for Lighter {
    fn venue(&self) -> &'static str {
        VENUE
    }

    fn name(&self) -> &'static str {
        "Lighter"
    }

    fn enabling_note(&self) -> &'static str {
        "Your wallet signs one message. It registers a key Lattice keeps on this device as an \
         API key of your Lighter account, which places and cancels orders and sets leverage for \
         it until you disable it here. On Lighter such a key can also ask for a withdrawal, \
         which is paid only to this wallet; it cannot send funds anywhere else."
    }

    fn queries(&self) -> &Queries {
        self.queries()
    }

    fn instruments_query(&self) -> QuerySpec<Instruments> {
        let this = self.downgrade();
        QuerySpec::new("instruments", move || {
            let this = this.clone();
            async move { Ok(this.upgrade()?.markets().await?.instruments()) }
        })
        .refresh_every(MARKETS_REFRESH)
        .subject(subject("Perpetuals", "Instruments"))
    }

    fn open_orders_query(&self, address: &str) -> QuerySpec<OpenOrders> {
        let (this, user) = (self.downgrade(), address.to_owned());
        QuerySpec::new(orders_key(address), move || {
            let (this, user) = (this.clone(), user.clone());
            async move {
                let this = this.upgrade()?;
                let token = this.keys.token(&user, now_ms() / 1000);
                let (Some(token), Some(account)) = (token, this.keys.account(&user)) else {
                    // Lighter tells them only to the account's own key.
                    this.kept.orders.borrow_mut().remove(&user);
                    return Ok(OpenOrders::default());
                };
                let request = Request::ActiveOrders {
                    account_index: account,
                };
                let reply: ActiveOrdersReply = this.api.get_as(&request, &token).await?;
                let active = Active::from(reply);
                let listed = active.listed(&this.api.directory);
                // What the stream's changes are applied to from here on.
                this.kept.orders.borrow_mut().insert(user, active);
                Ok(listed)
            }
        })
        .refresh_every(ORDERS_REFRESH)
        .subject(subject(&short_address(address), "Open orders"))
    }

    fn margin_query(&self, address: &str) -> Option<QuerySpec<MarginAccount>> {
        // The portfolio's request, key and all, refetched sooner: a
        // request is refetched as often as its keenest watcher asks.
        <Self as AccountSource>::margin_query(self, address)
            .map(|spec| spec.refresh_every(MARGIN_REFRESH))
    }

    fn leverage_query(
        &self,
        address: &str,
        instrument: &str,
    ) -> Option<QuerySpec<Option<Leverage>>> {
        let (this, user, symbol) = (self.downgrade(), address.to_owned(), instrument.to_owned());
        let query = QuerySpec::new(leverage_key(address, instrument), move || {
            let (this, user, symbol) = (this.clone(), user.clone(), symbol.clone());
            async move {
                let this = this.upgrade()?;
                let markets = this.markets().await?;
                let Some(market) = markets.find(&symbol) else {
                    return Ok(None);
                };
                // Its own setting, or the market's start for an account
                // that never traded it.
                let accounts = this.accounts(&user).await?;
                Ok(accounts
                    .setting(&market.symbol)
                    .or_else(|| market.default_setting()))
            }
        });
        Some(query.refresh_every(LEVERAGE_REFRESH).subject(subject(
            &short_address(address),
            &format!("{instrument} leverage"),
        )))
    }

    fn nearest_price(&self, instrument: &Instrument, price: f64) -> Option<f64> {
        let markets = self.api.directory.markets()?;
        markets.find(&instrument.symbol)?.nearest_price(price)
    }

    fn follow(&self, address: &str, hears: Hears) -> Option<Following> {
        Some(Following::new(self.follow_account(address, hears)))
    }

    fn enable(&self, signer: WalletSigner) -> Doing<()> {
        let this = self.clone();
        Box::pin(async move {
            let owner = format!("{:#x}", signer.address());
            // What an earlier run kept, if it still holds: no signature.
            if this.resume_key(&owner).await? {
                return Ok(());
            }
            let account = this.accounts(&owner).await?.trading().ok_or_else(|| {
                refused(
                    "This wallet has no Lighter account yet. Deposit on Lighter first, then \
                     enable trading here.",
                )
            })?;
            let index = this.index_for(&owner, account).await?;
            let key = ApiKey::random(account, index)
                .map_err(|error| ApiError::new(ErrorKind::Internal, "NO_KEY", error))?;
            let request = Request::NextNonce {
                account_index: account,
                api_key_index: index,
            };
            let nonce = this.api.get_now::<NextNonce>(&request).await?.nonce;
            let envelope = key.envelope(this.network, nonce, now_ms() + TRANSACTION_LIFE);
            let message = register_message(key.public(), &envelope);
            let signature = signer
                .sign_message(message.as_bytes())
                .await
                .map_err(|error| refused(describe_signing(&error)))?;
            let signed_by = signature
                .recover_address_from_msg(message.as_bytes())
                .map_err(|error| {
                    refused(format!("The wallet's signature does not read: {error}"))
                })?;
            if signed_by != signer.address() {
                return Err(refused(format!(
                    "Signed by {signed_by}, not by this account. Lighter would refuse it (a \
                     contract wallet cannot register a key this way)."
                )));
            }
            let register = Transaction::ChangePubKey {
                key: key.public(),
                wallet_signature: hex::encode_prefixed(signature.as_bytes()),
            };
            this.api.send(&key.sign(&register, &envelope)).await?;
            // Kept from here: Lighter has it, and lists it once it has run
            // it. Should that take longer than is waited for, enabling
            // again takes it up with no signature once it is listed.
            key::keep(this.network, &owner, &key);
            for _ in 0..LISTED_TRIES {
                timer(LISTED_EVERY).await;
                if this.lists(&key).await? {
                    this.take_up(&owner, key);
                    return Ok(());
                }
            }
            Err(ApiError::api(
                None,
                "NOT_LISTED",
                "Lighter took the key and does not list it yet. Enable trading again in a moment.",
            ))
        })
    }

    fn kept(&self, address: &str) -> bool {
        owner(address).is_ok_and(|owner| key::kept(self.network, &owner).is_some())
    }

    fn resume(&self, address: &str) -> Doing<bool> {
        let (this, address) = (self.clone(), address.to_owned());
        Box::pin(async move { this.resume_key(&owner(&address)?).await })
    }

    fn release(&self) {
        self.keys.forget();
        // The orders only the key could read are read as none again.
        let read: Vec<String> = self
            .kept
            .orders
            .borrow_mut()
            .drain()
            .map(|(of, _)| of)
            .collect();
        for address in read {
            self.queries.refresh(&orders_key(&address));
        }
    }

    fn disable(&self, address: &str) {
        let Ok(owner) = owner(address) else {
            return;
        };
        // The key held, or the one kept without being held.
        let key = self
            .keys
            .take(&owner)
            .or_else(|| key::kept(self.network, &owner));
        if let Some(key) = key {
            key::retire(self.network, &owner, &key);
        }
        self.keys.forget();
        self.kept.orders.borrow_mut().remove(&owner);
        self.queries.refresh(&orders_key(&owner));
    }

    fn place(&self, address: &str, order: OrderRequest) -> Doing<Placed> {
        let (this, address) = (self.clone(), address.to_owned());
        Box::pin(async move {
            let owner = owner(&address)?;
            let markets = this.markets().await?;
            let market = listed(&markets, &order.instrument)?;
            let base_amount = market.size_units(order.size)?;
            let buys = order.side == Side::Buy;
            let resting = now_ms() + ORDER_LIFE;
            // What it is and at what price, and the price its exits are
            // measured from: its own, or the market's now.
            let (kind, standing, price, expires_at, from) = match order.kind {
                OrderKind::Limit {
                    price,
                    time_in_force,
                } => {
                    let (standing, expires_at) = match time_in_force {
                        TimeInForce::GoodTilCancelled => (Standing::GoodTillTime, resting),
                        TimeInForce::PostOnly => (Standing::PostOnly, resting),
                        TimeInForce::ImmediateOrCancel => (Standing::ImmediateOrCancel, 0),
                    };
                    let units = market.price_units(price)?;
                    (OrderType::Limit, standing, units, expires_at, price)
                }
                OrderKind::Market { slippage } => {
                    let request = Request::BookTop {
                        market_id: market.id,
                    };
                    let top: BookTop = this.api.get_now(&request).await?;
                    let mid = top
                        .mid()
                        .ok_or_else(|| refused(format!("{} has no price now.", market.symbol)))?;
                    let bound = market.bound_units(mid, buys, slippage)?;
                    let standing = Standing::ImmediateOrCancel;
                    (OrderType::Market, standing, bound, 0, mid)
                }
            };
            if !order.exits.is_empty() && order.reduce_only {
                return Err(refused(
                    "An order that only reduces opens nothing to take profit on or stop.",
                ));
            }
            // One on the wrong side of the price would trigger at once.
            if let Some(why) = order.exits.misplaced(buys, from) {
                return Err(refused(why));
            }
            if !order.reduce_only {
                if market.reduce_only {
                    return Err(refused(format!(
                        "{} takes only orders that reduce a position on Lighter now.",
                        market.symbol
                    )));
                }
                if let Some(why) = market.too_small(order.size, from) {
                    return Err(refused(why));
                }
            }
            let placed = Order {
                market: market.id,
                client_index: 0,
                base_amount,
                price,
                ask: !buys,
                kind,
                standing,
                reduce_only: order.reduce_only,
                trigger_price: 0,
                expires_at,
            };
            let exits = exit_orders(order.exits, !buys, market, resting)?;
            let made = this.act(&owner, placing(placed, exits)).await;
            // Whatever it said: one it did not answer may have been taken.
            this.refresh_after(&owner, None);
            made.map(|()| Placed::Accepted)
        })
    }

    fn cancel(&self, address: &str, cancel: CancelRequest) -> Doing<()> {
        let (this, address) = (self.clone(), address.to_owned());
        Box::pin(async move {
            let owner = owner(&address)?;
            let order: u64 = cancel
                .id
                .parse()
                .map_err(|_| refused("This order has no Lighter index."))?;
            let markets = this.markets().await?;
            let market = markets.find(&cancel.instrument).ok_or_else(|| {
                refused(format!(
                    "{} is not a market Lattice trades on Lighter.",
                    cancel.instrument
                ))
            })?;
            let cancelling = Transaction::CancelOrder {
                market: market.id,
                order,
            };
            let made = this.act(&owner, cancelling).await;
            this.refresh_after(&owner, None);
            made
        })
    }

    fn set_leverage(&self, address: &str, instrument: &str, leverage: Leverage) -> Doing<()> {
        let (this, address, instrument) = (self.clone(), address.to_owned(), instrument.to_owned());
        Box::pin(async move {
            let owner = owner(&address)?;
            let markets = this.markets().await?;
            let market = listed(&markets, &instrument)?;
            let margin_fraction = margin_fraction(leverage.value, market)?;
            let isolated = leverage.mode == MarginMode::Isolated;
            if !isolated && market.isolated_only {
                return Err(refused(format!(
                    "{} is traded with isolated margin only.",
                    market.symbol
                )));
            }
            let setting = Transaction::UpdateLeverage {
                market: market.id,
                margin_fraction,
                isolated,
            };
            let made = this.act(&owner, setting).await;
            this.refresh_after(&owner, Some(leverage_key(&owner, &instrument)));
            made
        })
    }

    fn protect(&self, address: &str, instrument: &str, exits: Exits) -> Doing<()> {
        let (this, address, instrument) = (self.clone(), address.to_owned(), instrument.to_owned());
        Box::pin(async move {
            let owner = owner(&address)?;
            if exits.is_empty() {
                return Err(refused("Enter a take profit or a stop loss."));
            }
            let markets = this.markets().await?;
            let market = listed(&markets, &instrument)?;
            let account = AccountSource::margin_query(&this, &owner)
                .ok_or_else(|| refused("Lighter reports no positions."))?;
            let account = this.queries.get(account.fresh_for(POSITION_FRESH)).await?;
            let position = account
                .positions
                .iter()
                .find(|position| position.instrument == market.symbol)
                .ok_or_else(|| {
                    refused(format!(
                        "This account holds no {} position on Lighter.",
                        market.symbol
                    ))
                })?;
            let long = position.direction == Direction::Long;
            // One on the wrong side of the mark would close it at once.
            if let Some(mark) = position.mark_price
                && let Some(why) = exits.misplaced(long, mark)
            {
                return Err(refused(why));
            }
            let orders = exit_orders(exits, !long, market, now_ms() + ORDER_LIFE)?;
            let Some(protecting) = protecting(orders) else {
                return Err(refused("Enter a take profit or a stop loss."));
            };
            let made = this.act(&owner, protecting).await;
            this.refresh_after(&owner, None);
            made
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{super::markets::tests::markets, *};

    #[test]
    fn leverage_is_the_fraction_of_a_positions_worth_its_margin_is() {
        let markets = markets();
        let btc = markets.find("BTC").unwrap();
        // 20x is 5%, in hundredths of a percent; the most BTC gives is 50x.
        assert_eq!(margin_fraction(20.0, btc), Ok(500));
        assert_eq!(margin_fraction(50.0, btc), Ok(200));
        assert_eq!(margin_fraction(1.0, btc), Ok(10_000));
        // Down to a whole fraction: 3x is 33.33%, and 12.7x a little under
        // what was asked.
        assert_eq!(margin_fraction(3.0, btc), Ok(3_333));
        assert_eq!(margin_fraction(12.7, btc), Ok(787));
        for refused in [0.0, 0.5, 50.5, 100.0, -3.0, f64::NAN, f64::INFINITY] {
            assert_eq!(
                margin_fraction(refused, btc).unwrap_err().message,
                "Leverage of BTC is from 1 to 50.",
                "{refused}"
            );
        }
        // A market that gives 3x at the most.
        let asels = markets.find("ASELS").unwrap();
        assert_eq!(margin_fraction(3.0, asels), Ok(3_333));
        assert_eq!(
            margin_fraction(4.0, asels).unwrap_err().message,
            "Leverage of ASELS is from 1 to 3."
        );
    }

    #[test]
    fn an_exit_waits_for_its_trigger_and_trades_within_a_tenth_past_it() {
        let markets = markets();
        let btc = markets.find("BTC").unwrap();
        let exits = Exits {
            take_profit: Some(65_000.0),
            stop_loss: Some(58_000.0),
        };
        // A long's exits sell: the take profit first, then the stop loss,
        // each no worse than 10% under its trigger, only reducing and of
        // the whole position.
        let orders = exit_orders(exits, false, btc, 7).unwrap();
        let read: Vec<(OrderType, u32, u32)> = orders
            .iter()
            .map(|order| {
                assert!(order.ask && order.reduce_only);
                assert_eq!(
                    (order.base_amount, order.market, order.expires_at),
                    (0, 1, 7)
                );
                assert_eq!(order.standing, Standing::ImmediateOrCancel);
                (order.kind, order.trigger_price, order.price)
            })
            .collect();
        assert_eq!(
            read,
            [
                (OrderType::TakeProfit, 650_000, 585_000),
                (OrderType::StopLoss, 580_000, 522_000),
            ]
        );
        // A short's buy back, no worse than 10% over.
        let stop = Exits {
            take_profit: None,
            stop_loss: Some(64_143.0),
        };
        let orders = exit_orders(stop, true, btc, 7).unwrap();
        assert_eq!(orders.len(), 1);
        assert!(!orders[0].ask);
        assert_eq!(
            (orders[0].trigger_price, orders[0].price),
            (641_430, 705_573)
        );
        // None named is none placed, and a trigger no price may have is
        // refused, not rounded.
        assert_eq!(exit_orders(Exits::default(), false, btc, 7), Ok(vec![]));
        let fine = Exits {
            take_profit: Some(65_000.55),
            stop_loss: None,
        };
        assert_eq!(
            exit_orders(fine, false, btc, 7).unwrap_err().message,
            "A price of BTC has 1 decimal place at most."
        );
    }

    #[test]
    fn exits_are_grouped_with_what_they_close() {
        let markets = markets();
        let btc = markets.find("BTC").unwrap();
        let order = Order {
            market: 1,
            client_index: 0,
            base_amount: 150,
            price: 611_257,
            ask: false,
            kind: OrderType::Limit,
            standing: Standing::GoodTillTime,
            reduce_only: false,
            trigger_price: 0,
            expires_at: 7,
        };
        let exits = |take_profit: Option<f64>, stop_loss: Option<f64>| {
            let exits = Exits {
                take_profit,
                stop_loss,
            };
            exit_orders(exits, false, btc, 7).unwrap()
        };
        // An order alone, with one exit that waits for it, and with both,
        // each of which cancels the other.
        assert_eq!(placing(order, vec![]), Transaction::CreateOrder(order));
        let Transaction::CreateGroupedOrders { grouping, orders } =
            placing(order, exits(None, Some(58_000.0)))
        else {
            panic!("a group");
        };
        assert_eq!(grouping, Grouping::WithExit);
        assert_eq!((orders.len(), orders[0]), (2, order));
        let Transaction::CreateGroupedOrders { grouping, orders } =
            placing(order, exits(Some(65_000.0), Some(58_000.0)))
        else {
            panic!("a group");
        };
        assert_eq!(grouping, Grouping::WithExits);
        assert_eq!(orders.len(), 3);

        // For a position held: the one exit as an order of its own, or both
        // as a group of the two.
        let one = exits(Some(65_000.0), None);
        assert_eq!(
            protecting(one.clone()),
            Some(Transaction::CreateOrder(one[0]))
        );
        let both = exits(Some(65_000.0), Some(58_000.0));
        assert_eq!(
            protecting(both.clone()),
            Some(Transaction::CreateGroupedOrders {
                grouping: Grouping::Exits,
                orders: both,
            })
        );
        assert_eq!(protecting(vec![]), None);
    }

    #[test]
    fn a_market_is_traded_while_lighter_takes_orders_for_it() {
        let markets = markets();
        assert_eq!(listed(&markets, "btc").map(|market| market.id), Ok(1));
        assert_eq!(
            listed(&markets, "DOGE").unwrap_err().message,
            "Lighter lists no DOGE perpetual."
        );
        // The retired market of a shared symbol is never the one found; one
        // found retired says so.
        assert_eq!(
            listed(&markets, "ASELS").map(|market| market.id),
            Ok(10_635)
        );
        let retired = Markets::default();
        assert!(listed(&retired, "BTC").is_err());
    }

    #[test]
    fn an_address_is_written_one_way_and_keys_its_requests() {
        assert_eq!(
            owner(" 0xE117D3D94B30F7E7AA4F29704D0D2F43C009B9A8 ").as_deref(),
            Ok("0xe117d3d94b30f7e7aa4f29704d0d2f43c009b9a8")
        );
        assert_eq!(
            owner("2180").unwrap_err().message,
            "This account's address is not a Lighter address."
        );
        assert_eq!(orders_key("0xab"), "open orders 0xab");
        assert_eq!(leverage_key("0xab", "BTC"), "leverage 0xab BTC");
    }
}
