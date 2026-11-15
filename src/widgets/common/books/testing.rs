//! What the tests of the widgets that draw order books share: books as
//! the old tests gave them ([`book`]), colors that tell the series apart
//! ([`colors`], [`palette`]), a canvas's drawing read back ([`Record`]),
//! a shader layer drawn by NSH's renderer ([`drawn`], [`pixel`]), and a
//! `fetchOrderBook` a test answers ([`FERRIS`], [`stub_book`]).

use std::{
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

use nsh_core::{
    Color,
    canvas::{CanvasOp, Drawing, Num, Paint, PathOp},
    prelude::*,
    shader::ShaderLayer,
};
use nsh_engine::{
    HttpRequest,
    shader::{Frame as Picture, PixelFormat, Renderer, ShaderView},
};
use nsh_testing::{StubReply, StubResponse};
use serde_json::json;

use super::{Colors, Palette};
use crate::data::ferris::{BookLevel, OrderBook};

/// One side as the old tests gave it: prices and amounts from the best
/// price outward, with their running amounts.
pub(crate) fn side(rows: &[(f64, f64)]) -> Vec<BookLevel> {
    let mut cumulative = 0.0;
    rows.iter()
        .map(|&(price, amount)| {
            cumulative += amount;
            BookLevel {
                price,
                amount,
                total: price * amount,
                cumulative,
            }
        })
        .collect()
}

/// A book that has arrived.
pub(crate) fn book(bids: &[(f64, f64)], asks: &[(f64, f64)]) -> Option<Rc<OrderBook>> {
    Some(Rc::new(OrderBook {
        bids: side(bids),
        asks: side(asks),
        timestamp: 1.0,
    }))
}

/// Opaque colors that tell the series apart.
pub(crate) fn palette() -> Palette {
    Palette {
        bid: [0.0, 1.0, 0.0, 1.0].into(),
        ask: [1.0, 0.0, 0.0, 1.0].into(),
        pair_bid: [0.0, 0.0, 1.0, 1.0].into(),
        pair_ask: [1.0, 1.0, 0.0, 1.0].into(),
    }
}

/// The overlay's colors, each its own.
pub(crate) fn colors() -> Colors {
    let shade = |hue: f32| hsla(hue, 1.0, 0.5, 1.0);
    Colors {
        text: shade(0.0),
        dim: shade(0.1),
        grid: shade(0.2),
        panel: shade(0.3),
        bid: shade(0.4),
        ask: shade(0.5),
        accent: shade(0.6),
        amber: shade(0.7),
    }
}

/// `layer` drawn by NSH's renderer (wgpu) on `width` by `height` points
/// at `scale` pixels a point; none on a machine without a GPU.
pub(crate) fn drawn(layer: ShaderLayer, width: f32, height: f32, scale: f32) -> Option<Picture> {
    let renderer = Renderer::new(PixelFormat::Rgba)?;
    let mut view = ShaderView::default();
    view.set_layer(Some(Arc::new(layer)));
    view.set_size(Size::new(px(width), px(height)), scale);
    let started = Instant::now();
    while !view.render(&renderer, Instant::now()) {
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "the GPU returned no frame"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    Some(view.frame().clone())
}

/// The pixel at (`x`, `y`) of `picture`: premultiplied red, green, blue
/// and alpha.
pub(crate) fn pixel(picture: &Picture, x: u32, y: u32) -> [u8; 4] {
    let at = (y * picture.width + x) as usize * 4;
    picture.tight()[at..at + 4]
        .try_into()
        .expect("a pixel is four bytes")
}

pub(crate) const CLEAR: [u8; 4] = [0, 0, 0, 0];
pub(crate) const GREEN: [u8; 4] = [0, 255, 0, 255];
pub(crate) const RED: [u8; 4] = [255, 0, 0, 255];

/// A text of a drawing: where it was written (and from which origin, when
/// it is turned on its side), and in what color.
#[derive(Clone, Debug)]
pub(crate) struct Written {
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub origin: (f32, f32),
    pub color: Hsla,
}

/// A stroked line of a drawing: where it starts, and how it is drawn.
#[derive(Clone, Debug)]
pub(crate) struct Ruled {
    pub from: (f32, f32),
    pub color: Hsla,
    pub dashed: bool,
}

/// What a drawing wrote, ruled and filled, in order.
#[derive(Default)]
pub(crate) struct Record {
    pub texts: Vec<Written>,
    pub lines: Vec<Ruled>,
    /// Filled rectangles: left, top, width, height.
    pub boxes: Vec<([f32; 4], Hsla)>,
}

impl Record {
    pub fn of(drawing: &Drawing) -> Self {
        #[derive(Clone)]
        struct State {
            origin: (f32, f32),
            fill: Hsla,
            stroke: Hsla,
            dashed: bool,
        }
        let number = |num: &Num| match num {
            Num::Const(value) => *value,
            _ => panic!("the overlay draws no animated value"),
        };
        let paint = |paint: &Paint| match paint {
            Paint::Color(color) => Hsla::from(*color),
            _ => panic!("the overlay paints plain colors"),
        };
        let mut record = Self::default();
        let mut state = State {
            origin: (0.0, 0.0),
            fill: Hsla::default(),
            stroke: Hsla::default(),
            dashed: false,
        };
        let mut saved = Vec::new();
        let mut at = (0.0, 0.0);
        let mut path = Vec::new();
        for op in &drawing.ops {
            match op {
                CanvasOp::Save => saved.push(state.clone()),
                CanvasOp::Restore => state = saved.pop().expect("a save for every restore"),
                CanvasOp::Translate(x, y) => {
                    state.origin = (state.origin.0 + number(x), state.origin.1 + number(y));
                }
                CanvasOp::FillStyle(fill) => state.fill = paint(fill),
                CanvasOp::StrokeStyle(stroke) => state.stroke = paint(stroke),
                CanvasOp::LineDash(dashes) => state.dashed = !dashes.is_empty(),
                CanvasOp::BeginPath => path.clear(),
                CanvasOp::Path(PathOp::MoveTo(x, y)) => at = (number(x), number(y)),
                CanvasOp::Path(PathOp::LineTo(x, y)) => {
                    path.push(at);
                    at = (number(x), number(y));
                }
                CanvasOp::Stroke(None) => {
                    record.lines.extend(path.iter().map(|from| Ruled {
                        from: *from,
                        color: state.stroke,
                        dashed: state.dashed,
                    }));
                }
                CanvasOp::FillRect(rect) => {
                    record.boxes.push((rect.each_ref().map(number), state.fill));
                }
                CanvasOp::FillText { text, x, y, .. } => record.texts.push(Written {
                    text: text.to_string(),
                    x: number(x),
                    y: number(y),
                    origin: state.origin,
                    color: state.fill,
                }),
                _ => {}
            }
        }
        assert!(saved.is_empty(), "a restore for every save");
        record
    }

    /// Every text, in the order written.
    pub fn words(&self) -> Vec<&str> {
        self.texts.iter().map(|text| text.text.as_str()).collect()
    }

    pub fn count(&self, text: &str) -> usize {
        self.texts
            .iter()
            .filter(|written| written.text == text)
            .count()
    }

    pub fn find(&self, text: &str) -> &Written {
        self.texts
            .iter()
            .find(|written| written.text == text)
            .unwrap_or_else(|| panic!("no {text:?} among {:?}", self.words()))
    }

    /// The lines ruled in `color`.
    pub fn ruled(&self, color: Hsla) -> Vec<&Ruled> {
        self.lines
            .iter()
            .filter(|line| same(line.color, color))
            .collect()
    }
}

pub(crate) fn same(left: Hsla, right: Hsla) -> bool {
    Color::from(left) == Color::from(right)
}

/// A Ferris no test reaches: its HTTP is the test's stub network, and
/// nothing listens where its socket would connect.
pub(crate) const FERRIS: &str = "https://ferris.test";

/// Answers `fetchOrderBook` with two levels a side about 100.25.
pub(crate) fn stub_book(_: HttpRequest, reply: StubReply) {
    let book = json!({
        "bids": [[100.0, 2.0], [99.5, 4.0]],
        "asks": [[100.5, 1.0], [101.0, 3.0]],
        "timestamp": 1_700_000_000_000_i64,
        "nonce": null,
        "symbol": "BTC/USDT:USDT",
    });
    let mut response = StubResponse::default();
    response.status = 200;
    response.body = serde_json::to_vec(&book).expect("a book is JSON");
    reply.send(response);
}
