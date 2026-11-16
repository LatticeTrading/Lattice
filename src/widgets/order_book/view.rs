//! The order book's view: the top bar (the old `WebGLOrderBookTopBar`),
//! the notices of indicative books, and the plot: depth bars on shader
//! tiles ([`super::shader`]), everything written and ruled over them on
//! one canvas ([`super::overlay`]), and the pointer's handling (the old
//! `WebGLOrderBookRenderer`'s).
//!
//! The top bar is the one every widget that draws order books has
//! ([`books::top_bar`]: a segment per book, `+` for a second pair), with
//! the selects Center, Group and Depth: the book the ladder follows (or
//! none), the price grouping, and the levels a side.
//!
//! Pointer, on one surface over the plot:
//! - moving shows a line at the pointer and the box of the level under it;
//! - the wheel moves a ladder running down the pane, a row a notch (a
//!   touchpad: a row per row of travel), which stops it following a book
//!   and saves that;
//! - a double click on the price column centers the ladder again.

use std::{cell::Cell, rc::Rc};

use nsh_base::SelectItem;
use nsh_core::{prelude::*, shader::UniformValue};

use super::{
    OrderBookSettings,
    frame::{Frame, ROW},
    overlay,
    session::OrderBookSession,
    shader::{self, BAR_ALPHA},
};
use crate::{
    theme,
    widgets::{
        TabContext,
        common::{
            Clicks, Shared,
            books::{self, BookLayout, BookSettings, Colors, Layer, Pair, Palette},
        },
    },
};

/// The line labels' exchange logos, at most: two books' mid prices and
/// bid/ask edges.
const LOGOS: usize = 4;
/// What the Center select calls following no book.
const UNFOLLOWED: &str = "none";

/// The order book of `session`, as its tab's settings say.
pub(super) fn order_book_view(
    session: &Rc<OrderBookSession>,
    tab: &TabContext<OrderBookSettings>,
) -> View {
    let saved = tab.settings;
    let layers = computed(move || saved.with(OrderBookSettings::layers));
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
        .test_id("order-book")
        .on_layout(move |bounds: &Bounds<Points>| width.set(bounds.size.width.0))
        .child(top_bar(tab.clone(), layers, narrow, picking))
        .child(books::notices(
            OrderBookSettings::TEST_ID,
            layers,
            session.traits,
            true,
        ))
        .child(plot(session.clone(), tab.clone()))
        .child(books::picker(tab, picking))
}

/// The top bar: the books' segments, then the selects.
fn top_bar(
    tab: TabContext<OrderBookSettings>,
    layers: Memo<Vec<Layer>>,
    narrow: Memo<bool>,
    picking: Signal<Option<Pair>>,
) -> View {
    let selects = view()
        .child(center_select(tab.clone(), layers))
        .child(books::grouping_select(tab.clone()))
        .child(books::depth_select(tab));
    books::top_bar::<OrderBookSettings>("Order book controls", layers, picking, narrow, selects)
}

/// "Center": the book the ladder follows, or none.
fn center_select(tab: TabContext<OrderBookSettings>, layers: Memo<Vec<Layer>>) -> impl IntoElement {
    let saved = tab.settings;
    let center = computed(move || saved.with(OrderBookSettings::center));
    let followed = computed(move || {
        let pair = center.get()?;
        layers.with(|layers| layers.iter().find(|layer| layer.pair == pair).cloned())
    });
    let shown = view()
        .min_w(px(0.0))
        .flex_row()
        .justify_end()
        .child(dynamic(move || match followed.get() {
            Some(layer) => books::logo_and_pair(&layer),
            None => view().child(text("None").number_of_lines(1)),
        }));
    books::quick_select(
        "Center",
        170.0,
        shown,
        move || {
            layers.with(|layers| {
                std::iter::once(SelectItem::new(UNFOLLOWED, "None"))
                    .chain(
                        layers
                            .iter()
                            .map(|layer| SelectItem::new(layer.pair.id(), books::book_name(layer))),
                    )
                    .collect()
            })
        },
        move || SharedString::from(center.get().map_or(UNFOLLOWED, Pair::id)),
        move |id| {
            let target = Pair::from_id(id);
            if target.is_some() || id == UNFOLLOWED {
                tab.save(&saved.get_untracked().with_center(target));
            }
        },
    )
}

/// The plot: the depth bars' tiles, the overlay's canvas and logos over
/// them, and the surface that takes the pointer.
fn plot(session: Rc<OrderBookSession>, tab: TabContext<OrderBookSettings>) -> View {
    let (frame, size) = (session.frame, session.size);
    // Where the pointer is over the plot, in points from its top left.
    let pointer = signal(None::<(f32, f32)>);
    // Where no shader can draw the bars, the canvas does.
    let compiled = shader::program().is_some();
    let drawn = computed(move || {
        let at = pointer.get().map(|(x, y)| (f64::from(x), f64::from(y)));
        let colors = Colors::themed();
        let bars = !(compiled && shaders_available());
        frame.with(|frame| Shared::new(overlay::draw(&frame.0, at, &colors, bars)))
    });
    view()
        .flex_1()
        .min_h(px(0.0))
        .min_w(px(0.0))
        .relative()
        .overflow_hidden()
        .on_layout(move |bounds: &Bounds<Points>| {
            // Whole points, as the old canvases were sized.
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
                .test_id("order-book-overlay"),
        )
        .children(books::logos(LOGOS, drawn))
        .child(surface(session, tab, pointer))
}

/// The depth bars: one shader view per tile of the frame, one after
/// another along the ladder. On a machine without a GPU, or in a build
/// without NSH's shader renderer, they draw nothing and are hidden: the
/// overlay draws the bars then (where the old widget said "WebGL2 is
/// required").
fn tiles(frame: Signal<Shared<Frame>>) -> View {
    let area = view()
        .absolute()
        .left(move || px(frame.with(|frame| frame.0.plot_left as f32)))
        .top(move || px(frame.with(|frame| frame.0.plot_top as f32)))
        .right(px(0.0))
        .bottom(px(0.0));
    let Some(program) = shader::program() else {
        return area;
    };
    let horizontal = computed(move || frame.with(|frame| frame.0.config.horizontal()));
    let count = computed(move || frame.with(|frame| frame.0.tiles.len()));
    let bars = dynamic(move || {
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
                            Some(tile) => {
                                shader::uniforms(&frame.0, tile, &Palette::themed(BAR_ALPHA))
                            }
                            None => UniformValue::List(Vec::new()),
                        })
                    })
                    .flex_none()
                    .test_id(&format!("order-book-tile-{at}"));
                if horizontal {
                    tile.h_full().w(length)
                } else {
                    tile.w_full().h(length)
                }
            },
        ))
    });
    area.child(bars)
}

/// How many rows a wheel event moves the ladder toward higher prices.
///
/// The old widget moved a row for each hundred pixels the browser said a
/// wheel turned, which is a notch: here a notch is a row whatever its
/// size, and a touchpad's points move a row per row of travel, kept in
/// `travel` until they add up to one. Down is toward lower prices.
fn wheel_rows(event: &WheelEvent, travel: &Cell<f32>) -> f64 {
    match event.delta {
        WheelDelta::Lines(lines) if lines.y != 0.0 && lines.y.is_finite() => {
            travel.set(0.0);
            -f64::from(lines.y.signum())
        }
        WheelDelta::Points(points) if points.y.0.is_finite() => {
            let total = travel.get() + points.y.0;
            let rows = (total / ROW as f32).trunc();
            travel.set(total - rows * ROW as f32);
            -f64::from(rows)
        }
        _ => 0.0,
    }
}

/// The surface over the plot: the pointer's line and box, moving the
/// ladder, and centering it again.
fn surface(
    session: Rc<OrderBookSession>,
    tab: TabContext<OrderBookSettings>,
    pointer: Signal<Option<(f32, f32)>>,
) -> View {
    let saved = tab.settings;
    let clicks = Clicks::default();
    let travel = Cell::new(0.0_f32);
    let (wheel, release) = (session.clone(), session);
    view()
        .absolute()
        .size_full()
        .label("Order book ladder")
        .test_id("order-book-plot")
        .on_mouse_move(move |event: &MouseEvent| {
            pointer.set(Some((event.position.x.0, event.position.y.0)));
        })
        .on_mouse_leave(move |_: &MouseEvent| pointer.set(None))
        .on_wheel(move |event: &WheelEvent| {
            let rows = wheel_rows(event, &travel);
            // Moving the ladder stops it following a book, for good.
            if rows != 0.0 && wheel.pan(rows) {
                tab.save(&saved.get_untracked().with_center(None));
            }
        })
        .claim_touches(TouchClaim::on_start())
        .keep_touches(true)
        .on_responder_move(move |event: &ResponderEvent| {
            pointer.set(Some((event.location.x.0, event.location.y.0)));
        })
        .on_responder_release(move |event: &ResponderEvent| {
            let on_prices = release.frame.with_untracked(|frame| {
                let frame = &frame.0;
                let (x, y) = (f64::from(event.location.x.0), f64::from(event.location.y.0));
                frame.config.layout == BookLayout::Vertical
                    && (0.0..=frame.price_column).contains(&x)
                    && (frame.plot_top..=frame.plot_bottom).contains(&y)
            });
            // Both clicks on the prices.
            if on_prices && clicks.doubled(event) {
                release.recenter();
            }
        })
}
