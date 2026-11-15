//! When a series' slots are ([`Timeline`]): the time at any slot and the
//! slot at any time, so what a widget fixes to a time (a drawing's point)
//! stays on its bars through a pan, a zoom, a page of older history and
//! another interval.
//!
//! Bar `i` fills slot `i..i + 1` ([`super::viewport`]), from its own time
//! to the next bar's: a place within a slot is that far between the two,
//! so a gap in the series is as wide as it is drawn. Past either end, where
//! there is no bar, time goes on at a step: a candle's length, or for a
//! series with none of its own (ticks) the mean time between its bars.

use crate::data::ferris::Candle;

/// The step of a series that has neither one of its own nor two bars.
const ONE_SECOND: f64 = 1.0;

/// The times of a series' slots.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Timeline<'a> {
    bars: &'a [Candle],
    /// The seconds a slot lasts past either end of the series.
    step: f64,
}

impl<'a> Timeline<'a> {
    /// The slots of `bars` (oldest first), going on at `step` seconds a
    /// slot where it has none (a candle's length); without one, at the
    /// mean time between its bars.
    pub fn new(bars: &'a [Candle], step: Option<f64>) -> Self {
        let mean = || match (bars.first(), bars.last()) {
            (Some(first), Some(last)) if bars.len() > 1 => {
                (last.time - first.time) / (bars.len() - 1) as f64
            }
            _ => ONE_SECOND,
        };
        let usable = |step: &f64| step.is_finite() && *step > 0.0;
        let step = step
            .filter(usable)
            .or_else(|| Some(mean()).filter(usable))
            .unwrap_or(ONE_SECOND);
        Self { bars, step }
    }

    /// The time (Unix seconds, as a bar's) at `slot`; none without a bar.
    pub fn time_at(&self, slot: f64) -> Option<f64> {
        let first = self.bars.first()?;
        if !slot.is_finite() {
            return None;
        }
        if slot < 0.0 {
            return Some(first.time + slot * self.step);
        }
        let at = (slot as usize).min(self.bars.len() - 1);
        let opens = self.bars[at].time;
        Some(opens + (slot - at as f64) * self.lasts(at))
    }

    /// The slot at `time`; none without a bar.
    pub fn slot_of(&self, time: f64) -> Option<f64> {
        let first = self.bars.first()?;
        if !time.is_finite() {
            return None;
        }
        if time < first.time {
            return Some((time - first.time) / self.step);
        }
        // The last bar that opened by then: the next one opens later, so
        // the bar lasts some time.
        let at = self.bars.partition_point(|bar| bar.time <= time) - 1;
        let lasts = self.lasts(at);
        let into = if lasts > 0.0 {
            (time - self.bars[at].time) / lasts
        } else {
            0.0
        };
        Some(at as f64 + into)
    }

    /// How long bar `at`'s slot lasts: until the next bar opens, the step
    /// for the newest.
    fn lasts(&self, at: usize) -> f64 {
        match self.bars.get(at + 1) {
            Some(next) => next.time - self.bars[at].time,
            None => self.step,
        }
    }
}
