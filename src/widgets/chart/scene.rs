//! What the chart draws for a series, a viewport and a plot size: the bars
//! that show, merged into buckets where they are narrower than the screen
//! can tell apart, and cut into the tiles the shader draws
//! ([`super::shader`]). The old `ChartDataEngine`'s snapshot.
//!
//! - Buckets: where a slot would be narrower than [`CANDLE_PITCH`] (a line's
//!   [`LINE_PITCH`]), every `bucket` bars are merged into one (first open,
//!   highest high, lowest low, last close). Buckets are counted from the
//!   series' first bar, not from the plot's edge as the old engine's were,
//!   so panning never changes which bars a bucket holds.
//! - Tiles: a shader view takes 4096 bytes of uniforms, 248 candles, so the
//!   plot is a row of tiles at most [`TILE_WIDTH`] points wide, each given
//!   the buckets under it and one more on each side (a line's segments
//!   cross tile edges). Their prices are fractions of the plot's height, so
//!   the shader needs no price or index beyond 32-bit floats' precision.

use super::{
    shader::{CANDLE_CAPACITY, LINE_CAPACITY},
    viewport::{Prices, Viewport},
};
use crate::data::ferris::Candle;

/// A tile's widest, in points.
pub(super) const TILE_WIDTH: f64 = 480.0;
/// A candle's narrowest slot, in points; narrower slots are merged.
const CANDLE_PITCH: f64 = 2.0;
/// The narrowest step between a line's points, in points.
const LINE_PITCH: f64 = 1.0;
/// How far off the plot (in plot heights) a price is drawn at most: what
/// keeps a far-off price from costing the shader its precision.
const FAR: f64 = 64.0;

/// How a series is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Mode {
    Candles,
    /// A line through the closes (a tick chart).
    Line,
}

/// One shader view's stretch of the plot.
#[derive(Debug, PartialEq)]
pub(super) struct Tile {
    /// Where it starts and how wide it is, in whole points from the plot's
    /// left edge.
    pub left: f32,
    pub width: f32,
    /// The bucket coordinate at its left edge, counted from its first
    /// value's bucket.
    pub first: f32,
    /// The buckets across it.
    pub per: f32,
    /// The buckets it has values for.
    pub count: usize,
    /// Their prices as fractions down the plot (0 its top): open, high,
    /// low and close of each for candles, the close of each for a line.
    pub values: Vec<f32>,
}

/// The newest bar's close, which the plot marks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Last {
    pub close: f64,
    /// It closed at or above its open.
    pub rising: bool,
}

/// Everything the chart's view shows of its series.
#[derive(Debug)]
pub(super) struct Scene {
    pub mode: Mode,
    /// The plot's size in points.
    pub width: f32,
    pub height: f32,
    /// The bars in the series.
    pub bars: usize,
    /// The slots at the plot's left and right edges.
    pub left: f64,
    pub right: f64,
    /// The bars merged into each bucket.
    pub bucket: usize,
    /// The range the plot shows.
    pub prices: Prices,
    /// The range that fits the bars that show.
    pub fit: Prices,
    /// The buckets under the plot and one more on each side, and the first
    /// one's number.
    pub buckets: Vec<Candle>,
    pub first_bucket: usize,
    pub tiles: Vec<Tile>,
    pub last: Option<Last>,
}

impl Scene {
    /// Nothing to draw yet.
    pub fn empty(mode: Mode) -> Self {
        Self::build(&[], &Viewport::showing(1.0), mode, 1.0, 1.0)
    }

    /// What a plot of `width` by `height` points shows of `bars` through
    /// `view`.
    pub fn build(bars: &[Candle], view: &Viewport, mode: Mode, width: f32, height: f32) -> Self {
        let (left, right) = view.window(bars.len());
        let (width, height) = (width.max(1.0), height.max(1.0));
        let (pitch, capacity) = match mode {
            Mode::Candles => (CANDLE_PITCH, CANDLE_CAPACITY),
            Mode::Line => (LINE_PITCH, LINE_CAPACITY),
        };
        let bucket = ((pitch * (right - left) / f64::from(width)).ceil() as usize).max(1);
        let (from, to) = (left / bucket as f64, right / bucket as f64);
        let total = bars.len().div_ceil(bucket);
        let first_bucket = index(from.floor() - 1.0).min(total);
        let end = index(to.floor() + 2.0).min(total);
        let buckets: Vec<Candle> = (first_bucket..end)
            .map(|at| merged(&bars[at * bucket..((at + 1) * bucket).min(bars.len())]))
            .collect();

        let showing = buckets
            .iter()
            .zip(first_bucket..)
            .filter(|(_, at)| (*at as f64) < to && (*at + 1) as f64 > from)
            .map(|(bar, _)| bar);
        let (low, high) = showing.fold((f64::INFINITY, f64::NEG_INFINITY), |(low, high), bar| {
            (low.min(bar.low), high.max(bar.high))
        });
        let fit = if low <= high {
            Prices::fitting(low, high)
        } else {
            Prices::fitting(0.0, 0.0)
        };
        let prices = view
            .prices
            .filter(|prices| prices.is_valid())
            .unwrap_or(fit);
        let down = |price: f64| prices.fraction(price).clamp(-FAR, FAR) as f32;

        let plot = f64::from(width);
        let tiles = (plot / TILE_WIDTH).ceil().max(1.0) as usize;
        let edge = |tile: usize| (tile as f64 * plot / tiles as f64).round();
        let tiles = (0..tiles)
            .map(|tile| {
                let (x0, x1) = (edge(tile), edge(tile + 1));
                let (u0, u1) = (
                    from + x0 / plot * (to - from),
                    from + x1 / plot * (to - from),
                );
                let start = index(u0.floor() - 1.0).clamp(first_bucket, end);
                let stop = index(u1.floor() + 2.0).clamp(start, end);
                let shown = &buckets[start - first_bucket..stop - first_bucket];
                let shown = &shown[..shown.len().min(capacity)];
                let values = match mode {
                    Mode::Candles => shown
                        .iter()
                        .flat_map(|bar| [bar.open, bar.high, bar.low, bar.close])
                        .map(down)
                        .collect(),
                    Mode::Line => shown.iter().map(|bar| down(bar.close)).collect(),
                };
                Tile {
                    left: x0 as f32,
                    width: (x1 - x0) as f32,
                    first: (u0 - start as f64) as f32,
                    per: (u1 - u0) as f32,
                    count: shown.len(),
                    values,
                }
            })
            .collect();

        Self {
            mode,
            width,
            height,
            bars: bars.len(),
            left,
            right,
            bucket,
            prices,
            fit,
            buckets,
            first_bucket,
            tiles,
            last: bars.last().map(|bar| Last {
                close: bar.close,
                rising: bar.close >= bar.open,
            }),
        }
    }

    /// The plot's edges in bucket coordinates.
    fn span(&self) -> (f64, f64) {
        (
            self.left / self.bucket as f64,
            self.right / self.bucket as f64,
        )
    }

    /// The bucket numbers that show, first to last; none without a bar
    /// under the plot.
    fn showing(&self) -> Option<(usize, usize)> {
        let (from, to) = self.span();
        let first = index(from.floor()).max(self.first_bucket);
        let last =
            index(to.ceil() - 1.0).min((self.first_bucket + self.buckets.len()).checked_sub(1)?);
        (first <= last).then_some((first, last))
    }

    /// Bucket `at`'s middle, in points from the plot's left edge.
    fn middle(&self, at: usize) -> f32 {
        let (from, to) = self.span();
        ((at as f64 + 0.5 - from) / (to - from) * f64::from(self.width)) as f32
    }

    /// The bucket under `x` points from the plot's left edge, else the
    /// nearest one that shows (as the old crosshair snapped): its middle's
    /// x and its bar.
    pub fn bucket_near(&self, x: f32) -> Option<(f32, Candle)> {
        let (first, last) = self.showing()?;
        let (from, to) = self.span();
        let under = from + f64::from(x) / f64::from(self.width) * (to - from);
        let at = index(under.floor()).clamp(first, last);
        Some((self.middle(at), self.buckets[at - self.first_bucket]))
    }

    /// Up to `marks` bars spread evenly over the ones that show, for the
    /// time axis to name (the old axis' label slots): each one's middle x
    /// and open time, left to right, none twice.
    pub fn time_marks(&self, marks: usize) -> Vec<(f32, f64)> {
        let Some((first, last)) = self.showing() else {
            return Vec::new();
        };
        let mut picked: Vec<usize> = (0..marks)
            .map(|mark| {
                let along = mark as f64 / (marks.max(2) - 1) as f64;
                first + ((last - first) as f64 * along).round() as usize
            })
            .collect();
        picked.dedup();
        picked
            .into_iter()
            .map(|at| (self.middle(at), self.buckets[at - self.first_bucket].time))
            .collect()
    }
}

/// A bucket number from a coordinate: nothing is before the first.
fn index(coordinate: f64) -> usize {
    coordinate.max(0.0) as usize
}

/// The bars of one bucket as one: the first's time and open, the highest
/// high, the lowest low, the last's close, their volumes summed.
fn merged(bars: &[Candle]) -> Candle {
    let mut bucket = bars[0];
    for bar in &bars[1..] {
        bucket.high = bucket.high.max(bar.high);
        bucket.low = bucket.low.min(bar.low);
        bucket.close = bar.close;
        bucket.volume = match (bucket.volume, bar.volume) {
            (Some(sum), Some(more)) => Some(sum + more),
            (sum, more) => sum.or(more),
        };
    }
    bucket
}
