//! Supertrend: a line that trails the bars `factor` average true ranges
//! from their middle, under them while they rise and over them while they
//! fall (as TradingView's `ta.supertrend`).
//!
//! A bar's true range is the most of its high less its low and of how far
//! either is from the close before; the average is Wilder's over `length`
//! bars, the plain mean of the first `length` and then a `length`th of the
//! way to each bar's. About a bar's middle (the mean of its high and its
//! low) are two bands, `factor` averages away: the lower one only ever
//! rises while the closes stay over it, the upper one only ever falls
//! while they stay under it. The line is the lower band until a bar closes
//! under it, then the upper one until a bar closes over that, and so on:
//! where it changes sides the trend turned, and the line starts over
//! ([`Line::trending`]). Its first value is at bar `length - 1`, over the
//! bars.

use super::series::Line;
use crate::data::ferris::Candle;

/// The bands kept from the bar before, and the side the line was on.
struct Held {
    upper: f64,
    lower: f64,
    rising: bool,
}

/// Its line over `bars` (oldest first).
pub(super) fn line(bars: &[Candle], length: usize, factor: f64) -> Line {
    let length = length.max(1);
    let first = length - 1;
    let mut values = Vec::with_capacity((bars.len() + 1).saturating_sub(length));
    let mut flips = Vec::new();
    // The average true range so far.
    let mut range = 0.0_f64;
    let mut held: Option<Held> = None;
    for (at, bar) in bars.iter().enumerate() {
        let closed = at.checked_sub(1).map(|before| bars[before].close);
        let reach = closed.map_or(0.0, |closed| {
            (bar.high - closed).abs().max((bar.low - closed).abs())
        });
        let true_range = (bar.high - bar.low).max(reach);
        range += if at < length {
            true_range / length as f64
        } else {
            (true_range - range) / length as f64
        };
        if at < first {
            continue;
        }
        let middle = (bar.high + bar.low) / 2.0;
        let (mut upper, mut lower) = (middle + factor * range, middle - factor * range);
        let rising = match (&held, closed) {
            (Some(held), Some(closed)) => {
                // A band keeps to the bars' side of where it was, until a
                // close crosses it.
                if upper >= held.upper && closed <= held.upper {
                    upper = held.upper;
                }
                if lower <= held.lower && closed >= held.lower {
                    lower = held.lower;
                }
                if held.rising {
                    bar.close >= lower
                } else {
                    bar.close > upper
                }
            }
            _ => false,
        };
        if held.as_ref().is_some_and(|held| held.rising != rising) {
            flips.push(at);
        }
        values.push(if rising { lower } else { upper });
        held = Some(Held {
            upper,
            lower,
            rising,
        });
    }
    Line::from(first, values).trending(false, flips)
}
