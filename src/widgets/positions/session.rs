//! The Positions' session ([`PositionsSession`]): the margin report of each
//! account its settings name, what their instruments are priced at since
//! those reports were made (the live price or the mark, as its settings
//! say), the rows its view shows of them together, and
//! the closing of a position that is the connected wallet's own (the
//! widget's data, described in the parent module).
//!
//! What shows is made here, so the view only lays it out and it is tested
//! without one: a position as its report says, or as it stands at a price
//! heard since ([`table_of`]); what the rows come to together
//! ([`Totals`]); the exits that wait on a position ([`exits_on`]); and what
//! closes one from what was typed ([`closing_order`]), said in the words
//! every trading widget says an order in
//! ([`orders`](crate::widgets::common::orders)).

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
};

use nsh_core::prelude::*;

use super::settings::PositionsSettings;
use crate::{
    data::{
        Portfolio, Prices, SessionScope, Trading,
        net::TaskSet,
        portfolio::{Account, Direction, MarginAccount, Position},
        prices::{PriceFeed, PriceKind},
        trading::{Access, Exits, Following, Instruments, OpenOrders, OrderRequest, Side},
    },
    format,
    widgets::common::{
        Keyed, Shared,
        access::{Said, Voice},
        accounts::{AccountSettings, Mark, NONE, Reports, Tone, follow_all, shown},
        orders::{placed_line, typed},
    },
};

/// What closing a row's position here takes. Only a position of the
/// connected wallet's own account has it, on an exchange Lattice trades on:
/// an account watched by its address alone is only ever read.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Close {
    /// The exchange it is closed on, by its id.
    pub venue: String,
    /// How much is held, as the exchange reports it.
    pub size: f64,
    /// Half of that, no finer than a size of the instrument may be. `None`
    /// until the exchange has said what it trades, for an instrument it
    /// does not list, and for a position too small to halve.
    pub half: Option<f64>,
}

/// One position's row, as its cells show it.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Row {
    /// What tells the row apart: its instrument and which way it faces,
    /// after its account when the widget shows several.
    pub key: Rc<str>,
    /// Whose it is, when the widget shows several accounts.
    pub owner: Option<Rc<Mark>>,
    pub instrument: String,
    pub direction: Direction,
    /// Its leverage setting (`20x Isolated`), empty when not reported.
    pub leverage: String,
    pub size: String,
    pub value: String,
    pub entry: String,
    pub mark: String,
    /// Its unrealized profit and loss, with its return on equity when
    /// reported (`-$0.01 (-0.27%)`).
    pub pnl: String,
    /// What colors [`Self::pnl`]; `None` when it is not reported.
    pub tone: Option<Tone>,
    pub liquidation: String,
    pub margin: String,
    /// The take profit and the stop loss that wait on it (`65000.0 /
    /// 58000.0`, a dash for one it has none of), for a position of the
    /// connected wallet's own account; `None` for anybody else's, whose
    /// orders are not read.
    pub exits: Option<String>,
    /// What closing it here takes, when it can be closed here.
    pub close: Option<Close>,
}

impl Row {
    fn of(position: &Position, owner: Option<Rc<Mark>>, key: Rc<str>, desk: Option<Desk>) -> Self {
        let pnl = match (position.unrealized_pnl, position.return_on_equity) {
            (Some(pnl), Some(return_on_equity)) => format!(
                "{} ({})",
                format::signed_usd(pnl),
                format::signed_percent(return_on_equity)
            ),
            (Some(pnl), None) => format::signed_usd(pnl),
            (None, _) => NONE.to_owned(),
        };
        let close = desk.map(|desk| Close {
            venue: desk.venue.to_owned(),
            size: position.size,
            half: desk
                .instruments
                .and_then(|listed| listed.find(&position.instrument))
                .and_then(|instrument| instrument.cut_size(position.size / 2.0)),
        });
        let exits = desk.map(|desk| {
            let exits = desk
                .orders
                .map_or_else(Exits::default, |orders| exits_on(orders, position));
            format!(
                "{} / {}",
                shown(exits.take_profit, format::price_text),
                shown(exits.stop_loss, format::price_text)
            )
        });
        Self {
            key,
            owner,
            instrument: position.instrument.clone(),
            direction: position.direction,
            leverage: position.leverage.map_or_else(String::new, |leverage| {
                format!(
                    "{}x {}",
                    format::quantity(leverage.value),
                    leverage.mode.label()
                )
            }),
            size: format::quantity(position.size),
            value: shown(position.value, format::usd),
            entry: shown(position.entry_price, format::price_text),
            mark: shown(position.mark_price, format::price_text),
            pnl,
            tone: position.unrealized_pnl.map(Tone::of),
            liquidation: shown(position.liquidation_price, format::price_text),
            margin: shown(position.margin_used, format::usd),
            exits,
            close,
        }
    }
}

/// What the rows come to together, as the bar over the table says it. It
/// moves with the prices as the rows do.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Totals {
    /// How many positions show.
    pub count: usize,
    /// What they are worth together, long and short alike.
    pub value: String,
    /// What closing them all now would make, before fees.
    pub pnl: String,
    /// What colors [`Self::pnl`]; `None` when none of them reports one.
    pub tone: Option<Tone>,
}

/// Where an account's positions are closed, and what is read of it besides
/// its report: its exchange, for the connected wallet's own account.
#[derive(Clone, Copy)]
pub(super) struct Desk<'a> {
    /// The exchange, by its id.
    pub venue: &'a str,
    /// What it trades, once it has said: how fine a size of each may be.
    pub instruments: Option<&'a Instruments>,
    /// The account's open orders, once they have arrived: those that wait
    /// to close a position show as its exits.
    pub orders: Option<&'a OpenOrders>,
}

/// One account's positions, as [`table_of`] takes them.
pub(super) struct Part<'a> {
    /// Whose they are, when the widget shows several accounts.
    pub mark: Option<Rc<Mark>>,
    pub positions: &'a [Position],
    /// What its exchange prices an instrument at now, for those a price was
    /// heard of since the report was made: a position in one shows as it
    /// stands there.
    pub prices: Option<&'a HashMap<String, f64>>,
    /// Where they can be closed; none for an account that is only read.
    pub desk: Option<Desk<'a>>,
}

/// The exits that wait on `position` among its account's open `orders`:
/// the orders on its other side that wait for a price and only make it
/// smaller, or are for the whole of it. One that waits past the position's
/// price in its favour is its take profit, one that waits against it its
/// stop loss; of several of a kind, the one the price reaches first. None
/// of a position with no price to tell them apart by.
pub(super) fn exits_on(orders: &OpenOrders, position: &Position) -> Exits {
    let Some(price) = position.mark_price.or(position.entry_price) else {
        return Exits::default();
    };
    let closes = Side::closing(position.direction);
    let (mut above, mut below) = (None::<f64>, None::<f64>);
    for order in &orders.0 {
        let waits = order.instrument == position.instrument
            && order.side == closes
            && (order.whole_position || order.reduce_only);
        let Some(trigger) = order.trigger_price.filter(|_| waits) else {
            continue;
        };
        if trigger > price {
            above = Some(above.map_or(trigger, |nearest| nearest.min(trigger)));
        } else {
            below = Some(below.map_or(trigger, |nearest| nearest.max(trigger)));
        }
    }
    // A long takes its profit above its price and is stopped below it; a
    // short the other way round.
    let (take_profit, stop_loss) = match position.direction {
        Direction::Long => (above, below),
        Direction::Short => (below, above),
    };
    Exits {
        take_profit,
        stop_loss,
    }
}

/// The rows of one account's `positions`, in the table's order.
#[cfg(test)]
pub(super) fn rows_of(positions: &[Position]) -> Vec<Row> {
    rows_of_all(&[Part {
        mark: None,
        positions,
        prices: None,
        desk: None,
    }])
}

/// The rows of [`table_of`] alone.
#[cfg(test)]
pub(super) fn rows_of_all(parts: &[Part]) -> Vec<Row> {
    table_of(parts).0
}

/// The rows of the positions `parts` hold, each account's with its mark
/// (none when the widget shows one account), and what they come to.
///
/// - Their order is the reports': the position worth the most first, those
///   of unknown worth last, then by instrument, whichever account each is
///   of. A price heard since does not move a row.
/// - A position whose instrument a price was heard of since
///   ([`Part::prices`]) shows as it stands there
///   ([`Position::marked_at`]): its mark, its worth, what it has made and
///   its return. Any other shows as reported.
pub(super) fn table_of(parts: &[Part]) -> (Vec<Row>, Totals) {
    let mut ordered: Vec<(&Part, &Position)> = parts
        .iter()
        .flat_map(|part| part.positions.iter().map(move |position| (part, position)))
        .collect();
    ordered.sort_by(|(_, a), (_, b)| {
        let worth = |position: &Position| position.value.unwrap_or(f64::NEG_INFINITY);
        worth(b)
            .total_cmp(&worth(a))
            .then_with(|| format::compare_text(&a.instrument, &b.instrument))
    });
    let mut rows: Vec<Row> = Vec::with_capacity(ordered.len());
    let (mut value, mut pnl) = (None::<f64>, None::<f64>);
    let add = |sum: &mut Option<f64>, part: Option<f64>| {
        if let Some(part) = part {
            *sum = Some(sum.unwrap_or(0.0) + part);
        }
    };
    for (part, reported) in ordered {
        let heard = part
            .prices
            .and_then(|prices| prices.get(&reported.instrument));
        let priced = heard.map(|price| reported.priced_at(*price));
        let position = priced.as_ref().unwrap_or(reported);
        add(&mut value, position.value);
        add(&mut pnl, position.unrealized_pnl);
        let mark = part.mark.as_ref();
        let own = format!("{}\n{}", position.instrument, position.direction.label());
        // An address's accounts on one exchange are one account here, and
        // each may hold the market: the second is a row of its own.
        let mut key = Mark::key(mark, &own);
        let mut nth = 1;
        while rows.iter().any(|row| row.key == key) {
            nth += 1;
            key = Mark::key(mark, &format!("{own}\n{nth}"));
        }
        rows.push(Row::of(position, mark.cloned(), key, part.desk));
    }
    let totals = Totals {
        count: rows.len(),
        value: shown(value, format::usd),
        pnl: shown(pnl, format::signed_usd),
        tone: pnl.map(Tone::of),
    };
    (rows, totals)
}

/// The order that closes `amount` (as typed) of `row`'s position at the
/// market's price, or why there is none: what was typed is no size, or is
/// more than is held.
pub(super) fn closing_order(row: &Row, amount: &str) -> Result<OrderRequest, String> {
    let close = row
        .close
        .as_ref()
        .ok_or("This position is not the connected wallet's to close.")?;
    let size = typed(amount)
        .filter(|size| *size > 0.0)
        .ok_or("Enter a size above zero (1.5).")?;
    if size > close.size {
        return Err(format!(
            "The position is {} {}: enter that or less.",
            format::quantity(close.size),
            row.instrument
        ));
    }
    Ok(OrderRequest::closing(&row.instrument, row.direction, size))
}

/// Why nothing can be closed on `venue` for the connected account yet, and
/// what there is to do about it; `None` once trading is enabled there
/// (tracked).
fn locked(trading: &Trading, venue: &str) -> Option<String> {
    let name = trading.venue(venue).map_or(venue, |venue| venue.name());
    Some(match trading.access(venue) {
        Access::Ready => return None,
        Access::Resuming => {
            format!("Checking whether trading is still enabled on {name}. Try again in a moment.")
        }
        Access::Enabling => {
            format!("Trading on {name} is waiting for the approval in your wallet.")
        }
        Access::Locked | Access::Failed(_) => format!(
            "Enable trading on {name} to close positions here: press your address in the \
             top bar, then Enable."
        ),
    })
}

/// `account`'s among the reports `arrived`.
fn of<'a, T>(arrived: &'a [(Account, Rc<T>)], account: &Account) -> Option<&'a T> {
    let found = arrived.iter().find(|(of, _)| of == account);
    found.map(|(_, report)| &**report)
}

/// What exchanges price instruments at now, as heard since the reports
/// were made: by exchange, then by instrument.
type Heard = HashMap<String, HashMap<String, f64>>;

/// Follows the price of the instrument of every position in `reports`, the
/// one `kind` names ([`Prices::follow`]), under the current owner:
/// each price lands in `prices` and `heard` is told; one whose stream
/// stopped is taken out again, so its positions show as reported. An
/// instrument no position is in any more is let go, with its price; when
/// another kind of price is asked for, every instrument is followed anew
/// and what was heard of the kind before is forgotten. Every hold goes
/// when the owner is cleaned up.
fn follow_prices(
    source: &Prices,
    reports: Reports<MarginAccount>,
    kind: Memo<PriceKind>,
    prices: &Rc<RefCell<Heard>>,
    heard: &Rc<dyn Fn()>,
) {
    type Held = (String, String);
    type Followed = (String, String, PriceKind);
    let feeds: Rc<Keyed<Followed, Option<PriceFeed>>> = Rc::default();
    {
        let feeds = feeds.clone();
        cleanup(move || {
            feeds.follow(
                std::iter::empty::<(Followed, Held)>(),
                |_, _| true,
                |_| None,
            );
        });
    }
    // The instruments held, each once, in the order they are met.
    let held = computed(move || {
        let mut held: Vec<Held> = Vec::new();
        for (account, margin) in reports.arrived().unwrap_or_default() {
            for position in &margin.positions {
                let key = (account.venue.clone(), position.instrument.clone());
                if !held.contains(&key) {
                    held.push(key);
                }
            }
        }
        held
    });
    let (source, prices, heard) = (source.clone(), prices.clone(), heard.clone());
    let before = Cell::new(None::<PriceKind>);
    effect(move || {
        let (held, kind) = (held.get(), kind.get());
        untracked(|| {
            if before
                .replace(Some(kind))
                .is_some_and(|before| before != kind)
            {
                // Another kind of price: those heard so far are not it.
                prices.borrow_mut().clear();
                heard();
            }
            for (venue, prices) in prices.borrow_mut().iter_mut() {
                prices.retain(|instrument, _| {
                    let same = |(of, held): &Held| of == venue && held == instrument;
                    held.iter().any(same)
                });
            }
            feeds.follow(
                held.into_iter().map(|(venue, instrument)| {
                    (
                        (venue.clone(), instrument.clone(), kind),
                        (venue, instrument),
                    )
                }),
                |_, _| true,
                |(venue, instrument)| {
                    let (prices, heard) = (prices.clone(), heard.clone());
                    let (of, held) = (venue.clone(), instrument.clone());
                    source.follow(venue, instrument, kind, move |price| {
                        {
                            let mut prices = prices.borrow_mut();
                            match price {
                                Some(price) => {
                                    let known = prices.entry(of.clone()).or_default();
                                    known.insert(held.clone(), price);
                                }
                                None => {
                                    if let Some(known) = prices.get_mut(&of) {
                                        known.remove(&held);
                                    }
                                }
                            }
                        }
                        heard();
                    })
                },
            );
        });
    });
}

/// How much of a position a press starts closing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Take {
    Half,
    Whole,
}

/// A Positions widget's session state: its accounts' reports, what its
/// view shows of them, and the position being closed (its watches live in
/// the effects `follow_all` starts).
pub(crate) struct PositionsSession {
    /// The exchanges that report accounts, which its account's rows list.
    pub(super) portfolio: Portfolio,
    /// What closes a position: orders for the connected wallet's account.
    trading: Trading,
    /// The margin report of each account the settings name, and where
    /// they stand as one.
    pub(super) reports: Reports<MarginAccount>,
    /// The rows, in the table's order, updated while the widget is visible:
    /// with each report, and with the prices heard since, once a frame.
    pub(super) rows: Signal<Shared<Vec<Row>>>,
    /// What the rows come to together, updated with them.
    pub(super) totals: Signal<Totals>,
    /// The price the positions are valued at since their report, as the
    /// settings say: the live one, or the exchange's mark.
    pub(super) pricing: Memo<PriceKind>,
    /// The row whose position is being closed (its key): its cell holds
    /// the size to close instead of the buttons that start it.
    pub(super) closing: Signal<Option<Rc<str>>>,
    /// How much of it to close, as typed.
    pub(super) amount: Signal<String>,
    /// Whether an order that closes one is on its way.
    pub(super) sending: Signal<bool>,
    /// What the last one came to, or why none was sent.
    pub(super) said: Signal<Option<Said>>,
    /// What waits for the exchange's answers. Dropped with the session:
    /// what was sent stays sent.
    tasks: TaskSet,
}

impl PositionsSession {
    /// A session following `settings`, the session's copy of the tab's.
    pub(super) fn new(
        scope: SessionScope,
        portfolio: Portfolio,
        prices: Prices,
        trading: Trading,
        settings: Signal<PositionsSettings>,
    ) -> Self {
        let accounts = computed(move || settings.with(|settings| settings.accounts().to_vec()));
        let reports = {
            let portfolio = portfolio.clone();
            follow_all(scope, accounts, move |account| {
                portfolio.watch_margin(account)
            })
        };

        // The accounts that are more than read here: the connected
        // wallet's own, on an exchange that can be traded on.
        let own = {
            let trading = trading.clone();
            computed(move || {
                accounts.with(|accounts| {
                    let own = |account: &&Account| {
                        trading.venue(&account.venue).is_some()
                            && trading.account(&account.venue).as_ref() == Some(*account)
                    };
                    accounts.iter().filter(own).cloned().collect::<Vec<_>>()
                })
            })
        };
        // What each of their exchanges trades: how fine a size may be,
        // which half a position is cut to.
        let listings = {
            let trading = trading.clone();
            follow_all(scope, own, move |account| {
                trading.watch_instruments(&account.venue)
            })
        };
        // Their open orders: those that wait to close a position show as
        // its take profit and its stop loss.
        let orders = {
            let trading = trading.clone();
            follow_all(scope, own, move |account| {
                trading.watch_open_orders(account)
            })
        };
        // They are followed live from the moment the wallet connects,
        // trading enabled or not: their positions and orders are current,
        // and show what changed them as the exchange says it. Held by the
        // session, hidden or not.
        {
            let trading = trading.clone();
            let following = Keyed::<(Account, bool), Option<Following>>::default();
            effect(move || {
                // Asked for again when trading comes to be enabled for one
                // or stops being: an exchange may stream an account only to
                // what the account allowed, and has no stream to give
                // before.
                let wanted: Vec<((Account, bool), Account)> = own.with(|own| {
                    let keyed = |account: &Account| {
                        let enabled = trading.access(&account.venue) == Access::Ready;
                        ((account.clone(), enabled), account.clone())
                    };
                    own.iter().map(keyed).collect()
                });
                untracked(|| {
                    following.follow(
                        wanted,
                        |_, _| true,
                        // Under the session, so its topics are the widget's.
                        |account| scope.run(|| trading.follow(account, |_| {})).ok(),
                    );
                });
            });
        }
        // Those of them that trading is enabled for.
        let tradable = {
            let trading = trading.clone();
            computed(move || {
                own.with(|own| {
                    let enabled =
                        |account: &&Account| trading.access(&account.venue) == Access::Ready;
                    own.iter().filter(enabled).cloned().collect::<Vec<_>>()
                })
            })
        };

        // The price the positions are valued at since their report.
        let pricing = computed(move || settings.with(PositionsSettings::price));
        let rows = signal(Shared::new(Vec::new()));
        let totals = signal(Totals::default());
        let closing = signal(None::<Rc<str>>);
        {
            let portfolio = portfolio.clone();
            scope.while_visible(move || {
                let marks: Rc<RefCell<Heard>> = Rc::default();
                // A price was heard since the rows were made. The first one
                // wakes the effect below, which makes them again on the next
                // frame however many more are heard before it: a busy
                // market's price is said many times a second, and an
                // exchange says every instrument's mark at the same moment.
                // A frame is all that is waited: a price heard shows on the
                // next one.
                let stale = Rc::new(Cell::new(false));
                let wake = signal(0_u64);
                let heard: Rc<dyn Fn()> = {
                    let (stale, wakes) = (stale.clone(), Cell::new(0_u64));
                    Rc::new(move || {
                        if !stale.replace(true) {
                            wakes.set(wakes.get().wrapping_add(1));
                            wake.try_set(wakes.get());
                        }
                    })
                };
                follow_prices(&prices, reports, pricing, &marks, &heard);

                let make: Rc<dyn Fn()> = {
                    let (portfolio, marks, stale) =
                        (portfolio.clone(), marks.clone(), stale.clone());
                    Rc::new(move || {
                        stale.set(false);
                        // A position says whose it is once there are several
                        // accounts to be of.
                        let several = accounts.with(|accounts| accounts.len() > 1);
                        let arrived = reports.arrived().unwrap_or_default();
                        let listed = listings.arrived().unwrap_or_default();
                        let waiting = orders.arrived().unwrap_or_default();
                        let own = own.get();
                        let (made, sums) = {
                            let marks = marks.borrow();
                            let parts: Vec<Part> = arrived
                                .iter()
                                .map(|(account, margin)| Part {
                                    mark: Mark::when(several, &portfolio, account),
                                    positions: &margin.positions[..],
                                    prices: marks.get(&account.venue),
                                    desk: own.contains(account).then(|| Desk {
                                        venue: &account.venue,
                                        instruments: of(&listed, account),
                                        orders: of(&waiting, account),
                                    }),
                                })
                                .collect();
                            table_of(&parts)
                        };
                        // A position that is gone, or no longer the wallet's
                        // to close, is not being closed.
                        untracked(|| {
                            if let Some(key) = closing.try_get().flatten()
                                && !made.iter().any(|row| row.key == key && row.close.is_some())
                            {
                                closing.try_set(None);
                            }
                        });
                        rows.try_set(Shared::new(made));
                        totals.try_set(sums);
                    })
                };
                // Each report makes the rows again, at once.
                {
                    let make = make.clone();
                    effect(move || make());
                }
                // And so do the prices heard since, once a frame.
                effect(move || {
                    wake.get();
                    if stale.get() {
                        let make = make.clone();
                        spawn(async move {
                            next_frame().await;
                            make();
                        });
                    }
                });
            });
        }
        let said = signal(None);
        // What was said for one account, or while trading was not enabled,
        // is not said once that has changed.
        effect(move || {
            let _ = tradable.get();
            untracked(|| said.set(None));
        });

        Self {
            portfolio,
            trading,
            reports,
            rows,
            totals,
            pricing,
            closing,
            amount: signal(String::new()),
            sending: signal(false),
            said,
            tasks: TaskSet::default(),
        }
    }

    /// The row whose key is `key`, as it is now.
    fn row(&self, key: &str) -> Option<Row> {
        self.rows
            .with_untracked(|rows| rows.0.iter().find(|row| &*row.key == key).cloned())
    }

    /// Says `text` over the table.
    fn say(&self, text: impl Into<String>, voice: Voice) {
        self.said.set(Some(Said {
            text: text.into(),
            voice,
        }));
    }

    /// Starts closing `take` of the position of the row `key`: its cell
    /// becomes the size to close, filled in, with the cell that sends it.
    /// While trading is not enabled for the account on its exchange, what
    /// there is to do about that is said instead.
    pub(super) fn start(&self, key: &Rc<str>, take: Take) {
        let Some(close) = self.row(key).and_then(|row| row.close) else {
            return;
        };
        if let Some(why) = untracked(|| locked(&self.trading, &close.venue)) {
            self.closing.set(None);
            return self.say(why, Voice::Waiting);
        }
        let size = match take {
            Take::Whole => Some(close.size),
            Take::Half => close.half,
        };
        // Written as it would be typed: no grouping, every digit.
        self.amount
            .set(size.map_or_else(String::new, |size| format!("{size}")));
        self.said.set(None);
        self.closing.set(Some(key.clone()));
    }

    /// Lets go of the position being closed, with nothing sent.
    pub(super) fn cancel(&self) {
        self.closing.set(None);
    }

    /// Sends the order that closes the size typed of the position being
    /// closed; what is wrong with what was typed is said instead.
    pub(super) fn send(&self) {
        if self.sending.get_untracked() {
            return;
        }
        let closing = self.closing;
        let Some(row) = closing.get_untracked().and_then(|key| self.row(&key)) else {
            return;
        };
        let Some(venue) = row.close.as_ref().map(|close| close.venue.clone()) else {
            return;
        };
        let order = match closing_order(&row, &self.amount.get_untracked()) {
            Ok(order) => order,
            Err(why) => return self.say(why, Voice::Failed),
        };
        self.said.set(None);
        self.sending.set(true);
        let placing = self.trading.place(&venue, order.clone());
        let (sending, said, key) = (self.sending, self.said, row.key);
        self.tasks.spawn(async move {
            let came = match placing.await {
                Ok(placed) => {
                    // Sent: the cell is the row's buttons again, unless
                    // another row's was started meanwhile. The session may
                    // be gone by now.
                    if closing.try_get().flatten().as_deref() == Some(&*key) {
                        closing.try_set(None);
                    }
                    Said {
                        text: placed_line(&order, &placed),
                        voice: Voice::Passed,
                    }
                }
                Err(error) => Said {
                    text: error.message,
                    voice: Voice::Failed,
                },
            };
            sending.try_set(false);
            said.try_set(Some(came));
        });
    }
}
