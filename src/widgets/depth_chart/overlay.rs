//! What the depth chart draws over its curves, on one canvas, as the
//! order book's overlay does ([`crate::widgets::order_book`]): the rules
//! of its depth scale and of the prices it marks, with their names; the
//! mid price's line and its label; the warning; the status; and the
//! pointer's lines and its box.
//!
//! The pointer reads the chart the way it is meant to be read: how much
//! rests within a distance of the mid price. A line at the pointer and one
//! as far the other side of the center bound a band, and the box says what
//! each book holds in it: its bids down to the band's low price and its
//! asks up to its high one, as an amount and as a value, and both books'
//! together (the old chart's two tooltips, one of them mirrored).
//!
//! Where no shader can draw the curves (no GPU, or a build without NSH's
//! shader renderer), the overlay draws them too, under the rest, from the
//! frame's columns: the widget needs no GPU to work.
//!
//! Text is placed without measuring it ([`width_of`]). The tiny exchange
//! logos beside the mid prices' labels are not drawn here (a canvas draws
//! no SVG): [`draw`] says where they go, and the view puts them there.

use nsh_core::{
    canvas::{CanvasTextAlign, CanvasTextBaseline},
    prelude::*,
};

use super::{
    frame::{Frame, SERIES},
    shader::FILL,
};
use crate::{
    exchanges,
    format::price_text,
    widgets::common::{
        FONT, Pen, SMALL_FONT,
        books::{Colors, Drawn, Layer, Logo, Side, status},
        width_of,
    },
};

/// The side of a line label's exchange logo.
const LOGO: f64 = 8.0;
/// The room a mark's name takes down the price column: a line of its text.
const NAME: f64 = 14.0;
/// How opaque the rules are, of the color dividers are drawn in.
const RULE: f32 = 0.5;
/// How opaque the pointer's line, its mirror and the band between them
/// are, of the text's color.
const POINTER: f32 = 0.7;
const MIRROR: f32 = 0.35;
const BAND: f32 = 0.05;
/// How opaque the shade under the first line of the pointer's box is, of
/// the text's color, and the band under each side's line, of the side's.
const HEAD: f32 = 0.04;
const SIDE: f32 = 0.1;
/// What the pointer's box stresses is written in: its distance, its prices
/// and both books' together.
const STRONG: &str = "500 11px IBM Plex Mono";

/// The overlay of `frame` in `colors`, with the pointer at `pointer`
/// (points from the canvas's top left) while it is over the widget.
/// With `areas`, the curves too, under everything else: what the widget
/// shows them with where no shader can draw them.
pub(super) fn draw(
    frame: &Frame,
    pointer: Option<(f64, f64)>,
    colors: &Colors,
    areas: bool,
) -> Drawn {
    let config = &frame.config;
    let mut pen = Pen(Painter::new());
    let mut logos = Vec::new();
    pen.save();
    pen.font(FONT);
    pen.with(|painter| painter.text_baseline(CanvasTextBaseline::Middle));
    if areas {
        curves(&mut pen, frame, colors);
    }

    // The curves are on the tiles below; every line goes under the text.
    rules(&mut pen, frame, colors);
    let mut markers = Vec::new();
    if config.mid_price {
        mid_prices(&mut pen, frame, colors, &mut markers);
    }
    let pointed = pointer.and_then(|(x, y)| frame.pointed(x, y));
    if let Some(price) = pointed {
        pointer_lines(&mut pen, frame, colors, price);
    }

    if let Some(warning) = config.warning {
        pen.fill(colors.dim);
        pen.text_within(warning, 8.0, 10.0, frame.width - 16.0);
    }
    if config.horizontal() {
        price_strip(&mut pen, frame, colors);
    } else {
        price_column(&mut pen, frame, colors);
    }
    depth_names(&mut pen, frame, colors);
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
    if let (Some(at), Some(price)) = (pointer, pointed) {
        pointer_box(&mut pen, frame, colors, at, price);
    }
    pen.restore();
    Drawn {
        painter: pen.0,
        logos,
    }
}

fn plot_clip(pen: &mut Pen, frame: &Frame) {
    pen.clip(
        frame.plot_left,
        frame.plot_top,
        frame.plot_right - frame.plot_left,
        frame.plot_bottom - frame.plot_top,
    );
}

/// `color` at `alpha` of its own opacity.
fn faded(color: Hsla, alpha: f32) -> Hsla {
    Hsla {
        a: color.a * alpha,
        ..color
    }
}

/// The curves as the shader draws them, for a machine without a GPU (or a
/// build without NSH's shader renderer): each series' area as a rectangle
/// per run of columns it reaches as far in, its line along their top, and
/// a line up the edge between two runs.
fn curves(pen: &mut Pen, frame: &Frame, colors: &Colors) {
    let config = &frame.config;
    let horizontal = config.horizontal();
    let breadth = frame.breadth();
    // The plot's own columns: all but the one before it and the one after.
    let own = frame
        .columns
        .get(1..frame.columns.len().saturating_sub(1))
        .unwrap_or_default();
    // A stretch across the plot (`near` to `far` points from its
    // baseline) over `length` points along the prices from `start`.
    let place = |start: f64, length: f64, near: f64, far: f64| {
        if horizontal {
            let top = frame.plot_bottom - far;
            (frame.plot_left + start, top, length, far - near)
        } else {
            let left = frame.plot_left + near;
            (left, frame.plot_top + start, far - near, length)
        }
    };
    pen.save();
    plot_clip(pen, frame);
    for series in 0..SERIES {
        let Some((pair, side)) = config.tint(series) else {
            continue;
        };
        let color = colors.side(pair, side);
        pen.fill(faded(color, FILL));
        pen.stroke(color);
        let stacked = config.stacked() && series >= 2;
        // Where the series starts and ends in a column, in points.
        let band = |column: &[f32; SERIES]| {
            let near = if stacked { column[series - 2] } else { 0.0 };
            let far = column[series].max(near);
            (f64::from(near) * breadth, f64::from(far) * breadth)
        };
        // The run before: where it ended, and whether it had depth.
        let mut before = None::<(f64, bool)>;
        let mut at = 0;
        while let Some(column) = own.get(at) {
            let (near, far) = band(column);
            let mut end = at + 1;
            while own.get(end).is_some_and(|next| band(next) == (near, far)) {
                end += 1;
            }
            let (start, length) = (at as f64, (end - at) as f64);
            let thick = far > near;
            if thick {
                let (x, y, width, height) = place(start, length, near, far);
                pen.rect(x, y, width, height);
                let (x, y, width, height) = place(start, length, far - 0.5, far - 0.5);
                pen.line(x, y, x + width, y + height);
            }
            if let Some((ended, had_depth)) = before
                && (thick || had_depth)
                && ended != far
            {
                let (x, y, width, height) = place(start, 0.0, ended.min(far), ended.max(far));
                pen.line(x, y, x + width, y + height);
            }
            before = Some((far, thick));
            at = end;
        }
    }
    pen.restore();
}

/// The rules: one across the plot at every step of the depth scale, and
/// one along it at every place it marks a price.
fn rules(pen: &mut Pen, frame: &Frame, colors: &Colors) {
    if !frame.has_prices {
        return;
    }
    let horizontal = frame.config.horizontal();
    // On whole pixels, so a rule is one crisp line.
    let crisp = |at: f64| at.floor() + 0.5;
    pen.save();
    plot_clip(pen, frame);
    pen.stroke(faded(colors.grid, RULE));
    for depth in frame.rules() {
        let reach = crisp(frame.reach_of(depth));
        if horizontal {
            pen.line(frame.plot_left, reach, frame.plot_right, reach);
        } else {
            pen.line(reach, frame.plot_top, reach, frame.plot_bottom);
        }
    }
    for at in frame.marks().at {
        let axis = crisp(at);
        if horizontal {
            pen.line(axis, frame.plot_top, axis, frame.plot_bottom);
        } else {
            pen.line(frame.plot_left, axis, frame.plot_right, axis);
        }
    }
    pen.restore();
}

/// A mid price's line across the plot, and its label.
struct Marker {
    layer: usize,
    /// Where the line is along the prices.
    axis: f64,
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

/// A dashed line at the mid price, labelled with it: each book's own
/// where two are drawn apart, else the one between the highest bid and
/// the lowest ask of them all.
fn mid_prices(pen: &mut Pen, frame: &Frame, colors: &Colors, markers: &mut Vec<Marker>) {
    let config = &frame.config;
    let horizontal = config.horizontal();
    let paired = config.layers.len() > 1 && !config.combined();
    let mids: Vec<(usize, f64)> = if paired {
        let mids = frame.mids.iter().enumerate();
        mids.filter_map(|(layer, mid)| Some((layer, (*mid)?)))
            .collect()
    } else {
        frame.mid.map(|mid| (0, mid)).into_iter().collect()
    };
    pen.save();
    plot_clip(pen, frame);
    pen.with(|painter| painter.line_dash(&[px(6.0), px(4.0)]));
    for (layer, mid) in mids {
        let Some(book) = config.layers.get(layer) else {
            continue;
        };
        let axis = frame.axis_of(mid);
        if !frame.shows(axis) {
            continue;
        }
        let color = colors.line(book.pair);
        let line = axis.floor() + 0.5;
        pen.stroke(color);
        if horizontal {
            pen.line(line, frame.plot_bottom, line, frame.plot_top);
        } else {
            pen.line(frame.plot_left, line, frame.plot_right, line);
        }
        let text = price_text(mid);
        let width = width_of(&text, 10.0) + if paired { LOGO + 3.0 } else { 0.0 };
        markers.push(Marker {
            layer,
            axis,
            text,
            exchange: paired.then(|| book.market.exchange.clone()),
            color,
            // At the line's far end: above the middle of a chart whose
            // prices run along the bottom, where the curves are lowest.
            x: if horizontal {
                axis - width / 2.0
            } else {
                frame.plot_right - width - 6.0
            },
            y: if horizontal {
                frame.plot_top + 2.0
            } else {
                axis - 16.0
            },
            width,
            height: 14.0,
        });
    }
    pen.restore();
}

/// The mid prices' labels, kept inside the plot and off one another: each
/// at its line, or under the one placed before it where they would
/// overlap.
fn labels(pen: &mut Pen, frame: &Frame, markers: &mut [Marker], logos: &mut Vec<Logo>) {
    if markers.is_empty() {
        return;
    }
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
        for _ in 0..placed.len() {
            let Some(beside) = placed.iter().find(|other| marker.overlaps(other)) else {
                break;
            };
            marker.y = beside.y + beside.height + 2.0;
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

/// The pointer's line at `price`, its mirror as far the other side of the
/// center, and the band between them: the prices whose depth the
/// pointer's box counts.
fn pointer_lines(pen: &mut Pen, frame: &Frame, colors: &Colors, price: f64) {
    let horizontal = frame.config.horizontal();
    let at = frame.axis_of(price);
    let mirror = frame.axis_of(2.0 * frame.center - price);
    let (from, to) = (at.min(mirror), at.max(mirror));
    pen.save();
    plot_clip(pen, frame);
    pen.fill(faded(colors.text, BAND));
    if horizontal {
        pen.rect(
            from,
            frame.plot_top,
            to - from,
            frame.plot_bottom - frame.plot_top,
        );
    } else {
        pen.rect(
            frame.plot_left,
            from,
            frame.plot_right - frame.plot_left,
            to - from,
        );
    }
    for (axis, alpha) in [(mirror, MIRROR), (at, POINTER)] {
        let line = axis.floor() + 0.5;
        pen.stroke(faded(colors.text, alpha));
        if horizontal {
            pen.line(line, frame.plot_top, line, frame.plot_bottom);
        } else {
            pen.line(frame.plot_left, line, frame.plot_right, line);
        }
    }
    pen.restore();
}

/// What each of the plot's marks is named: where it is along the prices,
/// and the price that is there now. The places are the plot's own
/// ([`Frame::marks`]), so as prices move only the names change.
fn mark_names(frame: &Frame) -> impl Iterator<Item = (f64, String)> {
    let marks = frame.marks();
    let places = marks.places;
    marks.at.into_iter().filter_map(move |at| {
        let price = frame.price_at(at);
        (price > 0.0).then(|| (at, format!("{price:.places$}")))
    })
}

/// The prices of a chart whose prices run along the bottom: a line under
/// the plot, and each mark's price under it where it fits whole.
fn price_strip(pen: &mut Pen, frame: &Frame, colors: &Colors) {
    pen.save();
    pen.stroke(colors.grid);
    pen.line(
        frame.plot_left,
        frame.plot_bottom + 0.5,
        frame.plot_right,
        frame.plot_bottom + 0.5,
    );
    if frame.has_prices {
        let middle = (frame.plot_bottom + frame.height) / 2.0 + 0.5;
        pen.fill(colors.dim);
        pen.align(CanvasTextAlign::Center);
        for (axis, name) in mark_names(frame) {
            let half = width_of(&name, 11.0) / 2.0;
            if axis - half >= frame.plot_left + 2.0 && axis + half <= frame.plot_right - 2.0 {
                pen.text(&name, axis, middle);
            }
        }
    }
    pen.restore();
}

/// The price column of a chart whose prices run down the pane: each
/// mark's price where it fits whole, on a panel left of the plot (the
/// order book's).
fn price_column(pen: &mut Pen, frame: &Frame, colors: &Colors) {
    if frame.price_column <= 0.0 {
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
    if frame.has_prices {
        pen.align(CanvasTextAlign::Left);
        pen.fill(colors.text);
        for (axis, name) in mark_names(frame) {
            if axis - NAME / 2.0 >= frame.plot_top && axis + NAME / 2.0 <= frame.plot_bottom {
                pen.text(&name, 6.0, axis);
            }
        }
    }
    pen.restore();
}

/// A depth in a few digits: billions, millions and thousands with their
/// letter (`1.25M`, `19.3k`), anything smaller in three digits (`250`,
/// `12.5`, `0.375`), without the zeros a fraction ends in.
pub(super) fn depth_text(depth: f64) -> String {
    if !depth.is_finite() {
        return "--".to_owned();
    }
    let (scaled, letter) = match depth.abs() {
        size if size >= 1e9 => (depth / 1e9, "B"),
        size if size >= 1e6 => (depth / 1e6, "M"),
        size if size >= 1e3 => (depth / 1e3, "k"),
        _ => (depth, ""),
    };
    let places = match scaled.abs() {
        size if size >= 100.0 => 0,
        size if size >= 10.0 => 1,
        size if size >= 1.0 => 2,
        size if size >= 0.1 => 3,
        size if size >= 0.01 => 4,
        _ => 5,
    };
    let text = format!("{scaled:.places$}");
    let digits = if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.')
    } else {
        text.as_str()
    };
    // Never a negative zero.
    let digits = if digits == "-0" { "0" } else { digits };
    format!("{digits}{letter}")
}

/// The depth scale's names: each rule's depth beside it, as many as fit
/// apart, and on the one at the plot's far edge the asset depth is
/// counted in.
fn depth_names(pen: &mut Pen, frame: &Frame, colors: &Colors) {
    if !frame.has_prices {
        return;
    }
    let horizontal = frame.config.horizontal();
    let rules: Vec<f64> = frame.rules().collect();
    let Some(last) = rules.len().checked_sub(1) else {
        return;
    };
    let names: Vec<String> = rules
        .iter()
        .enumerate()
        .map(|(at, depth)| match frame.config.asset() {
            Some(asset) if at == last => format!("{} {asset}", depth_text(*depth)),
            _ => depth_text(*depth),
        })
        .collect();
    // The room a name takes along the scale, and what two rules are apart.
    let room = if horizontal {
        14.0
    } else {
        let widest = names.iter().map(|name| width_of(name, 10.0));
        widest.fold(0.0, f64::max) + 10.0
    };
    let apart = frame.rule / frame.top * frame.breadth();
    let every = if apart > 0.0 {
        (room / apart).ceil().clamp(1.0, 64.0) as usize
    } else {
        1
    };
    pen.save();
    plot_clip(pen, frame);
    pen.font(SMALL_FONT);
    pen.fill(colors.dim);
    pen.align(CanvasTextAlign::Right);
    for (at, (depth, name)) in rules.iter().zip(&names).enumerate() {
        // Counted from the far edge, whose name says the asset.
        if !(last - at).is_multiple_of(every) {
            continue;
        }
        let reach = frame.reach_of(*depth);
        if horizontal {
            pen.text(name, frame.plot_right - 6.0, reach + 8.0);
        } else {
            pen.text(name, reach - 4.0, frame.plot_bottom - 8.0);
        }
    }
    pen.restore();
}

/// The asset every book of `layers` names with `asset`; none while two
/// name different ones.
fn shared<'a>(layers: &'a [Layer], asset: impl Fn(&'a Layer) -> &'a str) -> Option<&'a str> {
    let mut assets = layers.iter().map(asset);
    let first = assets.next()?;
    assets.all(|asset| asset == first).then_some(first)
}

/// A column of the pointer's box: a book, or both books together, and
/// what it holds within the pointer's band as an amount and a value: its
/// bids, then its asks. None for a book that has not arrived.
struct Held<'a> {
    book: Option<&'a Layer>,
    sides: Option<[(f64, f64); 2]>,
}

/// A number of the pointer's box, and the asset it is in, where its books
/// agree on one.
type Number<'a> = (String, Option<&'a str>);

/// The pointer's box: how far from the center the pointer is, and what
/// each book holds no further from it than that: its bids down to the
/// band's low price and its asks up to its high one, as an amount of the
/// base asset and as its value in the quote asset; then both books'
/// together.
///
/// It reads from the top: the distance on a shaded line, the books' names
/// over their columns, then a band a side in its color, saying how far its
/// prices go, over its two lines. What is read first is stressed
/// ([`STRONG`]): the distance, the two prices and both books' together;
/// what is only a name is dim: the lines' names and the numbers' assets.
fn pointer_box(pen: &mut Pen, frame: &Frame, colors: &Colors, pointer: (f64, f64), price: f64) {
    let layers = &frame.config.layers;
    if layers.is_empty() {
        return;
    }
    let away = (price - frame.center).abs();
    let (low, high) = (frame.center - away, frame.center + away);
    let percent = if frame.center > 0.0 {
        away / frame.center * 100.0
    } else {
        0.0
    };
    let through = |layer: usize, side: Side, price: f64| {
        frame
            .through(layer, side, price)
            .map_or((0.0, 0.0), |step| (step.amount, step.value))
    };
    let mut columns: Vec<Held> = layers
        .iter()
        .enumerate()
        .map(|(index, layer)| Held {
            book: Some(layer),
            sides: frame.loaded.get(index).copied().unwrap_or(false).then(|| {
                [
                    through(index, Side::Bid, low),
                    through(index, Side::Ask, high),
                ]
            }),
        })
        .collect();
    if columns.len() > 1 {
        let mut total = [(0.0, 0.0); 2];
        for sides in columns.iter().filter_map(|column| column.sides) {
            for (sum, (amount, value)) in total.iter_mut().zip(sides) {
                *sum = (sum.0 + amount, sum.1 + value);
            }
        }
        columns.push(Held {
            book: None,
            sides: Some(total),
        });
    }
    // Both books' together are in an asset only while the books share it.
    let base = shared(layers, |layer| layer.market.base.as_str());
    let quote = shared(layers, |layer| layer.market.quote.as_str());
    // Each column's four numbers, each with the asset it is in: its bids'
    // amount and value, then its asks'.
    let numbers: Vec<Option<[Number; 4]>> = columns
        .iter()
        .map(|column| {
            let (base, quote) = match column.book {
                Some(layer) => (
                    Some(layer.market.base.as_str()),
                    Some(layer.market.quote.as_str()),
                ),
                None => (base, quote),
            };
            let [bids, asks] = column.sides?;
            Some([
                (depth_text(bids.0), base),
                (depth_text(bids.1), quote),
                (depth_text(asks.0), base),
                (depth_text(asks.1), quote),
            ])
        })
        .collect();

    let size = 11.0;
    let padding = 12.0;
    let space = width_of(" ", size);
    let names = width_of("Amount", size) + 12.0;
    let mut column = 96.0_f64;
    for (held, numbers) in columns.iter().zip(&numbers) {
        let mut fit = |width: f64| column = column.max(width + 12.0);
        if let Some(layer) = held.book {
            fit(width_of(
                exchanges::display_name(&layer.market.exchange),
                size,
            ));
            fit(width_of(&layer.market.symbol, size));
        }
        for (number, asset) in numbers.iter().flatten() {
            let asset = asset.map_or(0.0, |asset| space + width_of(asset, size));
            fit(width_of(number, size) + asset);
        }
    }
    let column = column.min(180.0);
    let distance = format!("±{percent:.2}%");
    let (bid_price, ask_price) = (price_text(low), price_text(high));
    // A line's name, and after it the price or the distance it is about.
    let headed = |name: &str, value: &str| {
        padding * 2.0 + width_of(name, size) + space + width_of(value, size)
    };
    let box_width = (padding * 2.0 + names + column * columns.len() as f64)
        .max(headed("From mid", &distance))
        .max(headed("Bids to", &bid_price))
        .max(headed("Asks to", &ask_price));
    let box_height = 190.0;
    // The first line's shade, the books' names under it, and each side's
    // band, with the side's own lines from the band's top.
    let (head, exchange, symbol) = (28.0, 42.0, 57.0);
    let (band, name, amount, value) = (20.0, 10.0, 31.0, 48.0);
    let sides = [
        ("Bids to", colors.bid, &bid_price, 68.0),
        ("Asks to", colors.ask, &ask_price, 128.0),
    ];
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
    // Inside the box's edge: the first line's shade, ruled off from the
    // books, and a band a side with a bar of its color over the edge.
    pen.fill(faded(colors.text, HEAD));
    pen.rect(1.0, 1.0, box_width - 2.0, head - 1.0);
    pen.line(1.0, head + 0.5, box_width - 1.0, head + 0.5);
    for (_, color, _, top) in sides {
        pen.fill(faded(color, SIDE));
        pen.rect(1.0, top, box_width - 2.0, band);
        pen.fill(color);
        pen.rect(0.0, top, 3.0, band);
    }
    // What each line is about, and after it the distance or the price it
    // is about.
    pen.align(CanvasTextAlign::Left);
    let lines = [("From mid", colors.dim, &distance, head / 2.0)]
        .into_iter()
        .chain(sides.map(|(text, color, price, top)| (text, color, price, top + name)));
    for (text, color, about, y) in lines {
        pen.font(FONT);
        pen.fill(color);
        pen.text(text, padding, y);
        pen.font(STRONG);
        pen.fill(colors.text);
        pen.text(about, padding + width_of(text, size) + space, y);
    }
    let rows = sides.map(|(.., top)| [top + amount, top + value]);
    pen.font(FONT);
    pen.fill(colors.dim);
    for [amount, value] in rows {
        pen.text("Amount", padding, amount);
        pen.text("Value", padding, value);
    }

    pen.align(CanvasTextAlign::Right);
    for (at, (held, numbers)) in columns.iter().zip(&numbers).enumerate() {
        let right = padding + names + (at + 1) as f64 * column - 6.0;
        let fit = column - 12.0;
        let font = match held.book {
            Some(layer) => {
                pen.font(FONT);
                pen.fill(colors.line(layer.pair));
                pen.text_within(
                    exchanges::display_name(&layer.market.exchange),
                    right,
                    exchange,
                    fit,
                );
                pen.fill(colors.dim);
                pen.text_within(&layer.market.symbol, right, symbol, fit);
                FONT
            }
            None => {
                pen.font(STRONG);
                pen.fill(colors.text);
                pen.text("Total", right, exchange);
                STRONG
            }
        };
        let rows = rows.as_flattened();
        match numbers {
            Some(numbers) => {
                for ((number, asset), y) in numbers.iter().zip(rows) {
                    // The asset at the column's edge, the number before it.
                    let taken = match asset {
                        Some(asset) => {
                            pen.font(FONT);
                            pen.fill(colors.dim);
                            pen.text_within(asset, right, *y, fit);
                            width_of(asset, size) + space
                        }
                        None => 0.0,
                    };
                    pen.font(font);
                    pen.fill(colors.text);
                    pen.text_within(number, right - taken, *y, fit - taken);
                }
            }
            // A book that has not arrived holds nothing to count yet.
            None => {
                pen.font(FONT);
                pen.fill(colors.dim);
                for y in rows {
                    pen.text("—", right, *y);
                }
            }
        }
    }
    pen.restore();
}
