//! The Trade Stream: the latest prints of one or more markets, newest
//! first, with each market's buys and sells and the trades-per-second
//! rate (the old app's `TradeStream` and `TradeStreamController`).
//!
//! Data (the widget's session, [`Stream`]):
//!
//! - It holds one lease per market its settings watch on the market's
//!   shared trade feed ([`Ferris::trades`]): two streams of one market
//!   share one topic, one snapshot and one buffer. A market the settings
//!   drop is let go; one they keep keeps its feed and prints. A hidden
//!   stream keeps streaming, so showing it again is instant.
//! - Feeds tell it of each change from the socket's callback; it only
//!   marks itself dirty there. While it shows, it merges the feeds' prints
//!   into the rows its view shows at most once a frame, and samples the
//!   rate four times a second. A hidden stream does none of it.
//!
//! Settings ([`TradeStreamSettings`]: the markets, how many prints show)
//! are saved in its tab record; the session keeps a copy the view brings
//! up to date, so what it streams follows them while no view shows it.
//! The pane's gear opens its settings panel ([`panel`]), whose markets
//! come from the market picker.

mod panel;
mod settings;
mod view;

pub(crate) use settings::TradeStreamSettings;

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

use nsh_core::{prelude::*, time::unix_ms};

use crate::{
    data::{
        DataServices, SessionScope, WidgetView,
        ferris::{Ferris, MarketSelection, TradeFeed, TradeRow},
    },
    layout::TabContext,
};

/// How often the trades-per-second rate is sampled while shown.
const RATE_EVERY: Duration = Duration::from_millis(250);

/// The stream's view of `widget`: its rows, from its session, for the
/// markets its tab's settings watch.
pub(crate) fn trade_stream(widget: &WidgetView, tab: &TabContext<TradeStreamSettings>) -> View {
    let Some(services) = DataServices::try_get() else {
        log::error!("Trade Stream: no data services above the widget");
        return view().flex_1();
    };
    let saved = tab.settings;
    let stream =
        widget.state(|scope| Stream::new(scope, services.ferris.clone(), saved.get_untracked()));
    // The tab record is where the settings live; the session follows it.
    let session = stream.settings;
    effect(move || {
        let next = saved.get();
        if session.with_untracked(|current| *current != next) {
            session.set(next);
        }
    });
    let panel = {
        let tab = tab.clone();
        dynamic(move || tab.settings_open().then(|| panel::panel(tab.clone())))
    };
    view::stream_view(&stream, saved).child(panel)
}

/// A value shared by reference, equal only to itself: what a signal of a
/// value rebuilt whole (the rows shown) holds.
struct Shared<T>(Rc<T>);

impl<T> Clone for Shared<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T> PartialEq for Shared<T> {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

/// One print the view shows, and its market.
#[derive(Clone)]
struct Row {
    /// The market's slot in the session (what keys the row with the
    /// print's sequence: two markets' prints never share a key).
    slot: u64,
    market: Rc<MarketSelection>,
    trade: Rc<TradeRow>,
}

impl Row {
    fn key(&self) -> (u64, u64) {
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
struct Shown {
    rows: Vec<Row>,
    /// Some market has had no snapshot and no print yet.
    loading: bool,
    /// The markets whose topic the server refused.
    failed: Vec<Rc<MarketSelection>>,
}

/// One market the session watches.
struct Watched {
    identity: String,
    slot: u64,
    market: Rc<MarketSelection>,
    feed: TradeFeed,
}

/// A stream's session state: its feeds and settings, and what its view
/// shows.
struct Stream {
    /// The tab's settings, as the view last saw them.
    settings: Signal<TradeStreamSettings>,
    /// The rows the view shows, updated while the widget is visible.
    shown: Signal<Shared<Shown>>,
    /// Live prints per second across the markets, while visible.
    rate: Signal<f64>,
}

impl Stream {
    fn new(scope: SessionScope, ferris: Ferris, saved: TradeStreamSettings) -> Self {
        let settings = signal(saved);
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

        Self {
            settings,
            shown,
            rate,
        }
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
