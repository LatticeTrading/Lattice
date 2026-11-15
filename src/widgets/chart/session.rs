//! The chart's session ([`ChartSession`]): the feed its settings chart and
//! its plot, whose viewport and scene its view shows (the widget's data,
//! described in the parent module), and when its series' slots are, which
//! its drawings are placed by ([`ChartSession::with_times`]).

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use nsh_core::prelude::*;

use super::{ChartSettings, settings::Interval};
use crate::{
    data::{
        SessionScope,
        ferris::{Candle, CandleFeed, Ferris, MarketSelection, Timeframe, TradeFeed},
    },
    widgets::common::plot::{DEFAULT_CANDLES, DEFAULT_TICKS, Mode, Plot, Timeline, Viewport},
};

/// The ticks a tick chart keeps (the old chart's bound).
const MAX_TICKS: usize = 25_000;
/// A page of older candles is asked for once the plot's left edge is
/// within this share of its slots of the oldest bar (the old chart's).
const OLDER_WITHIN: f64 = 0.25;

/// What the plot says about its data: in its middle while it has no bar,
/// above its bars otherwise.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Status {
    Ready,
    LoadingHistory,
    NoHistory,
    HistoryUnavailable,
    LiveOnly,
    WaitingForTicks,
    Unavailable,
}

impl Status {
    /// What it says (the old chart's messages), nothing while all is well.
    pub fn text(self) -> &'static str {
        match self {
            Self::Ready => "",
            Self::LoadingHistory => "Loading historical candles...",
            Self::NoHistory => "No historical candles available; waiting for live data...",
            Self::HistoryUnavailable => "Historical fetch unavailable; waiting for candles...",
            Self::LiveOnly => "Showing live candles; historical candles unavailable",
            Self::WaitingForTicks => "Waiting for live ticks...",
            Self::Unavailable => "Live data is unavailable for this market.",
        }
    }
}

/// A tick chart's points: every print of its market's shared trade feed,
/// each a bar of one price, in time order.
///
/// The trade feed keeps only its newest prints, so the ticks are taken in
/// as they arrive (in the feed's callback), whether or not the chart shows.
struct Ticks {
    feed: RefCell<Option<TradeFeed>>,
    bars: RefCell<Vec<Candle>>,
    /// The newest print taken in, by the feed's arrival number.
    taken: Cell<Option<u64>>,
    /// How many ticks were added after the newest one so far.
    appended: Cell<u64>,
}

impl Ticks {
    /// The ticks of `market`, from the prints its feed already has;
    /// `on_change` hears each change after its prints are taken in.
    fn open(ferris: &Ferris, market: &MarketSelection, on_change: Rc<dyn Fn()>) -> Rc<Self> {
        let ticks = Rc::new(Self {
            feed: RefCell::new(None),
            bars: RefCell::new(Vec::new()),
            taken: Cell::new(None),
            appended: Cell::new(0),
        });
        let weak = Rc::downgrade(&ticks);
        let feed = ferris.trades(market, move || {
            if let Some(ticks) = weak.upgrade() {
                ticks.take();
            }
            on_change();
        });
        *ticks.feed.borrow_mut() = Some(feed);
        ticks.take();
        ticks
    }

    /// Takes in the prints that arrived since the last call.
    fn take(&self) {
        let feed = self.feed.borrow();
        let Some(feed) = feed.as_ref() else {
            return;
        };
        let taken = self.taken.get();
        feed.with_rows(|rows| {
            let fresh: Vec<_> = rows
                .iter()
                .filter(|row| taken.is_none_or(|taken| row.sequence > taken))
                .collect();
            let Some(newest) = fresh.iter().map(|row| row.sequence).max() else {
                return;
            };
            self.taken.set(Some(newest));
            // The feed keeps its prints in time order, so these are too.
            let mut bars = self.bars.borrow_mut();
            for row in fresh {
                let price = row.trade.price;
                if !(price.is_finite() && price > 0.0) {
                    continue;
                }
                let time = row.trade.timestamp / 1_000.0;
                let at = bars.partition_point(|kept| kept.time <= time);
                if at == bars.len() {
                    self.appended.set(self.appended.get() + 1);
                }
                bars.insert(
                    at,
                    Candle {
                        time,
                        open: price,
                        high: price,
                        low: price,
                        close: price,
                        volume: Some(row.trade.amount),
                    },
                );
            }
            let extra = bars.len().saturating_sub(MAX_TICKS);
            bars.drain(..extra);
        });
    }

    fn failed(&self) -> bool {
        self.feed.borrow().as_ref().is_some_and(TradeFeed::failed)
    }
}

/// What a chart charts: a market's candles at a timeframe, or its ticks.
enum Source {
    Candles(CandleFeed, Timeframe),
    Ticks(Rc<Ticks>),
}

impl Source {
    /// A lease on `market`'s feed for `interval`; `on_change` hears each
    /// change of its bars or status.
    fn open(
        ferris: &Ferris,
        market: &MarketSelection,
        interval: Interval,
        on_change: Rc<dyn Fn()>,
    ) -> Self {
        match interval {
            Interval::Tick => Self::Ticks(Ticks::open(ferris, market, on_change)),
            Interval::Candles(timeframe) => Self::Candles(
                ferris.candles(market, timeframe, move || on_change()),
                timeframe,
            ),
        }
    }

    fn mode(&self) -> Mode {
        match self {
            Self::Candles(..) => Mode::Candles,
            Self::Ticks(_) => Mode::Line,
        }
    }

    /// Its bars, oldest first.
    fn with_bars<R>(&self, f: impl FnOnce(&[Candle]) -> R) -> R {
        match self {
            Self::Candles(feed, _) => feed.with_candles(f),
            Self::Ticks(ticks) => f(&ticks.bars.borrow()),
        }
    }

    /// How long one of its bars lasts, in seconds; none for ticks, which
    /// come when they come.
    fn step(&self) -> Option<f64> {
        match self {
            Self::Candles(_, timeframe) => Some(timeframe.seconds()),
            Self::Ticks(_) => None,
        }
    }

    /// How many bars were ever added after its newest.
    fn appended(&self) -> u64 {
        match self {
            Self::Candles(feed, _) => feed.appended(),
            Self::Ticks(ticks) => ticks.appended.get(),
        }
    }

    /// What the plot says, with (`empty`) or without bars.
    fn status(&self, empty: bool) -> Status {
        match self {
            Self::Candles(feed, _) if empty => {
                if feed.loading() {
                    Status::LoadingHistory
                } else if feed.failed() {
                    Status::Unavailable
                } else if feed.history_failed() {
                    Status::HistoryUnavailable
                } else {
                    Status::NoHistory
                }
            }
            Self::Candles(feed, _) if feed.history_failed() => Status::LiveOnly,
            Self::Ticks(ticks) if empty && ticks.failed() => Status::Unavailable,
            Self::Ticks(_) if empty => Status::WaitingForTicks,
            _ => Status::Ready,
        }
    }
}

/// A chart's session state: its plot, what its view says about its data,
/// and the feed it charts (which the effects `new` starts keep on its
/// settings).
pub(crate) struct ChartSession {
    /// Its plot. Where it looks starts over when the market or the interval
    /// changes.
    pub(super) plot: Rc<Plot>,
    pub(super) status: Signal<Status>,
    /// A page of older candles is on its way.
    pub(super) loading_older: Signal<bool>,
    /// What it charts; nothing before its first lease.
    source: Rc<RefCell<Option<Source>>>,
}

impl ChartSession {
    /// A chart following `settings`, the session's copy of the tab's.
    pub(super) fn new(
        scope: SessionScope,
        ferris: Ferris,
        settings: Signal<ChartSettings>,
    ) -> Self {
        let plot = Plot::new(Mode::Candles, DEFAULT_CANDLES);
        let status = signal(Status::LoadingHistory);
        let loading_older = signal(false);

        let source: Rc<RefCell<Option<Source>>> = Rc::default();
        // The source's `appended` when the scene was last made.
        let appended = Rc::new(Cell::new(0_u64));

        // The feed follows the market and interval the settings chart.
        let market = computed(move || settings.with(ChartSettings::market));
        let interval = computed(move || settings.with(ChartSettings::interval));
        {
            let (source, appended, plot) = (source.clone(), appended.clone(), plot.clone());
            effect(move || {
                let (market, interval) = (market.get(), interval.get());
                untracked(|| {
                    // Leased before the old one goes, so what they share
                    // (a market's trades) keeps streaming.
                    let next = Source::open(&ferris, &market, interval, plot.marker());
                    appended.set(next.appended());
                    let previous = source.borrow_mut().replace(next);
                    drop(previous);
                    plot.viewport.set(Viewport::showing(match interval {
                        Interval::Tick => DEFAULT_TICKS,
                        Interval::Candles(_) => DEFAULT_CANDLES,
                    }));
                });
                plot.mark();
            });
        }

        {
            let (plot, source) = (plot.clone(), source.clone());
            // The plot's `moves` a page of older candles was last asked for
            // at: they are asked for once per move of the viewport, as the
            // old chart did, so a plot resting at the oldest bar does not
            // page through a venue's whole history.
            let asked = Rc::new(Cell::new(0_u64));
            scope.while_visible(move || {
                let (source, appended, asked) = (source.clone(), appended.clone(), asked.clone());
                let drawn = plot.clone();
                plot.drive(move || {
                    let plot = &drawn;
                    let source = source.borrow();
                    let Some(source) = source.as_ref() else {
                        return;
                    };
                    let now = source.appended();
                    let added = now.saturating_sub(appended.replace(now));
                    let ((held, next), said) = source.with_bars(|bars| {
                        (
                            plot.frame(bars, added, source.mode()),
                            source.status(bars.is_empty()),
                        )
                    });
                    if let Source::Candles(feed, _) = source {
                        let near_oldest = next.left <= (next.right - next.left) * OLDER_WITHIN;
                        if next.bars > 0 && near_oldest && asked.get() != plot.moves() {
                            asked.set(plot.moves());
                            feed.load_older();
                        }
                    }
                    let older = match source {
                        Source::Candles(feed, _) => feed.loading_older(),
                        Source::Ticks(_) => false,
                    };
                    batch(|| {
                        plot.show(held, next);
                        status.try_set(said);
                        loading_older.try_set(older);
                    });
                });
            });
        }

        Self {
            plot,
            status,
            loading_older,
            source,
        }
    }

    /// Calls `f` with when the slots of the series it charts are (what
    /// places its drawings); nothing before its first lease. `f` only
    /// reads: the series is borrowed while it runs.
    pub(super) fn with_times<R>(&self, f: impl FnOnce(Timeline<'_>) -> R) -> Option<R> {
        let source = self.source.borrow();
        let source = source.as_ref()?;
        Some(source.with_bars(|bars| f(Timeline::new(bars, source.step()))))
    }
}
