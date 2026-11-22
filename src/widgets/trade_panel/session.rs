//! The Trade Panel's session ([`TradeSession`]): whose account it trades
//! for and where that account stands, what it reads of the exchange, the
//! order being written ([`Ticket`]) and what the last one came to
//! (the widget's data, described in the parent module).
//!
//! While it has an account to trade for it follows that account live
//! ([`Trading::follow`]): the open orders and the figures it watches are
//! then brought up to date as the market changes them, and each fill is
//! said under the ticket as it happens ([`fill_line`]).
//!
//! Besides the order: how the account is set to trade the market (its
//! leverage and margin mode, watched and set: [`TradeSession::set_leverage`]),
//! and, when it holds a position in it, that position with the exits typed
//! for it ([`TradeSession::protect`]).
//!
//! What is sent is made here from what was typed ([`request_of`]) and said
//! in the words every trading widget says an order in ([`order_line`],
//! [`placed_line`]: [`orders`](crate::widgets::common::orders)), so the
//! view only lays it out and both are tested without one.

use std::{cell::RefCell, rc::Rc};

use nsh_core::prelude::*;

use super::settings::TradePanelSettings;
use crate::{
    data::{
        Prices, SessionScope, Trading, Wallet,
        net::{QueryState, TaskSet, Watch},
        portfolio::{
            Account, Direction, Leverage, MarginAccount, MarginMode, Position, Unavailable,
        },
        prices::PriceKind,
        trading::{
            Access, AccountEvent, CancelRequest, Exits, Following, Instrument, Instruments,
            OpenOrder, OpenOrders, OrderKind, OrderRequest, Side, TimeInForce,
        },
    },
    exchanges, format,
    widgets::common::{
        access::Voice,
        accounts::{Reports, follow_all},
        orders::fill_line,
    },
};
// What an order is typed and said in, and how far a market order may trade,
// are every trading widget's: named here for the view and the tests.
pub(super) use crate::{
    data::trading::MARKET_SLIPPAGE as SLIPPAGE,
    widgets::common::{
        access::Said,
        orders::{order_line, placed_line, typed},
    },
};

/// What the ticket holds as it is typed: plain values, read from its
/// signals ([`Ticket::draft`]).
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Draft {
    pub side: Side,
    /// Whether it trades at the market's price; else at [`Self::price`].
    pub market: bool,
    pub time_in_force: TimeInForce,
    pub price: String,
    pub size: String,
    pub reduce_only: bool,
    /// The exits placed with it, as typed; nothing typed is none.
    pub take_profit: String,
    pub stop_loss: String,
}

/// The exits typed as `take_profit` and `stop_loss`: nothing typed is
/// none, and anything else is a price above zero or said not to be.
pub(super) fn exits_of(take_profit: &str, stop_loss: &str) -> Result<Exits, &'static str> {
    let price = |text: &str, missing: &'static str| {
        if text.trim().is_empty() {
            return Ok(None);
        }
        let price = typed(text).filter(|price| *price > 0.0);
        price.map(Some).ok_or(missing)
    };
    Ok(Exits {
        take_profit: price(
            take_profit,
            "Enter a take profit price above zero, or leave it empty.",
        )?,
        stop_loss: price(
            stop_loss,
            "Enter a stop loss price above zero, or leave it empty.",
        )?,
    })
}

/// The order `draft` is for `instrument`, or what is missing of it. `mid`
/// is the market's price now, where it is known: what a market order's
/// exits are measured from.
pub(super) fn request_of(
    draft: &Draft,
    instrument: &str,
    mid: Option<f64>,
) -> Result<OrderRequest, &'static str> {
    let above_zero = |text: &str| typed(text).filter(|number| *number > 0.0);
    let size = above_zero(&draft.size).ok_or("Enter a size above zero (1.5).")?;
    let (kind, from) = if draft.market {
        (OrderKind::Market { slippage: SLIPPAGE }, mid)
    } else {
        let price = above_zero(&draft.price).ok_or("Enter a price above zero (61000.5).")?;
        let kind = OrderKind::Limit {
            price,
            time_in_force: draft.time_in_force,
        };
        (kind, Some(price))
    };
    // An order that only reduces opens nothing to exit: what was typed for
    // one before it was switched to that is not sent with it.
    let exits = if draft.reduce_only {
        Exits::default()
    } else {
        exits_of(&draft.take_profit, &draft.stop_loss)?
    };
    // One on the wrong side of the order's price would trigger at once.
    if let Some(from) = from
        && let Some(why) = exits.misplaced(draft.side == Side::Buy, from)
    {
        return Err(why);
    }
    Ok(OrderRequest {
        instrument: instrument.to_owned(),
        side: draft.side,
        size,
        kind,
        reduce_only: draft.reduce_only,
        exits,
    })
}

/// What was typed as a leverage, with the margin `mode` picked: a whole
/// number of times (`20`, `20x`), or said not to be.
pub(super) fn leverage_of(text: &str, mode: MarginMode) -> Result<Leverage, &'static str> {
    let times = text.trim().trim_end_matches(['x', 'X']);
    typed(times)
        .filter(|times| *times >= 1.0 && times.fract() == 0.0)
        .map(|value| Leverage { value, mode })
        .ok_or("Enter leverage as a whole number (20).")
}

/// `20x Isolated`: a leverage setting.
pub(super) fn leverage_text(leverage: Leverage) -> String {
    format!(
        "{}x {}",
        format::quantity(leverage.value),
        leverage.mode.label()
    )
}

/// The position `margin` holds in `instrument`, if it holds one.
pub(super) fn position_in(margin: &MarginAccount, instrument: &str) -> Option<Position> {
    let held = |position: &&Position| position.instrument == instrument;
    margin.positions.iter().find(held).cloned()
}

/// `Take profit at 65000 and stop loss at 58000 placed for the BTC
/// position.`: what placing `exits` for a position came to.
pub(super) fn protected_line(instrument: &str, exits: Exits) -> String {
    let named = [
        exits
            .take_profit
            .map(|price| format!("take profit at {price}")),
        exits.stop_loss.map(|price| format!("stop loss at {price}")),
    ];
    let named: Vec<String> = named.into_iter().flatten().collect();
    let mut line = named.join(" and ");
    if let Some(first) = line.get_mut(..1) {
        first.make_ascii_uppercase();
    }
    format!("{line} placed for the {instrument} position.")
}

/// The price an open order is known by: the one it waits for when it
/// waits for one (what it may trade at past it is only a bound), else its
/// own.
pub(super) fn order_price(order: &OpenOrder) -> Option<f64> {
    order.trigger_price.or(order.price)
}

/// `4.5 BTC`: how much of what an open order is for; `BTC position` for
/// the exit of a whole position, which has no size of its own.
pub(super) fn order_amount(order: &OpenOrder) -> String {
    if order.whole_position {
        return format!("{} position", order.instrument);
    }
    let size = order.size.map_or("?".to_owned(), format::quantity);
    format!("{size} {}", order.instrument)
}

/// `Sell 4.5 BTC at 29792.0`: an open order, as its line's label and what
/// is said of cancelling it name it.
pub(super) fn open_order_line(order: &OpenOrder) -> String {
    format!(
        "{} {} at {}",
        order.side.label(),
        order_amount(order),
        order_price(order).map_or("?".to_owned(), format::price_text)
    )
}

/// Where the instrument the settings name stands on the exchange.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Listing {
    /// The exchange has not said what it trades yet.
    Unknown,
    Listed(Instrument),
    /// It trades nothing of this name.
    Missing,
    /// It no longer takes orders for it: its name, as the exchange
    /// writes it.
    Delisted(String),
}

impl Listing {
    pub(super) fn of(instruments: Option<&Instruments>, symbol: &str) -> Self {
        match instruments.map(|instruments| instruments.find(symbol)) {
            None => Self::Unknown,
            Some(None) => Self::Missing,
            Some(Some(instrument)) if instrument.tradable => Self::Listed(instrument.clone()),
            Some(Some(instrument)) => Self::Delisted(instrument.symbol.clone()),
        }
    }
}

/// The order being written. Kept in the session, so a tab switch or
/// another page leaves it as it was; a restart starts it over.
#[derive(Clone, Copy)]
pub(super) struct Ticket {
    pub side: Signal<Side>,
    pub market: Signal<bool>,
    pub time_in_force: Signal<TimeInForce>,
    pub price: Signal<String>,
    pub size: Signal<String>,
    pub reduce_only: Signal<bool>,
    pub take_profit: Signal<String>,
    pub stop_loss: Signal<String>,
}

impl Ticket {
    fn new() -> Self {
        Self {
            side: signal(Side::Buy),
            market: signal(false),
            time_in_force: signal(TimeInForce::default()),
            price: signal(String::new()),
            size: signal(String::new()),
            reduce_only: signal(false),
            take_profit: signal(String::new()),
            stop_loss: signal(String::new()),
        }
    }

    /// What it holds now (tracked).
    pub(super) fn draft(&self) -> Draft {
        Draft {
            side: self.side.get(),
            market: self.market.get(),
            time_in_force: self.time_in_force.get(),
            price: self.price.get(),
            size: self.size.get(),
            reduce_only: self.reduce_only.get(),
            take_profit: self.take_profit.get(),
            stop_loss: self.stop_loss.get(),
        }
    }
}

/// The watch `watch` makes of the exchange `venue` names, followed as it
/// changes (none while it names none): the value it last gave. Made under
/// the session (`scope`), so it is watched for as long as the widget is
/// in the workspace.
fn follow_venue<T: PartialEq + 'static>(
    scope: SessionScope,
    venue: Memo<Option<String>>,
    watch: impl Fn(&str) -> Result<Watch<T>, Unavailable> + 'static,
) -> Memo<Option<Rc<T>>> {
    let current = signal(None);
    let held: Rc<RefCell<Option<Watch<T>>>> = Rc::default();
    effect(move || {
        let venue = venue.get();
        untracked(|| {
            // Under the session, not this run of the effect, which the
            // watch's state must outlive.
            let made = venue.and_then(|venue| scope.run(|| watch(&venue)).ok());
            current.set(made.as_ref().map(Watch::state));
            // The new watch is held before the old one goes.
            *held.borrow_mut() = made;
        });
    });
    computed(move || {
        current
            .get()
            .and_then(|state| state.with(|state| state.data.clone()))
    })
}

/// A Trade Panel's session state.
pub(crate) struct TradeSession {
    pub(super) trading: Trading,
    /// The wallet whose account is traded for, which the panel connects
    /// while there is none.
    pub(super) wallet: Wallet,
    /// The exchange traded on: the settings', else the first there is.
    pub(super) venue: Memo<String>,
    /// What that exchange is called: its own name for itself, else the one
    /// Lattice knows its id by (a save from a build that traded on more).
    pub(super) venue_name: Memo<String>,
    /// What is traded there, as the settings name it.
    pub(super) symbol: Memo<String>,
    /// The account orders are placed for: the connected wallet's, once it
    /// has enabled trading on the exchange. None until then, and nothing
    /// is asked of the exchange for it.
    pub(super) trader: Memo<Option<Account>>,
    /// Its perpetuals account: what it has to trade with.
    pub(super) margin: Reports<MarginAccount>,
    /// Its open orders.
    pub(super) orders: Reports<OpenOrders>,
    /// Where the instrument stands on the exchange.
    pub(super) listing: Memo<Listing>,
    /// The instrument's price now (the middle of its best bid and offer),
    /// followed live while the widget shows; none where nothing prices
    /// the exchange.
    pub(super) mid: Signal<Option<f64>>,
    pub(super) ticket: Ticket,
    /// The order the user was asked about again, until it is sent or the
    /// ticket changes.
    pub(super) armed: Signal<Option<OrderRequest>>,
    /// Whether an order is on its way.
    pub(super) sending: Signal<bool>,
    /// The open orders being cancelled, by id.
    pub(super) cancelling: Signal<Vec<String>>,
    pub(super) said: Signal<Option<Said>>,
    /// How the account is set to trade the instrument now: its leverage
    /// and margin mode there. None until the exchange has said.
    pub(super) leverage: Memo<Option<Leverage>>,
    /// Whether there is such a setting to show and to set: the exchange
    /// has one, for a market it lists. Without, there is no row for it.
    pub(super) leverage_offered: Signal<bool>,
    /// The setting as it is being changed: the leverage as typed and the
    /// mode picked, which start over from the setting whenever it changes.
    pub(super) leverage_draft: Signal<String>,
    pub(super) mode_draft: Signal<MarginMode>,
    /// Whether a new setting is on its way.
    pub(super) setting: Signal<bool>,
    /// The position the account holds in the instrument, if it holds one.
    pub(super) position: Memo<Option<Position>>,
    /// The exits being typed for that position.
    pub(super) protect_take: Signal<String>,
    pub(super) protect_stop: Signal<String>,
    /// Whether exits for it are on their way.
    pub(super) protecting: Signal<bool>,
    /// What waits for the exchange's answers. Dropped with the session:
    /// what was sent stays sent.
    tasks: TaskSet,
}

impl TradeSession {
    /// A session following `settings`, the session's copy of the tab's.
    pub(super) fn new(
        scope: SessionScope,
        trading: Trading,
        prices: Prices,
        wallet: Wallet,
        settings: Signal<TradePanelSettings>,
    ) -> Self {
        let first = trading
            .venues()
            .next()
            .map_or("", |venue| venue.venue())
            .to_owned();
        let venue = computed(move || {
            settings.with(|settings| {
                settings
                    .venue()
                    .map_or_else(|| first.clone(), str::to_owned)
            })
        });
        let venue_name = {
            let trading = trading.clone();
            computed(move || {
                let id = venue.get();
                let named = trading.venue(&id).map(|venue| venue.name());
                named.map_or_else(|| exchanges::display_name(&id).to_owned(), str::to_owned)
            })
        };
        let symbol = computed(move || settings.with(|settings| settings.instrument().to_owned()));
        let trader = {
            let trading = trading.clone();
            computed(move || {
                let venue = venue.get();
                (trading.access(&venue) == Access::Ready)
                    .then(|| trading.account(&venue))
                    .flatten()
            })
        };
        // The exchange, while there is an account to trade for on it.
        let traded = computed(move || trader.with(Option::is_some).then(|| venue.get()));

        // The account's reports, watched while it trades.
        let accounts = computed(move || trader.get().into_iter().collect::<Vec<_>>());
        let margin = {
            let trading = trading.clone();
            follow_all(scope, accounts, move |account| {
                trading.watch_margin(account)
            })
        };
        let orders = {
            let trading = trading.clone();
            follow_all(scope, accounts, move |account| {
                trading.watch_open_orders(account)
            })
        };

        let instruments = {
            let trading = trading.clone();
            follow_venue(scope, traded, move |venue| trading.watch_instruments(venue))
        };
        let listing = computed(move || {
            instruments.with(|instruments| Listing::of(instruments.as_deref(), &symbol.get()))
        });

        // The market's price is followed live, where something prices the
        // exchange: only while the widget shows and has an account to trade
        // a listed market for.
        let priced = computed(move || {
            let venue = traded.get()?;
            match listing.get() {
                Listing::Listed(instrument) => Some((venue, instrument.symbol)),
                _ => None,
            }
        });
        let mid = signal(None);
        scope.while_visible(move || {
            let prices = prices.clone();
            effect(move || {
                // Owned by this run: let go with another exchange or market,
                // no account, or the widget hidden.
                mid.set(None);
                let Some((venue, symbol)) = priced.get() else {
                    return;
                };
                let hears = move |price: Option<f64>| {
                    // The session may be gone by the time it is told.
                    mid.try_set(price);
                };
                let feed = untracked(|| prices.follow(&venue, &symbol, PriceKind::Live, hears));
                cleanup(move || drop(feed));
            });
        });

        let ticket = Ticket::new();
        let armed = signal(None);
        // An order asked about again is the order as it stood: any change
        // of the ticket, the instrument or the exchange lets it go.
        effect(move || {
            let _ = (ticket.draft(), symbol.get(), venue.get());
            untracked(|| armed.set(None));
        });
        let said = signal(None);
        // What was said of one account's order is not said of another's.
        effect(move || {
            let _ = trader.get();
            untracked(|| said.set(None));
        });

        // The account is followed live for as long as it trades here: what
        // the watches above read arrives as it changes, and a fill is said
        // as it happens. Held by the session, so a hidden widget's orders
        // are current the moment it shows.
        let following: Rc<RefCell<Option<Following>>> = Rc::default();
        {
            let trading = trading.clone();
            effect(move || {
                let trader = trader.get();
                untracked(|| {
                    let hears = move |event: &AccountEvent| match event {
                        AccountEvent::Filled(fill) => {
                            // The session may be gone by the time it is told.
                            said.try_set(Some(Said {
                                text: fill_line(fill),
                                voice: Voice::Passed,
                            }));
                        }
                    };
                    // Under the session, so what it asks for is the
                    // widget's; the new hold is taken before the old goes.
                    let held = trader
                        .and_then(|account| scope.run(|| trading.follow(&account, hears)).ok());
                    *following.borrow_mut() = held;
                });
            });
        }

        // How the account is set to trade the instrument, watched for the
        // account and the instrument as the exchange names it.
        let set_for = computed(move || {
            let account = trader.get()?;
            match listing.get() {
                Listing::Listed(instrument) => Some((account, instrument.symbol)),
                _ => None,
            }
        });
        let leverage_offered = signal(false);
        let leverage = {
            let trading = trading.clone();
            let current = signal(None::<ReadSignal<QueryState<Option<Leverage>>>>);
            let held: Rc<RefCell<Option<Watch<Option<Leverage>>>>> = Rc::default();
            effect(move || {
                let wanted = set_for.get();
                untracked(|| {
                    // Under the session, not this run of the effect.
                    let made = wanted.map(|(account, symbol)| {
                        scope.run(|| trading.watch_leverage(&account, &symbol))
                    });
                    // There is one to show and to set for a market the
                    // exchange lists, on an exchange that has such a
                    // setting.
                    let made = made.and_then(Result::ok);
                    leverage_offered.set(made.is_some());
                    current.set(made.as_ref().map(Watch::state));
                    *held.borrow_mut() = made;
                });
            });
            computed(move || {
                let state = current.get()?;
                state.with(|state| state.data.as_deref().copied().flatten())
            })
        };
        // What is being typed starts over from the setting when it changes
        // (it was set, here or elsewhere, or it is another market's).
        let (leverage_draft, mode_draft) = (signal(String::new()), signal(MarginMode::Cross));
        effect(move || {
            let current = leverage.get();
            untracked(|| match current {
                Some(current) => {
                    leverage_draft.set(format::quantity(current.value));
                    mode_draft.set(current.mode);
                }
                None => leverage_draft.set(String::new()),
            });
        });

        let position = computed(move || {
            let Listing::Listed(instrument) = listing.get() else {
                return None;
            };
            let (_, account) = margin.arrived()?.into_iter().next()?;
            position_in(&account, &instrument.symbol)
        });
        let (protect_take, protect_stop) = (signal(String::new()), signal(String::new()));

        Self {
            trading,
            wallet,
            venue,
            venue_name,
            symbol,
            trader,
            margin,
            orders,
            listing,
            mid,
            ticket,
            armed,
            sending: signal(false),
            cancelling: signal(Vec::new()),
            said,
            leverage,
            leverage_offered,
            leverage_draft,
            mode_draft,
            setting: signal(false),
            position,
            protect_take,
            protect_stop,
            protecting: signal(false),
            tasks: TaskSet::default(),
        }
    }

    /// The account's perpetuals account, once it has arrived (tracked).
    pub(super) fn standing(&self) -> Option<Rc<MarginAccount>> {
        self.margin
            .arrived()
            .and_then(|arrived| arrived.into_iter().next())
            .map(|(_, margin)| margin)
    }

    /// The account's open orders, once they have arrived (tracked).
    pub(super) fn open_orders(&self) -> Option<Rc<OpenOrders>> {
        self.orders
            .arrived()
            .and_then(|arrived| arrived.into_iter().next())
            .map(|(_, orders)| orders)
    }

    /// Sends the ticket's order. With `confirm` the first press only arms
    /// it (its button then asks again) and the second sends it; what is
    /// missing of it is said instead.
    pub(super) fn submit(&self, confirm: bool) {
        if self.sending.get_untracked() {
            return;
        }
        let symbol = match self.listing.get_untracked() {
            Listing::Listed(instrument) => instrument.symbol,
            _ => self.symbol.get_untracked(),
        };
        let mid = self.mid.get_untracked();
        let order = match request_of(&untracked(|| self.ticket.draft()), &symbol, mid) {
            Ok(order) => order,
            Err(missing) => {
                self.said.set(Some(Said {
                    text: missing.to_owned(),
                    voice: Voice::Failed,
                }));
                return;
            }
        };
        if confirm && self.armed.get_untracked().as_ref() != Some(&order) {
            self.said.set(None);
            self.armed.set(Some(order));
            return;
        }
        self.armed.set(None);
        self.said.set(None);
        self.sending.set(true);
        let placing = self
            .trading
            .place(&self.venue.get_untracked(), order.clone());
        let (sending, said) = (self.sending, self.said);
        self.tasks.spawn(async move {
            let came = match placing.await {
                Ok(placed) => Said {
                    text: placed_line(&order, &placed),
                    voice: Voice::Passed,
                },
                Err(error) => Said {
                    text: error.message,
                    voice: Voice::Failed,
                },
            };
            sending.try_set(false);
            said.try_set(Some(came));
        });
    }

    /// Writes the mid price into the ticket, as a price an order for the
    /// instrument may have (a mid is often between two ticks).
    pub(super) fn price_at_mid(&self) {
        let (Some(mid), Listing::Listed(instrument)) =
            (self.mid.get_untracked(), self.listing.get_untracked())
        else {
            return;
        };
        let venue = self.venue.get_untracked();
        if let Some(price) = self.trading.nearest_price(&venue, &instrument, mid) {
            self.ticket.price.set(format!("{price}"));
        }
    }

    /// Lets go of the order the user was asked about again.
    pub(super) fn disarm(&self) {
        self.armed.set(None);
    }

    /// Says `text` under the ticket as something that did not happen.
    fn refuse(&self, text: impl Into<String>) {
        self.said.set(Some(Said {
            text: text.into(),
            voice: Voice::Failed,
        }));
    }

    /// Whether what is typed for the leverage and picked for the margin
    /// mode is another setting than the account's (tracked): what there is
    /// to set. What is typed that is no leverage counts, so that setting it
    /// says why.
    pub(super) fn leverage_differs(&self) -> bool {
        let typed = leverage_of(&self.leverage_draft.get(), self.mode_draft.get());
        typed.ok() != self.leverage.get()
    }

    /// Sets the account to trade the instrument at the leverage typed and
    /// the margin mode picked.
    pub(super) fn set_leverage(&self) {
        if self.setting.get_untracked() {
            return;
        }
        let Listing::Listed(instrument) = self.listing.get_untracked() else {
            return;
        };
        let typed = self.leverage_draft.get_untracked();
        let wanted = match leverage_of(&typed, self.mode_draft.get_untracked()) {
            Ok(wanted) => wanted,
            Err(missing) => return self.refuse(missing),
        };
        self.said.set(None);
        self.setting.set(true);
        let venue = self.venue.get_untracked();
        let setting_it = self
            .trading
            .set_leverage(&venue, &instrument.symbol, wanted);
        let (setting, said) = (self.setting, self.said);
        self.tasks.spawn(async move {
            let came = match setting_it.await {
                Ok(()) => Said {
                    text: format!("{} set to {}.", instrument.symbol, leverage_text(wanted)),
                    voice: Voice::Passed,
                },
                Err(error) => Said {
                    text: error.message,
                    voice: Voice::Failed,
                },
            };
            setting.try_set(false);
            said.try_set(Some(came));
        });
    }

    /// Places the exits typed for the position held in the instrument.
    pub(super) fn protect(&self) {
        if self.protecting.get_untracked() {
            return;
        }
        let Some(position) = self.position.get_untracked() else {
            return;
        };
        let (take, stop) = (self.protect_take, self.protect_stop);
        let exits = match exits_of(&take.get_untracked(), &stop.get_untracked()) {
            Ok(exits) if exits.is_empty() => {
                return self.refuse("Enter a take profit or a stop loss.");
            }
            Ok(exits) => exits,
            Err(missing) => return self.refuse(missing),
        };
        // One on the wrong side of the mark would close it at once.
        let long = position.direction == Direction::Long;
        if let Some(mark) = position.mark_price
            && let Some(why) = exits.misplaced(long, mark)
        {
            return self.refuse(why);
        }
        self.said.set(None);
        self.protecting.set(true);
        let venue = self.venue.get_untracked();
        let protecting_it = self.trading.protect(&venue, &position.instrument, exits);
        let (protecting, said) = (self.protecting, self.said);
        self.tasks.spawn(async move {
            let came = match protecting_it.await {
                Ok(()) => {
                    // Placed: the fields are for the next ones.
                    take.try_set(String::new());
                    stop.try_set(String::new());
                    Said {
                        text: protected_line(&position.instrument, exits),
                        voice: Voice::Passed,
                    }
                }
                Err(error) => Said {
                    text: error.message,
                    voice: Voice::Failed,
                },
            };
            protecting.try_set(false);
            said.try_set(Some(came));
        });
    }

    /// Cancels the open order `order`.
    pub(super) fn cancel(&self, order: &OpenOrder) {
        if self
            .cancelling
            .with_untracked(|cancelling| cancelling.contains(&order.id))
        {
            return;
        }
        let id = order.id.clone();
        self.cancelling
            .update(|cancelling| cancelling.push(id.clone()));
        let cancel = CancelRequest {
            instrument: order.instrument.clone(),
            id: id.clone(),
        };
        let cancelling_it = self.trading.cancel(&self.venue.get_untracked(), cancel);
        let (cancelling, said, line) = (self.cancelling, self.said, open_order_line(order));
        self.tasks.spawn(async move {
            let came = match cancelling_it.await {
                Ok(()) => Said {
                    text: format!("Cancelled: {line}."),
                    voice: Voice::Passed,
                },
                Err(error) => Said {
                    text: format!("Not cancelled: {}", error.message),
                    voice: Voice::Failed,
                },
            };
            // The session may be gone by now.
            if let Some(mut waiting) = cancelling.try_get() {
                waiting.retain(|waiting| *waiting != id);
                cancelling.try_set(waiting);
            }
            said.try_set(Some(came));
        });
    }
}
