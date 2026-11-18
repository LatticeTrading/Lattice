//! The Account Performance's session ([`AccountPerformanceSession`]): the
//! history of the account its settings name, the series it plots of it,
//! and its plot (the widget's data, described in the parent module).

use std::{cell::RefCell, rc::Rc};

use nsh_core::prelude::*;

use super::settings::{AccountPerformanceSettings, Metric, Style};
use crate::{
    data::{
        Portfolio, SessionScope,
        ferris::Candle,
        portfolio::{Account, Performance, Period, Scope, Window},
    },
    format,
    widgets::common::{
        accounts::{AccountSettings, NONE, Report, Standing, Tone, follow, shown},
        plot::{Mode, Plot, Viewport},
    },
};

/// The slots a plot with nothing to draw yet looks at.
const UNFITTED: f64 = 100.0;

/// What the plot says in its middle while it has no point.
pub(super) const LOADING: &str = "Loading account history...";
pub(super) const UNAVAILABLE: &str = "Account history is unavailable.";
pub(super) const EMPTY: &str = "No history for this period.";

/// What `metric` plots of `window`: a bar of one value a sample, oldest
/// first, a second apart at least (two samples of one second are the
/// later one).
pub(super) fn bars_of(window: &Window, metric: Metric) -> Vec<Candle> {
    let points = match metric {
        Metric::Value => &window.value,
        Metric::Pnl => &window.pnl,
    };
    let mut bars: Vec<Candle> = Vec::with_capacity(points.len());
    for point in points {
        let bar = Candle {
            time: (point.time_ms / 1000.0).floor(),
            open: point.value,
            high: point.value,
            low: point.value,
            close: point.value,
            volume: None,
        };
        match bars.last_mut() {
            Some(last) if last.time >= bar.time => {
                *last = Candle {
                    time: last.time,
                    ..bar
                };
            }
            _ => bars.push(bar),
        }
    }
    bars
}

/// How `style` draws the series: green above its baseline, red below.
pub(super) fn mode_of(style: Style) -> Mode {
    match style {
        Style::Line => Mode::Signed,
        Style::Area => Mode::SignedArea,
    }
}

/// What `metric`'s line of `window` is green above and red below: no
/// profit for its profit and loss, and for its value what it was worth as
/// the window began (a value is never below nothing).
pub(super) fn baseline_of(window: Option<&Window>, metric: Metric) -> f64 {
    match metric {
        Metric::Pnl => 0.0,
        Metric::Value => window
            .and_then(|window| window.value.first())
            .map_or(0.0, |first| first.value),
    }
}

/// What the widget says of the window plotted, beside its controls.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Readout {
    /// What the account made over the window (`+$2,450.00`): its profit
    /// and loss at the window's end less that at its start.
    pub pnl: String,
    /// What colors [`Self::pnl`]; `None` with nothing to say.
    pub tone: Option<Tone>,
    /// What it traded in the window.
    pub volume: String,
}

impl Readout {
    /// The readout of `window`; dashes without one, or with no sample.
    pub fn of(window: Option<&Window>) -> Self {
        let made = window.and_then(|window| {
            let (first, last) = (window.pnl.first()?, window.pnl.last()?);
            Some(last.value - first.value)
        });
        Self {
            pnl: shown(made, format::signed_usd),
            tone: made.map(Tone::of),
            volume: window
                .and_then(|window| window.volume)
                .map_or_else(|| NONE.to_owned(), format::usd),
        }
    }
}

/// What the plot says in its middle of a report standing as `standing`
/// with no point to plot; nothing once it has one.
pub(super) fn said(standing: &Standing, plotted: bool) -> &'static str {
    match standing {
        _ if plotted => "",
        Standing::Unset => "",
        Standing::Loading => LOADING,
        Standing::Unavailable(_) | Standing::Failed(_) => UNAVAILABLE,
        Standing::Ready | Standing::Stale(_) => EMPTY,
    }
}

/// The series the plot last started over for, and how it stood then.
#[derive(Clone, PartialEq)]
struct Fitted {
    /// What was plotted: whose, and which window and measure of it.
    series: (Option<Account>, Period, Scope, Metric),
    /// How many points it had.
    points: usize,
    /// How many times the user had moved the viewport.
    moves: u64,
}

/// An Account Performance's session state: its account's history, its
/// plot, and what its view says of the window plotted (its watch lives in
/// the effect `follow` starts).
pub(crate) struct AccountPerformanceSession {
    /// The exchanges that report accounts, which its account's rows list.
    pub(super) portfolio: Portfolio,
    /// The history of the account the settings name.
    pub(super) report: Report<Performance>,
    /// Its plot. Where it looks starts over, on the whole window, when
    /// another series is plotted, and follows the window as it grows until
    /// the user moves it.
    pub(super) plot: Rc<Plot>,
    /// What the window plotted made and traded, updated while it shows.
    pub(super) readout: Signal<Readout>,
    /// What the plot says in its middle while it has no point.
    pub(super) status: Signal<&'static str>,
}

impl AccountPerformanceSession {
    /// A session following `settings`, the session's copy of the tab's.
    pub(super) fn new(
        scope: SessionScope,
        portfolio: Portfolio,
        settings: Signal<AccountPerformanceSettings>,
    ) -> Self {
        let account = computed(move || settings.with(|settings| settings.account().cloned()));
        let report = {
            let portfolio = portfolio.clone();
            follow(scope, account, move |account| {
                portfolio.watch_performance(account)
            })
        };
        // The window and the measure plotted, and how.
        let picked = computed(move || {
            settings.with(|settings| {
                (
                    settings.period(),
                    settings.scope(),
                    settings.metric(),
                    settings.style(),
                )
            })
        });
        let plot = Plot::new(Mode::Signed, UNFITTED);
        let readout = signal(Readout::of(None));
        let status = signal(LOADING);
        let fitted: Rc<RefCell<Option<Fitted>>> = Rc::default();

        {
            let plot = plot.clone();
            scope.while_visible(move || {
                // A new report, another account or another window: the plot
                // draws again.
                {
                    let plot = plot.clone();
                    effect(move || {
                        report.data();
                        report.standing.get();
                        account.get();
                        picked.get();
                        plot.mark();
                    });
                }
                let (drawn, fitted) = (plot.clone(), fitted.clone());
                plot.drive(move || {
                    let plot = &drawn;
                    let (period, part, metric, style) = picked.get_untracked();
                    let (history, standing) = untracked(|| (report.data(), report.standing.get()));
                    let window = history
                        .as_ref()
                        .and_then(|history| history.window(period, part));
                    let bars = window.map_or_else(Vec::new, |window| bars_of(window, metric));
                    // Another series starts over on the whole of it; the
                    // same one grown is followed until the user looks
                    // elsewhere.
                    let series = (account.get_untracked(), period, part, metric);
                    let refit = match &*fitted.borrow() {
                        None => true,
                        Some(last) => {
                            last.series != series
                                || (last.points != bars.len() && last.moves == plot.moves())
                        }
                    };
                    if refit {
                        *fitted.borrow_mut() = Some(Fitted {
                            series,
                            points: bars.len(),
                            moves: plot.moves(),
                        });
                        plot.viewport
                            .set(Viewport::showing(bars.len().max(2) as f64));
                    }
                    let (held, scene) = plot.frame(&bars, 0, mode_of(style));
                    let scene = scene.with_baseline(baseline_of(window, metric));
                    batch(|| {
                        plot.show(held, scene);
                        readout.try_set(Readout::of(window));
                        status.try_set(said(&standing, !bars.is_empty()));
                    });
                });
            });
        }

        Self {
            portfolio,
            report,
            plot,
            readout,
            status,
        }
    }
}
