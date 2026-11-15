//! The order book's GPU program: one WGSL fragment shader that draws a
//! tile of the plot's depth bars ([`super::frame::Tile`]). The old
//! `WebGLOrderBookGeometryRenderer`, whose instanced rectangles these are.
//!
//! NSH runs it through its shader view, as the chart's
//! ([`crate::widgets::chart`] says how): with wgpu on desktop, translated
//! to GLSL ES for WebGL 2 in the web runtime. A shader view has no vertex
//! buffers, only 4096 bytes of uniforms, so the bars are not sent as
//! rectangles: the program gets one `array<vec4f, 256>`, [`HEAD`] vectors
//! of geometry and colors and then a vector per row of the ladder, how far
//! each of the four series' bars reaches in it (the first book's bids and
//! asks, then the second's). It finds the row under each pixel by
//! division, in constant time: drawing costs the same for ten levels and
//! for a thousand.
//!
//! A bar fills its row less the point between two rows, snapped to whole
//! pixels; the bars of a row are laid over one another in the old order
//! (the first book's under the second's), each at [`BAR_ALPHA`]. Prices,
//! amounts, lines and the pointer are drawn over the tiles by the overlay
//! ([`super::overlay`]), so moving the pointer costs no GPU frame.

use std::cell::OnceCell;

use nsh_core::shader::{Shader, UniformValue};

use super::{
    frame::{Frame, Tile},
    settings::Pair,
};
use crate::theme;

/// The vectors of the uniform array.
const SLOTS: usize = 256;
/// The vectors before the rows: the view and the four series' colors.
const HEAD: usize = 5;
/// The rows a tile can hold: a vector each.
pub(super) const ROW_CAPACITY: usize = SLOTS - HEAD;
/// How opaque a depth bar is (the old renderer's).
pub(super) const BAR_ALPHA: f32 = 0.15;

/// The program. `d[0]` is the view (where the first row starts, in points
/// from the tile's start along the ladder; how many rows it has; 1 when
/// prices run across the view instead of down it); `d[1]` to `d[4]` the
/// colors of the first book's bids and asks and the second's; then a
/// vector per row: each series' reach across the view (0 to 1) plus 2 when
/// its bar joins the rows beyond it (the last level loaded) or 4 when it
/// joins both neighbors (beyond the last level loaded).
const SOURCE: &str = r"
struct Uniforms { d: array<vec4f, 256> }

const HEAD: i32 = 5;
// A row of the ladder, in points.
const ROW: f32 = 22.0;

// `rgb` at `alpha` over `under` (straight alpha, as NSH takes colors).
fn over(under: vec4f, rgb: vec3f, alpha: f32) -> vec4f {
    let a = alpha + under.a * (1.0 - alpha);
    if (a <= 0.0) {
        return vec4f(0.0);
    }
    return vec4f((rgb * alpha + under.rgb * under.a * (1.0 - alpha)) / a, a);
}

// The first pixel of the gap between row `j` and the one before it: the
// point about their shared edge, snapped to whole pixels.
fn gap(j: f32) -> f32 {
    return round((uniforms.d[0].x + j * ROW - 0.5) * nsh.scale);
}

// Row `j`'s reaches; none outside the tile's rows.
fn row(j: i32) -> vec4f {
    if (j < 0 || j >= i32(uniforms.d[0].y)) {
        return vec4f(0.0);
    }
    return uniforms.d[HEAD + j];
}

fn shade(uv: vec2f) -> vec4f {
    let across = uniforms.d[0].z > 0.5;
    let size = nsh.size * nsh.scale;
    let p = uv * size;
    // Along the ladder, and from the bars' baseline: down and from the
    // left where prices run down the view, right and from the bottom
    // where they run across it.
    var along = p.y;
    var off = p.x;
    var span = size.x;
    if (across) {
        along = p.x;
        off = size.y - p.y;
        span = size.y;
    }
    let pixel = floor(along);
    var j = floor((along / nsh.scale - uniforms.d[0].x + 0.5) / ROW);
    if (pixel < gap(j)) {
        j = j - 1.0;
    } else if (pixel >= gap(j + 1.0)) {
        j = j + 1.0;
    }
    let between = pixel - gap(j) < max(1.0, round(nsh.scale));
    let here = row(i32(j));
    let before = row(i32(j) - 1);

    var color = vec4f(0.0);
    for (var k = 0; k < 4; k++) {
        let join_here = floor(here[k] / 2.0);
        let join_before = floor(before[k] / 2.0);
        var reach = here[k] - 2.0 * join_here;
        if (between) {
            // The gap is filled where a bar runs on across it: from the
            // row before toward later rows, or from this row toward
            // earlier ones. Bids run on down the view and asks up it;
            // across the view, bids toward its start and asks its end.
            let onward = (k % 2 == 0) != across;
            reach = 0.0;
            if (join_before > 1.5 || (join_before > 0.5 && onward)) {
                reach = before[k] - 2.0 * join_before;
            }
            if (join_here > 1.5 || (join_here > 0.5 && !onward)) {
                reach = max(reach, here[k] - 2.0 * join_here);
            }
        }
        let tint = uniforms.d[1 + k];
        color = over(color, tint.rgb, tint.a * step(off, reach * span));
    }
    return color;
}
";

thread_local! {
    /// The compiled program, made the first time an order book shows.
    static PROGRAM: OnceCell<Option<Shader>> = const { OnceCell::new() };
}

/// The compiled program; none (logged once) where this build has no shader
/// compiler or it refuses the source.
pub(super) fn program() -> Option<Shader> {
    PROGRAM.with(|program| {
        program
            .get_or_init(|| {
                Shader::compile(SOURCE)
                    .inspect_err(|error| {
                        log::error!("Order book: its shader does not compile: {error}");
                    })
                    .ok()
            })
            .clone()
    })
}

/// The colors the program draws depth bars with, at their opacity.
pub(super) struct Palette {
    /// The primary book's bids and asks.
    pub bid: UniformValue,
    pub ask: UniformValue,
    /// The second pair's bids and asks.
    pub pair_bid: UniformValue,
    pub pair_ask: UniformValue,
}

impl Palette {
    /// The old order book's colors in the theme now (tracked: what reads
    /// them runs again when the theme changes): buy and sell for the
    /// primary book, the accent and amber for the second pair.
    pub fn themed() -> Self {
        let bar = |role: theme::Role| {
            UniformValue::from(nsh_core::element::color(role.opacity(BAR_ALPHA).get()))
        };
        Self {
            bid: bar(theme::BID),
            ask: bar(theme::ASK),
            pair_bid: bar(theme::ACCENT),
            pair_ask: bar(theme::WARNING),
        }
    }

    /// The color of `pair`'s bids (`bid`) or asks.
    fn of(&self, pair: Pair, bid: bool) -> UniformValue {
        match (pair, bid) {
            (Pair::Primary, true) => &self.bid,
            (Pair::Primary, false) => &self.ask,
            (Pair::Secondary, true) => &self.pair_bid,
            (Pair::Secondary, false) => &self.pair_ask,
        }
        .clone()
    }
}

/// The program's uniform `d` for tile `tile` of `frame` in `palette`'s
/// colors.
pub(super) fn uniforms(frame: &Frame, tile: &Tile, palette: &Palette) -> UniformValue {
    let rows = &tile.rows[..tile.rows.len().min(ROW_CAPACITY)];
    let across = if frame.config.horizontal() { 1.0 } else { 0.0 };
    let mut slots: Vec<UniformValue> = Vec::with_capacity(HEAD + rows.len());
    slots.push([tile.start, rows.len() as f32, across, 0.0].into());
    // Each series in its book's colors; a book that is not drawn has none.
    slots.extend(
        (0..4).map(|series| match frame.config.layers.get(series / 2) {
            Some(layer) => palette.of(layer.pair, series % 2 == 0),
            None => [0.0_f32; 4].into(),
        }),
    );
    debug_assert_eq!(slots.len(), HEAD);
    slots.extend(rows.iter().map(|row| UniformValue::from(*row)));
    UniformValue::List(slots)
}
