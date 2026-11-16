//! What the order book draws over its depth bars, on one canvas: the price
//! ladder, each row's quantities, the mid price and bid/ask lines with
//! their labels, the warning, the status, and the pointer's line and its
//! box of the level under it. The old `OrderBookOverlayRenderer`, drawn
//! with NSH's canvas (the same 2D operations) instead of a second
//! `<canvas>`.
//!
//! The chart draws its labels as views; here a frame has a text or two for
//! every row and book, and prices running across the pane are written
//! turned on their side, so one drawing replaces a few hundred views that
//! would each change several times a second.
//!
//! Where no shader can draw the bars (no GPU, or a build without NSH's
//! shader renderer), the overlay draws them too, under the rest, as the
//! rectangles the old renderer drew: the widget needs no GPU to work.
//!
//! Text is placed without measuring it ([`width_of`]), as the chart's
//! labels are placed. The tiny exchange logos beside the line labels are
//! not drawn here (a canvas draws no SVG): [`draw`] says where they go,
//! and the view puts them there.

use std::collections::HashMap;

use nsh_core::{
    canvas::{CanvasTextAlign, CanvasTextBaseline},
    prelude::*,
};

use super::{
    frame::{Frame, ROW},
    shader::BAR_ALPHA,
};
use crate::{
    exchanges,
    format::{compact_count, price_text},
    widgets::common::{
        FONT, Pen, SMALL_FONT,
        books::{Colors, Drawn, Logo, Side, round, status},
        width_of,
    },
};

/// The side of a line label's exchange logo.
const LOGO: f64 = 8.0;

/// A line across the plot (a book's mid price, or the edge between its
/// bids and asks) and its label.
struct Marker {
    layer: usize,
    /// Where the line is along the ladder.
    axis: f64,
    /// Where the line ends across the plot.
    extent: f64,
    text: String,
    /// The exchange whose logo goes before the text, if one does.
    exchange: Option<String>,
    color: Hsla,
    /// The label's box.
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

impl Marker {
    fn overlaps(&self, other: &Self) -> bool {
        self.x < other.x + other.width
            && self.x + self.width > other.x
            && self.y < other.y + other.height
            && self.y + self.height > other.y
    }
}

/// The overlay of `frame` in `colors`, with the pointer at `pointer`
/// (points from the canvas's top left) while it is over the widget.
/// With `bars`, the depth bars too, under everything else: what the
/// widget shows them with where no shader can draw them.
pub(super) fn draw(
    frame: &Frame,
    pointer: Option<(f64, f64)>,
    colors: &Colors,
    bars: bool,
) -> Drawn {
    let config = &frame.config;
    let mut pen = Pen(Painter::new());
    let mut logos = Vec::new();
    pen.save();
    pen.font(FONT);
    pen.with(|painter| painter.text_baseline(CanvasTextBaseline::Middle));
    if bars {
        depth_bars(&mut pen, frame, colors);
    }

    // The bars are on the tiles below; every line goes under the text.
    let mut markers = Vec::new();
    if config.divider {
        dividers(&mut pen, frame, colors, &mut markers);
    }
    if config.mid_price {
        mid_prices(&mut pen, frame, colors, &mut markers);
    }
    let pointer = pointer.filter(|_| frame.has_prices);
    if let Some((x, y)) = pointer {
        pen.stroke(colors.accent);
        if config.horizontal() {
            pen.line(x + 0.5, frame.plot_top, x + 0.5, frame.plot_bottom);
        } else {
            pen.line(frame.plot_left, y + 0.5, frame.plot_right, y + 0.5);
        }
    }

    if let Some(warning) = frame.warning {
        pen.fill(colors.dim);
        pen.text_within(warning, 8.0, 10.0, frame.width - 16.0);
    }
    if config.horizontal() {
        price_row(&mut pen, frame, colors);
    } else {
        price_column(&mut pen, frame, colors);
    }
    row_values(&mut pen, frame, colors);
    labels(&mut pen, frame, &mut markers, &mut logos);

    let status = status(&frame.loaded, &frame.failed);
    if !status.is_empty() {
        pen.fill(colors.dim);
        pen.align(CanvasTextAlign::Center);
        pen.text(
            status,
            (frame.plot_left + frame.plot_right) / 2.0,
            (frame.plot_top + frame.plot_bottom) / 2.0,
        );
        pen.align(CanvasTextAlign::Left);
    }
    if let Some(at) = pointer {
        level_box(&mut pen, frame, colors, at);
    }
    pen.restore();
    Drawn {
        painter: pen.0,
        logos,
    }
}

/// The depth bars as rectangles, each in its series' color at the bars'
/// opacity, inside the plot: the old renderer's instances, for a machine
/// without a GPU (or a build without NSH's shader renderer).
fn depth_bars(pen: &mut Pen, frame: &Frame, colors: &Colors) {
    pen.save();
    plot_clip(pen, frame);
    for level in &frame.levels {
        let Some(layer) = frame.config.layers.get(level.layer) else {
            continue;
        };
        let [left, top, right, bottom] = level.rect;
        let shows = right > frame.plot_left
            && left < frame.plot_right
            && bottom > frame.plot_top
            && top < frame.plot_bottom;
        if !shows {
            continue;
        }
        let color = colors.side(layer.pair, level.side);
        pen.fill(Hsla {
            a: color.a * BAR_ALPHA,
            ..color
        });
        pen.rect(left, top, right - left, bottom - top);
    }
    pen.restore();
}

/// A dashed line at the edge between each book's bids and asks, named
/// after its exchange.
fn dividers(pen: &mut Pen, frame: &Frame, colors: &Colors, markers: &mut Vec<Marker>) {
    let horizontal = frame.config.horizontal();
    pen.save();
    pen.with(|painter| painter.line_dash(&[px(4.0), px(3.0)]));
    for (index, layer) in frame.config.layers.iter().enumerate() {
        let (mut bid, mut ask) = (f64::NEG_INFINITY, f64::INFINITY);
        for level in &frame.levels {
            if level.layer != index || level.amount <= 0.0 {
                continue;
            }
            match level.side {
                Side::Bid => bid = bid.max(level.price),
                Side::Ask => ask = ask.min(level.price),
            }
        }
        if !bid.is_finite() || !ask.is_finite() || bid >= ask {
            continue;
        }
        // The asks end at this row's edge, even across an empty spread;
        // the raw mid price can lie inside a row.
        let edge = if horizontal { -ROW / 2.0 } else { ROW / 2.0 };
        let axis = frame.axis_of(ask) + edge;
        if !frame.shows(axis) {
            continue;
        }
        let color = colors.line(layer.pair);
        pen.stroke(color);
        if horizontal {
            pen.line(axis, frame.plot_bottom, axis, frame.plot_top);
        } else {
            pen.line(frame.plot_left, axis, frame.plot_right, axis);
        }
        let text = exchanges::display_name(&layer.market.exchange).to_owned();
        let width = width_of(&text, 10.0) + LOGO + 3.0;
        markers.push(Marker {
            layer: index,
            axis,
            extent: if horizontal {
                frame.plot_top
            } else {
                frame.plot_right
            },
            text,
            exchange: Some(layer.market.exchange.clone()),
            color,
            x: if horizontal {
                axis - width / 2.0
            } else {
                frame.plot_right - width - 6.0
            },
            y: if horizontal { frame.plot_top } else { axis } - 16.0,
            width,
            height: 14.0,
        });
    }
    pen.restore();
}

/// A line at each book's mid price, labelled with it: across the plot,
/// or from the end of the bar of a level at that very price.
fn mid_prices(pen: &mut Pen, frame: &Frame, colors: &Colors, markers: &mut Vec<Marker>) {
    let horizontal = frame.config.horizontal();
    let paired = frame.config.layers.len() > 1;
    let across = if horizontal {
        frame.plot_top
    } else {
        frame.plot_right
    };
    let first = markers.len();
    for (index, layer) in frame.config.layers.iter().enumerate() {
        let Some(mid) = frame.mids.get(index).copied().flatten() else {
            continue;
        };
        let axis = frame.axis_of(mid);
        if !frame.shows(axis) {
            continue;
        }
        let near = f64::EPSILON * mid.abs().max(1.0);
        let mut extent = across;
        for level in &frame.levels {
            if level.layer == index && (level.price - mid).abs() <= near {
                extent = if horizontal {
                    extent.min(level.rect[1])
                } else {
                    frame.plot_left.max(level.rect[2])
                };
            }
        }
        let text = price_text(mid);
        let width = width_of(&text, 10.0) + if paired { LOGO + 3.0 } else { 0.0 };
        markers.push(Marker {
            layer: index,
            axis,
            extent,
            text,
            exchange: paired.then(|| layer.market.exchange.clone()),
            color: colors.line(layer.pair),
            x: if horizontal {
                axis - width / 2.0
            } else {
                frame.plot_right - width - 6.0
            },
            y: if horizontal { extent } else { axis } - 16.0,
            width,
            height: 14.0,
        });
    }
    for marker in markers.iter().skip(first) {
        pen.stroke(marker.color);
        if horizontal {
            pen.line(
                marker.axis + 0.5,
                frame.plot_bottom,
                marker.axis + 0.5,
                marker.extent,
            );
        } else {
            pen.line(
                frame.plot_left,
                marker.axis + 0.5,
                marker.extent,
                marker.axis + 0.5,
            );
        }
    }
}

/// The lines' labels, kept inside the plot and off one another: each at
/// its line, else beside a label placed before it (side by side first for
/// a ladder running down, stacked first for one running across).
fn labels(pen: &mut Pen, frame: &Frame, markers: &mut [Marker], logos: &mut Vec<Logo>) {
    if markers.is_empty() {
        return;
    }
    let horizontal = frame.config.horizontal();
    pen.save();
    pen.font(SMALL_FONT);
    pen.align(CanvasTextAlign::Left);
    markers.sort_by(|a, b| a.axis.total_cmp(&b.axis).then(a.layer.cmp(&b.layer)));
    for index in 0..markers.len() {
        let (placed, rest) = markers.split_at_mut(index);
        let Some(marker) = rest.first_mut() else {
            break;
        };
        marker.x = (frame.plot_right - marker.width).min(frame.plot_left.max(marker.x));
        marker.y = (frame.plot_bottom - marker.height).min(frame.plot_top.max(marker.y));
        let anchor = (marker.x, marker.y);
        let mut fits = false;
        for candidate in std::iter::once(None).chain((0..index * 4).map(Some)) {
            (marker.x, marker.y) = anchor;
            if let Some(candidate) = candidate {
                let other_way = candidate >= index * 2;
                let edge = candidate - if other_way { index * 2 } else { 0 };
                let Some(beside) = placed.get(edge / 2) else {
                    continue;
                };
                let after = edge % 2 == 1;
                if horizontal != other_way {
                    marker.y = if after {
                        beside.y + beside.height + 2.0
                    } else {
                        beside.y - marker.height - 2.0
                    };
                } else {
                    marker.x = if after {
                        beside.x + beside.width + 2.0
                    } else {
                        beside.x - marker.width - 2.0
                    };
                }
            }
            let inside = marker.x >= frame.plot_left
                && marker.x + marker.width <= frame.plot_right
                && marker.y >= frame.plot_top
                && marker.y + marker.height <= frame.plot_bottom;
            if inside && !placed.iter().any(|other| marker.overlaps(other)) {
                fits = true;
                break;
            }
        }
        if !fits {
            (marker.x, marker.y) = anchor;
        }

        let mut text_x = marker.x;
        if let Some(exchange) = &marker.exchange
            && exchanges::logo(exchange).is_some()
        {
            logos.push(Logo {
                exchange: exchange.clone(),
                x: marker.x as f32,
                y: (marker.y + (marker.height - LOGO) / 2.0) as f32,
                size: LOGO as f32,
            });
            text_x += LOGO + 3.0;
        }
        pen.fill(marker.color);
        pen.text(&marker.text, text_x, marker.y + marker.height / 2.0);
    }
    pen.restore();
}

/// The ladder's rows that show: where each is along the ladder, and its
/// price.
///
/// With a row for every price step, rows are counted from the step
/// nearest the center price. Otherwise they are packed, one per price
/// holding orders; past either end of those the ladder goes on by the
/// step.
fn ladder(frame: &Frame) -> Vec<(f64, f64)> {
    let horizontal = frame.config.horizontal();
    let (start, end) = if horizontal {
        (frame.plot_left, frame.plot_right)
    } else {
        (frame.plot_top, frame.plot_bottom)
    };
    // Prices rise along the ladder across the pane, and fall down it.
    let rising = if horizontal { 1.0 } else { -1.0 };
    let stepped = frame.config.all_levels.then(|| {
        let steps = round((frame.center_price - frame.anchor_price) / frame.step);
        frame.anchor_price + steps * frame.step
    });
    let Some(base) = stepped.or_else(|| frame.occupied.first().copied()) else {
        return Vec::new();
    };
    let base_axis = frame.axis_of(base);
    let (first, last) = (
        ((start - base_axis) / ROW).ceil(),
        ((end - base_axis) / ROW).floor(),
    );
    // A plot has a few dozen rows; a ladder that maps nowhere has none.
    if !(first.is_finite() && last.is_finite()) || last - first > 4_096.0 {
        return Vec::new();
    }
    (first as i64..=last as i64)
        .map(|offset| {
            let axis = base_axis + offset as f64 * ROW;
            let price = match stepped {
                Some(base) => base + rising * offset as f64 * frame.step,
                None => frame.price_at(axis),
            };
            (axis, price)
        })
        .collect()
}

/// The price column of a ladder running down the pane: a price beside
/// each row, on a panel left of the plot.
fn price_column(pen: &mut Pen, frame: &Frame, colors: &Colors) {
    if !frame.steps() || frame.price_column <= 0.0 {
        return;
    }
    let height = frame.plot_bottom - frame.plot_top;
    pen.save();
    pen.clip(0.0, frame.plot_top, frame.price_column, height);
    pen.fill(colors.panel);
    pen.rect(0.0, frame.plot_top, frame.price_column, height);
    pen.stroke(colors.grid);
    pen.line(
        frame.plot_left - 0.5,
        frame.plot_top,
        frame.plot_left - 0.5,
        frame.plot_bottom,
    );
    pen.align(CanvasTextAlign::Left);
    pen.fill(colors.text);
    for (axis, price) in ladder(frame) {
        if price > 0.0 {
            pen.text(&price_text(price), 6.0, axis);
        }
    }
    pen.restore();
}

/// The prices of a ladder running across the pane: each written upward
/// from the plot's bottom, in its row.
fn price_row(pen: &mut Pen, frame: &Frame, colors: &Colors) {
    if !frame.steps() || frame.plot_right <= frame.plot_left {
        return;
    }
    pen.save();
    plot_clip(pen, frame);
    pen.align(CanvasTextAlign::Left);
    pen.fill(colors.text);
    for (axis, price) in ladder(frame) {
        if price > 0.0 {
            pen.save();
            pen.turn(axis, frame.plot_bottom - 8.0);
            pen.text(&price_text(price), 0.0, 0.0);
            pen.restore();
        }
    }
    pen.restore();
}

fn plot_clip(pen: &mut Pen, frame: &Frame) {
    pen.clip(
        frame.plot_left,
        frame.plot_top,
        frame.plot_right - frame.plot_left,
        frame.plot_bottom - frame.plot_top,
    );
}

/// Each row's total (price times amount) and, when the settings ask for
/// it, its running amount: the books' summed per row, or each book's in a
/// lane of its own.
fn row_values(pen: &mut Pen, frame: &Frame, colors: &Colors) {
    let config = &frame.config;
    let horizontal = config.horizontal();
    pen.save();
    plot_clip(pen, frame);
    pen.fill(colors.text);

    // One entry per text drawn: its level, its numbers, its lane.
    let mut entries: Vec<(usize, f64, f64, usize)> = Vec::new();
    if config.separate {
        for layer in 0..config.layers.len() {
            entries.extend(
                frame
                    .levels
                    .iter()
                    .enumerate()
                    .filter(|(_, level)| level.layer == layer)
                    .map(|(index, level)| (index, level.total, level.cumulative, layer)),
            );
        }
    } else {
        // Levels in one row of the screen are one entry, at the first's.
        let mut rows: HashMap<i64, usize> = HashMap::new();
        for (index, level) in frame.levels.iter().enumerate() {
            let axis = frame.axis_of(level.price);
            let row = round(if horizontal {
                (axis - frame.plot_left) / ROW
            } else {
                (frame.plot_bottom - axis) / ROW
            }) as i64;
            match rows.get(&row).and_then(|at| entries.get_mut(*at)) {
                Some(entry) => {
                    entry.1 += level.total;
                    entry.2 += level.cumulative;
                }
                None => {
                    rows.insert(row, entries.len());
                    entries.push((index, level.total, level.cumulative, 0));
                }
            }
        }
    }

    let lanes = if config.separate {
        config.layers.len().max(1)
    } else {
        1
    };
    let lane_width = (frame.plot_right - frame.plot_left) / lanes as f64;
    // Text turned on its side: the total a column up from the prices, the
    // running amount another one up.
    let column = ((frame.plot_bottom - frame.plot_top) / 3.0).clamp(64.0, 128.0);
    for (index, total, cumulative, lane) in entries {
        let Some(level) = frame.levels.get(index) else {
            continue;
        };
        let axis = frame.axis_of(level.price);
        if !frame.shows(axis) {
            continue;
        }
        let Some(layer) = config.layers.get(level.layer) else {
            continue;
        };
        let total = compact_count(total);
        let cumulative = format!("{} {}", compact_count(cumulative), layer.market.base);
        if horizontal {
            let shift = lane as f64 * 14.0;
            pen.save();
            pen.turn(axis, frame.plot_bottom - 8.0);
            pen.align(CanvasTextAlign::Left);
            pen.text(&total, column + shift, 0.0);
            if config.cumulative {
                pen.text(&cumulative, column * 2.0 + shift, 0.0);
            }
            pen.restore();
        } else {
            let left = frame.plot_left + lane_width * lane as f64;
            pen.align(CanvasTextAlign::Left);
            pen.text(&total, left + 8.0, axis);
            if config.cumulative {
                pen.align(CanvasTextAlign::Center);
                pen.text(&cumulative, left + lane_width / 2.0, axis);
            }
        }
    }
    pen.restore();
}

/// The level whose row the pointer is in: the nearest one showing within
/// half a row, the first of those as near.
fn pointed(frame: &Frame, pointer: (f64, f64)) -> Option<usize> {
    let along = if frame.config.horizontal() {
        pointer.0
    } else {
        pointer.1
    };
    let mut nearest = ROW / 2.0;
    let mut found = None;
    for (index, level) in frame.levels.iter().enumerate() {
        let axis = frame.axis_of(level.price);
        if !frame.shows(axis) {
            continue;
        }
        let distance = (axis - along).abs();
        if distance > nearest || (found.is_some() && distance == nearest) {
            continue;
        }
        found = Some(index);
        nearest = distance;
    }
    found
}

/// What a column of the pointer's box is about.
enum Column {
    /// A level at the pointed price.
    Level(usize),
    /// A book (by its layer) with no order at that price.
    Without(usize),
}

/// The pointer's box: the price of the level under it, and for each book
/// what rests at that very price (never a level near it): its side,
/// amount, total and running amount, a book's two sides apart.
fn level_box(pen: &mut Pen, frame: &Frame, colors: &Colors, pointer: (f64, f64)) {
    let Some(price) = pointed(frame, pointer)
        .and_then(|index| frame.levels.get(index))
        .map(|level| level.price)
    else {
        return;
    };
    let layers = &frame.config.layers;
    let mut columns = Vec::new();
    for layer in 0..layers.len() {
        let at = |side: Side| {
            frame.levels.iter().rposition(|level| {
                level.layer == layer && level.price == price && level.side == side
            })
        };
        let (bid, ask) = (at(Side::Bid), at(Side::Ask));
        columns.extend(bid.map(Column::Level));
        columns.extend(ask.map(Column::Level));
        if bid.is_none() && ask.is_none() {
            columns.push(Column::Without(layer));
        }
    }
    // Each column's book, and what it says under its name: its level's
    // side and three numbers, or that it has none.
    let described: Vec<_> = columns
        .iter()
        .filter_map(|column| {
            let (layer, level) = match *column {
                Column::Level(index) => {
                    let level = frame.levels.get(index)?;
                    (level.layer, Some(level))
                }
                Column::Without(layer) => (layer, None),
            };
            let book = layers.get(layer)?;
            let market = &book.market;
            let numbers = level.map(|level| {
                [
                    format!("{} {}", compact_count(level.amount), market.base),
                    format!("{} {}", compact_count(level.total), market.quote),
                    format!("{} {}", compact_count(level.cumulative), market.base),
                ]
            });
            Some((layer, book, level, numbers))
        })
        .collect();

    let size = 11.0;
    let padding = 12.0;
    let names = width_of("Cumulative", size) + 12.0;
    let mut column = 96.0_f64;
    for (layer, book, _, numbers) in &described {
        let mut fit = |text: &str| column = column.max(width_of(text, size) + 12.0);
        fit(exchanges::display_name(&book.market.exchange));
        fit(&book.market.symbol);
        match numbers {
            Some(numbers) => numbers.iter().for_each(|number| fit(number)),
            None => fit(without(frame, *layer)),
        }
    }
    let column = column.min(180.0);
    let price = price_text(price);
    let box_width = (padding * 2.0 + names + column * described.len() as f64)
        .max(padding * 3.0 + width_of("Price level", size) + width_of(&price, size));
    let box_height = 150.0;
    let margin = 6.0_f64.min(frame.width / 4.0).min(frame.height / 4.0);
    let scale = 1.0_f64
        .min((frame.width - margin * 2.0) / box_width)
        .min((frame.height - margin * 2.0) / box_height);
    let (width, height) = (box_width * scale, box_height * scale);
    // Beside the pointer, on its other side at the right and bottom
    // edges, and inside the widget either way.
    let beside = |at: f64, length: f64, room: f64| {
        let after = if at + 12.0 + length <= room - margin {
            at + 12.0
        } else {
            at - length - 12.0
        };
        margin.max((room - length - margin).min(after))
    };
    let (x, y) = (
        beside(pointer.0, width, frame.width),
        beside(pointer.1, height, frame.height),
    );

    pen.save();
    pen.fill(colors.panel);
    pen.rect(x, y, width, height);
    pen.stroke(colors.grid);
    pen.outline(x + 0.5, y + 0.5, width - 1.0, height - 1.0);
    pen.with(|painter| {
        painter
            .translate(x as f32, y as f32)
            .scale(scale as f32, scale as f32)
    });
    pen.align(CanvasTextAlign::Left);
    pen.fill(colors.dim);
    pen.text("Price level", padding, 15.0);
    pen.align(CanvasTextAlign::Right);
    pen.fill(colors.text);
    pen.text(&price, box_width - padding, 15.0);
    for rule in [29.5, 88.5] {
        pen.line(padding, rule, box_width - padding, rule);
    }
    pen.align(CanvasTextAlign::Left);
    pen.fill(colors.dim);
    for (name, y) in [("Amount", 101.0), ("Total", 119.0), ("Cumulative", 137.0)] {
        pen.text(name, padding, y);
    }

    pen.align(CanvasTextAlign::Right);
    for (at, (layer, book, level, numbers)) in described.iter().enumerate() {
        let right = padding + names + (at + 1) as f64 * column - 6.0;
        let fit = column - 12.0;
        pen.fill(colors.line(book.pair));
        pen.text_within(
            exchanges::display_name(&book.market.exchange),
            right,
            43.0,
            fit,
        );
        pen.fill(colors.dim);
        pen.text_within(&book.market.symbol, right, 59.0, fit);
        match (level, numbers) {
            (Some(level), Some(numbers)) => {
                pen.fill(colors.side(book.pair, level.side));
                pen.text(
                    match level.side {
                        Side::Bid => "BID",
                        Side::Ask => "ASK",
                    },
                    right,
                    77.0,
                );
                pen.fill(colors.text);
                for (number, y) in numbers.iter().zip([101.0, 119.0, 137.0]) {
                    pen.text_within(number, right, y, fit);
                }
            }
            _ => {
                pen.text_within(without(frame, *layer), right, 77.0, fit);
                for y in [101.0, 119.0, 137.0] {
                    pen.text("—", right, y);
                }
            }
        }
    }
    pen.restore();
}

/// What the pointer's box says of a book with no level at its price: that
/// it has no order there, or that it has not arrived.
fn without(frame: &Frame, layer: usize) -> &'static str {
    if frame.loaded.get(layer).copied().unwrap_or(false) {
        "No orders"
    } else {
        "Waiting"
    }
}
