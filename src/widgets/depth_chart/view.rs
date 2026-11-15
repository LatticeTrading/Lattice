//! The depth chart's view: the top bar and the notices of indicative
//! books that every widget drawing order books has ([`books`]), and the
//! plot: the curves on shader tiles ([`super::shader`]), everything
//! written and ruled over them on one canvas ([`super::overlay`]), and the
//! pointer's handling.
//!
//! The top bar's selects are Pairs (how two books are drawn together,
//! while two are drawn), Group and Depth.
//!
//! Pointer, on one surface over the plot:
//! - moving shows the pointer's line, its mirror the other side of the
//!   mid price, and the box of what rests between them;
//! - the wheel zooms the window of prices about its center, a tenth a
//!   notch as the chart's (a touchpad: by its travel);
//! - a double click fits the window to the books again.

use std::rc::Rc;

use nsh_base::SelectItem;
use nsh_core::{prelude::*, shader::UniformValue};

use super::{
    DepthChartSettings, frame::Frame, overlay, session::DepthChartSession, settings::Blend, shader,
};
use crate::{
    theme,
    widgets::{
        TabContext,
        common::{
            Clicks, Shared,
            books::{self, BookSettings, Colors, Layer, Pair, Palette},
            wheel_factor, wheel_steps,
        },
    },
};

/// The mid prices' exchange logos, at most: one a book.
const LOGOS: usize = 2;
/// What a notch of the wheel toward the user widens the window by (the
/// chart's).
const ZOOM: f64 = 1.1;

/// The depth chart of `session`, as its tab's settings say.
pub(super) fn depth_chart_view(
    session: &Rc<DepthChartSession>,
    tab: &TabContext<DepthChartSettings>,
) -> View {
    let saved = tab.settings;
    let layers = computed(move || saved.with(DepthChartSettings::layers));
    // Unmeasured counts as wide, so the first frame has the selects.
    let width = signal(f32::INFINITY);
    let narrow = computed(move || width.get() <= books::NARROW);
    // The pair the top bar's market picker is open for.
    let picking = signal(None::<Pair>);
    view()
        .flex_1()
        .min_h(px(0.0))
        .min_w(px(0.0))
        .flex_col()
        .overflow_hidden()
        .bg(theme::BACKGROUND)
        .text_color(theme::TEXT)
        .test_id("depth-chart")
        .on_layout(move |bounds: &Bounds<Points>| width.set(bounds.size.width.0))
        .child(top_bar(tab.clone(), layers, narrow, picking))
        .child(books::notices(DepthChartSettings::TEST_ID, layers))
        .child(plot(session.clone()))
        .child(books::picker(tab, picking))
}

/// The top bar: the books' segments, then the selects.
fn top_bar(
    tab: TabContext<DepthChartSettings>,
    layers: Memo<Vec<Layer>>,
    narrow: Memo<bool>,
    picking: Signal<Option<Pair>>,
) -> View {
    let selects = view()
        .child(blend_select(tab.clone(), layers))
        .child(books::grouping_select(tab.clone()))
        .child(books::depth_select(tab));
    books::top_bar::<DepthChartSettings>("Depth chart controls", layers, picking, narrow, selects)
}

/// "Pairs": how two books are drawn together. Hidden while one is drawn,
/// which it says nothing about.
fn blend_select(tab: TabContext<DepthChartSettings>, layers: Memo<Vec<Layer>>) -> View {
    let saved = tab.settings;
    let blend = computed(move || saved.with(DepthChartSettings::blend));
    let paired = computed(move || layers.with(|layers| layers.len() > 1));
    let select = books::quick_select(
        "Pairs",
        150.0,
        books::shown_text(move || blend.get().label().to_owned()),
        || {
            Blend::ALL
                .iter()
                .map(|blend| SelectItem::new(blend.id(), blend.label()))
                .collect()
        },
        move || SharedString::from(blend.get().id()),
        move |id| {
            if let Some(blend) = Blend::from_id(id) {
                tab.save(&saved.get_untracked().with_blend(blend));
            }
        },
    );
    view()
        .h_full()
        .flex_none()
        .style_when(move || !paired.get(), |s| s.hidden())
        .child(select)
}

/// The plot: the curves' tiles, the overlay's canvas and logos over them,
/// and the surface that takes the pointer.
fn plot(session: Rc<DepthChartSession>) -> View {
    let (frame, size) = (session.frame, session.size);
    // Where the pointer is over the plot, in points from its top left.
    let pointer = signal(None::<(f32, f32)>);
    // Where no shader can draw the curves, the canvas does.
    let compiled = shader::program().is_some();
    let drawn = computed(move || {
        let at = pointer.get().map(|(x, y)| (f64::from(x), f64::from(y)));
        let colors = Colors::themed();
        let areas = !(compiled && shaders_available());
        frame.with(|frame| Shared::new(overlay::draw(&frame.0, at, &colors, areas)))
    });
    view()
        .flex_1()
        .min_h(px(0.0))
        .min_w(px(0.0))
        .relative()
        .overflow_hidden()
        .on_layout(move |bounds: &Bounds<Points>| {
            // Whole points: a column of the plot is one.
            size.set((
                bounds.size.width.0.floor().max(1.0),
                bounds.size.height.0.floor().max(1.0),
            ));
        })
        .child(tiles(frame))
        .child(
            canvas(move || drawn.with(|drawn| drawn.0.painter.clone()))
                .absolute()
                .size_full()
                .test_id("depth-chart-overlay"),
        )
        .children(books::logos(LOGOS, drawn))
        .child(surface(session, pointer))
}

/// The curves: one shader view per tile of the frame, one after another
/// along the prices. On a machine without a GPU, or in a build without
/// NSH's shader renderer, they draw nothing and are hidden: the overlay
/// draws the curves then.
fn tiles(frame: Signal<Shared<Frame>>) -> View {
    let edge = move |read: fn(&Frame) -> f64| move || px(frame.with(|frame| read(&frame.0) as f32));
    let area = view()
        .absolute()
        .left(edge(|frame| frame.plot_left))
        .top(edge(|frame| frame.plot_top))
        .w(edge(|frame| frame.plot_right - frame.plot_left))
        .h(edge(|frame| frame.plot_bottom - frame.plot_top));
    let Some(program) = shader::program() else {
        return area;
    };
    let horizontal = computed(move || frame.with(|frame| frame.0.config.horizontal()));
    let count = computed(move || frame.with(|frame| frame.0.tiles.len()));
    let curves = dynamic(move || {
        let horizontal = horizontal.get();
        let program = program.clone();
        let row = view()
            .size_full()
            .style_when(|| !shaders_available(), |s| s.hidden());
        let row = if horizontal {
            row.flex_row()
        } else {
            row.flex_col()
        };
        row.child(each(
            move || (0..count.get()).collect::<Vec<_>>(),
            |tile: &usize| *tile,
            move |tile, _| {
                let at = tile.get_untracked();
                let length = move || {
                    px(frame.with(|frame| frame.0.tiles.get(at).map_or(0.0, |tile| tile.length)))
                };
                let tile = shader()
                    .source(program.clone())
                    .uniform("d", move || {
                        frame.with(|frame| match frame.0.tiles.get(at) {
                            Some(tile) => shader::uniforms(&frame.0, tile, &Palette::themed(1.0)),
                            None => UniformValue::List(Vec::new()),
                        })
                    })
                    .flex_none()
                    .test_id(&format!("depth-chart-tile-{at}"));
                if horizontal {
                    tile.h_full().w(length)
                } else {
                    tile.w_full().h(length)
                }
            },
        ))
    });
    area.child(curves)
}

/// The surface over the plot: the pointer's lines and box, zooming the
/// window, and fitting it again.
fn surface(session: Rc<DepthChartSession>, pointer: Signal<Option<(f32, f32)>>) -> View {
    let clicks = Clicks::default();
    let (wheel, release) = (session.clone(), session);
    view()
        .absolute()
        .size_full()
        .cursor(Cursor::Crosshair)
        .label("Depth chart plot")
        .test_id("depth-chart-plot")
        .on_mouse_move(move |event: &MouseEvent| {
            pointer.set(Some((event.position.x.0, event.position.y.0)));
        })
        .on_mouse_leave(move |_: &MouseEvent| pointer.set(None))
        .on_wheel(move |event: &WheelEvent| {
            let steps = wheel_steps(event);
            if steps != 0.0 {
                wheel.zoom(wheel_factor(steps, ZOOM));
            }
        })
        .claim_touches(TouchClaim::on_start())
        .keep_touches(true)
        .on_responder_move(move |event: &ResponderEvent| {
            pointer.set(Some((event.location.x.0, event.location.y.0)));
        })
        .on_responder_release(move |event: &ResponderEvent| {
            if clicks.doubled(event) {
                release.fit();
            }
        })
}
