//! A plot's view ([`plot_view`]): the tiles the shader draws
//! ([`super::shader`]) with what the old `ChartOverlayRenderer` drew over
//! them as ordinary views (the price and time axes, the newest price's tag,
//! the status and the crosshair), and the pointer's handling (the old
//! `WebGLPriceChart`'s). A widget lays what is its own over the plot (its
//! [`header`]).
//!
//! Layout, in points, as the old chart's: the plot is inset 12 from the
//! left and top, 74 from the right (the price axis) and 24 from the bottom
//! (the time axis).
//!
//! Pointer, on three surfaces laid over everything else:
//! - the plot: moving shows the crosshair; dragging pans (and moves the
//!   prices once they are free); the wheel zooms about the pointer, and
//!   with Shift or Alt pans. A widget with tools of its own for the plot
//!   (the Chart's drawing tools) hears the pointer first ([`Pointer`]) and
//!   may take a press, which then pans nothing;
//! - the price axis: dragging or the wheel scales the prices, which frees
//!   them from fitting the bars; a double click fits them again;
//! - the time axis: dragging or the wheel zooms; a double click fits the
//!   prices.

use std::{cell::Cell, rc::Rc};

use nsh_core::{prelude::*, shader::UniformValue};

use super::{
    Plot,
    scene::{Mode, Scene},
    shader::{self, GRID_ROWS, Palette},
    viewport::{Prices, Viewport},
};
use crate::{
    format::price_text,
    shell::clock::{self, Clock, ClockPrefs},
    theme, ui,
    widgets::common::{CLICK_SLOP, Clicks, Shared, wheel_factor, wheel_steps},
};

/// The plot's insets (the old chart's paddings).
const LEFT: f32 = 12.0;
const RIGHT: f32 = 74.0;
const TOP: f32 = 12.0;
const BOTTOM: f32 = 24.0;
/// Where the plot's area ([`area`]) starts in the plot's view, in points
/// from its top left: with the plot's size ([`Plot::size`]), what a widget
/// places a view of its own on the plot by.
pub(crate) const AREA_ORIGIN: (f32, f32) = (LEFT, TOP);
/// Where a plot's header ([`header`]) starts in the plot's view, in points
/// from its top left, and its text's size.
pub(crate) const HEADER_ORIGIN: (f32, f32) = (14.0, 14.0);
const HEADER_SIZE: f32 = 12.0;
/// Where what a widget lays under its plot's header starts (rows of its
/// own, a note), in points from the view's top left.
pub(crate) const UNDER_HEADER: (f32, f32) = (HEADER_ORIGIN.0, 32.0);
/// The labels the time axis shows at most (the old axis' slots).
const TIME_MARKS: usize = 7;
/// A label's box height, and half of it.
const LABEL: f32 = 14.0;
/// A character's width over its text size: the app's font (IBM Plex Mono)
/// is this wide everywhere, which is how labels are placed without
/// measuring them.
const ADVANCE: f32 = 0.6;

/// What a widget's plot shows besides its series.
pub(crate) struct PlotLook {
    /// What its test ids start with (`chart`: `chart-plot`,
    /// `chart-tile-0`).
    pub id: &'static str,
    /// Whether the grid is drawn.
    pub grid: Memo<bool>,
    /// Whether the crosshair names the hovered bar's time and prices.
    pub bar_prices: Memo<bool>,
    /// What the plot says about its data, nothing while all is well: in
    /// its middle while it has no bar ("Waiting for market data..." when
    /// there is nothing to say), above its bars otherwise.
    pub status: Memo<&'static str>,
    /// What hears the pointer over the plot before the plot does, for a
    /// widget with tools of its own there.
    pub pointer: Option<Rc<dyn Pointer>>,
    /// How many points down from [`UNDER_HEADER`] the widget's own rows
    /// under the header reach (the Chart's indicators), 0 for none: the
    /// hovered bar's prices show under them.
    pub legend: Memo<f32>,
}

/// What a widget does with the pointer over its plot before the plot does
/// (its drawing tools): it hears where the pointer is, and may take a
/// press and the drag that follows, which the plot pans with otherwise.
/// Places are in points from the plot's top left.
pub(crate) trait Pointer {
    /// The pointer is at `at` with no button down; nowhere once it left.
    fn hover(&self, at: Option<(f32, f32)>);
    /// A press began at `at`: whether the widget takes it and the drag
    /// that follows (the plot pans with them otherwise).
    fn press(&self, at: (f32, f32)) -> bool;
    /// The press it took moved to `at`.
    fn drag(&self, at: (f32, f32));
    /// The press it took ended at `at`; at nowhere when it was taken away.
    fn release(&self, at: Option<(f32, f32)>);
    /// Whether it would take a press where the pointer is (tracked): the
    /// plot shows a hand there.
    fn grabs(&self) -> bool;
}

/// A view over the plot's area, inside its insets: what a widget lays its
/// own layers of the plot in (its drawings), in the `over` it gives
/// [`plot_view`].
pub(crate) fn area() -> View {
    view()
        .absolute()
        .left(px(LEFT))
        .top(px(TOP))
        .right(px(RIGHT))
        .bottom(px(BOTTOM))
}

/// The view of `plot`, as `look` says, with `over` (what the widget shows
/// of its own, its [`header`]) over the plot and under the crosshair.
pub(crate) fn plot_view(plot: &Rc<Plot>, look: PlotLook, over: View) -> View {
    let PlotLook {
        id,
        grid,
        bar_prices,
        status,
        pointer,
        legend,
    } = look;
    let (scene, size) = (plot.scene, plot.size);
    let has_bars = computed(move || scene.with(|scene| scene.0.bars > 0));
    // Where the pointer is over the plot: over no view of it yet.
    let hover = plot.pointer;
    hover.set(None);
    let clock = use_context::<Clock>();
    let prefs = move || clock.map(Clock::get).unwrap_or_default();
    let cross = computed(move || {
        let (x, y) = hover.get()?;
        let prefs = prefs();
        scene.with(|scene| Cross::at(&scene.0, x, y, prefs))
    });

    view()
        .flex_1()
        .min_h(px(0.0))
        .min_w(px(0.0))
        .relative()
        .overflow_hidden()
        .bg(theme::BACKGROUND)
        .text_color(theme::TEXT_MUTED)
        .on_layout(move |bounds: &Bounds<Points>| {
            size.set((
                (bounds.size.width.0 - LEFT - RIGHT).max(1.0),
                (bounds.size.height.0 - TOP - BOTTOM).max(1.0),
            ));
        })
        .child(tiles(id, scene, grid, has_bars))
        .child(
            // The time axis' line.
            view()
                .absolute()
                .left(px(LEFT))
                .right(px(RIGHT))
                .bottom(px(BOTTOM - 1.0))
                .h(px(1.0))
                .bg(theme::BORDER.opacity(0.5))
                .style_when(move || !has_bars.get(), |s| s.hidden()),
        )
        .children(price_labels(scene, size))
        .children(time_labels(scene, prefs))
        .children(last_price(id, scene, size))
        .child(over)
        .child(messages(id, status, has_bars))
        .children(crosshair(id, cross, bar_prices, legend, size))
        .child(plot_surface(id, plot.clone(), hover, pointer))
        .child(price_axis_surface(id, plot.clone()))
        .child(time_axis_surface(id, plot.clone()))
}

/// A plot's header, in its top left corner (the old chart's): what it
/// charts, on one line. The widget names its parts and gives it a test id.
pub(crate) fn header(content: impl Fn() -> String + 'static) -> Text {
    text(content)
        .absolute()
        .left(px(HEADER_ORIGIN.0))
        .top(px(HEADER_ORIGIN.1))
        .number_of_lines(1)
        .text_size(px(HEADER_SIZE))
        .text_color(theme::TEXT)
}

/// Where a [`header`] saying `text` ends, in points from the plot's
/// view's left edge: what a widget places a control of its own beside its
/// header by.
pub(crate) fn header_end(text: &str) -> f32 {
    HEADER_ORIGIN.0 + text.chars().count() as f32 * HEADER_SIZE * ADVANCE
}

/// The plot: one shader view per tile of the scene, side by side.
fn tiles(
    id: &'static str,
    scene: Signal<Shared<Scene>>,
    grid: Memo<bool>,
    has_bars: Memo<bool>,
) -> View {
    let frame = area();
    let Some(program) = shader::program() else {
        return unavailable(frame);
    };
    let count = computed(move || scene.with(|scene| scene.0.tiles.len()));
    frame
        .flex_row()
        .style_when(
            move || !(has_bars.get() && shaders_available()),
            |s| s.hidden(),
        )
        .child(each(
            move || (0..count.get()).collect::<Vec<_>>(),
            |tile: &usize| *tile,
            move |tile, _| {
                let at = tile.get_untracked();
                let width = computed(move || {
                    scene.with(|scene| scene.0.tiles.get(at).map_or(0.0, |tile| tile.width))
                });
                shader()
                    .source(program.clone())
                    .uniform("d", move || {
                        scene.with(|scene| match scene.0.tiles.get(at) {
                            Some(tile) => {
                                shader::uniforms(&scene.0, tile, grid.get(), &Palette::themed())
                            }
                            None => UniformValue::List(Vec::new()),
                        })
                    })
                    .flex_none()
                    .h_full()
                    .w(move || px(width.get()))
                    .test_id(&format!("{id}-tile-{at}"))
            },
        ))
        .child(
            unavailable(view().absolute().size_full())
                .style_when(shaders_available, |s| s.hidden()),
        )
}

/// `frame` saying the plot cannot be drawn here (the old chart's "WebGL2
/// is required" notice): no GPU, or a build without NSH's shader renderer.
fn unavailable(frame: View) -> View {
    frame
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(12.0))
        .text_color(theme::DANGER)
        .child(text("GPU rendering is unavailable in this environment."))
}

/// A bar's time as the axis and the crosshair name it: the time of day for
/// a line's point, the day and the time for a candle.
fn time_text(mode: Mode, time: f64, prefs: ClockPrefs) -> String {
    let secs = time.floor() as i64;
    match mode {
        Mode::Line => clock::format_time(secs, prefs),
        Mode::Candles => clock::format_day_time(secs, prefs),
    }
}

/// The price axis: a price beside each grid row, right of the plot.
fn price_labels(
    scene: Signal<Shared<Scene>>,
    size: Signal<(f32, f32)>,
) -> impl Iterator<Item = Text> {
    let rows = GRID_ROWS as usize;
    (0..=rows).map(move |row| {
        let down = row as f32 / rows as f32;
        text(move || {
            scene.with(|scene| match scene.0.bars {
                0 => String::new(),
                _ => price_text(scene.0.prices.price_at(f64::from(down))),
            })
        })
        .absolute()
        .left(move || px(LEFT + size.get().0 + 6.0))
        .top(move || px(TOP + size.get().1 * down - LABEL / 2.0))
        .h(px(LABEL))
        .inline_style(ui::middle())
        .number_of_lines(1)
        .text_size(px(11.0))
    })
}

/// One label of the time axis: its text and where it starts, in points
/// from the plot's left edge.
#[derive(Clone, PartialEq)]
struct Mark {
    left: f32,
    text: String,
}

/// The time axis' labels for `scene`: each centered under its bar, kept
/// inside the plot, and left out where it would touch the one before.
fn marks(scene: &Scene, prefs: ClockPrefs) -> Vec<Mark> {
    const SIZE: f32 = 10.0;
    let mut marks: Vec<Mark> = Vec::new();
    for (x, time) in scene.time_marks(TIME_MARKS) {
        let text = time_text(scene.mode, time, prefs);
        let width = text.chars().count() as f32 * SIZE * ADVANCE;
        let left = (x - width / 2.0).min(scene.width - width).max(0.0);
        let free = marks.last().is_none_or(|before| {
            let taken = before.text.chars().count() as f32 * SIZE * ADVANCE;
            left > before.left + taken + 8.0
        });
        if free {
            marks.push(Mark { left, text });
        }
    }
    marks
}

/// The time axis: up to [`TIME_MARKS`] labels under the plot.
fn time_labels(
    scene: Signal<Shared<Scene>>,
    prefs: impl Fn() -> ClockPrefs + Copy + 'static,
) -> impl Iterator<Item = Text> {
    let marks = computed(move || {
        let prefs = prefs();
        scene.with(|scene| marks(&scene.0, prefs))
    });
    (0..TIME_MARKS).map(move |slot| {
        text(move || {
            marks.with(|marks| {
                marks
                    .get(slot)
                    .map_or_else(String::new, |mark| mark.text.clone())
            })
        })
        .absolute()
        .left(move || px(LEFT + marks.with(|marks| marks.get(slot).map_or(0.0, |mark| mark.left))))
        .bottom(px(4.0))
        .h(px(LABEL))
        .inline_style(ui::middle())
        .number_of_lines(1)
        .text_size(px(10.0))
        .style_when(
            move || marks.with(|marks| slot >= marks.len()),
            |s| s.hidden(),
        )
    })
}

/// A price in a filled tag on the price axis, `down` the plot.
fn price_tag(
    label: impl Fn() -> String + 'static,
    fill: impl Fn() -> AdaptiveColor + 'static,
    top: impl Fn() -> f32 + 'static,
    size: Signal<(f32, f32)>,
) -> View {
    view()
        .absolute()
        .left(move || px(LEFT + size.get().0 + 4.0))
        .top(move || px(top()))
        .h(px(16.0))
        .px(px(6.0))
        .flex()
        .items_center()
        .bg(fill)
        // What reads on the theme's accent and its buy and sell colors.
        .text_color(theme::BACKGROUND)
        .text_size(px(11.0))
        .child(text(label).number_of_lines(1))
}

/// The newest price while it is on the plot, in its bar's color: a dashed
/// line across the plot and its tag on the price axis (views, so a trade
/// moves them without drawing a tile again).
fn last_price(id: &str, scene: Signal<Shared<Scene>>, size: Signal<(f32, f32)>) -> [View; 2] {
    let last = computed(move || {
        scene.with(|scene| {
            let last = scene.0.last?;
            let down = scene.0.prices.fraction(last.close);
            (0.0..=1.0)
                .contains(&down)
                .then(|| (down as f32, last.rising, price_text(last.close)))
        })
    });
    let color = move || match last.with(|last| last.as_ref().map(|last| last.1)) {
        Some(false) => theme::ASK.get(),
        _ => theme::BID.get(),
    };
    let top =
        move || TOP + size.get().1 * last.with(|last| last.as_ref().map_or(0.0, |last| last.0));
    let hidden = move || last.with(Option::is_none);
    let line = view()
        .absolute()
        .left(px(LEFT))
        .top(move || px(top() - 0.5))
        .w(move || px(size.get().0))
        .border_t_1()
        .border_dashed()
        .border_color(color)
        .style_when(hidden, |s| s.hidden());
    let tag = price_tag(
        move || {
            last.with(|last| {
                last.as_ref()
                    .map_or_else(String::new, |last| last.2.clone())
            })
        },
        color,
        move || top() - 8.0,
        size,
    )
    .test_id(&format!("{id}-last-price"))
    .style_when(hidden, |s| s.hidden());
    [line, tag]
}

/// What the plot says about its data: in the middle while it has no bar
/// ("Waiting for market data..." when `status` has nothing to say), along
/// its top otherwise.
fn messages(id: &str, status: Memo<&'static str>, has_bars: Memo<bool>) -> View {
    let waiting = move || match status.get() {
        "" => "Waiting for market data...".to_owned(),
        said => said.to_owned(),
    };
    view()
        .absolute()
        .size_full()
        .child(
            view()
                .absolute()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(13.0))
                .style_when(move || has_bars.get(), |s| s.hidden())
                .child(text(waiting).text_center().test_id(&format!("{id}-status"))),
        )
        .child(
            text(move || status.get().to_owned())
                .absolute()
                .left(px(LEFT))
                .right(px(RIGHT))
                .top(px(TOP + 18.0 - 7.0))
                .text_center()
                .number_of_lines(1)
                .text_size(px(12.0))
                .style_when(move || !has_bars.get(), |s| s.hidden()),
        )
}

/// What the crosshair shows for a pointer over the plot.
#[derive(Clone, PartialEq)]
struct Cross {
    /// The hovered bar's middle and the pointer, in points from the plot's
    /// top left.
    x: f32,
    y: f32,
    /// The pointer's x, which the bar's prices follow.
    pointer: f32,
    /// The price under the pointer.
    price: String,
    /// The hovered bar's time and open, high, low and close.
    lines: [String; 5],
}

impl Cross {
    /// The crosshair for a pointer at (`x`, `y`) on `scene`'s plot; none
    /// while no bar shows.
    fn at(scene: &Scene, x: f32, y: f32, prefs: ClockPrefs) -> Option<Self> {
        let (middle, bar) = scene.bucket_near(x)?;
        let y = y.clamp(0.0, scene.height);
        Some(Self {
            x: middle,
            y,
            pointer: x,
            price: price_text(scene.prices.price_at(f64::from(y / scene.height))),
            lines: [
                format!("T {}", time_text(scene.mode, bar.time, prefs)),
                format!("O {}", price_text(bar.open)),
                format!("H {}", price_text(bar.high)),
                format!("L {}", price_text(bar.low)),
                format!("C {}", price_text(bar.close)),
            ],
        })
    }
}

/// The crosshair: a line down the hovered bar and one across at the
/// pointer, the pointer's price on the axis, and the bar's prices in a box
/// beside the pointer when `bar_prices` says so, under the widget's own
/// rows (`legend` points of them) when it has some.
fn crosshair(
    id: &str,
    cross: Memo<Option<Cross>>,
    bar_prices: Memo<bool>,
    legend: Memo<f32>,
    size: Signal<(f32, f32)>,
) -> [View; 4] {
    const SIZE: f32 = 11.0;
    let of = move |read: fn(&Cross) -> f32| cross.with(|cross| cross.as_ref().map_or(0.0, read));
    let hidden = move || cross.with(Option::is_none);
    let line = || {
        view()
            .absolute()
            .bg(theme::ACCENT.opacity(0.75))
            .style_when(hidden, |s| s.hidden())
    };
    let down = line()
        .left(move || px(LEFT + of(|cross| cross.x) - 0.5))
        .top(px(TOP))
        .w(px(1.0))
        .h(move || px(size.get().1));
    let across = line()
        .left(px(LEFT))
        .top(move || px(TOP + of(|cross| cross.y) - 0.5))
        .w(move || px(size.get().0))
        .h(px(1.0));
    let price = price_tag(
        move || {
            cross.with(|cross| {
                cross
                    .as_ref()
                    .map_or_else(String::new, |cross| cross.price.clone())
            })
        },
        || theme::ACCENT.get(),
        move || (TOP + of(|cross| cross.y) - 8.0).clamp(TOP, TOP + (size.get().1 - 16.0).max(0.0)),
        size,
    )
    .style_when(hidden, |s| s.hidden());
    let width = move || {
        let longest = cross.with(|cross| {
            cross.as_ref().map_or(0, |cross| {
                cross
                    .lines
                    .iter()
                    .map(|line| line.chars().count())
                    .max()
                    .unwrap_or(0)
            })
        });
        longest as f32 * SIZE * ADVANCE + 16.0
    };
    let lines = (0..5).map(move |at| {
        text(move || {
            cross.with(|cross| {
                cross
                    .as_ref()
                    .map_or_else(String::new, |cross| cross.lines[at].clone())
            })
        })
        .h(px(LABEL))
        .inline_style(ui::middle())
        .number_of_lines(1)
    });
    let bar = view()
        .absolute()
        .left(move || {
            let (plot_width, _) = size.get();
            let rightmost = (LEFT + plot_width + RIGHT - width() - 4.0).max(LEFT + 4.0);
            px((LEFT + of(|cross| cross.pointer) + 12.0).clamp(LEFT + 4.0, rightmost))
        })
        .top(move || {
            // Under the widget's rows while the plot has the room, never
            // off its bottom.
            let lowest = TOP + size.get().1 - (5.0 * LABEL + 8.0);
            let top = match legend.get() {
                rows if rows > 0.0 => (UNDER_HEADER.1 + rows + 4.0).min(lowest),
                _ => TOP + 8.0,
            };
            px(top.max(TOP + 8.0))
        })
        .w(move || px(width()))
        .flex_col()
        .px(px(8.0))
        .py(px(4.0))
        .bg(theme::PANEL.opacity(0.92))
        .text_color(theme::TEXT)
        .text_size(px(SIZE))
        .test_id(&format!("{id}-hover"))
        .style_when(move || hidden() || !bar_prices.get(), |s| s.hidden())
        .children(lines);
    [down, across, price, bar]
}

/// Zooms `plot`'s viewport by a wheel over the plot or the time axis,
/// about the pointer `x` points from the plot's left edge.
fn zoom_slots(plot: &Plot, event: &WheelEvent, x: f32) {
    let steps = wheel_steps(event);
    if steps == 0.0 {
        return;
    }
    let (width, _) = plot.size.get_untracked();
    let anchor = f64::from((x / width).clamp(0.0, 1.0));
    plot.move_view(|view, bars| view.zoomed(wheel_factor(steps, 1.1), anchor, bars));
}

/// The surface over the plot: the crosshair, panning and zooming, after
/// `pointer` (a widget's own tools for the plot) had the press.
fn plot_surface(
    id: &str,
    plot: Rc<Plot>,
    hover: Signal<Option<(f32, f32)>>,
    pointer: Option<Rc<dyn Pointer>>,
) -> View {
    // The viewport a drag began at.
    let origin = Rc::new(Cell::new(None::<Viewport>));
    // The press that is down is the widget's, not a pan.
    let taken = Rc::new(Cell::new(false));
    let (grant, drag, wheel) = (plot.clone(), plot.clone(), plot);
    let (began, ended, cancelled) = (origin.clone(), origin.clone(), origin.clone());
    let (took, takes, gave, lost) = (taken.clone(), taken.clone(), taken.clone(), taken);
    let (grabs, moved, left) = (pointer.clone(), pointer.clone(), pointer.clone());
    let (pressed, dragged) = (pointer.clone(), pointer.clone());
    let (released, terminated) = (pointer.clone(), pointer);
    view()
        .absolute()
        .left(px(LEFT))
        .top(px(TOP))
        .right(px(RIGHT))
        .bottom(px(BOTTOM))
        .cursor(Cursor::Crosshair)
        .style_when(
            move || grabs.as_ref().is_some_and(|pointer| pointer.grabs()),
            |s| s.cursor(Cursor::Pointer),
        )
        .label("Chart plot")
        .test_id(&format!("{id}-plot"))
        .on_mouse_move(move |event: &MouseEvent| {
            let at = (event.position.x.0, event.position.y.0);
            hover.set(Some(at));
            if let Some(pointer) = &moved
                && event.pressed_button.is_none()
            {
                pointer.hover(Some(at));
            }
        })
        .on_mouse_leave(move |_: &MouseEvent| {
            hover.set(None);
            if let Some(pointer) = &left {
                pointer.hover(None);
            }
        })
        .on_wheel(move |event: &WheelEvent| {
            if event.modifiers.shift || event.modifiers.alt {
                let steps = wheel_steps(event);
                if steps != 0.0 {
                    wheel.move_view(|view, bars| {
                        let by = (view.visible * 0.08).round().max(1.0);
                        view.panned(view.pan + steps.signum() * by, bars)
                    });
                }
            } else {
                zoom_slots(&wheel, event, event.position.x.0);
            }
        })
        .claim_touches(TouchClaim::on_start())
        .keep_touches(true)
        .on_responder_grant(move |event: &ResponderEvent| {
            let at = (event.location.x.0, event.location.y.0);
            if pressed.as_ref().is_some_and(|pointer| pointer.press(at)) {
                took.set(true);
            } else {
                began.set(Some(grant.viewport.get_untracked()));
            }
        })
        .on_responder_move(move |event: &ResponderEvent| {
            let at = (event.location.x.0, event.location.y.0);
            hover.set(Some(at));
            if takes.get() {
                if let Some(pointer) = &dragged {
                    pointer.drag(at);
                }
                return;
            }
            let Some(start) = origin.get() else {
                return;
            };
            let (width, height) = drag.size.get_untracked();
            let (dx, dy) = (
                f64::from(event.gesture.delta.x.0),
                f64::from(event.gesture.delta.y.0),
            );
            drag.move_view(|_, bars| Viewport {
                // Free prices follow the pointer too.
                prices: start
                    .prices
                    .map(|prices| prices.dragged(dy, f64::from(height))),
                ..start.panned(start.pan + dx * start.visible / f64::from(width), bars)
            });
        })
        .on_responder_release(move |event: &ResponderEvent| {
            ended.set(None);
            if gave.replace(false)
                && let Some(pointer) = &released
            {
                pointer.release(Some((event.location.x.0, event.location.y.0)));
            }
        })
        .on_responder_terminate(move |_: &ResponderEvent| {
            cancelled.set(None);
            if lost.replace(false)
                && let Some(pointer) = &terminated
            {
                pointer.release(None);
            }
        })
}

/// The surface over the price axis: scaling the prices, and a double click
/// to fit them to the bars again.
fn price_axis_surface(id: &str, plot: Rc<Plot>) -> View {
    // The prices a drag began at.
    let origin = Rc::new(Cell::new(None::<Prices>));
    let clicks = Clicks::default();
    let (grant, drag, release, wheel) = (plot.clone(), plot.clone(), plot.clone(), plot);
    let (began, ended, cancelled) = (origin.clone(), origin.clone(), origin.clone());
    view()
        .absolute()
        .right(px(0.0))
        .top(px(TOP))
        .bottom(px(BOTTOM))
        .w(px(RIGHT))
        .cursor(Cursor::RowResize)
        .label("Price axis")
        .test_id(&format!("{id}-price-axis"))
        .on_wheel(move |event: &WheelEvent| {
            let steps = wheel_steps(event);
            let (prices, least, height) = wheel
                .scene
                .with_untracked(|scene| (scene.0.prices, scene.0.fit.least_span(), scene.0.height));
            if steps == 0.0 || wheel.scene.with_untracked(|scene| scene.0.bars == 0) {
                return;
            }
            let anchor = f64::from((event.position.y.0 / height).clamp(0.0, 1.0));
            let next = prices.zoomed(wheel_factor(steps, 1.08), anchor, least);
            wheel.move_view(|view, _| Viewport {
                prices: Some(next),
                ..view
            });
        })
        .claim_touches(TouchClaim::on_start())
        .keep_touches(true)
        .on_responder_grant(move |_: &ResponderEvent| {
            began.set(
                grant
                    .scene
                    .with_untracked(|scene| (scene.0.bars > 0).then_some(scene.0.prices)),
            );
        })
        .on_responder_move(move |event: &ResponderEvent| {
            let Some(start) = origin.get() else {
                return;
            };
            let dy = event.gesture.delta.y.0;
            if dy.abs() <= CLICK_SLOP {
                return;
            }
            let least = drag.scene.with_untracked(|scene| scene.0.fit.least_span());
            drag.move_view(|view, _| Viewport {
                prices: Some(start.scaled(f64::from(dy), least)),
                ..view
            });
        })
        .on_responder_release(move |event: &ResponderEvent| {
            ended.set(None);
            if clicks.doubled(event) {
                fit_prices(&release);
            }
        })
        .on_responder_terminate(move |_: &ResponderEvent| cancelled.set(None))
}

/// The surface over the time axis: zooming, and a double click to fit the
/// prices to the bars again.
fn time_axis_surface(id: &str, plot: Rc<Plot>) -> View {
    // The viewport a drag began at.
    let origin = Rc::new(Cell::new(None::<Viewport>));
    let clicks = Clicks::default();
    let (grant, drag, release, wheel) = (plot.clone(), plot.clone(), plot.clone(), plot);
    let (began, ended, cancelled) = (origin.clone(), origin.clone(), origin.clone());
    view()
        .absolute()
        .left(px(LEFT))
        .right(px(RIGHT))
        .bottom(px(0.0))
        .h(px(BOTTOM))
        .cursor(Cursor::ColResize)
        .label("Time axis")
        .test_id(&format!("{id}-time-axis"))
        .on_wheel(move |event: &WheelEvent| zoom_slots(&wheel, event, event.position.x.0))
        .claim_touches(TouchClaim::on_start())
        .keep_touches(true)
        .on_responder_grant(move |_: &ResponderEvent| {
            began.set(Some(grant.viewport.get_untracked()));
        })
        .on_responder_move(move |event: &ResponderEvent| {
            let Some(start) = origin.get() else {
                return;
            };
            let dx = event.gesture.delta.x.0;
            if dx.abs() <= CLICK_SLOP {
                return;
            }
            drag.move_view(|view, bars| Viewport {
                prices: view.prices,
                ..Viewport::stretched(start, f64::from(dx), bars)
            });
        })
        .on_responder_release(move |event: &ResponderEvent| {
            ended.set(None);
            if clicks.doubled(event) {
                fit_prices(&release);
            }
        })
        .on_responder_terminate(move |_: &ResponderEvent| cancelled.set(None))
}

/// Has the prices fit the bars that show again (the old chart's axis
/// lock).
fn fit_prices(plot: &Plot) {
    plot.move_view(|view, _| Viewport {
        prices: None,
        ..view
    });
}
