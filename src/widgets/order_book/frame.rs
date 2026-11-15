//! What the order book draws for its books, a center price and a plot
//! size: every level that shows, where its depth bar is, and the rows the
//! shader draws them from ([`super::shader`]). The old
//! `OrderBookFrameBuilder`, with its arithmetic kept as it was.
//!
//! - Levels: each book's best `depth` levels a side, merged into price
//!   groups where the settings ask for them (bids down to the group's
//!   price, asks up to it). A level's bar is as long as its running amount
//!   over the deepest of its book: two books scale apart, on one price
//!   ladder.
//! - Rows: the ladder is rows of [`ROW`] points, either one per price that
//!   holds orders (the default: empty prices take no room) or one per
//!   price step ("Show all levels"). The center price sits in the row at
//!   the plot's middle; that row's edge, not the price, is what stays put,
//!   so rows never drift by a fraction as the price moves inside its row.
//! - The last level loaded of each side says how deep the book is at
//!   least: its bar runs on to the plot's edge unless the settings stop it
//!   at the loaded levels.
//! - Tiles: a shader view takes 4096 bytes of uniforms, [`ROW_CAPACITY`]
//!   rows, so the plot is cut along the ladder into tiles that long. One
//!   holds any pane a display has room for.

use std::rc::Rc;

use super::{
    settings::{OrderBookSettings, Toggle},
    shader::ROW_CAPACITY,
};
use crate::{
    data::ferris::{BookLevel, OrderBook},
    widgets::common::books::{self, BookLayout, Layer, Side, round},
};

/// A row of the ladder, in points (the old `ORDER_BOOK_PRICE_ROW_SIZE`).
pub(super) const ROW: f64 = 22.0;
/// The price column of a vertical ladder, in points.
pub(super) const PRICE_COLUMN: f64 = 66.0;
/// A depth bar's thickness: a row less the point between two.
const BAR: f64 = ROW - 1.0;
/// The room a warning takes above the plot.
const WARNING: f64 = 20.0;
/// A tile's longest, in points: its rows and the ones it adds at its ends
/// fit a shader view's uniforms.
const TILE: f64 = (ROW_CAPACITY - 3) as f64 * ROW;

/// What the settings ask the frame for.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Config {
    pub layout: BookLayout,
    /// The levels a side shows.
    pub depth: usize,
    /// The price step levels merge into; zero for raw prices.
    pub grouping: f64,
    pub all_levels: bool,
    pub only_loaded: bool,
    pub mid_price: bool,
    pub divider: bool,
    pub cumulative: bool,
    pub separate: bool,
    /// The books drawn, the primary first.
    pub layers: Vec<Layer>,
    pub warning: Option<&'static str>,
}

impl Config {
    pub fn of(settings: &OrderBookSettings) -> Self {
        Self {
            layout: settings.layout(),
            depth: usize::from(settings.depth()),
            grouping: settings.grouping().step(),
            all_levels: settings.shows(Toggle::AllLevels),
            only_loaded: settings.shows(Toggle::OnlyLoaded),
            mid_price: settings.shows(Toggle::MidPrice),
            divider: settings.shows(Toggle::Divider),
            cumulative: settings.shows(Toggle::Cumulative),
            separate: settings.shows(Toggle::Separate),
            layers: settings.layers(),
            warning: settings.warning(),
        }
    }

    /// No book, as a widget is before its settings arrive.
    pub fn empty() -> Self {
        Self {
            layout: BookLayout::Vertical,
            depth: 0,
            grouping: 0.0,
            all_levels: false,
            only_loaded: false,
            mid_price: true,
            divider: false,
            cumulative: false,
            separate: false,
            layers: Vec::new(),
            warning: None,
        }
    }

    pub fn horizontal(&self) -> bool {
        self.layout == BookLayout::Horizontal
    }

    /// Whether going from `self` to `next` lays the ladder out anew, which
    /// lets go of where the user moved it (the old `setConfiguration`):
    /// another layout, grouping or row rule, or other books.
    pub fn relaid_by(&self, next: &Self) -> bool {
        let keys = |config: &Self| -> Vec<String> {
            config
                .layers
                .iter()
                .map(|layer| layer.key.clone())
                .collect()
        };
        self.layout != next.layout
            || self.grouping != next.grouping
            || self.all_levels != next.all_levels
            || keys(self) != keys(next)
    }
}

/// One level the frame shows: a price of one side of one book.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Level {
    /// Its book's place among the frame's layers.
    pub layer: usize,
    pub side: Side,
    /// The price its row is at: its group's when prices are grouped.
    pub price: f64,
    pub amount: f64,
    pub total: f64,
    /// The amounts from its side's best price through it.
    pub cumulative: f64,
    /// Its running amount over the deepest of its book, 0 to 1: how far
    /// its bar reaches.
    pub reach: f64,
    /// It is its side's last level loaded and runs on to the plot's edge.
    pub open: bool,
    /// Its bar in points from the widget's top left: left, top, right and
    /// bottom.
    pub rect: [f64; 4],
}

/// One shader view's stretch of the plot along the ladder.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Tile {
    /// Where it starts and how long it is, in points from the plot's
    /// start along the ladder.
    pub offset: f32,
    pub length: f32,
    /// Where its first row starts, in points from its own start (at or
    /// before it).
    pub start: f32,
    /// Its rows, from the one before the first it shows to the one after
    /// the last: per row, how far each series' bar reaches (the first
    /// book's bids and asks, then the second's), as [`reach`] packs it.
    pub rows: Vec<[f32; 4]>,
}

/// How a series' bar joins its row's neighbors, packed with its reach.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Join {
    /// A bar of its own, a point clear of the rows beside it.
    Apart,
    /// The last level loaded: joined to the rows beyond it.
    Outward,
    /// Beyond the last level loaded: joined on both sides.
    Through,
}

/// A bar's reach (0 to 1) and how it joins its neighbors, as one number
/// the shader takes apart again: the reach plus two for each step of
/// `join`.
pub(super) fn reach(reach: f32, join: Join) -> f32 {
    reach.clamp(0.0, 1.0) + 2.0 * f32::from(join as u8)
}

/// The reach and the join [`reach`] packed.
fn unpacked(packed: f32) -> (f32, Join) {
    let join = match (packed / 2.0).floor() as u8 {
        0 => Join::Apart,
        1 => Join::Outward,
        _ => Join::Through,
    };
    (packed - 2.0 * f32::from(join as u8), join)
}

/// Everything the order book's view draws.
#[derive(Clone, Debug)]
pub(super) struct Frame {
    pub config: Rc<Config>,
    /// The widget's size below its top bar, in points.
    pub width: f64,
    pub height: f64,
    /// The plot: right of the price column, below the warning.
    pub plot_left: f64,
    pub plot_top: f64,
    pub plot_right: f64,
    pub plot_bottom: f64,
    /// Where bars start: the plot's left edge, or its bottom when prices
    /// run along it.
    pub baseline: f64,
    /// The price row zero is at while every price step has a row.
    pub anchor_price: f64,
    /// The price the ladder is centered on.
    pub center_price: f64,
    /// The row at the plot's middle: a row's middle, never a fraction of
    /// one.
    pub center_row: f64,
    /// The price a row stands for where no level says.
    pub step: f64,
    /// The prices that hold orders, ascending, each a row; empty while
    /// every price step has one.
    pub occupied: Vec<f64>,
    /// The price column's width; none when prices run along the bottom.
    pub price_column: f64,
    /// A book brought a price: there is something to point at.
    pub has_prices: bool,
    /// The levels, book by book: bids from the best down, then asks from
    /// the best up.
    pub levels: Vec<Level>,
    /// Each layer's price between its best bid and ask, when it has both.
    pub mids: Vec<Option<f64>>,
    /// Whether each layer's book has arrived.
    pub loaded: Vec<bool>,
    /// Whether each layer's topic was refused.
    pub failed: Vec<bool>,
    pub tiles: Vec<Tile>,
}

impl Frame {
    /// Nothing to draw yet.
    pub fn empty() -> Self {
        Self::build(Rc::new(Config::empty()), &[], 1.0, 1.0, None, None)
    }

    /// What a widget of `width` by `height` points shows of `books` (one
    /// per layer of `config`, none while it has not arrived): centered on
    /// `center` and stepping by `step` where the user or an earlier frame
    /// fixed them, else on the first book's mid price and by the step the
    /// prices imply.
    pub fn build(
        config: Rc<Config>,
        books: &[Option<Rc<OrderBook>>],
        width: f64,
        height: f64,
        center: Option<f64>,
        step: Option<f64>,
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
        let plot_left = price_column;
        let plot_top = (height - 1.0).min(if config.warning.is_some() {
            WARNING
        } else {
            0.0
        });
        let plot_right = width;
        let plot_bottom = (plot_top + 1.0).max(height);
        let plot_width = (plot_right - plot_left).max(1.0);
        let plot_height = (plot_bottom - plot_top).max(1.0);

        let mut levels = Vec::new();
        let mut mids = Vec::with_capacity(config.layers.len());
        let mut loaded = Vec::with_capacity(config.layers.len());
        for layer in 0..config.layers.len() {
            let book = books.get(layer).and_then(Option::as_ref);
            loaded.push(book.is_some());
            mids.push(book.and_then(|book| book.midpoint()));
            if let Some(book) = book {
                pack(&mut levels, &book.bids, &config, Side::Bid, layer);
                pack(&mut levels, &book.asks, &config, Side::Ask, layer);
            }
        }
        // Each book's deepest running amount: what its bars are scaled by.
        let mut deepest = vec![0.0_f64; config.layers.len()];
        for level in &levels {
            if let Some(deepest) = deepest.get_mut(level.layer)
                && level.cumulative > *deepest
            {
                *deepest = level.cumulative;
            }
        }
        let mut prices: Vec<f64> = levels.iter().map(|level| level.price).collect();
        prices.sort_by(f64::total_cmp);

        let first_mid = mids.iter().copied().flatten().next();
        let primary_mid = mids.first().copied().flatten();
        let fitted =
            primary_mid
                .or(first_mid)
                .unwrap_or_else(|| match (prices.first(), prices.last()) {
                    (Some(lowest), Some(highest)) => (lowest + highest) / 2.0,
                    _ => 0.0,
                });
        let center_price = center.filter(|center| center.is_finite()).unwrap_or(fitted);
        let implied = if config.grouping > 0.0 {
            config.grouping
        } else {
            raw_step(&prices, center_price)
        };
        let step = step
            .filter(|step| step.is_finite() && *step > 0.0)
            .unwrap_or(implied);
        let anchor_price = if config.grouping > 0.0 {
            0.0
        } else {
            prices.first().copied().unwrap_or(center_price)
        };
        let has_prices = !levels.is_empty() || first_mid.is_some();
        let occupied = if config.all_levels {
            Vec::new()
        } else {
            prices.dedup();
            prices
        };

        let mut frame = Self {
            config,
            width,
            height,
            plot_left,
            plot_top,
            plot_right,
            plot_bottom,
            baseline: if horizontal { plot_bottom } else { plot_left },
            anchor_price,
            center_price,
            center_row: 0.5,
            step,
            occupied,
            price_column,
            has_prices,
            levels,
            mids,
            loaded,
            failed: Vec::new(),
            tiles: Vec::new(),
        };
        // The plot's middle is a row's middle, never a fraction of a row:
        // only the price's own mark moves inside its row as the book does.
        let target = frame.row_of(center_price);
        let tolerance = f64::EPSILON * target.abs().max(1.0) * 4.0;
        frame.center_row = (target + tolerance).floor() + 0.5;

        let half = BAR / 2.0;
        let only_loaded = frame.config.only_loaded;
        for index in 0..frame.levels.len() {
            let (layer, side, price, cumulative) = {
                let level = &frame.levels[index];
                (level.layer, level.side, level.price, level.cumulative)
            };
            let deepest = deepest.get(layer).copied().unwrap_or(0.0);
            let reach = if deepest > 0.0 {
                (cumulative / deepest).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let axis = frame.axis_of(price);
            let [mut x1, mut y1, mut x2, mut y2] = if horizontal {
                [
                    axis - half,
                    plot_top.max(frame.baseline - reach * plot_height),
                    axis + half,
                    frame.baseline,
                ]
            } else {
                [
                    frame.baseline,
                    axis - half,
                    plot_right.min(frame.baseline + reach * plot_width),
                    axis + half,
                ]
            };
            // The last level loaded caps how deep the book is, not how far
            // prices go: its bar runs on outward, with no order made up.
            let last = frame
                .levels
                .get(index + 1)
                .is_none_or(|next| next.layer != layer || next.side != side);
            let open = last && !only_loaded;
            if open {
                match (horizontal, side) {
                    (false, Side::Bid) => y2 = y2.max(plot_bottom),
                    (false, Side::Ask) => y1 = y1.min(plot_top),
                    (true, Side::Bid) => x1 = x1.min(plot_left),
                    (true, Side::Ask) => x2 = x2.max(plot_right),
                }
            }
            let level = &mut frame.levels[index];
            level.reach = reach;
            level.open = open;
            level.rect = [x1, y1, x2, y2];
        }
        frame.tiles = frame.cut();
        frame
    }

    /// The row `price` is at; a fraction between two rows' prices.
    ///
    /// Prices that hold orders share one row each across the books;
    /// between two of them a price is a fraction of the way, and past
    /// either end rows go on by the step.
    pub fn row_of(&self, price: f64) -> f64 {
        let prices = &self.occupied;
        let (Some(&lowest), Some(&highest)) = (prices.first(), prices.last()) else {
            return (price - self.anchor_price) / self.step;
        };
        let last = prices.len() - 1;
        if price <= lowest {
            return (price - lowest) / self.step;
        }
        if price >= highest {
            return last as f64 + (price - highest) / self.step;
        }
        let (mut low, mut high) = (0, last);
        while high - low > 1 {
            let middle = (low + high) / 2;
            if prices[middle] <= price {
                low = middle;
            } else {
                high = middle;
            }
        }
        low as f64 + (price - prices[low]) / (prices[high] - prices[low])
    }

    /// The price row `row` stands for.
    pub fn price_of(&self, row: f64) -> f64 {
        let prices = &self.occupied;
        let (Some(&lowest), Some(&highest)) = (prices.first(), prices.last()) else {
            return self.anchor_price + row * self.step;
        };
        let last = prices.len() - 1;
        if row <= 0.0 {
            return lowest + row * self.step;
        }
        if row >= last as f64 {
            return highest + (row - last as f64) * self.step;
        }
        let low = row.floor() as usize;
        match (prices.get(low), prices.get(low + 1)) {
            (Some(&from), Some(&to)) => from + (row - low as f64) * (to - from),
            _ => highest,
        }
    }

    /// The plot's middle along the ladder.
    fn middle(&self) -> f64 {
        if self.config.horizontal() {
            (self.plot_left + self.plot_right) / 2.0
        } else {
            (self.plot_top + self.plot_bottom) / 2.0
        }
    }

    /// Where `price` is along the ladder, in points: down from the
    /// widget's top (higher prices higher up), or right from its left edge
    /// (higher prices further right). The old `mapOrderBookPriceToAxis`.
    pub fn axis_of(&self, price: f64) -> f64 {
        let offset = (self.row_of(price) - self.center_row) * ROW;
        if self.config.horizontal() {
            self.middle() + offset
        } else {
            self.middle() - offset
        }
    }

    /// The price at `axis` points along the ladder (the old
    /// `mapOrderBookAxisToPrice`).
    pub fn price_at(&self, axis: f64) -> f64 {
        let offset = if self.config.horizontal() {
            axis - self.middle()
        } else {
            self.middle() - axis
        };
        self.price_of(self.center_row + offset / ROW)
    }

    /// The center price `rows` rows up the ladder (toward higher prices).
    pub fn shifted_center(&self, rows: f64) -> f64 {
        self.price_of(self.row_of(self.center_price) + rows)
    }

    /// Whether rows stand for a price step at all (a step that is no
    /// number does not).
    pub fn steps(&self) -> bool {
        self.step > 0.0
    }

    /// Whether a row at `axis` shows: inside the plot along the ladder.
    pub fn shows(&self, axis: f64) -> bool {
        if self.config.horizontal() {
            axis >= self.plot_left && axis <= self.plot_right
        } else {
            axis >= self.plot_top && axis <= self.plot_bottom
        }
    }

    /// The plot's rows as the shader draws them, cut into tiles.
    ///
    /// Rows are counted along the screen from the plot's middle, whose
    /// edges every row's are whole rows from: a level is in the row its
    /// middle is in. (Where a stale step puts two levels of one series in
    /// one row, the longer bar is drawn.)
    fn cut(&self) -> Vec<Tile> {
        let horizontal = self.config.horizontal();
        let (start, end) = if horizontal {
            (self.plot_left, self.plot_right)
        } else {
            (self.plot_top, self.plot_bottom)
        };
        let middle = self.middle();
        let row_at = |axis: f64| ((axis - middle) / ROW).floor() as i64;
        let after = |axis: f64| ((axis - middle) / ROW).ceil() as i64;
        let (first, last) = (row_at(start) - 1, after(end));
        let mut rows = vec![[0.0_f32; 4]; (last - first + 1).max(1) as usize];
        let mut put = |row: i64, series: usize, reached: f32, join: Join| {
            let cell = usize::try_from(row - first)
                .ok()
                .and_then(|at| rows.get_mut(at))
                .and_then(|row| row.get_mut(series));
            if let Some(cell) = cell {
                let (before, joined) = unpacked(*cell);
                *cell = reach(before.max(reached), joined.max(join));
            }
        };
        for level in &self.levels {
            let series = level.layer * 2 + usize::from(level.side == Side::Ask);
            let row = row_at(self.axis_of(level.price));
            let reached = level.reach as f32;
            if !level.open {
                put(row, series, reached, Join::Apart);
                continue;
            }
            put(row, series, reached, Join::Outward);
            // Outward is down the screen for bids and up for asks where
            // prices run down it; across it, left for bids, right for asks.
            let onward = (level.side == Side::Bid) != horizontal;
            let beyond = if onward {
                row.saturating_add(1).max(first)..=last
            } else {
                first..=row.saturating_sub(1).min(last)
            };
            for row in beyond {
                put(row, series, reached, Join::Through);
            }
        }

        let pieces = ((end - start) / TILE).ceil().max(1.0) as usize;
        (0..pieces)
            .map(|piece| {
                let from = start + piece as f64 * TILE;
                let to = (from + TILE).min(end);
                let (opens, closes) = (row_at(from) - 1, after(to));
                let held = |row: i64| usize::try_from(row - first).unwrap_or(0).min(rows.len());
                Tile {
                    offset: (from - start) as f32,
                    length: (to - from) as f32,
                    start: (middle + opens as f64 * ROW - from) as f32,
                    rows: rows[held(opens)..held(closes + 1)].to_vec(),
                }
            })
            .collect()
    }
}

/// Adds one side of a book to `levels`: its best `depth` levels, each a
/// level of its own, or merged while they share a price group
/// ([`books::pack`]). A level with no amount is left out unless every
/// price step has a row.
fn pack(
    levels: &mut Vec<Level>,
    side_levels: &[BookLevel],
    config: &Config,
    side: Side,
    layer: usize,
) {
    let packed = books::pack(
        side_levels,
        config.depth,
        config.grouping,
        side,
        config.all_levels,
    );
    levels.extend(packed.into_iter().map(|packed| Level {
        layer,
        side,
        price: packed.price,
        amount: packed.amount,
        total: packed.total,
        cumulative: packed.cumulative,
        reach: 0.0,
        open: false,
        rect: [0.0; 4],
    }));
}

/// How many decimals `value` is written with, at most eight.
fn decimals(value: f64) -> i32 {
    let text = format!("{:.8}", value.abs());
    let text = text.trim_end_matches('0');
    text.find('.')
        .map_or(0, |point| (text.len() - point - 1) as i32)
}

/// The step of a ladder of raw prices: the largest one every price is a
/// whole number of steps from the lowest by, in the decimals they are
/// written with (the old `inferRawLadderStep`). Without prices, or with
/// ones too large to count exactly, one unit of `center`'s last decimal.
fn raw_step(sorted: &[f64], center: f64) -> f64 {
    let unit = |price: f64| 10_f64.powi(-decimals(price));
    let Some(&lowest) = sorted.first() else {
        return unit(center);
    };
    let places = sorted
        .iter()
        .map(|price| decimals(*price))
        .max()
        .unwrap_or(0);
    let scale = 10_f64.powi(places);
    // Whole numbers a float holds exactly.
    let whole = |price: f64| {
        let scaled = round(price * scale);
        (scaled.abs() <= 9_007_199_254_740_991.0).then_some(scaled)
    };
    let Some(base) = whole(lowest) else {
        return unit(center);
    };
    let mut divisor = 0_u64;
    for price in &sorted[1..] {
        let Some(scaled) = whole(*price) else {
            return unit(center);
        };
        divisor = gcd(divisor, (scaled - base).abs() as u64);
    }
    if divisor > 0 {
        divisor as f64 / scale
    } else {
        unit(lowest)
    }
}

fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}
