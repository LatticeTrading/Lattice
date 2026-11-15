//! The exponential moving average: the mean of the closes in which the
//! newest weigh the most. Its first value is at bar `length - 1`, the
//! plain mean of the first `length` closes; each one after it is
//! `2 / (length + 1)` of the way from the one before to its bar's close.

use super::series::Line;

/// The exponential average of `values` over `length`: one a value from
/// the `length`th on, none while they are fewer.
pub(super) fn averaged(values: &[f64], length: usize) -> Vec<f64> {
    let length = length.max(1);
    if values.len() < length {
        return Vec::new();
    }
    let weight = 2.0 / (length as f64 + 1.0);
    let mut average = values[..length].iter().sum::<f64>() / length as f64;
    let mut averaged = Vec::with_capacity(values.len() - length + 1);
    averaged.push(average);
    for value in &values[length..] {
        average += (value - average) * weight;
        averaged.push(average);
    }
    averaged
}

/// Its line over `closes`.
pub(super) fn line(closes: &[f64], length: usize) -> Line {
    Line::from(length.max(1) - 1, averaged(closes, length))
}
