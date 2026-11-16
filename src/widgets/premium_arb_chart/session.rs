//! The Premium/Arb Chart's session ([`PremiumArbSession`]): the pair its
//! settings compare, the series it plots of it, and its plot (the widget's
//! data, described in the parent module).

use std::{cell::RefCell, collections::VecDeque, rc::Rc};

use nsh_core::prelude::*;

use super::{PremiumArbSettings, settings::Display};
use crate::{
    data::{
        SessionScope,
        ferris::{BookTraits, Candle, Ferris, PairSample, PricePairFeed},
    },
    widgets::common::{
        books::Pair,
        plot::{DEFAULT_TICKS, Mode, Plot, Viewport},
    },
};

/// What the plot says about its data, in its middle while it has no point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Status {
    Ready,
    Waiting,
    Unavailable,
}

impl Status {
    /// What it says (the old chart's message while it waited), nothing
    /// while all is well.
    pub fn text(self) -> &'static str {
        match self {
            Self::Ready => "",
            Self::Waiting => "Waiting for both live prices...",
            Self::Unavailable => "Live prices are unavailable for one of these markets.",
        }
    }
}

/// What `display` plots of `sample` (the old `toDeltaCandle`): a bar of one
/// value, the second market's price less the primary's, or that as a
/// percentage of the primary's. None where there is no such value (a
/// percentage of a price of zero).
pub(super) fn bar(display: Display, sample: &PairSample) -> Option<Candle> {
    let delta = sample.secondary - sample.primary;
    let value = match display {
        Display::Raw => delta,
        Display::Percent if sample.primary == 0.0 => return None,
        Display::Percent => delta / sample.primary * 100.0,
    };
    value.is_finite().then_some(Candle {
        time: sample.time,
        open: value,
        high: value,
        low: value,
        close: value,
        volume: None,
    })
}

/// The series a chart plots: its pair's samples as bars of one display
/// mode, kept in step with the feed's.
pub(super) struct Series {
    display: Display,
    /// The bars, oldest first.
    pub bars: Vec<Candle>,
    /// How many of the feed's samples were taken in, by the feed's count.
    taken: u64,
}

impl Series {
    pub fn new(display: Display) -> Self {
        Self {
            display,
            bars: Vec::new(),
            taken: 0,
        }
    }

    /// Takes in the feed's `samples` (`made` in all) that came since the
    /// last call, as `display` plots them, and lets go of the bars whose
    /// samples the feed no longer keeps. Another `display` than last time
    /// makes every bar again. Says how many bars were added after the
    /// newest since the last call.
    pub fn take(&mut self, samples: &VecDeque<PairSample>, made: u64, display: Display) -> u64 {
        let before = self.taken;
        if display != self.display {
            self.display = display;
            self.bars.clear();
            self.taken = 0;
        }
        // The number of the oldest sample kept, then of the first to take.
        let first = made.saturating_sub(samples.len() as u64);
        let from = self.taken.max(first);
        let mut added = 0;
        let fresh = (first..made).zip(samples).skip((from - first) as usize);
        for (number, sample) in fresh {
            if let Some(bar) = bar(display, sample) {
                self.bars.push(bar);
                added += u64::from(number >= before);
            }
        }
        self.taken = made;
        let oldest = samples.front().map_or(f64::INFINITY, |sample| sample.time);
        let gone = self.bars.partition_point(|bar| bar.time < oldest);
        self.bars.drain(..gone);
        added
    }
}

/// What a chart plots: its lease on the pair's prices, and its series of
/// them.
struct Source {
    feed: PricePairFeed,
    series: Series,
}

/// A Premium/Arb Chart's session state: its plot, and what its view says
/// about its data (its feed and settings live in the effects `new` starts).
pub(crate) struct PremiumArbSession {
    /// Its plot. Where it looks starts over when a market changes, and its
    /// prices fit the line again when the display mode does.
    pub(super) plot: Rc<Plot>,
    pub(super) status: Signal<Status>,
    /// What the two books say of their liquidity, by their layers' keys.
    pub(super) traits: Signal<Vec<(String, BookTraits)>>,
}

impl PremiumArbSession {
    /// A chart following `settings`, the session's copy of the tab's.
    pub(super) fn new(
        scope: SessionScope,
        ferris: Ferris,
        settings: Signal<PremiumArbSettings>,
    ) -> Self {
        let plot = Plot::new(Mode::Line, DEFAULT_TICKS);
        let status = signal(Status::Waiting);
        let traits = signal(Vec::<(String, BookTraits)>::new());
        let layers = computed(move || settings.with(PremiumArbSettings::layers));
        let source: Rc<RefCell<Option<Source>>> = Rc::default();

        // The feed follows the two markets the settings compare; with one
        // missing there is none.
        let markets = computed(move || settings.with(PremiumArbSettings::markets));
        let display = computed(move || settings.with(PremiumArbSettings::display));
        {
            let (source, plot) = (source.clone(), plot.clone());
            effect(move || {
                let markets = markets.get();
                untracked(|| {
                    // Leased before the old one goes, so the books they
                    // share keep streaming.
                    let next = markets.map(|(primary, secondary)| {
                        let mark = plot.marker();
                        Source {
                            feed: ferris.price_pair(&primary, &secondary, move || mark()),
                            series: Series::new(display.get_untracked()),
                        }
                    });
                    let previous = std::mem::replace(&mut *source.borrow_mut(), next);
                    drop(previous);
                    plot.viewport.set(Viewport::showing(DEFAULT_TICKS));
                });
                plot.mark();
            });
        }
        // Another display mode is other values: a price range the user set
        // for the last one is let go, as the old chart let go of it.
        {
            let plot = plot.clone();
            effect(move || {
                display.get();
                untracked(|| {
                    if plot.viewport.with_untracked(|view| view.prices.is_some()) {
                        plot.viewport.update(|view| view.prices = None);
                    }
                });
                plot.mark();
            });
        }

        {
            let plot = plot.clone();
            scope.while_visible(move || {
                let source = source.clone();
                let drawn = plot.clone();
                plot.drive(move || {
                    let plot = &drawn;
                    let ((held, scene), said) = match source.borrow_mut().as_mut() {
                        Some(Source { feed, series }) => {
                            let display = display.get_untracked();
                            let added = feed
                                .with_samples(|samples, made| series.take(samples, made, display));
                            let books = feed.traits();
                            let next: Vec<_> = layers.with_untracked(|layers| {
                                layers
                                    .iter()
                                    .filter_map(|layer| {
                                        let leg = usize::from(layer.pair != Pair::Primary);
                                        Some((layer.key.clone(), books[leg].clone()?))
                                    })
                                    .collect()
                            });
                            if traits.with_untracked(|kept| *kept != next) {
                                traits.try_set(next);
                            }
                            let said = if !series.bars.is_empty() {
                                Status::Ready
                            } else if feed.failed() {
                                Status::Unavailable
                            } else {
                                Status::Waiting
                            };
                            (plot.frame(&series.bars, added, Mode::Line), said)
                        }
                        // A market is missing: nothing to plot.
                        None => (plot.frame(&[], 0, Mode::Line), Status::Waiting),
                    };
                    batch(|| {
                        plot.show(held, scene);
                        status.try_set(said);
                    });
                });
            });
        }

        Self {
            plot,
            status,
            traits,
        }
    }
}
