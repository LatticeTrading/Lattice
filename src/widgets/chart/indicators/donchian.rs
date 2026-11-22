//! Donchian Channels: the highest high and the lowest low of the last
//! `length` bars (the range a breakout leaves), and the price midway
//! between them (the basis), which is drawn fainter. Their first values
//! are at bar `length - 1`.

use std::collections::VecDeque;

use super::series::Line;
use crate::data::ferris::Candle;

/// Its lines over `bars` (oldest first): the basis, then the upper and the
/// lower band.
pub(super) fn lines(bars: &[Candle], length: usize) -> Vec<Line> {
    let length = length.max(1);
    let first = length - 1;
    let count = (bars.len() + 1).saturating_sub(length);
    let mut basis = Vec::with_capacity(count);
    let mut upper = Vec::with_capacity(count);
    let mut lower = Vec::with_capacity(count);
    // The bars that may yet be the highest and the lowest of the last
    // `length`, oldest first: each is higher (lower) than every one after
    // it, so the first is the highest (lowest) of them.
    let mut highs: VecDeque<usize> = VecDeque::new();
    let mut lows: VecDeque<usize> = VecDeque::new();
    for (at, bar) in bars.iter().enumerate() {
        while highs
            .back()
            .is_some_and(|held| bars[*held].high <= bar.high)
        {
            highs.pop_back();
        }
        while lows.back().is_some_and(|held| bars[*held].low >= bar.low) {
            lows.pop_back();
        }
        highs.push_back(at);
        lows.push_back(at);
        let Some(oldest) = at.checked_sub(first) else {
            continue;
        };
        while highs.front().is_some_and(|held| *held < oldest) {
            highs.pop_front();
        }
        while lows.front().is_some_and(|held| *held < oldest) {
            lows.pop_front();
        }
        // Each holds this bar at least.
        let (high, low) = (bars[highs[0]].high, bars[lows[0]].low);
        basis.push((high + low) / 2.0);
        upper.push(high);
        lower.push(low);
    }
    vec![
        Line::from(first, basis).quiet(),
        Line::from(first, upper),
        Line::from(first, lower),
    ]
}
