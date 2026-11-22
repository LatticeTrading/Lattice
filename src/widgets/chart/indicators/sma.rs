//! The simple moving average: the plain mean of the last `length` closes,
//! each weighing as much as another. Its first value is at bar
//! `length - 1`.

use super::series::Line;

/// The mean of each `length` of `values` in a row: one a value from the
/// `length`th on, none while they are fewer.
fn averaged(values: &[f64], length: usize) -> Vec<f64> {
    let length = length.max(1);
    let mut averaged = Vec::with_capacity((values.len() + 1).saturating_sub(length));
    // The sum of the last `length` values, taken about the first one so a
    // long run of large prices keeps its precision.
    let about = values.first().copied().unwrap_or(0.0);
    let mut sum = 0.0_f64;
    for (at, value) in values.iter().enumerate() {
        sum += value - about;
        if at >= length {
            sum -= values[at - length] - about;
        }
        if at + 1 >= length {
            averaged.push(about + sum / length as f64);
        }
    }
    averaged
}

/// Its line over `closes`.
pub(super) fn line(closes: &[f64], length: usize) -> Line {
    Line::from(length.max(1) - 1, averaged(closes, length))
}
