//! The chart's GPU program: one WGSL fragment shader that draws a tile of
//! the plot ([`super::scene::Tile`]): the grid, then candles or a line.
//! The old `WebGLGeometryRenderer`, and the grid of its
//! `ChartOverlayRenderer`.
//!
//! NSH runs it through its shader view (`nsh_core::shader`): with wgpu
//! (WebGPU's implementation over Direct3D 12, Vulkan and Metal) on desktop,
//! and translated to GLSL ES for WebGL 2 in the web runtime, from this one
//! source. A shader view gives a program no vertex buffers, only uniforms
//! of at most 4096 bytes, so the program gets everything in one
//! `array<vec4f, 256>`: [`HEAD`] vectors of geometry and colors, then the
//! tile's prices, and finds the candle under each pixel by its slot, in
//! constant time. Drawing costs the same for 24 candles and for 2,000.
//!
//! A tile is drawn again only when its uniforms change (a bar of its own,
//! a pan, a zoom, the price range, a theme). The crosshair, the newest
//! price's mark and every label are ordinary views over the tiles, so
//! moving the pointer costs no GPU frame, and a trade that moves only the
//! newest bar draws only the tile it is in.

use std::cell::OnceCell;

use nsh_core::shader::{Shader, UniformValue};

use super::scene::{Mode, Scene, Tile};
use crate::theme;

/// The vectors of the uniform array.
const SLOTS: usize = 256;
/// The vectors before the prices: the view, the plot and four colors.
const HEAD: usize = 6;
/// The candles a tile can hold: a vector each (open, high, low, close).
pub(super) const CANDLE_CAPACITY: usize = SLOTS - HEAD;
/// The points of a line a tile can hold: four closes a vector.
pub(super) const LINE_CAPACITY: usize = (SLOTS - HEAD) * 4;

/// How many columns and rows the grid divides the plot into (the old
/// overlay's).
pub(super) const GRID_COLUMNS: f32 = 7.0;
pub(super) const GRID_ROWS: f32 = 6.0;

/// The program. `d[0]` is the view (the slot at the tile's left edge
/// counted from its first value, the slots across the tile, how many
/// values it has, 0 for candles or 1 for a line); `d[1]` the plot (the
/// tile's left edge and the plot's width in points, the grid's columns and
/// rows, none for no grid); `d[2]` to `d[5]` the colors of rising and
/// falling candles, the line and the grid.
const SOURCE: &str = r"
struct Uniforms { d: array<vec4f, 256> }

const HEAD: i32 = 6;

// `rgb` at `alpha` over `under` (straight alpha, as NSH takes colors).
fn over(under: vec4f, rgb: vec3f, alpha: f32) -> vec4f {
    let a = alpha + under.a * (1.0 - alpha);
    if (a <= 0.0) {
        return vec4f(0.0);
    }
    return vec4f((rgb * alpha + under.rgb * under.a * (1.0 - alpha)) / a, a);
}

// A one pixel line on the pixel holding `at` (kept inside `limit`): how
// much of the pixel centered on `p` it covers.
fn hairline(p: f32, at: f32, limit: f32) -> f32 {
    let center = min(floor(at) + 0.5, limit - 0.5);
    return clamp(1.0 - abs(p - center), 0.0, 1.0);
}

// The `i`th close of a line, as a fraction down the plot.
fn close_at(i: i32) -> f32 {
    let four = uniforms.d[HEAD + i / 4];
    return four[i % 4];
}

// How far `p` is from the segment `a` to `b`.
fn segment(p: vec2f, a: vec2f, b: vec2f) -> f32 {
    let along = b - a;
    let t = clamp(dot(p - a, along) / max(dot(along, along), 1e-6), 0.0, 1.0);
    return length(p - a - along * t);
}

fn shade(uv: vec2f) -> vec4f {
    let view = uniforms.d[0];
    let plot = uniforms.d[1];
    let size = nsh.size * nsh.scale;
    let p = uv * size;
    var color = vec4f(0.0);

    if (plot.z > 0.5 && plot.w > 0.5) {
        let grid = uniforms.d[5];
        let x = plot.x * nsh.scale + p.x;
        let width = plot.y * nsh.scale;
        let column = width / plot.z;
        let row = size.y / plot.w;
        let cover = max(
            hairline(x, round(x / column) * column, width),
            hairline(p.y, round(p.y / row) * row, size.y),
        );
        color = over(color, grid.rgb, grid.a * cover);
    }

    let pitch = size.x / max(view.y, 1e-6);
    let slot = view.x + p.x / pitch;
    let count = i32(view.z);

    if (view.w < 0.5) {
        let i = i32(floor(slot));
        if (i >= 0 && i < count) {
            // Open, high, low and close, in pixels from the top.
            let c = uniforms.d[HEAD + i] * size.y;
            let center = floor((f32(i) + 0.5 - view.x) * pitch) + 0.5;
            let dx = p.x - center;
            var tint = uniforms.d[3];
            if (c.w <= c.x) {
                tint = uniforms.d[2];
            }
            // The body: 72% of the slot in whole pixels, at most 16
            // points, a pixel short of the next candle at least; about
            // the wick's pixel, an even one a pixel further to the right.
            let most = min(16.0 * nsh.scale, max(floor(pitch) - 1.0, 1.0));
            let wide = clamp(round(pitch * 0.72), 1.0, most);
            let before = floor((wide - 1.0) / 2.0);
            let top = round(min(c.x, c.w));
            let bottom = max(round(max(c.x, c.w)), top + 1.0);
            let body = step(-before - 0.25, dx)
                * step(dx, wide - 1.0 - before + 0.25)
                * step(top, p.y)
                * step(p.y, bottom);
            let high = round(c.y);
            let low = max(round(c.z), high + 1.0);
            let wick = step(abs(dx), 0.25) * step(high, p.y) * step(p.y, low);
            color = over(color, tint.rgb, tint.a * max(body, wick));
        }
    } else {
        let stroke = uniforms.d[4];
        let j = i32(floor(slot - 0.5));
        var nearest = 1e9;
        for (var k = j - 1; k <= j + 1; k++) {
            if (k >= 0 && k + 1 < count) {
                let a = vec2f((f32(k) + 0.5 - view.x) * pitch, close_at(k) * size.y);
                let b = vec2f((f32(k) + 1.5 - view.x) * pitch, close_at(k + 1) * size.y);
                nearest = min(nearest, segment(p, a, b));
            }
        }
        let half = 0.75 * nsh.scale;
        color = over(color, stroke.rgb, stroke.a * clamp(half + 0.5 - nearest, 0.0, 1.0));
    }
    return color;
}
";

thread_local! {
    /// The compiled program, made the first time a chart shows.
    static PROGRAM: OnceCell<Option<Shader>> = const { OnceCell::new() };
}

/// The compiled program; none (logged once) where this build has no shader
/// compiler or it refuses the source.
pub(super) fn program() -> Option<Shader> {
    PROGRAM.with(|program| {
        program
            .get_or_init(|| {
                Shader::compile(SOURCE)
                    .inspect_err(|error| log::error!("Chart: its shader does not compile: {error}"))
                    .ok()
            })
            .clone()
    })
}

/// The colors the program draws with.
pub(super) struct Palette {
    /// A candle that closed at or above its open, and one that closed
    /// below.
    pub rising: UniformValue,
    pub falling: UniformValue,
    /// A tick chart's line.
    pub line: UniformValue,
    pub grid: UniformValue,
}

impl Palette {
    /// The old chart's colors in the theme now (tracked: what reads them
    /// runs again when the theme changes): buy and sell for candles, amber
    /// for ticks, the border at 40% for the grid.
    pub fn themed() -> Self {
        let color = |role: theme::Role| UniformValue::from(nsh_core::element::color(role.get()));
        Self {
            rising: color(theme::BID),
            falling: color(theme::ASK),
            line: color(theme::WARNING),
            grid: color(theme::BORDER.opacity(0.4)),
        }
    }
}

/// The program's uniform `d` for tile `tile` of `scene` in `palette`'s
/// colors, with the grid drawn or not.
pub(super) fn uniforms(scene: &Scene, tile: &Tile, grid: bool, palette: &Palette) -> UniformValue {
    let mode = match scene.mode {
        Mode::Candles => 0.0,
        Mode::Line => 1.0,
    };
    let (columns, rows) = if grid {
        (GRID_COLUMNS, GRID_ROWS)
    } else {
        (0.0, 0.0)
    };
    let mut slots: Vec<UniformValue> = Vec::with_capacity(HEAD + tile.values.len().div_ceil(4));
    slots.push([tile.first, tile.per, tile.count as f32, mode].into());
    slots.push([tile.left, scene.width, columns, rows].into());
    slots.extend([
        palette.rising.clone(),
        palette.falling.clone(),
        palette.line.clone(),
        palette.grid.clone(),
    ]);
    debug_assert_eq!(slots.len(), HEAD);
    slots.extend(tile.values.chunks(4).map(|chunk| {
        let mut four = [0.0_f32; 4];
        four[..chunk.len()].copy_from_slice(chunk);
        UniformValue::from(four)
    }));
    UniformValue::List(slots)
}
