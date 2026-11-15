//! The order book's view: the top bar (the old `WebGLOrderBookTopBar`),
//! the notices of indicative books, and the plot: depth bars on shader
//! tiles ([`super::shader`]), everything written and ruled over them on
//! one canvas ([`super::overlay`]), and the pointer's handling (the old
//! `WebGLOrderBookRenderer`'s).
//!
//! The top bar, 40 points tall:
//! - a segment per book (its exchange's logo and its pair), which opens
//!   the market picker for that book; and `+` after the primary while no
//!   second pair is tracked, which opens it for one;
//! - Center, Group and Depth: the book the ladder follows (or none), the
//!   price grouping, and the levels a side; hidden in a pane 500 points
//!   wide or less, as the old container query hid them (the settings
//!   panel has grouping and depth too).
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
    overlay::{self, Colors, Drawn},
    panel::market_picker,
    session::OrderBookSession,
    settings::{BookLayout, DEPTH_PRESETS, Grouping, Layer, Pair},
    shader::{self, Palette},
};
use crate::{
    data::ferris::Venue,
    exchanges, theme, ui,
    widgets::{
        TabContext,
        common::{Clicks, LogoStyle, Shared, exchange_logo, tag_text},
    },
};

/// The top bar's height (the old `.webgl-orderbook__top-bar`).
const BAR_HEIGHT: f32 = 40.0;
/// At or under this width the top bar's selects hide.
const NARROW: f32 = 500.0;
/// How the top bar draws exchange logos.
const LOGO: LogoStyle = LogoStyle {
    size: 14.0,
    color: theme::TEXT,
    initials_size: 7.0,
    boxed: true,
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
    let narrow = computed(move || width.get() <= NARROW);
    // The pair the top bar's market picker is open for.
    let picking = signal(None::<Pair>);
    let picker = {
        let tab = tab.clone();
        dynamic(move || {
            picking
                .get()
                .map(|pair| market_picker(&tab, pair, move || picking.set(None)))
        })
    };
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
        .child(notices(layers))
        .child(plot(session.clone(), tab.clone()))
        .child(picker)
}

/// The top bar: the books' segments, then the selects.
fn top_bar(
    tab: TabContext<OrderBookSettings>,
    layers: Memo<Vec<Layer>>,
    narrow: Memo<bool>,
    picking: Signal<Option<Pair>>,
) -> View {
    let pairs = view()
        .h_full()
        .min_w(px(0.0))
        .flex_row()
        .child(dynamic(move || {
            layers.with(|layers| {
                let tracks = |pair: Pair| layers.iter().any(|layer| layer.pair == pair);
                let segments = layers
                    .iter()
                    .map(|layer| segment(layer, picking))
                    .fold(view().h_full().flex_row(), View::child);
                if tracks(Pair::Primary) && !tracks(Pair::Secondary) {
                    segments.child(track_another(picking))
                } else {
                    segments
                }
            })
        }));
    let selects = view()
        .h_full()
        .flex_none()
        .flex_row()
        .style_when(move || narrow.get(), |s| s.hidden())
        .child(center_select(tab.clone(), layers))
        .child(grouping_select(tab.clone()))
        .child(depth_select(tab));
    view()
        .h(px(BAR_HEIGHT))
        .w_full()
        .flex_none()
        .flex_row()
        .justify_between()
        .bg(theme::SURFACE)
        .border_b_1()
        .border_color(theme::BORDER)
        .text_color(theme::TEXT)
        .label("Order book controls")
        .child(pairs)
        .child(selects)
}

/// `Bybit BTC/USDT`: a book's exchange and pair.
fn book_name(layer: &Layer) -> String {
    format!(
        "{} {}",
        exchanges::display_name(&layer.market.exchange),
        layer.market.symbol
    )
}

/// A book's logo and pair.
fn logo_and_pair(layer: &Layer) -> View {
    view()
        .min_w(px(0.0))
        .flex_row()
        .items_center()
        .gap(px(7.0))
        .child(exchange_logo(&layer.market.exchange, LOGO))
        .child(
            text(layer.market.symbol.clone())
                .min_w(px(0.0))
                .truncate()
                .text_size(px(12.0)),
        )
}

/// One book's segment of the top bar, which opens the market picker for
/// it.
fn segment(layer: &Layer, picking: Signal<Option<Pair>>) -> View {
    let (pair, name) = (layer.pair, book_name(layer));
    let hint = name.clone();
    logo_and_pair(layer)
        .w(px(105.0))
        .h_full()
        .flex_none()
        .overflow_hidden()
        .px(px(8.0))
        .border_r_1()
        .border_color(theme::BORDER)
        .text_size(px(11.0))
        .role(AccessibilityRole::Button)
        .label(format!("Change {}: {name}", pair.label()))
        .test_id(&format!("order-book-pair-{}", pair.id()))
        .tooltip(ui::hint(move || hint.clone()))
        .hover(|s| s.bg(theme::PANEL))
        .active(|s| s.bg(theme::BORDER))
        .on_press(move || picking.set(Some(pair)))
}

/// `+`: opens the market picker for a second pair.
fn track_another(picking: Signal<Option<Pair>>) -> View {
    view()
        .w(px(BAR_HEIGHT))
        .h_full()
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .border_r_1()
        .border_color(theme::BORDER)
        .text_size(px(16.0))
        .text_color(theme::TEXT_FAINT)
        .role(AccessibilityRole::Button)
        .label("Track another pair")
        .test_id("order-book-track-pair")
        .tooltip(ui::hint(|| "Track another pair".to_owned()))
        .hover(|s| s.bg(theme::PANEL))
        .active(|s| s.bg(theme::BORDER))
        .on_press(move || picking.set(Some(Pair::Secondary)))
        .child(text("+"))
}

/// One of the top bar's selects: its name in faint text, then what it
/// shows, which opens its list (the old `OrderBookDropdown`).
fn quick_select(
    name: &'static str,
    width: f32,
    shown: impl IntoElement,
    items: impl Fn() -> Vec<SelectItem> + 'static,
    value: impl Fn() -> SharedString + 'static,
    on_pick: impl Fn(&str) + 'static,
) -> impl IntoElement {
    let trigger = view()
        .size_full()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(5.0))
        .px(px(8.0))
        .overflow_hidden()
        .text_size(px(11.0))
        .text_color(theme::TEXT)
        .hover(|s| s.bg(theme::PANEL))
        .active(|s| s.bg(theme::BORDER))
        .child(
            text(name)
                .flex_none()
                .number_of_lines(1)
                .text_color(theme::TEXT_FAINT),
        )
        .child(shown);
    ui::select_from(items, value, on_pick, name, trigger)
        .w(px(width))
        .h_full()
        .flex_none()
        .border_l_1()
        .border_color(theme::BORDER)
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
            Some(layer) => logo_and_pair(&layer),
            None => view().child(text("None").number_of_lines(1)),
        }));
    quick_select(
        "Center",
        170.0,
        shown,
        move || {
            layers.with(|layers| {
                std::iter::once(SelectItem::new(UNFOLLOWED, "None"))
                    .chain(
                        layers
                            .iter()
                            .map(|layer| SelectItem::new(layer.pair.id(), book_name(layer))),
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

/// What a select shows when it is a word or a number: on one line, at the
/// trigger's right edge.
fn shown_text(content: impl Fn() -> String + 'static) -> Text {
    text(content)
        .min_w(px(25.0))
        .text_right()
        .number_of_lines(1)
        .tabular_nums()
}

/// "Group": raw prices, or the step levels merge into.
fn grouping_select(tab: TabContext<OrderBookSettings>) -> impl IntoElement {
    let saved = tab.settings;
    let grouping = computed(move || saved.with(OrderBookSettings::grouping));
    quick_select(
        "Group",
        120.0,
        shown_text(move || grouping.get().label().to_owned()),
        || {
            Grouping::ALL
                .iter()
                .map(|grouping| SelectItem::new(grouping.label(), grouping.label()))
                .collect()
        },
        move || SharedString::from(grouping.get().label()),
        move |label| {
            if let Some(grouping) = Grouping::from_label(label) {
                tab.save(&saved.get_untracked().with_grouping(grouping));
            }
        },
    )
}

/// "Depth": the levels a side, from the old presets (and the depth set in
/// the settings panel, when it is none of them).
fn depth_select(tab: TabContext<OrderBookSettings>) -> impl IntoElement {
    let saved = tab.settings;
    let depth = computed(move || saved.with(OrderBookSettings::depth));
    quick_select(
        "Depth",
        120.0,
        shown_text(move || depth.get().to_string()),
        move || {
            let depth = depth.get();
            std::iter::once(depth)
                .filter(|depth| !DEPTH_PRESETS.contains(depth))
                .chain(DEPTH_PRESETS)
                .map(|depth| SelectItem::new(depth.to_string(), depth.to_string()))
                .collect()
        },
        move || SharedString::from(depth.get().to_string()),
        move |picked| {
            if let Ok(depth) = picked.parse::<u16>() {
                tab.save(&saved.get_untracked().with_depth(depth));
            }
        },
    )
}

/// A strip under the top bar naming each book whose exchange says its
/// public book is not what it seems (the old `OrderBookNotice`): the
/// exchange's words are the tag's tooltip.
fn notices(layers: Memo<Vec<Layer>>) -> View {
    let noted = computed(move || {
        layers.with(|layers| {
            layers
                .iter()
                .filter_map(|layer| {
                    let notice = Venue::of(&layer.market.exchange).book_notice?;
                    Some((
                        format!("{}: {}", layer.pair.label(), book_name(layer)),
                        notice,
                    ))
                })
                .collect::<Vec<_>>()
        })
    });
    let strip = || view().flex_row().flex_wrap().items_center().gap(px(8.0));
    strip()
        .flex_none()
        .px(px(8.0))
        .py(px(4.0))
        .border_b_1()
        .border_color(theme::BORDER)
        .bg(theme::PANEL)
        .text_size(px(9.0))
        .text_color(theme::TEXT_MUTED)
        .test_id("order-book-notices")
        .style_when(move || noted.with(Vec::is_empty), |s| s.hidden())
        .child(dynamic(move || {
            noted.with(|noted| {
                noted
                    .iter()
                    .map(|(book, notice)| {
                        let notice = *notice;
                        view()
                            .flex_none()
                            .flex_row()
                            .items_center()
                            .gap(px(8.0))
                            .child(text(book.clone()).number_of_lines(1))
                            .child(
                                tag_text(text("Indicative book"))
                                    .flex_none()
                                    .px(px(5.0))
                                    .py(px(1.0))
                                    .border_1()
                                    .border_color(theme::BORDER)
                                    .bg(theme::BACKGROUND)
                                    .role(AccessibilityRole::Text)
                                    .label(format!("Indicative book: {notice}"))
                                    .tooltip(ui::hint(move || notice.to_owned())),
                            )
                    })
                    .fold(strip(), View::child)
            })
        }))
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
        .children(logos(drawn))
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
                            Some(tile) => shader::uniforms(&frame.0, tile, &Palette::themed()),
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

/// The exchange logos beside the overlay's line labels, where it placed
/// them: a canvas draws no SVG, so they are views over it.
fn logos(drawn: Memo<Shared<Drawn>>) -> impl Iterator<Item = View> {
    (0..LOGOS).map(move |slot| {
        let logo = computed(move || drawn.with(|drawn| drawn.0.logos.get(slot).cloned()));
        let source = computed(move || logo.with(|logo| exchanges::logo(&logo.as_ref()?.exchange)));
        let of = move |read: fn(&overlay::Logo) -> f32| {
            logo.with(|logo| logo.as_ref().map_or(0.0, read))
        };
        view()
            .absolute()
            .left(move || px(of(|logo| logo.x)))
            .top(move || px(of(|logo| logo.y)))
            .size(move || px(of(|logo| logo.size)))
            .text_color(theme::TEXT_MUTED)
            .style_when(move || source.with(Option::is_none), |s| s.hidden())
            .child(svg(move || source.get().unwrap_or_else(exchanges::blank_logo)).size_full())
    })
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
