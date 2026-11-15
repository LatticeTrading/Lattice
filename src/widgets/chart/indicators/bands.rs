//! Bollinger Bands: the closes' simple moving average over `length` bars
//! (the basis), and a band above and one below it, `deviations` standard
//! deviations of those closes away (of all `length` of them, the
//! population's, as TradingView's). Their first values are at bar
//! `length - 1`.

use super::series::Line;

/// Its lines over `closes`: the basis, then the upper and the lower band,
/// which are drawn fainter.
pub(super) fn lines(closes: &[f64], length: usize, deviations: f64) -> Vec<Line> {
    let length = length.max(1);
    let first = length - 1;
    let count = (closes.len() + 1).saturating_sub(length);
    let mut basis = Vec::with_capacity(count);
    let mut upper = Vec::with_capacity(count);
    let mut lower = Vec::with_capacity(count);
    // The sums of the last `length` closes and of their squares, taken
    // about the first close so the squares keep their precision.
    let about = closes.first().copied().unwrap_or(0.0);
    let (mut sum, mut squares) = (0.0_f64, 0.0_f64);
    for (at, close) in closes.iter().enumerate() {
        let value = close - about;
        sum += value;
        squares += value * value;
        if at >= length {
            let gone = closes[at - length] - about;
            sum -= gone;
            squares -= gone * gone;
        }
        if at >= first {
            let mean = sum / length as f64;
            let spread = (squares / length as f64 - mean * mean).max(0.0).sqrt() * deviations;
            basis.push(about + mean);
            upper.push(about + mean + spread);
            lower.push(about + mean - spread);
        }
    }
    vec![
        Line::from(first, basis),
        Line::from(first, upper).quiet(),
        Line::from(first, lower).quiet(),
    ]
}
