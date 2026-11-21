//! Fast mode: a candle chart whose newest candle is made from its market's
//! trades, and the badge that turns it on.
//!
//! A venue's trades reach the app sooner and more often than its candles
//! do, so the candle still forming lags behind the prints that make it. In
//! fast mode the chart leases its market's shared trade feed beside its
//! candle feed ([`FastBars`]) and lays what the prints say over the newest
//! bar ([`Forming`]):
//!
//! - The venue's candles stay the history. Only the candle still forming,
//!   and the ones after it the venue has not sent yet, are made from
//!   prints: a print before the venue's newest candle changes nothing, and
//!   once the venue sends a later candle the ones before it are the
//!   venue's again.
//! - Prints go in buckets one timeframe long, counted from the venue's
//!   newest candle (so they are on the venue's grid, whatever day a week
//!   starts on there).
//! - The forming candle keeps the venue's open and volume; its high and low
//!   reach the prints', and its close is the latest print's.
//! - A bucket after it, which the venue has not opened yet, is a candle of
//!   its prints alone: it opens at the first, closes at the last, and its
//!   volume is theirs. The venue's candle replaces it when it comes.
//! - Should the trades fall silent while the venue's candle still moves
//!   ([`SILENT_FOR_MS`] after the last print), the venue's close is the
//!   newer one and shows, until the next print.
//!
//! - A forced close (a liquidation) is not placed: a venue may report it
//!   at a price nothing traded at, and the venue's candle has what it
//!   traded at if it did.
//!
//! The trade feed keeps only its newest prints, so they are taken in as
//! they arrive (in the feeds' callbacks), whether or not the chart shows.
//!
//! The badge ([`badge`]) is the third beside the plot's header
//! ([`badges`]), right of the indicators': it says `FAST MODE`, is lit
//! while the mode is on, and a press turns it on or off (the chart's
//! `fastMode` setting). A tick chart is its market's trades already, so it
//! has no such badge.

use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
};

use nsh_core::{prelude::*, time::unix_ms};

use super::{ChartSettings, badges, settings::Interval};
use crate::{
    data::ferris::{Candle, CandleFeed, Ferris, MarketSelection, Timeframe, TradeFeed},
    ui,
    widgets::TabContext,
};

/// How long after the last print a change of the venue's forming candle
/// says the trades are behind it, in milliseconds.
const SILENT_FOR_MS: f64 = 2_000.0;
/// How far past the clock a print's time may be, in seconds: one stamped
/// later is not placed.
const AHEAD: f64 = 60.0;
/// The candles kept that the venue has not sent, at most.
const MAX_BUILT: usize = 1_000;

/// One bucket's prints.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Built {
    /// When the bucket opens, in Unix seconds.
    time: f64,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    volume: f64,
    /// When the prints its open and its close are from printed.
    first: f64,
    last: f64,
}

/// The candles a market's prints make from the venue's newest candle on.
#[derive(Debug)]
pub(super) struct Forming {
    /// A candle's length, in seconds.
    step: f64,
    /// The venue's newest candle, as last told.
    newest: Option<Candle>,
    /// When it last changed and when the last print was placed (Unix ms).
    feed_at: f64,
    print_at: f64,
    /// The buckets with prints, oldest first, none before `newest`.
    built: Vec<Built>,
}

impl Forming {
    pub fn new(step: f64) -> Self {
        Self {
            step,
            newest: None,
            feed_at: 0.0,
            print_at: 0.0,
            built: Vec::new(),
        }
    }

    /// Whether prints can be placed: the venue has sent a candle to count
    /// their buckets from.
    pub fn anchored(&self) -> bool {
        self.newest.is_some()
    }

    /// The venue's newest candle is `newest`, at `now` (Unix ms): what was
    /// made of prints before it is dropped.
    pub fn feed(&mut self, newest: Option<Candle>, now: f64) {
        if newest == self.newest {
            return;
        }
        // The first candle told of is where the venue was, not a change.
        if self.newest.is_some() {
            self.feed_at = now;
        }
        self.newest = newest;
        match newest {
            Some(newest) => self.built.retain(|built| built.time >= newest.time),
            None => self.built.clear(),
        }
    }

    /// Places a print of `amount` at `price`, printed at `at` (Unix
    /// seconds) and taken in at `now` (Unix ms).
    pub fn print(&mut self, at: f64, price: f64, amount: f64, now: f64) {
        let Some(newest) = self.newest else {
            return;
        };
        let placed = price.is_finite() && price > 0.0 && at >= newest.time;
        if !placed || at > now / 1_000.0 + AHEAD {
            return;
        }
        self.print_at = now;
        let time = newest.time + ((at - newest.time) / self.step).floor() * self.step;
        let index = self.built.partition_point(|built| built.time < time);
        match self.built.get_mut(index) {
            Some(built) if built.time == time => {
                built.high = built.high.max(price);
                built.low = built.low.min(price);
                built.volume += amount;
                if at < built.first {
                    (built.first, built.open) = (at, price);
                }
                if at >= built.last {
                    (built.last, built.close) = (at, price);
                }
            }
            _ => {
                self.built.insert(
                    index,
                    Built {
                        time,
                        open: price,
                        high: price,
                        low: price,
                        close: price,
                        volume: amount,
                        first: at,
                        last: at,
                    },
                );
                let extra = self.built.len().saturating_sub(MAX_BUILT);
                self.built.drain(..extra);
            }
        }
    }

    /// Lays what the prints made over `bars`, the venue's candles: over
    /// the newest, and after it.
    pub fn apply(&self, bars: &mut Vec<Candle>) {
        let Some(newest) = bars.last().map(|bar| bar.time) else {
            return;
        };
        // The venue's candle moved since the trades fell silent.
        let behind = self.feed_at - self.print_at > SILENT_FOR_MS;
        for built in self.built.iter().filter(|built| built.time >= newest) {
            match bars.last_mut() {
                Some(bar) if bar.time == built.time => {
                    bar.high = bar.high.max(built.high);
                    bar.low = bar.low.min(built.low);
                    if !behind {
                        bar.close = built.close;
                    }
                }
                _ => bars.push(Candle {
                    time: built.time,
                    open: built.open,
                    high: built.high,
                    low: built.low,
                    close: built.close,
                    volume: Some(built.volume),
                }),
            }
        }
    }
}

/// A market's candles at a timeframe with the newest made from its
/// trades: a lease on each feed, and the bars they make together.
pub(super) struct FastBars {
    candles: CandleFeed,
    trades: TradeFeed,
    forming: RefCell<Forming>,
    /// The newest print taken in, by the trade feed's arrival number.
    taken: Cell<Option<u64>>,
    /// The venue's candles with the prints' over them, as last made, and
    /// how many of them are the venue's.
    bars: RefCell<Vec<Candle>>,
    venue: Cell<usize>,
    /// A candle or a print changed since.
    stale: Cell<bool>,
}

impl FastBars {
    /// The bars of `market` at `timeframe`; `on_change` hears each change
    /// of either feed after its prints are taken in.
    pub fn open(
        ferris: &Ferris,
        market: &MarketSelection,
        timeframe: Timeframe,
        on_change: Rc<dyn Fn()>,
    ) -> Rc<Self> {
        let fast = Rc::new_cyclic(|weak: &Weak<Self>| {
            let heard = |on_change: Rc<dyn Fn()>| {
                let weak = weak.clone();
                move || {
                    if let Some(fast) = weak.upgrade() {
                        fast.take();
                    }
                    on_change();
                }
            };
            Self {
                candles: ferris.candles(market, timeframe, heard(on_change.clone())),
                trades: ferris.trades(market, heard(on_change)),
                forming: RefCell::new(Forming::new(timeframe.seconds())),
                taken: Cell::new(None),
                bars: RefCell::new(Vec::new()),
                venue: Cell::new(0),
                stale: Cell::new(true),
            }
        });
        fast.take();
        fast
    }

    /// Its candle feed: the history, its status and its older pages.
    pub fn candles(&self) -> &CandleFeed {
        &self.candles
    }

    /// Takes in the venue's newest candle and the prints that arrived
    /// since the last call. Prints wait in the trade feed until the venue
    /// has sent a candle.
    fn take(&self) {
        self.stale.set(true);
        let now = unix_ms();
        let mut forming = self.forming.borrow_mut();
        let newest = self.candles.with_candles(|bars| bars.last().copied());
        forming.feed(newest, now);
        if !forming.anchored() {
            return;
        }
        let taken = self.taken.get();
        let newest = self.trades.with_rows(|rows| {
            let fresh = rows
                .iter()
                .filter(|row| taken.is_none_or(|taken| row.sequence > taken));
            fresh
                .map(|row| {
                    let trade = &row.trade;
                    if !trade.liquidation {
                        forming.print(trade.timestamp / 1_000.0, trade.price, trade.amount, now);
                    }
                    row.sequence
                })
                .max()
        });
        if newest.is_some() {
            self.taken.set(newest);
        }
    }

    /// Makes the bars again, if a candle or a print changed since.
    fn refresh(&self) {
        if !self.stale.replace(false) {
            return;
        }
        let mut bars = self.bars.borrow_mut();
        bars.clear();
        self.candles
            .with_candles(|venue| bars.extend_from_slice(venue));
        self.venue.set(bars.len());
        self.forming.borrow().apply(&mut bars);
    }

    /// Its bars, oldest first.
    pub fn with_bars<R>(&self, f: impl FnOnce(&[Candle]) -> R) -> R {
        self.refresh();
        f(&self.bars.borrow())
    }

    /// How many bars were ever added after its newest: the venue's, and
    /// the ones of prints the venue has not sent yet (each is counted once:
    /// the venue's takes its place).
    pub fn appended(&self) -> u64 {
        self.refresh();
        let ahead = self.bars.borrow().len() - self.venue.get();
        self.candles.appended() + ahead as u64
    }
}

/// What the badge says, and how wide it is.
const WORD: &str = "FAST MODE";
pub(super) const BADGE_WIDTH: f32 = badges::width(WORD.len(), 0.0);

/// What the badge is named.
const NAME: &str = "Fast mode";

/// The badge that turns fast mode on and off for the chart in `tab`,
/// `left` points from the plot's view's left edge; none on a tick chart.
pub(super) fn badge(tab: &TabContext<ChartSettings>, left: impl Fn() -> f32 + 'static) -> View {
    let (saved, tab) = (tab.settings, tab.clone());
    let on = move || saved.with(ChartSettings::fast);
    let ticks = move || saved.with(|settings| settings.interval() == Interval::Tick);
    let press = move || {
        let settings = saved.get_untracked();
        let fast = !settings.fast();
        tab.save(&settings.with_fast(fast));
    };
    badges::switch(left, || BADGE_WIDTH, on, press)
        .label(NAME)
        .test_id("chart-fast")
        .tooltip(ui::hint(|| {
            "Make the newest candle from live trades".to_owned()
        }))
        .style_when(ticks, |s| s.hidden())
        .child(text(WORD).number_of_lines(1))
}
