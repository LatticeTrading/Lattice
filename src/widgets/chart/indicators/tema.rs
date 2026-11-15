//! The triple exponential moving average: `3 * e1 - 3 * e2 + e3`, where
//! `e1` is the closes' exponential average ([`super::ema`]), `e2` is
//! `e1`'s and `e3` is `e2`'s. It lags the closes less than `e1` does.
//! Each average starts `length - 1` values into what it averages, so its
//! first value is at bar `3 * (length - 1)`.

use super::{ema::averaged, series::Line};

/// Its line over `closes`.
pub(super) fn line(closes: &[f64], length: usize) -> Line {
    let lead = length.max(1) - 1;
    let once = averaged(closes, length);
    let twice = averaged(&once, length);
    let thrice = averaged(&twice, length);
    let values = thrice
        .iter()
        .enumerate()
        .map(|(at, third)| 3.0 * once[at + 2 * lead] - 3.0 * twice[at + lead] + third)
        .collect();
    Line::from(3 * lead, values)
}
