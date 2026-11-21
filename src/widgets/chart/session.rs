//! The chart's session ([`ChartSession`]): the feed its settings chart
//! (candles, candles whose newest is made from trades in fast mode,
//! [`FastBars`], or ticks) and its plot, whose viewport and scene its view
//! shows (the widget's data, described in the parent module), its
//! indicators over that feed's bars ([`Computed`]), and when its series'
//! slots are, which its drawings are placed by
//! ([`ChartSession::with_times`]).

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use nsh_core::prelude::*;

use super::{ChartSettings, fast::FastBars, indicators::Computed, settings::Interval};
use crate::{
    data::{
        SessionScope,
        ferris::{Candle, CandleFeed, Ferris, MarketSelection, Timeframe, TradeFeed},
    },
    widgets::common::{
        Shared,
        plot::{DEFAULT_CANDLES, DEFAULT_TICKS, Mode, Plot, Timeline, Viewport},
    },
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

/// What a chart charts: a market's candles at a timeframe, those with the
/// newest made from its trades (fast mode), or its ticks.
enum Source {
    Candles(CandleFeed, Timeframe),
    Fast(Rc<FastBars>, Timeframe),
    Ticks(Rc<Ticks>),
}

impl Source {
    /// A lease on `market`'s feed for `interval`, in fast mode if `fast`
    /// (a candle chart's); `on_change` hears each change of its bars or
    /// status.
    fn open(
        ferris: &Ferris,
        market: &MarketSelection,
        interval: Interval,
        fast: bool,
        on_change: Rc<dyn Fn()>,
    ) -> Self {
        match interval {
            Interval::Tick => Self::Ticks(Ticks::open(ferris, market, on_change)),
            Interval::Candles(timeframe) if fast => Self::Fast(
                FastBars::open(ferris, market, timeframe, on_change),
                timeframe,
            ),
            Interval::Candles(timeframe) => Self::Candles(
                ferris.candles(market, timeframe, move || on_change()),
                timeframe,
            ),
        }
    }

    fn mode(&self) -> Mode {
        match self {
            Self::Candles(..) | Self::Fast(..) => Mode::Candles,
            Self::Ticks(_) => Mode::Line,
        }
    }

    /// Its candle feed (its history, its status and its older pages); none
    /// for ticks.
    fn candles(&self) -> Option<&CandleFeed> {
        match self {
            Self::Candles(feed, _) => Some(feed),
            Self::Fast(fast, _) => Some(fast.candles()),
            Self::Ticks(_) => None,
        }
    }

    /// Its bars, oldest first.
    fn with_bars<R>(&self, f: impl FnOnce(&[Candle]) -> R) -> R {
        match self {
            Self::Candles(feed, _) => feed.with_candles(f),
            Self::Fast(fast, _) => fast.with_bars(f),
            Self::Ticks(ticks) => f(&ticks.bars.borrow()),
        }
    }

    /// How long one of its bars lasts, in seconds; none for ticks, which
    /// come when they come.
    fn step(&self) -> Option<f64> {
        match self {
            Self::Candles(_, timeframe) | Self::Fast(_, timeframe) => Some(timeframe.seconds()),
            Self::Ticks(_) => None,
        }
    }

    /// How many bars were ever added after its newest.
    fn appended(&self) -> u64 {
        match self {
            Self::Candles(feed, _) => feed.appended(),
            Self::Fast(fast, _) => fast.appended(),
            Self::Ticks(ticks) => ticks.appended.get(),
        }
    }

    /// What the plot says, with (`empty`) or without bars.
    fn status(&self, empty: bool) -> Status {
        let Some(feed) = self.candles() else {
            return match self {
                Self::Ticks(ticks) if empty && ticks.failed() => Status::Unavailable,
                _ if empty => Status::WaitingForTicks,
                _ => Status::Ready,
            };
        };
        if !empty {
            return if feed.history_failed() {
                Status::LiveOnly
            } else {
                Status::Ready
            };
        }
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
}

/// A chart's session state: its plot, its indicators over its bars, what
/// its view says about its data, and the feed it charts (which the
/// effects `new` starts keep on its settings).
pub(crate) struct ChartSession {
    /// Its plot. Where it looks starts over when the market or the interval
    /// changes.
    pub(super) plot: Rc<Plot>,
    /// Its settings' indicators over the bars its plot's scene shows: made
    /// with the scene, when a bar or an indicator changed since.
    pub(super) indicators: Signal<Shared<Computed>>,
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
        let indicators = signal(Shared::new(Computed::default()));
        let status = signal(Status::LoadingHistory);
        let loading_older = signal(false);

        let source: Rc<RefCell<Option<Source>>> = Rc::default();
        // The source's `appended` when the scene was last made.
        let appended = Rc::new(Cell::new(0_u64));
        // A bar or an indicator changed since the indicators were computed.
        let stale = Rc::new(Cell::new(true));
        // What a change of either does: the indicators are stale, and the
        // plot is marked.
        let changed: Rc<dyn Fn()> = {
            let (stale, mark) = (stale.clone(), plot.marker());
            Rc::new(move || {
                stale.set(true);
                mark();
            })
        };

        // The feed follows the market and interval the settings chart, and
        // whether its newest candle is made from trades: fast mode is a
        // candle chart's, a tick chart being its market's trades already.
        let market = computed(move || settings.with(ChartSettings::market));
        let interval = computed(move || settings.with(ChartSettings::interval));
        let fast = computed(move || {
            settings.with(|settings| settings.fast() && settings.interval() != Interval::Tick)
        });
        {
            let (source, appended, plot) = (source.clone(), appended.clone(), plot.clone());
            let changed = changed.clone();
            // What was last charted, which the plot looks at.
            let charted: RefCell<Option<(MarketSelection, Interval)>> = RefCell::default();
            effect(move || {
                let (market, interval, fast) = (market.get(), interval.get(), fast.get());
                untracked(|| {
                    // Leased before the old one goes, so what they share
                    // (a market's candles or trades) keeps streaming.
                    let next = Source::open(&ferris, &market, interval, fast, changed.clone());
                    appended.set(next.appended());
                    let previous = source.borrow_mut().replace(next);
                    drop(previous);
                    // Fast mode charts the same bars: the plot stays where
                    // it looks when only it changed.
                    let charting = Some((market, interval));
                    if *charted.borrow() != charting {
                        plot.viewport.set(Viewport::showing(match interval {
                            Interval::Tick => DEFAULT_TICKS,
                            Interval::Candles(_) => DEFAULT_CANDLES,
                        }));
                        *charted.borrow_mut() = charting;
                    }
                });
                changed();
            });
        }

        // The indicators follow the settings' too.
        {
            let listed = computed(move || settings.with(|settings| settings.indicators().to_vec()));
            effect(move || {
                listed.with(|_| ());
                changed();
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
                let stale = stale.clone();
                let drawn = plot.clone();
                plot.drive(move || {
                    let plot = &drawn;
                    let source = source.borrow();
                    let Some(source) = source.as_ref() else {
                        return;
                    };
                    let now = source.appended();
                    let added = now.saturating_sub(appended.replace(now));
                    let ((held, next), computed, said) = source.with_bars(|bars| {
                        // Computed again only over other bars or for other
                        // indicators: a pan or a zoom draws the same ones.
                        let fresh = stale.replace(false).then(|| {
                            Shared::new(settings.with_untracked(|settings| {
                                Computed::of(settings.indicators(), bars)
                            }))
                        });
                        let computed = fresh.clone().unwrap_or_else(|| indicators.get_untracked());
                        // The fitted prices make room for their lines.
                        let frame = plot.frame_with(bars, added, source.mode(), |from, to| {
                            computed.0.reach(from, to)
                        });
                        (frame, fresh, source.status(bars.is_empty()))
                    });
                    // A chart without indicators stays without, untold.
                    let computed = computed.filter(|computed| {
                        let had = indicators.with_untracked(|shown| !shown.0.is_empty());
                        had || !computed.0.is_empty()
                    });
                    if let Some(feed) = source.candles() {
                        let near_oldest = next.left <= (next.right - next.left) * OLDER_WITHIN;
                        if next.bars > 0 && near_oldest && asked.get() != plot.moves() {
                            asked.set(plot.moves());
                            feed.load_older();
                        }
                    }
                    let older = source.candles().is_some_and(CandleFeed::loading_older);
                    batch(|| {
                        plot.show(held, next);
                        if let Some(computed) = computed {
                            indicators.try_set(computed);
                        }
                        status.try_set(said);
                        loading_older.try_set(older);
                    });
                });
            });
        }

        Self {
            plot,
            indicators,
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
