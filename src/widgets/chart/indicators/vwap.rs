//! The volume-weighted average price: what a unit traded cost on average
//! since its period began ([`Period`]: the UTC day, week or month). Each
//! bar counts at its typical price (the mean of its high, its low and its
//! close) for what traded in it, so a tick, which is one print, counts at
//! its price for its size.
//!
//! It starts over with each period: its line is not joined from one
//! period's last bar to the next one's first. A bar has no value while
//! nothing traded in its period so far, which is every bar of a market
//! whose venue does not say what traded.

use super::series::Line;
use crate::{data::ferris::Candle, format::civil_date};

/// A day, in seconds.
const DAY: f64 = 86_400.0;

/// What the periods are called, in the order their input lists them
/// ([`Period::of`]).
pub(super) const PERIODS: [&str; 3] = ["Day", "Week", "Month"];

/// The span a volume-weighted average is taken over, in UTC.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Period {
    Day,
    /// From Monday.
    Week,
    Month,
}

impl Period {
    /// The period its input's `value` picks: its number among [`PERIODS`].
    pub fn of(value: f64) -> Self {
        match value as usize {
            0 => Self::Day,
            1 => Self::Week,
            _ => Self::Month,
        }
    }

    /// The number of the period `time` (Unix seconds) is in: another for
    /// each one.
    fn number(self, time: f64) -> i64 {
        let day = (time / DAY).floor() as i64;
        match self {
            Self::Day => day,
            // 1970-01-01 was a Thursday: three days into its week.
            Self::Week => (day + 3).div_euclid(7),
            Self::Month => {
                let (year, month, _) = civil_date(day);
                year * 12 + month
            }
        }
    }
}

/// Its line over `bars` (oldest first), starting over with each `period`.
pub(super) fn line(bars: &[Candle], period: Period) -> Line {
    let mut values = Vec::with_capacity(bars.len());
    let mut restarts = Vec::new();
    let mut within = None;
    // What was paid and what traded in the period so far.
    let (mut paid, mut traded) = (0.0_f64, 0.0_f64);
    for (at, bar) in bars.iter().enumerate() {
        let number = period.number(bar.time);
        if within != Some(number) {
            if within.is_some() {
                restarts.push(at);
            }
            within = Some(number);
            (paid, traded) = (0.0, 0.0);
        }
        let volume = bar
            .volume
            .filter(|volume| volume.is_finite() && *volume > 0.0);
        if let Some(volume) = volume {
            paid += (bar.high + bar.low + bar.close) / 3.0 * volume;
            traded += volume;
        }
        values.push(if traded > 0.0 {
            paid / traded
        } else {
            f64::NAN
        });
    }
    Line::from(0, values).restarting(restarts)
}
