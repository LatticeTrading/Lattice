//! Hyperliquid as trading asks for it ([`TradingVenue`]): the perpetuals of
//! its own dex, traded for the connected account by an agent the account
//! approves.
//!
//! - Enabling ([`TradingVenue::enable`]): the app makes a key
//!   ([`super::agent`]), the wallet signs the approval of it
//!   ([`signing::approve_agent`]), the signature is read back (a contract
//!   wallet's is not the account's, and Hyperliquid would take it for
//!   another address), the approval is posted, and the key is kept: held
//!   for this run and saved on this device for the next. The agent is
//!   named [`AGENT_NAME`], so enabling again (here or on another device)
//!   retires the agent before on Hyperliquid's side.
//! - Resuming ([`TradingVenue::resume`]): a key kept from an earlier run
//!   is taken up for its account once Hyperliquid lists it among the
//!   account's agents as still valid ([`Info::ExtraAgents`]); one it no
//!   longer lists is discarded, and the account enables trading again.
//!   Enabling tries this first, so the wallet is asked only when it must
//!   be.
//! - An order ([`TradingVenue::place`]) names its asset by its place in
//!   the universe ([`super::meta`]) and is written to Hyperliquid's
//!   precision: a size has the asset's decimal places at most, and a price
//!   five significant figures and [`PRICE_DECIMALS`] less the size's
//!   places (a whole price may have more figures). What was typed more
//!   finely is refused here, in words, and never rounded into another
//!   order. Hyperliquid has no market order: one is a limit order that
//!   cancels what does not fill at once, priced the slippage away from the
//!   mid price, as its own SDK and site do.
//! - Exits ([`Exits`]): a take profit and a stop loss are market trigger
//!   orders that only reduce, on the other side of what they close. Placed
//!   with an order ([`TradingVenue::place`]) they are of its size and
//!   grouped with it (`normalTpsl`): they wait for it to trade and go if it
//!   is cancelled. Placed for a position held ([`TradingVenue::protect`])
//!   they are sent with no size and take the position's as it changes
//!   (`positionTpsl`). Once triggered, each trades no worse than
//!   [`EXIT_SLIPPAGE`] past its trigger, which is what Hyperliquid's own
//!   site sends. An exit on the wrong side of the price it is measured
//!   from (the order's, or the position's mark) would trigger at once: it
//!   is refused here, in words.
//! - Leverage and margin mode ([`TradingVenue::set_leverage`]): one
//!   action sets both for an asset (`updateLeverage`). Leverage is a whole
//!   number up to the asset's most; an asset traded with isolated margin
//!   only is not set to cross. What Hyperliquid will not change while a
//!   position is open, it says itself. The setting is read with
//!   [`Info::ActiveAssetData`], which answers for an account that holds
//!   none of the asset too.
//! - What an action changed is asked for at once: the account's open
//!   orders and its perpetuals account, so every widget watching them
//!   shows it.
//! - An account is followed live ([`TradingVenue::follow`], [`super::live`]):
//!   what the market does to it (an order filled, one cancelled elsewhere)
//!   arrives over the WebSocket as it happens, and the polls below are what
//!   catches up after a gap.
//!
//! Not done: spot and builder-deployed dexes, trigger orders that are no
//! exit (a stop that opens a position) and limit ones (an exit that rests
//! at a price once triggered), adding margin to an isolated position,
//! vaults and sub-accounts (`vaultAddress`), batches, and an expiry of the
//! app's own for the agent (it lasts until it is disabled here or retired
//! on Hyperliquid).

use std::time::Duration;

use alloy::{primitives::Address, signers::Signer};
use nsh_core::time::unix_ms;
use serde_json::Value;

use super::{
    Hyperliquid, Info,
    accounts::VENUE,
    agent::{self, AgentKey, ExtraAgents},
    exchange::{self, Exit, Grouping, WireKind, WireOrder, decimal},
    leverage::ActiveAssetData,
    meta::{AllMids, Meta, Mids},
    orders::FrontendOpenOrders,
    pack::Packed,
    signing,
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

/// What Hyperliquid lists the app's agent under. An account has one agent
/// of a name: approving another retires the one before.
const AGENT_NAME: &str = "Lattice";
/// How often the universe is asked for again while watched: listings
/// change by the day.
const INSTRUMENTS_REFRESH: Duration = Duration::from_secs(10 * 60);
/// How often the mid prices would be asked for again were they watched (2
/// of weight each). Nothing watches them: a market order asks for them.
const MIDS_KEPT: Duration = Duration::from_secs(5);
/// How old a mid price a market order is priced from may be.
const MID_FRESH: Duration = Duration::from_secs(2);
/// How often an account's open orders are asked for again while watched
/// (20 of weight each); what the app itself places or cancels shows at
/// once.
const ORDERS_REFRESH: Duration = Duration::from_secs(20);
/// How often an account's perpetuals account is asked for again while a
/// trading widget watches it (2 of weight each): what it has to trade
/// with moves with the market, where the portfolio's widgets are content
/// with minutes.
const MARGIN_REFRESH: Duration = Duration::from_secs(10);
/// How often an account's setting for an asset is asked for again while
/// watched (20 of weight each): it changes when the account changes it,
/// here (shown at once) or elsewhere.
const LEVERAGE_REFRESH: Duration = Duration::from_secs(60);
/// How old a position the exits placed for it may be measured against.
const POSITION_FRESH: Duration = Duration::from_secs(2);
/// How far past its trigger an exit may trade once triggered: 10%, what
/// Hyperliquid's own site sends with a market take profit or stop loss.
const EXIT_SLIPPAGE: f64 = 0.10;
/// The decimal places a perpetual's price has at most, before its size's
/// are taken off.
const PRICE_DECIMALS: u32 = 6;
/// The significant figures a price has at most, unless it is whole.
const PRICE_FIGURES: usize = 5;

/// What the user is told instead of sending anything.
fn refused(message: impl AsRef<str>) -> ApiError {
    ApiError::invalid_request(message)
}

/// How many decimal places `written` (decimal text) has.
fn places(written: &str) -> usize {
    written
        .split_once('.')
        .map_or(0, |(_, places)| places.len())
}

/// `size` as an order for `instrument` writes it, or why it cannot be one.
fn size_text(size: f64, instrument: &Instrument) -> Result<String, ApiError> {
    let decimals = instrument.size_decimals;
    let too_fine = || {
        refused(match decimals {
            0 => format!("{} trades in whole units.", instrument.symbol),
            1 => format!(
                "A size of {} has 1 decimal place at most.",
                instrument.symbol
            ),
            _ => format!(
                "A size of {} has {decimals} decimal places at most.",
                instrument.symbol
            ),
        })
    };
    if !size.is_finite() || size <= 0.0 {
        return Err(refused("Enter a size above zero."));
    }
    let written = decimal(size).ok_or_else(too_fine)?;
    if places(&written) > decimals as usize {
        return Err(too_fine());
    }
    Ok(written)
}

/// The decimal places a price of `instrument` has at most.
fn price_places(instrument: &Instrument) -> u32 {
    PRICE_DECIMALS.saturating_sub(instrument.size_decimals)
}

/// `price` as a limit order for `instrument` writes it, or why it cannot
/// be one.
fn limit_price_text(price: f64, instrument: &Instrument) -> Result<String, ApiError> {
    let allowed = price_places(instrument);
    let too_fine = || {
        refused(format!(
            "A price of {} has {PRICE_FIGURES} significant figures and {allowed} decimal \
             {} at most.",
            instrument.symbol,
            if allowed == 1 { "place" } else { "places" }
        ))
    };
    if !price.is_finite() || price <= 0.0 {
        return Err(refused("Enter a price above zero."));
    }
    let written = decimal(price).ok_or_else(too_fine)?;
    let Some((whole, fraction)) = written.split_once('.') else {
        // A whole price is fine however many figures it has.
        return Ok(written);
    };
    let figures = format!("{whole}{fraction}").trim_start_matches('0').len();
    if fraction.len() > allowed as usize || figures > PRICE_FIGURES {
        return Err(too_fine());
    }
    Ok(written)
}

/// `price` cut to the figures and places a price of `instrument` may
/// have: five significant figures, then no more places than a price has
/// (the SDK's rounding of a market order's price). `None` for nothing
/// above zero.
fn cut_price(price: f64, instrument: &Instrument) -> Option<f64> {
    if !price.is_finite() || price <= 0.0 {
        return None;
    }
    let figures: f64 = format!("{price:.*e}", PRICE_FIGURES - 1).parse().ok()?;
    let scale = 10_f64.powi(i32::try_from(price_places(instrument)).unwrap_or(0));
    let cut = (figures * scale).round() / scale;
    (cut > 0.0).then_some(cut)
}

/// The price nearest `price` that a limit order for `instrument` may
/// have: `price` cut to five figures ([`cut_price`]), or the whole number
/// beside it when that is nearer (a whole price may have more figures).
fn nearest_price(price: f64, instrument: &Instrument) -> Option<f64> {
    let cut = cut_price(price, instrument)?;
    let whole = price.round();
    Some(
        if whole > 0.0 && (whole - price).abs() <= (cut - price).abs() {
            whole
        } else {
            cut
        },
    )
}

/// The limit price of a market order on `side` of `instrument` while its
/// mid price is `mid`: `slippage` (a fraction) worse than it, cut to the
/// figures and places a price may have (the SDK's `_slippage_price`).
fn market_price_text(
    mid: f64,
    side: Side,
    slippage: f64,
    instrument: &Instrument,
) -> Result<String, ApiError> {
    let slippage = slippage.clamp(0.0, 0.5);
    let price = match side {
        Side::Buy => mid * (1.0 + slippage),
        Side::Sell => mid * (1.0 - slippage),
    };
    cut_price(price, instrument)
        .and_then(decimal)
        .ok_or_else(|| refused(format!("{} has no usable price now.", instrument.symbol)))
}

/// The exit `exit` of `size` of `instrument` (the asset `asset`), which
/// waits for `trigger` and then buys (`buy`) or sells at the market's
/// price, no worse than the slippage past its trigger.
fn exit_order(
    exit: Exit,
    trigger: f64,
    buy: bool,
    size: &str,
    asset: u32,
    instrument: &Instrument,
) -> Result<WireOrder, ApiError> {
    let side = if buy { Side::Buy } else { Side::Sell };
    Ok(WireOrder {
        asset,
        buy,
        price: market_price_text(trigger, side, EXIT_SLIPPAGE, instrument)?,
        size: size.to_owned(),
        reduce_only: true,
        kind: WireKind::Trigger {
            trigger: limit_price_text(trigger, instrument)?,
            market: true,
            exit,
        },
    })
}

/// The orders `exits` are, each buying (`buy`) or selling `size` of
/// `instrument`: the take profit, then the stop loss, as Hyperliquid's own
/// SDK lists them after an order.
fn exit_orders(
    exits: Exits,
    buy: bool,
    size: &str,
    asset: u32,
    instrument: &Instrument,
) -> Result<Vec<WireOrder>, ApiError> {
    [
        (Exit::TakeProfit, exits.take_profit),
        (Exit::StopLoss, exits.stop_loss),
    ]
    .into_iter()
    .filter_map(|(exit, trigger)| Some((exit, trigger?)))
    .map(|(exit, trigger)| exit_order(exit, trigger, buy, size, asset, instrument))
    .collect()
}

/// `leverage` as the whole number Hyperliquid sets for `instrument`, or why
/// it cannot be one.
fn whole_leverage(leverage: f64, instrument: &Instrument) -> Result<u32, ApiError> {
    let most = instrument.max_leverage.unwrap_or(u32::MAX);
    let whole = leverage.is_finite() && leverage.fract() == 0.0;
    if whole && leverage >= 1.0 && leverage <= f64::from(most) {
        // A whole number within a `u32`, checked above.
        #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        return Ok(leverage as u32);
    }
    Err(refused(match instrument.max_leverage {
        Some(most) => format!(
            "Leverage of {} is a whole number from 1 to {most}.",
            instrument.symbol
        ),
        None => format!("Leverage of {} is a whole number.", instrument.symbol),
    }))
}

/// `address` as the address it is.
fn owner(address: &str) -> Result<Address, ApiError> {
    address
        .parse()
        .map_err(|_| refused("This account's address is not a Hyperliquid address."))
}

/// The instrument `symbol` names among `instruments`, with its asset's
/// index. A perpetual's index is its place in the universe, and the
/// instruments read from the universe keep every entry in its order
/// ([`super::meta`]): that is Hyperliquid's own, and nothing outside this
/// integration counts on an instrument's place.
fn asset<'a>(instruments: &'a Instruments, symbol: &str) -> Option<(u32, &'a Instrument)> {
    let found = instruments.find(symbol)?;
    let mut listed = instruments.0.iter();
    let place = listed.position(|listed| std::ptr::eq(listed, found))?;
    Some((u32::try_from(place).ok()?, found))
}

/// The instrument `symbol` names among `instruments`, with its asset's
/// index, when orders for it are taken.
fn listed<'a>(
    instruments: &'a Instruments,
    symbol: &str,
) -> Result<(u32, &'a Instrument), ApiError> {
    let (index, instrument) = asset(instruments, symbol)
        .ok_or_else(|| refused(format!("Hyperliquid lists no {symbol} perpetual.")))?;
    if !instrument.tradable {
        return Err(refused(format!(
            "{} is delisted on Hyperliquid.",
            instrument.symbol
        )));
    }
    Ok((index, instrument))
}

/// What the connection panel calls a request of trading's.
fn subject(name: &str, detail: &str) -> Subject {
    Subject {
        exchange: VENUE.to_owned(),
        name: name.to_owned(),
        detail: detail.to_owned(),
    }
}

impl Hyperliquid {
    /// The mid price of everything, which a market order is priced from
    /// (asked for when one is placed, no older than [`MID_FRESH`]). What
    /// widgets show as a market's price is the prices' ([`super::pricing`]).
    fn mids_query(&self) -> QuerySpec<Mids> {
        self.info_query(
            Info::AllMids,
            MIDS_KEPT,
            subject("All markets", "Mid prices"),
            |mids: AllMids| mids.into(),
        )
    }

    /// Signs `action` as `owner`'s agent and posts it: what Hyperliquid
    /// made of it.
    async fn act(&self, owner: Address, action: Packed) -> Result<Value, ApiError> {
        let nonce = self.agents.next_nonce();
        let hash =
            signing::l1_signing_hash(self.network, signing::action_hash(&action, nonce, None));
        let signature = self
            .agents
            .sign(owner, &hash)
            .ok_or_else(|| refused("Enable trading on Hyperliquid for this account first."))?
            .map_err(|error| ApiError::new(ErrorKind::Internal, "SIGNING_FAILED", error))?;
        self.api.exchange(&action, nonce, &signature).await
    }

    /// Takes up the agent kept for `owner` from an earlier run, once
    /// Hyperliquid lists it as still valid. `Ok(false)` when none is kept,
    /// or the one kept no longer holds there, which discards it.
    async fn resume_agent(&self, owner: Address) -> Result<bool, ApiError> {
        let Some(key) = agent::kept(self.network, owner) else {
            return Ok(false);
        };
        let request = Info::ExtraAgents {
            user: format!("{owner:#x}"),
        };
        let listed: ExtraAgents = self.api.info(&request).await?;
        if listed.holds(key.address(), unix_ms()) {
            self.agents.approve(owner, key);
            Ok(true)
        } else {
            // Retired there (trading was enabled elsewhere, or revoked) or
            // run out: the key signs for nothing any more.
            agent::discard(self.network, owner);
            Ok(false)
        }
    }

    /// Asks again for how `owner` is set to trade `coin`, and for its
    /// perpetuals account (a position's leverage is part of it), for
    /// whoever watches them.
    fn refresh_setting(&self, owner: Address, coin: &str) {
        let user = format!("{owner:#x}");
        for changed in [
            Info::ActiveAssetData {
                user: user.clone(),
                coin: coin.to_owned(),
            },
            Info::ClearinghouseState { user, dex: None },
        ] {
            self.queries.refresh(&changed.key());
        }
    }

    /// Asks again for what an action of `owner`'s changed: its open orders
    /// and its perpetuals account, for whoever watches them.
    fn refresh_account(&self, owner: Address) {
        let user = format!("{owner:#x}");
        for changed in [
            Info::FrontendOpenOrders { user: user.clone() },
            Info::ClearinghouseState { user, dex: None },
        ] {
            self.queries.refresh(&changed.key());
        }
    }
}

impl TradingVenue for Hyperliquid {
    fn venue(&self) -> &'static str {
        VENUE
    }

    fn name(&self) -> &'static str {
        "Hyperliquid"
    }

    fn enabling_note(&self) -> &'static str {
        "Your wallet signs one approval. It lets a key Lattice keeps on this device place and \
         cancel orders and set leverage for this account until you disable it here. The key \
         cannot withdraw or transfer."
    }

    fn queries(&self) -> &Queries {
        self.queries()
    }

    fn instruments_query(&self) -> QuerySpec<Instruments> {
        self.info_query(
            Info::Meta,
            INSTRUMENTS_REFRESH,
            subject("Perpetuals", "Markets"),
            |meta: Meta| meta.into(),
        )
    }

    fn open_orders_query(&self, address: &str) -> QuerySpec<OpenOrders> {
        let request = Info::FrontendOpenOrders {
            user: address.to_owned(),
        };
        self.info_query(
            request,
            ORDERS_REFRESH,
            subject(&short_address(address), "Open orders"),
            |orders: FrontendOpenOrders| orders.into(),
        )
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
        let request = Info::ActiveAssetData {
            user: address.to_owned(),
            coin: instrument.to_owned(),
        };
        Some(self.info_query(
            request,
            LEVERAGE_REFRESH,
            subject(&short_address(address), &format!("{instrument} leverage")),
            |setting: ActiveAssetData| setting.into(),
        ))
    }

    fn nearest_price(&self, instrument: &Instrument, price: f64) -> Option<f64> {
        nearest_price(price, instrument)
    }

    fn follow(&self, address: &str, hears: Hears) -> Option<Following> {
        Some(Following::new(self.follow_account(address, hears)))
    }

    fn enable(&self, signer: WalletSigner) -> Doing<()> {
        let this = self.clone();
        Box::pin(async move {
            let owner = signer.address();
            // What an earlier run kept, if it still holds: no signature.
            if this.resume_agent(owner).await? {
                return Ok(());
            }
            let chain_id = signer.chain_id().unwrap_or_default();
            let key = AgentKey::random()
                .map_err(|error| ApiError::new(ErrorKind::Internal, "NO_KEY", error))?;
            let nonce = this.agents.next_nonce();
            let approval =
                signing::approve_agent(this.network, chain_id, key.address(), AGENT_NAME, nonce)
                    .map_err(refused)?;
            let signature = signer
                .sign_dynamic_typed_data(&approval)
                .await
                .map_err(|error| refused(describe_signing(&error)))?;
            let signed_by = signing::signer_of(&approval, &signature).map_err(refused)?;
            if signed_by != owner {
                return Err(refused(format!(
                    "Signed by {signed_by}, not by this account. Hyperliquid would take it for \
                     that address (a contract wallet cannot sign for Hyperliquid)."
                )));
            }
            let action = exchange::approve_agent_action(
                this.network,
                chain_id,
                key.address(),
                AGENT_NAME,
                nonce,
            );
            this.api.exchange(&action, nonce, &signature).await?;
            agent::keep(this.network, owner, &key);
            this.agents.approve(owner, key);
            Ok(())
        })
    }

    fn kept(&self, address: &str) -> bool {
        owner(address).is_ok_and(|owner| agent::kept(self.network, owner).is_some())
    }

    fn resume(&self, address: &str) -> Doing<bool> {
        let (this, address) = (self.clone(), address.to_owned());
        Box::pin(async move { this.resume_agent(owner(&address)?).await })
    }

    fn release(&self) {
        self.agents.forget();
    }

    fn disable(&self, address: &str) {
        self.agents.forget();
        if let Ok(owner) = owner(address) {
            agent::discard(self.network, owner);
        }
    }

    fn place(&self, address: &str, order: OrderRequest) -> Doing<Placed> {
        let (this, address) = (self.clone(), address.to_owned());
        Box::pin(async move {
            let owner = owner(&address)?;
            let instruments = this.queries.get(this.instruments_query()).await?;
            let (asset, instrument) = listed(&instruments, &order.instrument)?;
            let size = size_text(order.size, instrument)?;
            let buy = order.side == Side::Buy;
            // What it trades at, in text, and the price its exits are
            // measured from: its own, or the market's now.
            let (price, time_in_force, from) = match order.kind {
                OrderKind::Limit {
                    price,
                    time_in_force,
                } => (limit_price_text(price, instrument)?, time_in_force, price),
                OrderKind::Market { slippage } => {
                    let mids = this
                        .queries
                        .get(this.mids_query().fresh_for(MID_FRESH))
                        .await?;
                    let mid = mids.of(&instrument.symbol).ok_or_else(|| {
                        refused(format!("{} has no mid price now.", instrument.symbol))
                    })?;
                    (
                        market_price_text(mid, order.side, slippage, instrument)?,
                        TimeInForce::ImmediateOrCancel,
                        mid,
                    )
                }
            };
            if !order.exits.is_empty() && order.reduce_only {
                return Err(refused(
                    "An order that only reduces opens nothing to take profit on or stop.",
                ));
            }
            // One on the wrong side of the price would trigger at once.
            if let Some(why) = order.exits.misplaced(buy, from) {
                return Err(refused(why));
            }
            let mut orders = vec![WireOrder {
                asset,
                buy,
                price,
                size: size.clone(),
                reduce_only: order.reduce_only,
                kind: WireKind::Limit(time_in_force),
            }];
            orders.extend(exit_orders(order.exits, !buy, &size, asset, instrument)?);
            let grouping = if orders.len() > 1 {
                Grouping::WithExits
            } else {
                Grouping::Apart
            };
            let action = exchange::orders_action(&orders, grouping);
            let made = this.act(owner, action).await;
            // Whatever it said: an order refused in part may have traded.
            this.refresh_account(owner);
            exchange::placed(&made?)
        })
    }

    fn cancel(&self, address: &str, cancel: CancelRequest) -> Doing<()> {
        let (this, address) = (self.clone(), address.to_owned());
        Box::pin(async move {
            let owner = owner(&address)?;
            let order: u64 = cancel
                .id
                .parse()
                .map_err(|_| refused("This order has no Hyperliquid id."))?;
            let instruments = this.queries.get(this.instruments_query()).await?;
            let (asset, _) = asset(&instruments, &cancel.instrument).ok_or_else(|| {
                refused(format!(
                    "{} is not a market Lattice trades on Hyperliquid yet.",
                    cancel.instrument
                ))
            })?;
            let made = this.act(owner, exchange::cancel_action(asset, order)).await;
            this.refresh_account(owner);
            exchange::cancelled(&made?)
        })
    }

    fn set_leverage(&self, address: &str, instrument: &str, leverage: Leverage) -> Doing<()> {
        let (this, address, instrument) = (self.clone(), address.to_owned(), instrument.to_owned());
        Box::pin(async move {
            let owner = owner(&address)?;
            let instruments = this.queries.get(this.instruments_query()).await?;
            let (asset, instrument) = listed(&instruments, &instrument)?;
            let times = whole_leverage(leverage.value, instrument)?;
            let cross = leverage.mode == MarginMode::Cross;
            if cross && instrument.isolated_only {
                return Err(refused(format!(
                    "{} is traded with isolated margin only.",
                    instrument.symbol
                )));
            }
            let action = exchange::leverage_action(asset, cross, times);
            let made = this.act(owner, action).await;
            this.refresh_setting(owner, &instrument.symbol);
            made.map(|_| ())
        })
    }

    fn protect(&self, address: &str, instrument: &str, exits: Exits) -> Doing<()> {
        let (this, address, instrument) = (self.clone(), address.to_owned(), instrument.to_owned());
        Box::pin(async move {
            let owner = owner(&address)?;
            if exits.is_empty() {
                return Err(refused("Enter a take profit or a stop loss."));
            }
            let instruments = this.queries.get(this.instruments_query()).await?;
            let (asset, instrument) = listed(&instruments, &instrument)?;
            let account = AccountSource::margin_query(&this, &address)
                .ok_or_else(|| refused("Hyperliquid reports no positions."))?;
            let account = this.queries.get(account.fresh_for(POSITION_FRESH)).await?;
            let position = account
                .positions
                .iter()
                .find(|position| position.instrument == instrument.symbol)
                .ok_or_else(|| {
                    refused(format!(
                        "This account holds no {} position on Hyperliquid.",
                        instrument.symbol
                    ))
                })?;
            let long = position.direction == Direction::Long;
            // One on the wrong side of the mark would close it at once.
            if let Some(mark) = position.mark_price
                && let Some(why) = exits.misplaced(long, mark)
            {
                return Err(refused(why));
            }
            // No size of their own: they take the position's.
            let orders = exit_orders(exits, !long, "0", asset, instrument)?;
            let action = exchange::orders_action(&orders, Grouping::PositionExits);
            let made = this.act(owner, action).await;
            this.refresh_account(owner);
            exchange::exits_placed(&made?)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instrument(symbol: &str, size_decimals: u32) -> Instrument {
        Instrument {
            symbol: symbol.to_owned(),
            size_decimals,
            max_leverage: Some(40),
            tradable: true,
            isolated_only: false,
        }
    }

    #[test]
    fn an_exit_waits_for_its_trigger_and_trades_within_a_tenth_past_it() {
        let btc = instrument("BTC", 5);
        let exits = Exits {
            take_profit: Some(65_000.0),
            stop_loss: Some(58_000.0),
        };
        // A long's exits sell: the take profit first, then the stop loss,
        // each no worse than 10% under its trigger, and only reducing.
        let orders = exit_orders(exits, false, "0.01", 0, &btc).unwrap();
        let read = |order: &WireOrder| {
            let WireKind::Trigger {
                trigger,
                market,
                exit,
            } = &order.kind
            else {
                panic!("an exit is a trigger order");
            };
            assert!(*market && order.reduce_only && !order.buy);
            assert_eq!(order.size, "0.01");
            (*exit, trigger.clone(), order.price.clone())
        };
        assert_eq!(
            orders.iter().map(read).collect::<Vec<_>>(),
            [
                (Exit::TakeProfit, "65000".to_owned(), "58500".to_owned()),
                (Exit::StopLoss, "58000".to_owned(), "52200".to_owned()),
            ]
        );
        // A short's buy back, no worse than 10% over: what Hyperliquid's
        // site sent for a stop at 64,143 on 2026-11-22 (70,557).
        let stop = Exits {
            take_profit: None,
            stop_loss: Some(64_143.0),
        };
        let orders = exit_orders(stop, true, "0", 0, &btc).unwrap();
        assert_eq!(orders.len(), 1);
        assert!(orders[0].buy);
        assert_eq!(
            (orders[0].price.as_str(), orders[0].size.as_str()),
            ("70557", "0")
        );
        // None named is none placed, and a trigger no price may have is
        // refused, not rounded.
        assert_eq!(
            exit_orders(Exits::default(), false, "1", 0, &btc),
            Ok(vec![])
        );
        let fine = Exits {
            take_profit: Some(65_000.55),
            stop_loss: None,
        };
        assert_eq!(
            exit_orders(fine, false, "1", 0, &btc).unwrap_err().message,
            "A price of BTC has 5 significant figures and 1 decimal place at most."
        );
    }

    #[test]
    fn leverage_is_a_whole_number_up_to_the_assets_most() {
        let btc = instrument("BTC", 5);
        assert_eq!(whole_leverage(1.0, &btc), Ok(1));
        assert_eq!(whole_leverage(40.0, &btc), Ok(40));
        for refused in [0.0, 41.0, 2.5, -3.0, f64::NAN, f64::INFINITY] {
            assert_eq!(
                whole_leverage(refused, &btc).unwrap_err().message,
                "Leverage of BTC is a whole number from 1 to 40.",
                "{refused}"
            );
        }
        // An asset that names no most takes any whole number.
        let open = Instrument {
            max_leverage: None,
            ..btc
        };
        assert_eq!(whole_leverage(100.0, &open), Ok(100));
        assert_eq!(
            whole_leverage(0.5, &open).unwrap_err().message,
            "Leverage of BTC is a whole number."
        );
    }

    #[test]
    fn a_size_is_written_to_the_assets_places_or_refused() {
        let btc = instrument("BTC", 5);
        assert_eq!(size_text(0.01, &btc).as_deref(), Ok("0.01"));
        assert_eq!(size_text(0.000_01, &btc).as_deref(), Ok("0.00001"));
        assert_eq!(size_text(2.0, &btc).as_deref(), Ok("2"));
        // One place too many is not rounded into another order.
        assert_eq!(
            size_text(0.000_001, &btc).unwrap_err().message,
            "A size of BTC has 5 decimal places at most."
        );
        let pepe = instrument("kPEPE", 0);
        assert_eq!(size_text(1500.0, &pepe).as_deref(), Ok("1500"));
        assert_eq!(
            size_text(1500.5, &pepe).unwrap_err().message,
            "kPEPE trades in whole units."
        );
        for nothing in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert_eq!(
                size_text(nothing, &btc).unwrap_err().message,
                "Enter a size above zero."
            );
        }
    }

    #[test]
    fn a_limit_price_has_five_figures_and_the_assets_places_at_most() {
        // BTC: sizes to 5 places, so prices to 1.
        let btc = instrument("BTC", 5);
        assert_eq!(limit_price_text(6_100.5, &btc).as_deref(), Ok("6100.5"));
        // Whole, however many figures.
        assert_eq!(limit_price_text(61_000.0, &btc).as_deref(), Ok("61000"));
        assert_eq!(limit_price_text(123_456.0, &btc).as_deref(), Ok("123456"));
        let refused = "A price of BTC has 5 significant figures and 1 decimal place at most.";
        // Two places.
        assert_eq!(
            limit_price_text(6_100.55, &btc).unwrap_err().message,
            refused
        );
        // Six figures with a fraction.
        assert_eq!(
            limit_price_text(61_000.5, &btc).unwrap_err().message,
            refused
        );

        // A cheap asset: figures count from the first that is not zero.
        let cheap = instrument("kPEPE", 0);
        assert_eq!(
            limit_price_text(0.012_345, &cheap).as_deref(),
            Ok("0.012345")
        );
        assert!(limit_price_text(0.012_345_6, &cheap).is_err());
        assert_eq!(
            limit_price_text(0.0, &btc).unwrap_err().message,
            "Enter a price above zero."
        );
    }

    #[test]
    fn a_market_order_is_priced_the_slippage_from_the_mid() {
        let btc = instrument("BTC", 5);
        // 5% above 61,012.5 is 64,063.125: five figures.
        assert_eq!(
            market_price_text(61_012.5, Side::Buy, 0.05, &btc).as_deref(),
            Ok("64063")
        );
        assert_eq!(
            market_price_text(61_012.5, Side::Sell, 0.05, &btc).as_deref(),
            Ok("57962")
        );
        // ETH: sizes to 4 places, prices to 2; five figures leave one.
        let eth = instrument("ETH", 4);
        assert_eq!(
            market_price_text(1891.4, Side::Buy, 0.05, &eth).as_deref(),
            Ok("1986")
        );
        // A price under one keeps its figures within its places.
        let pepe = instrument("kPEPE", 0);
        assert_eq!(
            market_price_text(0.011_111, Side::Buy, 0.05, &pepe).as_deref(),
            Ok("0.011667")
        );
        assert!(market_price_text(0.0, Side::Sell, 0.05, &btc).is_err());
    }

    #[test]
    fn a_price_off_the_market_is_turned_into_one_an_order_may_have() {
        // A mid between two ticks is no price: BTC's are whole here.
        let btc = instrument("BTC", 5);
        assert_eq!(nearest_price(61_012.7, &btc), Some(61_013.0));
        assert_eq!(nearest_price(61_012.5, &btc), Some(61_013.0));
        // Whole prices keep their figures past the fifth.
        assert_eq!(nearest_price(123_456.4, &btc), Some(123_456.0));
        // Five figures within the asset's places.
        let eth = instrument("ETH", 4);
        assert_eq!(nearest_price(1_891.44, &eth), Some(1_891.4));
        let pepe = instrument("kPEPE", 0);
        assert_eq!(nearest_price(0.011_111_4, &pepe), Some(0.011_111));
        assert_eq!(nearest_price(0.0, &btc), None);
        // What it gives is a price a limit order takes.
        for (price, instrument) in [(61_012.5, &btc), (1_891.44, &eth), (0.011_111_4, &pepe)] {
            let near = nearest_price(price, instrument).unwrap();
            assert!(limit_price_text(near, instrument).is_ok(), "{near}");
        }
    }

    #[test]
    fn an_order_names_a_listed_asset_by_its_place() {
        let mut delisted = instrument("MATIC", 1);
        delisted.tradable = false;
        let instruments = Instruments(vec![instrument("BTC", 5), delisted, instrument("ETH", 4)]);
        assert_eq!(listed(&instruments, "eth").map(|(index, _)| index), Ok(2));
        assert_eq!(
            listed(&instruments, "DOGE").unwrap_err().message,
            "Hyperliquid lists no DOGE perpetual."
        );
        assert_eq!(
            listed(&instruments, "MATIC").unwrap_err().message,
            "MATIC is delisted on Hyperliquid."
        );
    }
}
