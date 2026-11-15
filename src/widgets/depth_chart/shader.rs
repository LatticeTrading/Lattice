//! The depth chart's GPU program: one WGSL fragment shader that draws a
//! tile of the plot's curves ([`super::frame::Tile`]): under each, an area
//! that grows fainter toward the baseline, and along its top a line.
//!
//! NSH runs it through its shader view, as the chart's
//! ([`crate::widgets::chart`] says how): with wgpu on desktop, translated
//! to GLSL ES for WebGL 2 in the web runtime. A shader view has no vertex
//! buffers, only 4096 bytes of uniforms, so the curves are not sent as
//! outlines: the program gets one `array<vec4f, 256>`, [`HEAD`] vectors of
//! geometry and colors and then a vector per column of the plot (a point
//! of it along the prices), how far each of the four series reaches in
//! it. It finds the column under each pixel by division, in constant time:
//! drawing costs the same for ten levels and for a thousand.
//!
//! A series fills its column from where it starts (the baseline, or the
//! top of the series it is stacked on) to where it reaches, snapped to
//! whole pixels. Its line runs along its reach and, where the next
//! column's differs, up or down the columns' shared edge, so a curve is
//! the steps a book's levels make. Prices, depths, rules and the pointer
//! are drawn over the tiles by the overlay ([`super::overlay`]), so moving
//! the pointer costs no GPU frame.

use std::cell::OnceCell;

use nsh_core::shader::{Shader, UniformValue};

use super::frame::{Frame, SERIES, Tile};
use crate::widgets::common::books::Palette;

/// The vectors of the uniform array.
const SLOTS: usize = 256;
/// The vectors before the columns: the view and the four series' colors.
const HEAD: usize = 1 + SERIES;
/// The columns a tile can hold: a vector each.
pub(super) const COLUMN_CAPACITY: usize = SLOTS - HEAD;
/// How opaque the overlay fills an area where no shader can: the middle
/// of the program's gradient (`FILL_NEAR` to `FILL_FAR`).
pub(super) const FILL: f32 = 0.25;

/// The program. `d[0]` is the view (where the first column starts, in
/// points from the tile's start along the prices; how many columns it
/// has; 1 when prices run down the view instead of across it; 1 when the
/// last two series are stacked on the first two); `d[1]` to `d[4]` the
/// series' colors (the first book's bids and asks, then the second's);
/// then a vector per column: how far each series reaches across the view,
/// 0 to 1.
const SOURCE: &str = r"
struct Uniforms { d: array<vec4f, 256> }

const HEAD: i32 = 5;
// A curve's line, in points.
const LINE: f32 = 1.5;
// How opaque an area is at the baseline and at the view's far edge (the
// old chart's gradient).
const FILL_NEAR: f32 = 0.1;
const FILL_FAR: f32 = 0.4;

// `rgb` at `alpha` over `under` (straight alpha, as NSH takes colors).
fn over(under: vec4f, rgb: vec3f, alpha: f32) -> vec4f {
    let a = alpha + under.a * (1.0 - alpha);
    if (a <= 0.0) {
        return vec4f(0.0);
    }
    return vec4f((rgb * alpha + under.rgb * under.a * (1.0 - alpha)) / a, a);
}

// Column `j`'s reaches; none outside the tile's columns.
fn column(j: i32) -> vec4f {
    if (j < 0 || j >= i32(uniforms.d[0].y)) {
        return vec4f(0.0);
    }
    return uniforms.d[HEAD + j];
}

// Where series `k` starts in a column, in pixels from the baseline: at
// it, or on top of the series it is stacked on.
fn foot(reaches: vec4f, k: i32, span: f32) -> f32 {
    if (uniforms.d[0].w > 0.5 && k >= 2) {
        return round(reaches[k - 2] * span);
    }
    return 0.0;
}

// Where series `k` ends in a column: never under where it starts.
fn head(reaches: vec4f, k: i32, span: f32) -> f32 {
    return max(round(reaches[k] * span), foot(reaches, k, span));
}

// How far a pixel `off` the baseline is from the line up the edge of two
// columns, `across` pixels beside it, between their reaches `a` and `b`.
// Lines are drawn on the pixels just inside the area they outline: under
// its reach, and in the column that reaches further.
fn riser(off: f32, across: f32, a: f32, b: f32) -> f32 {
    let low = min(a, b) - 0.5;
    let high = max(a, b) - 0.5;
    return max(across, max(max(low - off, off - high), 0.0));
}

fn shade(uv: vec2f) -> vec4f {
    let size = nsh.size * nsh.scale;
    let p = uv * size;
    // Along the prices, and from the curves' baseline: right and from
    // the bottom where prices run across the view, down and from the left
    // where they run down it.
    var along = p.x;
    var off = size.y - p.y;
    var span = size.y;
    if (uniforms.d[0].z > 0.5) {
        along = p.y;
        off = p.x;
        span = size.x;
    }
    // Columns are a point wide.
    let at = along / nsh.scale - uniforms.d[0].x;
    let j = i32(floor(at));
    let here = column(j);
    let before = column(j - 1);
    let after = column(j + 1);
    // How far the pixel is past the column's start and before its end.
    let from_start = (at - floor(at)) * nsh.scale;
    let to_end = nsh.scale - from_start;
    let half = 0.5 * LINE * nsh.scale;

    var color = vec4f(0.0);
    for (var k = 0; k < 4; k++) {
        let low = foot(here, k, span);
        let high = head(here, k, span);
        let tint = uniforms.d[1 + k];
        let inside = step(low, off) * step(off, high);
        let fade = mix(FILL_NEAR, FILL_FAR, clamp(off / span, 0.0, 1.0));
        color = over(color, tint.rgb, tint.a * fade * inside);
    }
    for (var k = 0; k < 4; k++) {
        let low = foot(here, k, span);
        let high = head(here, k, span);
        let thick = high > low;
        var nearest = 1e9;
        if (thick) {
            nearest = abs(off - (high - 0.5));
        }
        let low_before = foot(before, k, span);
        let high_before = head(before, k, span);
        if (thick || high_before > low_before) {
            var beside = 0.5;
            if (high_before > high) {
                beside = -0.5;
            }
            nearest = min(nearest, riser(off, abs(from_start - beside), high_before, high));
        }
        let low_after = foot(after, k, span);
        let high_after = head(after, k, span);
        if (thick || high_after > low_after) {
            var beyond = 0.5;
            if (high > high_after) {
                beyond = -0.5;
            }
            nearest = min(nearest, riser(off, abs(to_end + beyond), high, high_after));
        }
        let tint = uniforms.d[1 + k];
        color = over(color, tint.rgb, tint.a * clamp(half + 0.5 - nearest, 0.0, 1.0));
    }
    return color;
}
";

thread_local! {
    /// The compiled program, made the first time a depth chart shows.
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
                        log::error!("Depth chart: its shader does not compile: {error}");
                    })
                    .ok()
            })
            .clone()
    })
}

/// The program's uniform `d` for tile `tile` of `frame` in `palette`'s
/// colors.
pub(super) fn uniforms(frame: &Frame, tile: &Tile, palette: &Palette) -> UniformValue {
    let columns = frame.columns.get(tile.first..).unwrap_or_default();
    let columns = &columns[..columns.len().min(tile.count).min(COLUMN_CAPACITY)];
    let flag = |on: bool| if on { 1.0 } else { 0.0 };
    let mut slots: Vec<UniformValue> = Vec::with_capacity(HEAD + columns.len());
    // The first column is the one before the tile's own.
    slots.push(
        [
            -1.0,
            columns.len() as f32,
            flag(!frame.config.horizontal()),
            flag(frame.config.stacked()),
        ]
        .into(),
    );
    // Each series in its book's colors; one that is not drawn has none.
    slots.extend((0..SERIES).map(|series| match frame.config.tint(series) {
        Some((pair, side)) => palette.of(pair, side),
        None => [0.0_f32; 4].into(),
    }));
    debug_assert_eq!(slots.len(), HEAD);
    slots.extend(columns.iter().map(|column| UniformValue::from(*column)));
    UniformValue::List(slots)
}
