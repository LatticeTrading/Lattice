//! The Trade Stream's session ([`Stream`]): one feed lease per market its
//! settings watch, and the rows its view shows (the widget's data,
//! described in the parent module).

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

use nsh_core::{prelude::*, time::unix_ms};

use super::TradeStreamSettings;
use crate::{
    data::{
        SessionScope,
        ferris::{Ferris, MarketSelection, TradeFeed, TradeRow},
    },
    widgets::common::Shared,
};

/// How often the trades-per-second rate is sampled while shown.
const RATE_EVERY: Duration = Duration::from_millis(250);

/// One print the view shows, and its market.
#[derive(Clone)]
pub(super) struct Row {
    /// The market's slot in the session (what keys the row with the
    /// print's sequence: two markets' prints never share a key).
    slot: u64,
    pub market: Rc<MarketSelection>,
    pub trade: Rc<TradeRow>,
}

impl Row {
    pub fn key(&self) -> (u64, u64) {
        (self.slot, self.trade.sequence)
    }
}

impl PartialEq for Row {
    fn eq(&self, other: &Self) -> bool {
        self.slot == other.slot && Rc::ptr_eq(&self.trade, &other.trade)
    }
}

/// What the view shows: the newest prints of every market, newest first.
#[derive(Default)]
pub(super) struct Shown {
    pub rows: Vec<Row>,
    /// Some market has had no snapshot and no print yet.
    pub loading: bool,
    /// The markets whose topic the server refused.
    pub failed: Vec<Rc<MarketSelection>>,
}

/// One market the session watches.
struct Watched {
    identity: String,
    slot: u64,
    market: Rc<MarketSelection>,
    feed: TradeFeed,
}

/// A stream's session state: what its view shows (its feeds and settings
/// live in the effects `new` starts).
pub(crate) struct Stream {
    /// The rows the view shows, updated while the widget is visible.
    pub(super) shown: Signal<Shared<Shown>>,
    /// Live prints per second across the markets, while visible.
    pub(super) rate: Signal<f64>,
}

impl Stream {
    /// A stream following `settings`, the session's copy of the tab's.
    pub(super) fn new(
        scope: SessionScope,
        ferris: Ferris,
        settings: Signal<TradeStreamSettings>,
    ) -> Self {
        let shown = signal(Shared(Rc::new(Shown {
            loading: true,
            ..Shown::default()
        })));
        let rate = signal(0.0_f64);
        let watched: Rc<RefCell<Vec<Watched>>> = Rc::default();
        // Prints arrived (or the markets changed) since the rows were made.
        let dirty = Rc::new(Cell::new(true));
        // Bumped when `dirty` is set, so the visible view schedules a frame.
        let wake = signal(0_u64);
        let mark: Rc<dyn Fn()> = {
            let (dirty, wakes) = (dirty.clone(), Cell::new(0_u64));
            Rc::new(move || {
                if !dirty.replace(true) {
                    wakes.set(wakes.get().wrapping_add(1));
                    wake.try_set(wakes.get());
                }
            })
        };

        // The feeds follow the markets the settings watch.
        let markets = computed(move || settings.with(TradeStreamSettings::markets));
        let max_trades = computed(move || settings.with(TradeStreamSettings::max_trades));
        {
            let (watched, mark) = (watched.clone(), mark.clone());
            let next_slot = Cell::new(0_u64);
            effect(move || {
                let markets = markets.get();
                untracked(|| follow_markets(&ferris, &watched, &markets, &mark, &next_slot));
                mark();
            });
        }
        {
            let mark = mark.clone();
            effect(move || {
                max_trades.get();
                mark();
            });
        }

        scope.while_visible(move || {
            let refresh: Rc<dyn Fn()> = {
                let (watched, dirty) = (watched.clone(), dirty.clone());
                Rc::new(move || {
                    dirty.set(false);
                    let next = merge(&watched.borrow(), max_trades.get_untracked());
                    shown.try_set(Shared(Rc::new(next)));
                })
            };
            refresh();
            {
                let dirty = dirty.clone();
                effect(move || {
                    wake.get();
                    if dirty.get() {
                        let refresh = refresh.clone();
                        spawn(async move {
                            next_frame().await;
                            refresh();
                        });
                    }
                });
            }
            let watched = watched.clone();
            spawn(async move {
                loop {
                    let now = unix_ms();
                    let total = watched
                        .borrow()
                        .iter()
                        .map(|watched| watched.feed.trades_per_second(now))
                        .sum::<f64>();
                    rate.try_set(total);
                    timer(RATE_EVERY).await;
                }
            });
        });

        Self { shown, rate }
    }
}

/// Keeps one feed lease per market in `markets`, in their order: a market
/// already watched keeps its lease (and its slot), a new one gets a lease
/// that marks the session dirty on each change, and the rest are let go.
fn follow_markets(
    ferris: &Ferris,
    watched: &RefCell<Vec<Watched>>,
    markets: &[MarketSelection],
    mark: &Rc<dyn Fn()>,
    next_slot: &Cell<u64>,
) {
    let mut previous = std::mem::take(&mut *watched.borrow_mut());
    let next: Vec<Watched> = markets
        .iter()
        .map(|market| {
            let identity = market.identity();
            if let Some(at) = previous.iter().position(|kept| kept.identity == identity) {
                let mut kept = previous.swap_remove(at);
                kept.market = Rc::new(market.clone());
                return kept;
            }
            let slot = next_slot.get();
            next_slot.set(slot + 1);
            let mark = mark.clone();
            Watched {
                identity,
                slot,
                market: Rc::new(market.clone()),
                feed: ferris.trades(market, move || mark()),
            }
        })
        .collect();
    *watched.borrow_mut() = next;
    // Markets no longer watched: their leases end here.
    drop(previous);
}

/// The newest `max` prints across `watched`, newest first (ties in the
/// markets' order, then arrival).
fn merge(watched: &[Watched], max: usize) -> Shown {
    let mut shown = Shown::default();
    for market in watched {
        shown.loading |= market.feed.loading();
        if market.feed.failed() {
            shown.failed.push(market.market.clone());
        }
        market.feed.with_rows(|rows| {
            shown
                .rows
                .extend(rows.iter().rev().take(max).map(|trade| Row {
                    slot: market.slot,
                    market: market.market.clone(),
                    trade: trade.clone(),
                }));
        });
    }
    if watched.len() > 1 {
        shown
            .rows
            .sort_by(|a, b| b.trade.trade.timestamp.total_cmp(&a.trade.trade.timestamp));
    }
    shown.rows.truncate(max);
    shown
}
