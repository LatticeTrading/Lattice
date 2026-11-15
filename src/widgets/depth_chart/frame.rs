//! What the depth chart draws for its books and a plot size: each side of
//! each book as a curve of running depth over price, the window of prices
//! the plot shows, the scale depth is drawn at, and the columns the shader
//! draws the curves from ([`super::shader`]).
//!
//! - Curves: each book's best `depth` levels a side, merged into price
//!   groups where the settings ask for them, as steps: at each level's
//!   price, what rests from the side's best price through it, as an amount
//!   of the base asset and as its value in the quote asset. Nearer the
//!   other side than its best price a side has no depth; past its last
//!   level loaded it keeps that level's (the book is at least that deep
//!   there, and no order is made up).
//! - Window: the plot is centered on the price between the highest bid and
//!   the lowest ask of all the books, and shows the same distance either
//!   side of it: as far as every side is loaded (the nearest of the sides'
//!   last levels), or what it is told, a fraction of the center price.
//! - Marks: the plot marks prices at places that never move, a fixed
//!   number of points apart either way from its middle, and names each
//!   with the price that is there now. As the mid price moves, the curves
//!   and the marks stay where they are and only the names change (as the
//!   order book's rows stay put while their prices change).
//! - Scale: the plot's far edge stands for a round depth just above the
//!   deepest the window shows, with a rule at every step up to it, so the
//!   scale only changes when the book grows or shrinks past a step.
//! - Series: what the shader draws, four at most. Two books are drawn
//!   over one another (each book's bids and asks), stacked (the second's
//!   on top of the first's) or combined (their sum, as one book's).
//! - Columns: the plot is cut along the prices into columns a point wide,
//!   each holding how far every series reaches at its middle. A shader
//!   view takes 4096 bytes of uniforms, [`COLUMN_CAPACITY`] columns, so
//!   the plot is cut along the prices into tiles that long.

use std::rc::Rc;

use super::{
    settings::{Blend, DepthChartSettings, Unit},
    shader::COLUMN_CAPACITY,
};
use crate::{
    data::ferris::{BookLevel, OrderBook},
    widgets::common::books::{BookLayout, BookSettings, Layer, Pair, Side, pack},
};

/// The strip under the plot that names prices running along the bottom,
/// in points.
pub(super) const AXIS: f64 = 22.0;
/// The price column of a chart whose prices run down the pane, in points
/// (the order book's).
pub(super) const PRICE_COLUMN: f64 = 66.0;
/// The room a warning takes above the plot.
const WARNING: f64 = 20.0;
/// A tile's longest, in points: its columns and the one it adds at each
/// end fit a shader view's uniforms.
const TILE: f64 = (COLUMN_CAPACITY - 2) as f64;
/// The series the shader draws.
pub(super) const SERIES: usize = 4;
/// The narrowest and the widest window the user can zoom to: half its
/// prices over its center price (the old chart's 0.01% to 50%).
pub(super) const MIN_ZOOM: f64 = 0.000_1;
pub(super) const MAX_ZOOM: f64 = 0.5;
/// How much deeper than the deepest the window shows the scale reaches at
/// least.
const HEADROOM: f64 = 1.05;
/// The points between two price marks: four of the order book's rows
/// along the bottom, two down the pane.
const MARK_PITCH_ACROSS: f64 = 88.0;
const MARK_PITCH_DOWN: f64 = 44.0;

/// What the settings ask the frame for.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Config {
    pub layout: BookLayout,
    /// The levels a side shows.
    pub depth: usize,
    /// The price step levels merge into; zero for raw prices.
    pub grouping: f64,
    pub blend: Blend,
    pub unit: Unit,
    pub mid_price: bool,
    /// The books drawn, the primary first.
    pub layers: Vec<Layer>,
    pub warning: Option<&'static str>,
}

impl Config {
    pub fn of(settings: &DepthChartSettings) -> Self {
        Self {
            layout: settings.layout(),
            depth: usize::from(settings.books().depth()),
            grouping: settings.books().grouping().step(),
            blend: settings.blend(),
            unit: settings.unit(),
            mid_price: settings.mid_price(),
            layers: settings.layers(),
            warning: settings.warning(),
        }
    }

    /// No book, as a widget is before its settings arrive.
    pub fn empty() -> Self {
        Self {
            layout: BookLayout::Horizontal,
            depth: 0,
            grouping: 0.0,
            blend: Blend::Overlaid,
            unit: Unit::Amount,
            mid_price: true,
            layers: Vec::new(),
            warning: None,
        }
    }

    pub fn horizontal(&self) -> bool {
        self.layout == BookLayout::Horizontal
    }

    /// Whether the second book's depth is drawn on top of the first's.
    pub fn stacked(&self) -> bool {
        self.blend == Blend::Stacked && self.layers.len() > 1
    }

    /// Whether two books are drawn as their sum.
    pub fn combined(&self) -> bool {
        self.blend == Blend::Combined && self.layers.len() > 1
    }

    /// Whose colors series `series` is drawn in, and which side's: each
    /// book's bids and asks in its own, or the sum of two in the first
    /// book's. None for a series that is not drawn.
    pub fn tint(&self, series: usize) -> Option<(Pair, Side)> {
        let side = if series.is_multiple_of(2) {
            Side::Bid
        } else {
            Side::Ask
        };
        let layer = if self.combined() {
            (series < 2).then_some(0)?
        } else {
            series / 2
        };
        Some((self.layers.get(layer)?.pair, side))
    }

    /// The asset every book's depth is counted in; none while two books
    /// count in different ones.
    pub fn asset(&self) -> Option<&str> {
        let mut assets = self
            .layers
            .iter()
            .map(|layer| self.unit.asset(&layer.market));
        let first = assets.next()?;
        assets.all(|asset| asset == first).then_some(first)
    }

    /// Whether going from `self` to `next` draws other books, or other
    /// levels of them (another depth or grouping), which lets go of how
    /// wide the window was: one fitted to some levels says nothing of
    /// others.
    pub fn refitted_by(&self, next: &Self) -> bool {
        let keys = |config: &Self| -> Vec<String> {
            config
                .layers
                .iter()
                .map(|layer| layer.key.clone())
                .collect()
        };
        self.depth != next.depth || self.grouping != next.grouping || keys(self) != keys(next)
    }
}

/// One step of a curve: a price, and what rests from its side's best
/// price through it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Step {
    /// The price it is at: its group's when prices are grouped.
    pub price: f64,
    /// In the base asset.
    pub amount: f64,
    /// In the quote asset: each level's price times its amount, summed.
    pub value: f64,
}

impl Step {
    /// Its depth in `unit`.
    pub fn depth(&self, unit: Unit) -> f64 {
        match unit {
            Unit::Amount => self.amount,
            Unit::Value => self.value,
        }
    }
}

/// One side of one book as a curve: its steps from the best price outward.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Curve {
    /// Its book's place among the frame's layers.
    pub layer: usize,
    pub side: Side,
    pub steps: Vec<Step>,
}

impl Curve {
    /// The curve of one side's `levels`, as `config` shows them.
    fn of(layer: usize, side: Side, levels: &[BookLevel], config: &Config) -> Self {
        let mut value = 0.0;
        let steps = pack(levels, config.depth, config.grouping, side, false)
            .into_iter()
            .map(|level| {
                value += level.total.max(0.0);
                Step {
                    price: level.price,
                    amount: level.cumulative,
                    value,
                }
            })
            .collect();
        Self { layer, side, steps }
    }

    /// The step `price` is on: the last one from the best price that is no
    /// further out than it. None nearer the other side than the best
    /// price; the last step for any price past it.
    pub fn through(&self, price: f64) -> Option<&Step> {
        let reached = match self.side {
            Side::Bid => self.steps.partition_point(|step| step.price >= price),
            Side::Ask => self.steps.partition_point(|step| step.price <= price),
        };
        self.steps.get(reached.checked_sub(1)?)
    }

    /// What rests from the best price through `price`, in `unit`.
    fn depth(&self, price: f64, unit: Unit) -> f64 {
        self.through(price).map_or(0.0, |step| step.depth(unit))
    }
}

/// One shader view's stretch of the plot along the prices.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Tile {
    /// Where it starts and how long it is, in points from the plot's
    /// start along the prices.
    pub offset: f32,
    pub length: f32,
    /// Its columns among the frame's: from the one before the first it
    /// shows to the one after the last.
    pub first: usize,
    pub count: usize,
}

/// Where a plot marks prices, and how it writes them.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Marks {
    /// Where each mark is along the prices, in points as
    /// [`Frame::axis_of`] gives them, ascending: the plot's middle, and
    /// every whole pitch from it that is inside the plot.
    pub at: Vec<f64>,
    /// The decimals their prices are written with.
    pub places: usize,
}

/// Everything the depth chart's view draws.
#[derive(Clone, Debug)]
pub(super) struct Frame {
    pub config: Rc<Config>,
    /// The widget's size below its top bar, in points.
    pub width: f64,
    pub height: f64,
    /// The plot: right of the price column, below the warning, above the
    /// strip of prices.
    pub plot_left: f64,
    pub plot_top: f64,
    pub plot_right: f64,
    pub plot_bottom: f64,
    /// The price column's width; none when prices run along the bottom.
    pub price_column: f64,
    /// The price at the plot's middle.
    pub center: f64,
    /// How far the plot's prices go either side of the center.
    pub span: f64,
    /// How far they go when nothing else is asked for: as far as every
    /// side is loaded.
    pub fit: f64,
    /// The depth the plot's far edge stands for, and the step between its
    /// rules.
    pub top: f64,
    pub rule: f64,
    /// A book brought a price: there is something to point at.
    pub has_prices: bool,
    /// The price between the highest bid and the lowest ask of all the
    /// books, when they have both.
    pub mid: Option<f64>,
    /// Each layer's price between its best bid and ask, when it has both.
    pub mids: Vec<Option<f64>>,
    /// The curves, book by book: its bids, then its asks.
    pub curves: Vec<Curve>,
    /// Whether each layer's book has arrived.
    pub loaded: Vec<bool>,
    /// Whether each layer's topic was refused.
    pub failed: Vec<bool>,
    /// The plot's columns along the prices, a point each, from the one
    /// before the plot to the one after it: how far each series reaches
    /// across the plot at the column's middle, 0 to 1.
    pub columns: Vec<[f32; SERIES]>,
    pub tiles: Vec<Tile>,
}

impl Frame {
    /// Nothing to draw yet.
    pub fn empty() -> Self {
        Self::build(Rc::new(Config::empty()), &[], 1.0, 1.0, None, None)
    }

    /// What a widget of `width` by `height` points shows of `books` (one
    /// per layer of `config`, none while it has not arrived): as far
    /// either side of the books' mid price as `zoom` says (half the window
    /// over its center price), or as every side is loaded while it says
    /// nothing. Without a price it stays about `held`, where an earlier
    /// frame was.
    pub fn build(
        config: Rc<Config>,
        books: &[Option<Rc<OrderBook>>],
        width: f64,
        height: f64,
        zoom: Option<f64>,
        held: Option<f64>,
    ) -> Self {
        let sized = |length: f64| {
            if length.is_finite() {
                length.max(1.0)
            } else {
                1.0
            }
        };
        let (width, height) = (sized(width), sized(height));
        let horizontal = config.horizontal();
        let price_column = if horizontal {
            0.0
        } else {
            PRICE_COLUMN.min((width - 1.0).max(0.0))
        };
        let plot_top = (height - 1.0).min(if config.warning.is_some() {
            WARNING
        } else {
            0.0
        });
        let axis = if horizontal { AXIS } else { 0.0 };
        let plot_bottom = (plot_top + 1.0).max(height - axis);

        let mut curves = Vec::with_capacity(config.layers.len() * 2);
        let mut mids = Vec::with_capacity(config.layers.len());
        let mut loaded = Vec::with_capacity(config.layers.len());
        // The highest bid and the lowest ask of all the books, as quoted.
        let (mut bid, mut ask) = (None::<f64>, None::<f64>);
        for layer in 0..config.layers.len() {
            let book = books.get(layer).and_then(Option::as_ref);
            loaded.push(book.is_some());
            mids.push(book.and_then(|book| book.midpoint()));
            let none: &[BookLevel] = &[];
            let (bids, asks) = book.map_or((none, none), |book| {
                (book.bids.as_slice(), book.asks.as_slice())
            });
            let best = |levels: &[BookLevel]| {
                let price = levels.first()?.price;
                price.is_finite().then_some(price)
            };
            if let Some(price) = best(bids) {
                bid = Some(bid.map_or(price, |bid| bid.max(price)));
            }
            if let Some(price) = best(asks) {
                ask = Some(ask.map_or(price, |ask| ask.min(price)));
            }
            curves.push(Curve::of(layer, Side::Bid, bids, &config));
            curves.push(Curve::of(layer, Side::Ask, asks, &config));
        }
        let mid = bid.zip(ask).map(|(bid, ask)| (bid + ask) / 2.0);
        let fitted = mid.or(bid).or(ask).filter(|price| price.is_finite());
        let center = fitted
            .or(held.filter(|held| held.is_finite()))
            .unwrap_or(0.0);

        // As far as every side is loaded: the nearest of their last levels.
        let loaded_to = curves
            .iter()
            .filter_map(|curve| curve.steps.last())
            .map(|step| (step.price - center).abs())
            .filter(|reach| *reach > 0.0)
            .fold(f64::INFINITY, f64::min);
        let (least, most) = (center.abs() * MIN_ZOOM, center.abs() * MAX_ZOOM);
        // A window is some prices wide, whatever the books say.
        let wide = |span: f64| {
            if span.is_finite() && span > 0.0 {
                span
            } else {
                1.0
            }
        };
        let fit = wide(if !loaded_to.is_finite() {
            least
        } else if most > 0.0 {
            loaded_to.clamp(least, most)
        } else {
            loaded_to
        });
        let span = match zoom.filter(|zoom| zoom.is_finite() && center > 0.0) {
            Some(zoom) => wide(zoom.clamp(MIN_ZOOM, MAX_ZOOM) * center),
            None => fit,
        };

        let mut frame = Self {
            config,
            width,
            height,
            plot_left: price_column,
            plot_top,
            plot_right: width,
            plot_bottom,
            price_column,
            center,
            span,
            fit,
            top: 1.0,
            rule: 0.25,
            has_prices: fitted.is_some(),
            mid,
            mids,
            curves,
            loaded,
            failed: Vec::new(),
            columns: Vec::new(),
            tiles: Vec::new(),
        };
        // Depth only grows outward, so the window's edges hold its deepest:
        // the bids' at its lowest price, the asks' at its highest.
        let (low, high) = (frame.depths(center - span), frame.depths(center + span));
        let deepest = low[0].max(low[2]).max(high[1]).max(high[3]);
        (frame.rule, frame.top) = scale(deepest);

        let length = frame.length();
        let count = length.ceil() as usize + 2;
        let start = frame.start();
        frame.columns = (0..count)
            .map(|column| {
                // Its middle: the first column is the one before the plot.
                let price = frame.price_at(start + column as f64 - 0.5);
                frame
                    .depths(price)
                    .map(|depth| (depth / frame.top).clamp(0.0, 1.0) as f32)
            })
            .collect();
        let pieces = (length / TILE).ceil().max(1.0) as usize;
        frame.tiles = (0..pieces)
            .map(|piece| {
                let from = piece as f64 * TILE;
                let to = (from + TILE).min(length);
                let first = from as usize;
                Tile {
                    offset: from as f32,
                    length: (to - from) as f32,
                    first,
                    count: ((to - from).ceil() as usize + 2).min(count.saturating_sub(first)),
                }
            })
            .collect();
        frame
    }

    /// How deep each series is at `price`, in the settings' unit: each
    /// book's bids and asks, or the first book's and then both books'
    /// (stacked), or both books' alone (combined).
    fn depths(&self, price: f64) -> [f64; SERIES] {
        let unit = self.config.unit;
        let of = |curve: usize| {
            self.curves
                .get(curve)
                .map_or(0.0, |curve| curve.depth(price, unit))
        };
        let (bid, ask, pair_bid, pair_ask) = (of(0), of(1), of(2), of(3));
        if self.config.combined() {
            [bid + pair_bid, ask + pair_ask, 0.0, 0.0]
        } else if self.config.stacked() {
            [bid, ask, bid + pair_bid, ask + pair_ask]
        } else {
            [bid, ask, pair_bid, pair_ask]
        }
    }

    /// Where the plot starts along the prices: its left edge, or its top
    /// where they run down the pane.
    fn start(&self) -> f64 {
        if self.config.horizontal() {
            self.plot_left
        } else {
            self.plot_top
        }
    }

    /// The plot's length along the prices.
    pub fn length(&self) -> f64 {
        if self.config.horizontal() {
            self.plot_right - self.plot_left
        } else {
            self.plot_bottom - self.plot_top
        }
        .max(1.0)
    }

    /// The plot's length across the prices, which depth is drawn along.
    pub fn breadth(&self) -> f64 {
        if self.config.horizontal() {
            self.plot_bottom - self.plot_top
        } else {
            self.plot_right - self.plot_left
        }
        .max(1.0)
    }

    /// Where `price` is along the prices, in points: right from the
    /// widget's left edge (higher prices further right), or down from its
    /// top (higher prices higher up).
    pub fn axis_of(&self, price: f64) -> f64 {
        let along = (price - (self.center - self.span)) / (2.0 * self.span) * self.length();
        if self.config.horizontal() {
            self.plot_left + along
        } else {
            self.plot_bottom - along
        }
    }

    /// The price at `axis` points along the prices.
    pub fn price_at(&self, axis: f64) -> f64 {
        let along = if self.config.horizontal() {
            axis - self.plot_left
        } else {
            self.plot_bottom - axis
        };
        self.center - self.span + along / self.length() * 2.0 * self.span
    }

    /// Where a depth of `depth` reaches across the plot, in points: up
    /// from the plot's bottom, or right from its left edge where prices
    /// run down the pane.
    pub fn reach_of(&self, depth: f64) -> f64 {
        let reach = (depth / self.top).clamp(0.0, 1.0) * self.breadth();
        if self.config.horizontal() {
            self.plot_bottom - reach
        } else {
            self.plot_left + reach
        }
    }

    /// Whether a price at `axis` shows: inside the plot along the prices.
    pub fn shows(&self, axis: f64) -> bool {
        if self.config.horizontal() {
            axis >= self.plot_left && axis <= self.plot_right
        } else {
            axis >= self.plot_top && axis <= self.plot_bottom
        }
    }

    /// The price under a pointer at (`x`, `y`), while it is over the plot
    /// and the plot has prices.
    pub fn pointed(&self, x: f64, y: f64) -> Option<f64> {
        let inside = (self.plot_left..=self.plot_right).contains(&x)
            && (self.plot_top..=self.plot_bottom).contains(&y);
        (self.has_prices && inside)
            .then(|| self.price_at(if self.config.horizontal() { x } else { y }))
    }

    /// What `layer`'s `side` holds from its best price through `price`.
    pub fn through(&self, layer: usize, side: Side, price: f64) -> Option<&Step> {
        self.curves
            .iter()
            .find(|curve| curve.layer == layer && curve.side == side)?
            .through(price)
    }

    /// The depths the plot rules: every step up to its far edge.
    pub fn rules(&self) -> impl Iterator<Item = f64> + use<> {
        let (rule, top) = (self.rule, self.top);
        let count = if rule > 0.0 {
            (top / rule).round().clamp(0.0, 64.0) as usize
        } else {
            0
        };
        (1..=count).map(move |at| at as f64 * rule)
    }

    /// Whether every book has arrived, or never will (`failed`: whether
    /// each layer's topic was refused): what the frame fits is then what
    /// its books hold, not what some of them brought so far.
    pub fn settled(&self, failed: &[bool]) -> bool {
        let loaded = self.loaded.iter().enumerate();
        loaded.fold(self.has_prices, |settled, (layer, loaded)| {
            settled && (*loaded || failed.get(layer).copied().unwrap_or(false))
        })
    }

    /// Where the plot marks prices: at its middle and every whole pitch
    /// from it, inside the plot. The places depend on the plot's size
    /// alone, never on its prices: what a mark names is the price that is
    /// there ([`Self::price_at`]).
    pub fn marks(&self) -> Marks {
        let pitch = if self.config.horizontal() {
            MARK_PITCH_ACROSS
        } else {
            MARK_PITCH_DOWN
        };
        let (start, length) = (self.start(), self.length());
        let middle = start + length / 2.0;
        // Short of the plot's edges; a plot has room for a few dozen.
        let a_side = ((length / 2.0 / pitch).ceil() - 1.0).clamp(0.0, 128.0) as i64;
        Marks {
            at: (-a_side..=a_side)
                .map(|at| middle + at as f64 * pitch)
                .collect(),
            places: places(2.0 * self.span * pitch / length),
        }
    }
}

/// The step between depth rules and the depth at the plot's far edge for
/// a window whose deepest is `deepest`: a round step that goes into it
/// three to six times, and the first rule clear of it.
fn scale(deepest: f64) -> (f64, f64) {
    if !(deepest.is_finite() && deepest > 0.0) {
        return (0.25, 1.0);
    }
    let step = round_down(deepest / 3.0);
    let top = (deepest * HEADROOM / step).ceil() * step;
    if top.is_finite() && top > 0.0 {
        (step, top)
    } else {
        (deepest / 4.0, deepest)
    }
}

/// The power of ten `value` is in, and `value` over it (1 to 10).
fn magnitude(value: f64) -> (f64, f64) {
    let power = 10_f64.powf(value.log10().floor());
    let digits = value / power;
    // A power of ten whose logarithm came out a hair under a whole number.
    if digits >= 10.0 - 1e-9 {
        (power * 10.0, 1.0)
    } else {
        (power, digits)
    }
}

/// The largest of 1, 2, 2.5 and 5 times a power of ten that is at most
/// `value`.
fn round_down(value: f64) -> f64 {
    let (power, digits) = magnitude(value);
    power
        * match digits {
            digits if digits >= 5.0 => 5.0,
            digits if digits >= 2.5 => 2.5,
            digits if digits >= 2.0 => 2.0,
            _ => 1.0,
        }
}

/// How many decimals prices `apart` from one another are written with,
/// eight at most: those that tell a fifth of what is between them. Marks
/// 7 apart are whole numbers, marks 3 apart have a decimal: coarse enough
/// that a name changes when its price really has, not on every tick.
pub(super) fn places(apart: f64) -> usize {
    if !(apart.is_finite() && apart > 0.0) {
        return 0;
    }
    (-(apart / 5.0).log10()).ceil().clamp(0.0, 8.0) as usize
}
