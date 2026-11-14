//! Where the chart looks (the old `WebGLPriceChart`'s viewport): how many
//! slots show, how far the newest bar is from the right edge, and the price
//! range when the user set one.
//!
//! Bars sit in slots: bar `i` of the series fills slot `i..i + 1`, and the
//! plot shows the slots [`Viewport::window`] gives. The view is kept from
//! the newest bar back (`pan`), so a page of older history added in front
//! leaves it where it was. Slots are fractions, so a drag moves the chart
//! by the pointer's distance, not by whole bars as the old chart did.
//!
//! Prices fit the bars that show until the user scales or moves them
//! (free roam); a double click on an axis fits them again.

/// The fewest and most slots the plot shows (the old chart's).
pub(super) const MIN_VISIBLE: f64 = 24.0;
pub(super) const MAX_VISIBLE: f64 = 25_000.0;
/// The slots a new candle chart and a new tick chart show.
pub(super) const DEFAULT_CANDLES: f64 = 220.0;
pub(super) const DEFAULT_TICKS: f64 = 700.0;

/// A price range, low to high.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Prices {
    pub low: f64,
    pub high: f64,
}

impl Prices {
    pub fn span(self) -> f64 {
        self.high - self.low
    }

    /// The range that shows prices from `low` to `high` as the old chart
    /// fitted them: 8% of room above and below, a flat range opened by 1%
    /// of its price first.
    pub fn fitting(low: f64, high: f64) -> Self {
        let (mut low, mut high) = (low, high);
        if high <= low {
            let spread = if high == 0.0 { 1.0 } else { high.abs() * 0.01 };
            low -= spread;
            high += spread;
        }
        let room = (high - low) * 0.08;
        Self {
            low: low - room,
            high: high + room,
        }
    }

    /// The smallest span a range may be scaled to while this one fits the
    /// bars: 0.05% of the price in its middle.
    pub fn least_span(self) -> f64 {
        (((self.low + self.high) / 2.0).abs() * 0.0005).max(1e-8)
    }

    /// Usable as the plot's range.
    pub fn is_valid(self) -> bool {
        self.low.is_finite() && self.high.is_finite() && self.high > self.low
    }

    /// Where `price` is down the plot: 0 at its top, 1 at its bottom.
    pub fn fraction(self, price: f64) -> f64 {
        (self.high - price) / self.span()
    }

    /// The price `fraction` of the way down the plot.
    pub fn price_at(self, fraction: f64) -> f64 {
        self.high - fraction * self.span()
    }

    /// Moved with a drag of `dy` points down a plot `height` points tall:
    /// the bars follow the pointer.
    pub fn dragged(self, dy: f64, height: f64) -> Self {
        let shift = dy / height.max(1.0) * self.span();
        Self {
            low: self.low + shift,
            high: self.high + shift,
        }
    }

    /// Scaled about its middle with a drag of `dy` points on the price
    /// axis (down widens it), never under `least`.
    pub fn scaled(self, dy: f64, least: f64) -> Self {
        let span = (self.span().max(least) * (dy / 160.0).exp()).max(least);
        let middle = (self.low + self.high) / 2.0;
        Self {
            low: middle - span / 2.0,
            high: middle + span / 2.0,
        }
    }

    /// Scaled by `factor` about the price `anchor` of the way down it (the
    /// pointer's), never under `least`.
    pub fn zoomed(self, factor: f64, anchor: f64, least: f64) -> Self {
        let span = self.span().max(least);
        let next = (span * factor).max(least);
        let high = self.high - anchor * span + anchor * next;
        Self {
            low: high - next,
            high,
        }
    }
}

/// What the plot shows of a series.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Viewport {
    /// The slots across the plot.
    pub visible: f64,
    /// How many bars the newest one is past the right edge: above zero the
    /// plot looks at older bars, below zero it has empty slots (the future)
    /// to the right of the newest.
    pub pan: f64,
    /// The price range the user set; none fits the bars that show.
    pub prices: Option<Prices>,
}

impl Viewport {
    /// The newest `visible` slots, prices fitted.
    pub fn showing(visible: f64) -> Self {
        Self {
            visible,
            pan: 0.0,
            prices: None,
        }
    }

    /// The slots at the plot's left and right edges for a series of `bars`.
    pub fn window(&self, bars: usize) -> (f64, f64) {
        let right = bars as f64 - self.pan;
        (right - self.visible, right)
    }

    /// `pan` kept between the newest bar in the leftmost slot and the
    /// oldest one there (the old chart's limits).
    fn pan_within(pan: f64, visible: f64, bars: usize) -> f64 {
        let oldest_at_left = (bars as f64 - visible).max(0.0);
        pan.clamp(-(visible - 1.0), oldest_at_left)
    }

    /// Looking `pan` bars back from the newest.
    pub fn panned(self, pan: f64, bars: usize) -> Self {
        Self {
            pan: Self::pan_within(pan, self.visible, bars),
            ..self
        }
    }

    /// Showing `factor` times the slots, the slot `anchor` of the way
    /// across the plot (the pointer's) staying where it is. As in the old
    /// chart, zooming never opens empty slots right of the newest bar, and
    /// where there are some the newest bar keeps its place across the plot.
    pub fn zoomed(self, factor: f64, anchor: f64, bars: usize) -> Self {
        let visible = (self.visible * factor).clamp(MIN_VISIBLE, MAX_VISIBLE);
        let pan = if self.pan < 0.0 {
            self.pan * visible / self.visible
        } else {
            let (left, _) = self.window(bars);
            let right = left + anchor * (self.visible - visible) + visible;
            (bars as f64 - right).max(0.0)
        };
        Self {
            visible,
            pan: Self::pan_within(pan, visible, bars),
            ..self
        }
    }

    /// `origin` after a drag of `dx` points on the time axis: right shows
    /// more slots, left fewer, the right edge staying on its bar.
    pub fn stretched(origin: Self, dx: f64, bars: usize) -> Self {
        let visible = (origin.visible * (dx / 220.0).exp()).clamp(MIN_VISIBLE, MAX_VISIBLE);
        let pan = if origin.pan < 0.0 {
            origin.pan * visible / origin.visible
        } else {
            origin.pan
        };
        Self {
            visible,
            pan: Self::pan_within(pan, visible, bars),
            ..origin
        }
    }

    /// The same bars after `appended` new ones came in after the newest:
    /// a plot at the newest bar follows it, one looking at older bars
    /// stays on them.
    pub fn held(self, appended: u64, bars: usize) -> Self {
        if self.pan > 0.0 {
            self.panned(self.pan + appended as f64, bars)
        } else {
            self
        }
    }
}
