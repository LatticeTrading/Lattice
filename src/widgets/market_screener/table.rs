//! The screener's view: a search field, the scopes' status lines, and the
//! table (titles that sort, move and resize their columns, then a
//! virtualized list of rows).
//!
//! Only the rows near the viewport exist. Each row's cells are made once
//! per column layout and bound to the row at their position, so scrolling
//! and new data rewrite text in place instead of making views; showing,
//! hiding or moving a column remakes the cells of the rows on screen, not
//! the list, which keeps its scroll position.
//!
//! A list binds every row near the viewport in one frame, so a row is kept
//! cheap to bind: each cell is one text, sized by its column, which is
//! typeset when its row changes but never measured (the exchange cell is
//! its logo or initials in a box). The table has one tooltip, made when it
//! shows: the cells only hear the pointer come and go, and the cell it
//! rests on says what its own tooltip would (its exact value, unit and
//! source), so binding a row writes no tooltip.
//!
//! Columns keep their widths in proportion as the table grows to fill the
//! pane, until the user sets a width: from then on the widths are as set
//! (the old app froze them at the first resize). A pane narrower than the
//! columns scrolls sideways, and the symbol column then stays at its left
//! edge: it is moved by the scroll offset natively (`scroll_value`), with
//! no engine work per frame, and NSH hit-tests it where it is drawn.
//!
//! The rows scroll vertically only as the table tells them to ([`scroll`]):
//! the list does not scroll natively, and the table has its own scrollbar
//! at the pane's right edge, outside the sideways scroll.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use nsh_core::{
    AccessibilityActionDef, AccessibilityValue, NodeId, PanActivation, PropertyValue, StyleKey,
    TouchEvent, TouchPhase,
    element::{Spawned, Style},
    overlay::{HINT_DELAY, Placement},
    prelude::*,
};

use super::{
    columns::{self, Column, NumberCell, Tone},
    session::{Screener, Table},
    settings::{self, MAX_WIDTH, MIN_WIDTH, ScreenerSettings, Side},
};
use crate::{
    theme,
    ui::{self, RowScroll},
    widgets::{
        TabContext,
        common::{LogoStyle, Shared, bound_exchange_logo},
    },
};

/// A row's height, and the title row's: the old app's Trade Stream density
/// plus half again.
const ROW_HEIGHT: f32 = 30.0;
/// Cells' side padding.
const CELL_PADDING: f32 = 8.0;
/// How the exchange column draws logos.
const LOGO: LogoStyle = LogoStyle {
    size: 12.0,
    color: theme::TEXT,
    initials_size: 6.0,
    boxed: true,
};
/// Panes narrower than this scroll the symbol column with the others (the
/// old app's container query): pinned, it would cover most of the table.
const PIN_MIN_WIDTH: f32 = 420.0;
/// Where the symbol column is "pinned" from when it is not: past any offset.
const NEVER: f32 = 1.0e7;
/// How far a title is dragged sideways before it moves its column instead
/// of sorting by it.
const REORDER_SLOP: f32 = 6.0;
/// The resize grip at a title's trailing edge.
const GRIP: f32 = 6.0;
/// One step of resizing a column from an assistive technology.
const RESIZE_STEP: f32 = 10.0;

/// The columns shown and where each is, as laid out.
#[derive(Clone, Debug, Default, PartialEq)]
struct Geometry {
    /// Each column, its left edge and its width.
    columns: Vec<(Column, f32, f32)>,
    total: f32,
}

impl Geometry {
    /// `visible` side by side, each `width_of` wide, all grown in
    /// proportion to `fill` when that is wider.
    fn lay_out(visible: &[Column], width_of: impl Fn(Column) -> f32, fill: Option<f32>) -> Self {
        let widths: Vec<f32> = visible.iter().map(|&column| width_of(column)).collect();
        let natural: f32 = widths.iter().sum();
        let scale = match fill {
            Some(fill) if natural > 0.0 && fill > natural => fill / natural,
            _ => 1.0,
        };
        let mut left = 0.0;
        let columns = visible
            .iter()
            .zip(widths)
            .map(|(&column, width)| {
                let width = width * scale;
                let placed = (column, left, width);
                left += width;
                placed
            })
            .collect();
        Self {
            columns,
            total: left,
        }
    }

    fn find(&self, column: Column) -> Option<(f32, f32)> {
        self.columns
            .iter()
            .find(|(shown, ..)| *shown == column)
            .map(|&(_, left, width)| (left, width))
    }

    fn width(&self, column: Column) -> f32 {
        self.find(column).map_or(0.0, |(_, width)| width)
    }

    fn left(&self, column: Column) -> f32 {
        self.find(column).map_or(0.0, |(left, _)| left)
    }

    /// Every column's width as laid out.
    fn widths(&self) -> Vec<(Column, f32)> {
        self.columns
            .iter()
            .map(|&(column, _, width)| (column, width))
            .collect()
    }

    /// The column under `x` (table coordinates; past either end, the end
    /// one) and which half of it `x` is in.
    fn drop_at(&self, x: f32) -> Option<(Column, Side)> {
        let &(column, left, width) = self
            .columns
            .iter()
            .find(|(_, left, width)| x < left + width)
            .or(self.columns.last())?;
        let side = if x < left + width / 2.0 {
            Side::Before
        } else {
            Side::After
        };
        Some((column, side))
    }

    /// The column `by` places from `column`.
    fn neighbour(&self, column: Column, by: isize) -> Option<Column> {
        let at = self
            .columns
            .iter()
            .position(|(shown, ..)| *shown == column)?;
        let to = at.checked_add_signed(by)?;
        self.columns.get(to).map(|&(column, ..)| column)
    }
}

/// A title being dragged onto another column's place.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Reorder {
    source: Column,
    /// Where it would go if dropped now.
    target: Option<(Column, Side)>,
}

/// What the table's parts share about its columns. Everything here is
/// view-only: a drag changes the settings once, when it ends.
#[derive(Clone, Copy)]
struct Columns {
    /// The columns shown, in order.
    visible: Memo<Vec<Column>>,
    geometry: Memo<Geometry>,
    /// The widths while a divider is dragged.
    preview: Signal<Option<Vec<(Column, f32)>>>,
    reorder: Signal<Option<Reorder>>,
    /// The table's horizontal scroll offset, as last reported.
    offset: Signal<f32>,
    /// Where the symbol column starts staying put (its left edge), or
    /// [`NEVER`].
    pin_from: Memo<f32>,
    /// How far the symbol column is moved right to stay at the left edge:
    /// the scroll offset past `pin_from`, natively.
    pin: AnimatedNode,
}

impl Columns {
    /// How far `column` is moved right of its place right now.
    fn shift(&self, column: Column) -> f32 {
        if column != Column::Symbol {
            return 0.0;
        }
        (self.offset.get_untracked() - self.pin_from.get_untracked()).max(0.0)
    }
}

pub(super) fn table_view(screener: Rc<Screener>, tab: TabContext<ScreenerSettings>) -> View {
    let list = element_ref();
    let table = screener.table;
    let saved = tab.settings;
    let vertical = RowScroll::new(list, ROW_HEIGHT, move || {
        table.with(|table| table.0.order.len())
    });

    let search = ui::text_field(screener.search)
        .placeholder("Search markets…")
        .label("Search markets")
        .flex_none()
        .w_full()
        .border_b_1()
        .border_color(theme::BORDER)
        .on_change(move |_: &str| vertical.to(0.0));

    // The pane's size, which the table fills (and scrolls past sideways).
    let viewport = signal(Size::new(px(0.0), px(0.0)));
    let preview = signal(None::<Vec<(Column, f32)>>);
    let visible = computed(move || saved.with(ScreenerSettings::visible));
    let geometry = computed(move || {
        let width = viewport.get().width.0;
        preview.with(|preview| {
            saved.with(|settings| {
                let width_of = |column: Column| {
                    preview
                        .as_ref()
                        .and_then(|preview| preview.iter().find(|(c, _)| *c == column))
                        .map(|&(_, width)| width)
                        .or_else(|| settings.width(column))
                        .unwrap_or_else(|| column.width())
                };
                let fill = (preview.is_none() && !settings.sized()).then_some(width);
                visible.with(|visible| Geometry::lay_out(visible, width_of, fill))
            })
        })
    });
    let pin_from = computed(move || {
        if viewport.get().width.0 < PIN_MIN_WIDTH {
            NEVER
        } else {
            geometry.with(|geometry| geometry.left(Column::Symbol))
        }
    });
    let scrolled = animated_value(0.0);
    let pinned_at = animated_value(NEVER);
    effect(move || pinned_at.set(pin_from.get()));
    let layout = Columns {
        visible,
        geometry,
        preview,
        reorder: signal(None),
        offset: signal(0.0),
        pin_from,
        pin: scrolled
            .sub(&pinned_at)
            .interpolate(&[0.0, NEVER], &[0.0, NEVER]),
    };

    // A new scope or product choice starts at the top, as a new search does.
    let demand = computed(move || saved.with(|s| (s.products(), s.scope().map(str::to_owned))));
    let first = Cell::new(true);
    effect(move || {
        demand.with(|_| ());
        if !first.replace(false) {
            vertical.to(0.0);
        }
    });

    let tips = Tips::new();
    // Remade cells hear no leaving: their tooltip goes with them.
    {
        let tips = tips.clone();
        effect(move || {
            visible.with(|_| ());
            tips.hide();
        });
    }

    let body = view()
        .w(move || px(viewport.get().width.0.max(geometry.with(|g| g.total))))
        .h(move || px(viewport.get().height.0))
        .flex_col()
        .child(header(screener.clone(), tab.clone(), layout, vertical))
        .child(rows(
            table,
            layout,
            list,
            vertical,
            screener.clone(),
            saved,
            tips.clone(),
        ))
        .child(tips.tooltip(table));
    // The scroll keys are the table's while the pointer is over it, as the
    // platform's keyboard scrolling follows the pointer.
    let hovered = signal(false);
    vertical.bind_keys(move || hovered.get());
    let area = view()
        .flex_1()
        .min_h(px(0.0))
        .min_w(px(0.0))
        .on_layout(move |bounds: &Bounds<Points>| viewport.set(bounds.size))
        .on_mouse_enter(move |_: &MouseEvent| hovered.set(true))
        .on_mouse_leave(move |_: &MouseEvent| hovered.set(false))
        .child(
            scroll()
                .horizontal()
                .absolute()
                .inset_0()
                .scroll_value(&scrolled)
                .on_scroll(move |event: &ScrollEvent| layout.offset.set(event.offset.x.0))
                .child(body),
        )
        // Beside the rows, at the pane's edge whatever the sideways scroll.
        .child(
            vertical
                .scrollbar("Scroll markets")
                .absolute()
                .top(px(ROW_HEIGHT))
                .bottom(px(0.0))
                .right(px(0.0)),
        );

    view()
        .flex_1()
        .min_h(px(0.0))
        .min_w(px(0.0))
        .flex_col()
        .overflow_hidden()
        .bg(theme::BACKGROUND)
        .text_color(theme::TEXT)
        .text_size(px(11.0))
        .child(search)
        .child(status(screener, tab))
        .child(area)
}

/// A small bordered button in a status line.
fn line_button(label: &'static str, on_press: impl Fn() + 'static) -> View {
    view()
        .px(px(6.0))
        .border_1()
        .border_color(theme::BORDER)
        .bg(theme::BACKGROUND)
        .text_color(theme::TEXT)
        .role(AccessibilityRole::Button)
        .hover(|s| s.bg(theme::SURFACE))
        .active(|s| s.bg(theme::BORDER))
        .on_press(on_press)
        .child(text(label))
}

/// Discovery's state, a saved scope no longer offered, and each scope's
/// report, one line each, above the table.
fn status(screener: Rc<Screener>, tab: TabContext<ScreenerSettings>) -> View {
    let discovery = screener.discovery();
    let table = screener.table;
    let discovering = computed(move || discovery.with(|state| state.is_loading()));
    let failure = computed(move || {
        discovery.with(|state| state.error.as_ref().map(|error| error.message.clone()))
    });
    let unknown = {
        let screener = screener.clone();
        computed(move || screener.scope_unknown())
    };
    let messages = computed(move || table.with(|table| table.0.universe.messages.clone()));
    let line = |content: View| {
        content
            .flex_none()
            .px(px(CELL_PADDING))
            .py(px(3.0))
            .text_size(px(10.0))
    };
    let retry = screener.clone();
    let saved = tab.settings;
    view()
        .flex_none()
        .flex_col()
        .child(
            line(view().text_color(theme::TEXT_MUTED))
                .style_when(move || !discovering.get(), |s| s.hidden())
                .child(text("Discovering statistics…")),
        )
        .child(
            line(
                view()
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .text_color(theme::DANGER),
            )
            .style_when(move || failure.with(Option::is_none), |s| s.hidden())
            .role(AccessibilityRole::Alert)
            .child(text(move || failure.get().unwrap_or_default()))
            .child(line_button("Retry discovery", move || {
                retry.retry_discovery()
            })),
        )
        .child(
            line(
                view()
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .text_color(theme::TEXT_MUTED),
            )
            .style_when(move || !unknown.get(), |s| s.hidden())
            .child(text("This saved scope is no longer advertised."))
            .child(line_button("Show all exchanges", move || {
                tab.save(&saved.get_untracked().with_scope(None))
            })),
        )
        .child(dynamic(move || {
            messages.with(|messages| {
                messages
                    .iter()
                    .map(|message| {
                        line(view().text_color(theme::DANGER)).child(text(message.clone()))
                    })
                    .collect::<Vec<_>>()
            })
        }))
}

/// A title's box: `column`'s laid-out width, with a divider on its trailing
/// edge unless it is the last column (the rows' cells are texts sized the
/// same way, [`cell_text`]). The symbol column's stays at the table's left
/// edge while the table scrolls past it, above the titles it passes over
/// (its background is what hides them).
fn title_box(column: Column, last: bool, layout: Columns) -> View {
    let geometry = layout.geometry;
    let cell = view()
        .h_full()
        .w(move || px(geometry.with(|g| g.width(column))))
        .flex_none()
        .min_w(px(0.0))
        .flex_row()
        .items_center()
        .overflow_hidden()
        .px(px(if column == Column::Exchange {
            4.0
        } else {
            CELL_PADDING
        }))
        .border_color(theme::BORDER);
    let cell = if last { cell } else { cell.border_r_1() };
    let cell = match column {
        Column::Exchange => cell.justify_center(),
        _ if column.numeric() => cell.justify_end(),
        _ => cell,
    };
    if column == Column::Symbol {
        cell.bind_translate_x(&layout.pin)
            .z_index(1)
            .bg(theme::SURFACE)
    } else {
        cell
    }
}

/// The title row: each title sorts by its column, moves it when dragged
/// sideways, and resizes it from its trailing edge.
fn header(
    screener: Rc<Screener>,
    tab: TabContext<ScreenerSettings>,
    layout: Columns,
    vertical: RowScroll,
) -> View {
    view()
        .h(px(ROW_HEIGHT))
        .flex_none()
        .flex_row()
        .bg(theme::SURFACE)
        .border_b_1()
        .border_color(theme::BORDER)
        .child(dynamic(move || {
            let visible = layout.visible.get();
            let count = visible.len();
            visible
                .into_iter()
                .enumerate()
                .map(|(index, column)| {
                    title(
                        column,
                        index + 1 == count,
                        screener.clone(),
                        tab.clone(),
                        layout,
                        vertical,
                    )
                })
                .collect::<Vec<_>>()
        }))
}

/// Moves `column` to `side` of `target` in the saved order.
fn move_column(tab: &TabContext<ScreenerSettings>, column: Column, target: Column, side: Side) {
    if let Some(next) = tab.settings.get_untracked().moved(column, target, side) {
        tab.save(&next);
    }
}

/// One title.
fn title(
    column: Column,
    last: bool,
    screener: Rc<Screener>,
    tab: TabContext<ScreenerSettings>,
    layout: Columns,
    vertical: RowScroll,
) -> View {
    let current = {
        let screener = screener.clone();
        move || screener.current_sort()
    };
    let label = {
        let current = current.clone();
        move || {
            let sort = current();
            match (sort.column == column, sort.descending) {
                (false, _) => column.title().to_owned(),
                (true, false) => format!("{} ↑", column.title()),
                (true, true) => format!("{} ↓", column.title()),
            }
        }
    };
    let state = move || {
        let sort = current();
        match (sort.column == column, sort.descending) {
            (false, _) => "",
            (true, false) => "sorted ascending",
            (true, true) => "sorted descending",
        }
        .to_owned()
    };
    let reorder = layout.reorder;
    let dragged = move || reorder.with(|r| r.is_some_and(|r| r.source == column));
    let dropping =
        move |side: Side| reorder.with(|r| r.and_then(|r| r.target) == Some((column, side)));
    // A press that ends a drag (of this title, or of its grip) does not sort.
    let dragging = Rc::new(Cell::new(false));
    // Where the pointer was in the table when the title was picked up.
    let start = Rc::new(Cell::new(None::<f32>));
    let tip = if column.denominated() {
        format!(
            "{} · native units depend on market family; see cell tooltip. Click to sort; drag to move.",
            column.name()
        )
    } else {
        format!("{}. Click to sort; drag to move.", column.name())
    };
    let sort_by = move || {
        let next = untracked(|| screener.current_sort()).pressed(column);
        screener.sort.set(Some(next));
        vertical.to(0.0);
    };
    let marker = |side: Side| {
        let bar = view()
            .absolute()
            .top(px(0.0))
            .bottom(px(0.0))
            .w(px(3.0))
            .bg(theme::ACCENT)
            .pointer_events(PointerEvents::None)
            .style_when(move || !dropping(side), |s| s.hidden());
        match side {
            Side::Before => bar.left(px(0.0)),
            Side::After => bar.right(px(0.0)),
        }
    };
    let actions = vec![
        AccessibilityActionDef {
            name: "moveLeft".into(),
            label: Some("Move column left".into()),
        },
        AccessibilityActionDef {
            name: "moveRight".into(),
            label: Some("Move column right".into()),
        },
    ];
    let grip = grip(column, tab.clone(), layout, dragging.clone());
    let on_action = {
        let tab = tab.clone();
        move |name: &str| {
            let (by, side) = match name {
                "moveLeft" => (-1, Side::Before),
                "moveRight" => (1, Side::After),
                _ => return,
            };
            if let Some(target) = layout.geometry.with_untracked(|g| g.neighbour(column, by)) {
                move_column(&tab, column, target, side);
            }
        }
    };
    let (grant_dragging, press_dragging, down_dragging) =
        (dragging.clone(), dragging.clone(), dragging);
    let (grant_start, move_start, release_start) = (start.clone(), start.clone(), start);
    title_box(column, last, layout)
        .text_size(px(10.0))
        .font_weight(FontWeight::MEDIUM)
        .letter_spacing(px(0.3))
        .text_color(theme::TEXT_MUTED)
        .role(AccessibilityRole::Button)
        .label(format!("Sort by {}", column.name()))
        .hint(state)
        .accessibility_actions(actions)
        .on_accessibility_action(on_action)
        .tooltip(ui::hint(move || tip.clone()))
        .opacity(move || if dragged() { 0.5_f32 } else { 1.0 })
        .hover(|s| s.bg(theme::ACCENT.opacity(0.08)))
        .active(|s| s.bg(theme::ACCENT.opacity(0.16)))
        .on_press_down(move || down_dragging.set(false))
        .on_press(move || {
            if !press_dragging.replace(false) {
                sort_by();
            }
        })
        .claim_touches(TouchClaim::on_pan(PanActivation::horizontal(
            px(REORDER_SLOP),
            px(ROW_HEIGHT),
        )))
        .keep_touches(true)
        .on_responder_grant(move |event: &ResponderEvent| {
            grant_dragging.set(true);
            let left = layout.geometry.with_untracked(|g| g.left(column));
            grant_start.set(Some(left + layout.shift(column) + event.location.x.0));
            reorder.set(Some(Reorder {
                source: column,
                target: None,
            }));
        })
        .on_responder_move(move |event: &ResponderEvent| {
            let Some(from) = move_start.get() else {
                return;
            };
            let x = from + event.gesture.delta.x.0;
            let target = layout
                .geometry
                .with_untracked(|g| g.drop_at(x))
                .filter(|(target, _)| *target != column);
            // Signal::set compares values: moves within a half redraw nothing.
            reorder.set(Some(Reorder {
                source: column,
                target,
            }));
        })
        .on_responder_release({
            let tab = tab.clone();
            move |_: &ResponderEvent| {
                release_start.set(None);
                let ended = reorder.get_untracked();
                reorder.set(None);
                if let Some(Reorder {
                    target: Some((target, side)),
                    ..
                }) = ended
                {
                    move_column(&tab, column, target, side);
                }
            }
        })
        .on_responder_terminate(move |_: &ResponderEvent| reorder.set(None))
        .child(
            text(label)
                .number_of_lines(1)
                .pointer_events(PointerEvents::None),
        )
        .child(marker(Side::Before))
        .child(marker(Side::After))
        .child(grip)
}

/// `widths` with `column`'s grown by `by` (within the limits).
fn resized(widths: &[(Column, f32)], column: Column, by: f32) -> Vec<(Column, f32)> {
    widths
        .iter()
        .map(|&(shown, width)| {
            if shown == column {
                (shown, settings::clamped(width + by))
            } else {
                (shown, width)
            }
        })
        .collect()
}

/// Saves `widths` over the widths already set (those of hidden columns
/// stay).
fn save_widths(tab: &TabContext<ScreenerSettings>, widths: &[(Column, f32)]) {
    let settings = tab.settings.get_untracked();
    let next = settings.with_widths(settings.widths().chain(widths.iter().copied()));
    tab.save(&next);
}

/// The grip on a title's trailing edge that resizes its column. The first
/// resize keeps every shown column at the width it has then, so the table
/// stops growing to fill the pane (the old app's behavior).
fn grip(
    column: Column,
    tab: TabContext<ScreenerSettings>,
    layout: Columns,
    dragging: Rc<Cell<bool>>,
) -> View {
    // The widths when the drag began.
    let start = Rc::new(std::cell::RefCell::new(None::<Vec<(Column, f32)>>));
    let preview = layout.preview;
    let geometry = layout.geometry;
    let on_action = {
        let tab = tab.clone();
        move |name: &str| {
            let by = match name {
                "increment" => RESIZE_STEP,
                "decrement" => -RESIZE_STEP,
                _ => return,
            };
            let widths = geometry.with_untracked(Geometry::widths);
            save_widths(&tab, &resized(&widths, column, by));
        }
    };
    let (grant_start, move_start, release_start, end_start) =
        (start.clone(), start.clone(), start.clone(), start);
    view()
        .absolute()
        .top(px(0.0))
        .bottom(px(0.0))
        .right(px(0.0))
        .w(px(GRIP))
        .cursor(Cursor::ColResize)
        .role(AccessibilityRole::Adjustable)
        .label(format!("Resize {} column", column.name()))
        .accessibility_value(move || {
            Some(AccessibilityValue {
                min: Some(MIN_WIDTH),
                max: Some(MAX_WIDTH),
                now: Some(geometry.with(|g| g.width(column)).round()),
                text: None,
            })
        })
        .accessibility_actions(vec![
            AccessibilityActionDef {
                name: "increment".into(),
                label: None,
            },
            AccessibilityActionDef {
                name: "decrement".into(),
                label: None,
            },
        ])
        .on_accessibility_action(on_action)
        .hover(|s| s.bg(theme::ACCENT.opacity(0.35)))
        .claim_touches(TouchClaim::on_start())
        .keep_touches(true)
        .on_responder_grant(move |_: &ResponderEvent| {
            dragging.set(true);
            let widths = geometry.with_untracked(Geometry::widths);
            preview.set(Some(widths.clone()));
            *grant_start.borrow_mut() = Some(widths);
        })
        .on_responder_move(move |event: &ResponderEvent| {
            let start = move_start.borrow();
            let Some(widths) = start.as_ref() else {
                return;
            };
            preview.set(Some(resized(widths, column, event.gesture.delta.x.0)));
        })
        .on_responder_release(move |event: &ResponderEvent| {
            let Some(widths) = release_start.borrow_mut().take() else {
                return;
            };
            let by = event.gesture.delta.x.0;
            batch(|| {
                if by != 0.0 {
                    save_widths(&tab, &resized(&widths, column, by));
                }
                preview.set(None);
            });
        })
        .on_responder_terminate(move |_: &ResponderEvent| {
            end_start.borrow_mut().take();
            preview.set(None);
        })
}

/// The rows, virtualized. They scroll vertically only as `vertical` says
/// (the wheel over them sets it); sideways, with the table.
fn rows(
    table: Signal<Shared<Table>>,
    layout: Columns,
    list: ElementRef,
    vertical: RowScroll,
    screener: Rc<Screener>,
    saved: Memo<ScreenerSettings>,
    tips: Tips,
) -> VirtualList {
    let discovery = screener.discovery();
    let empty = move || {
        let discovering = discovery.with(|state| state.is_loading());
        let none_shown = saved.with(|s| s.products().is_empty());
        table.with(|table| {
            let universe = &table.0.universe;
            if none_shown {
                "No products selected in settings."
            } else if discovering || universe.waiting {
                "Waiting for statistics; sources load independently…"
            } else if !universe.answered {
                "No supported product scopes."
            } else {
                "No markets match your search."
            }
        })
    };
    let pressed = tips.clone();
    vertical
        .drive(virtual_list(
            move || table.with(|table| table.0.order.len()),
            move |at: Position| row(table, layout, at, tips.clone()),
        ))
        .estimated_item_size(px(ROW_HEIGHT))
        .element_ref(&list)
        .flex_1()
        .min_h(px(0.0))
        .label("Market statistics")
        // A press anywhere in the rows hides the tooltip, as a press hides an
        // element's; it shows again once the pointer comes over a cell.
        .on_touch(move |event: &TouchEvent| {
            if event.phase == TouchPhase::Start {
                pressed.hide();
            }
        })
        .empty(
            view()
                .p(px(CELL_PADDING))
                .text_color(theme::TEXT_FAINT)
                .role(AccessibilityRole::Text)
                .child(text(move || empty().to_owned())),
        )
}

/// One row: its cells, bound to the market at its position, made again
/// when the columns shown change.
fn row(table: Signal<Shared<Table>>, layout: Columns, at: Position, tips: Tips) -> View {
    view()
        .h(px(ROW_HEIGHT))
        .w_full()
        .flex_row()
        .border_b_1()
        .border_color(theme::BORDER)
        .hover(|s| s.bg(theme::ACCENT.opacity(0.06)))
        .child(dynamic(move || {
            let visible = layout.visible.get();
            let count = visible.len();
            visible
                .into_iter()
                .enumerate()
                .map(|(index, column)| {
                    let last = index + 1 == count;
                    match column {
                        Column::Exchange => exchange_cell(table, layout, at, last, &tips),
                        _ if column.numeric() => {
                            number_cell(table, layout, column, at, last, &tips).into_element()
                        }
                        _ => text_cell(table, layout, column, at, last, &tips).into_element(),
                    }
                })
                .collect::<Vec<_>>()
        }))
}

/// Text in the middle of its box's height (a cell is as tall as its row).
fn middle() -> Style {
    Style::default().with(
        StyleKey::TextAlignVertical,
        PropertyValue::AlignVertical(Some(Percentage(0.5))),
    )
}

/// A row's cell as one text showing `content`: `column`'s laid-out width
/// and the row's height, with a divider on its trailing edge unless it is
/// the last column. Sized by its style, it is never measured: binding it
/// to another row only typesets it. The symbol column's stays at the
/// table's left edge while the table scrolls past it, above the cells it
/// passes over (its background is what hides them).
fn cell_text(
    column: Column,
    last: bool,
    layout: Columns,
    content: impl Fn() -> String + 'static,
) -> Text {
    let geometry = layout.geometry;
    let cell = text(content)
        .w(move || px(geometry.with(|g| g.width(column))))
        .h_full()
        .flex_none()
        .px(px(CELL_PADDING))
        .number_of_lines(1)
        .inline_style(middle())
        .border_color(theme::BORDER);
    let cell = if last { cell } else { cell.border_r_1() };
    let cell = if column.numeric() {
        cell.text_right().tabular_nums()
    } else {
        cell
    };
    if column == Column::Symbol {
        cell.bind_translate_x(&layout.pin)
            .z_index(1)
            .bg(theme::BACKGROUND)
    } else {
        cell
    }
}

/// The exchange's logo (or its initials), named for screen readers and
/// in the tooltip by its scope (`Bybit · Linear perpetuals`). Rows show
/// other exchanges as they are bound, so both stay and one is hidden.
fn exchange_cell(
    table: Signal<Shared<Table>>,
    layout: Columns,
    at: Position,
    last: bool,
    tips: &Tips,
) -> AnyElement {
    let shown = computed(move || {
        table.with(|table| {
            let table = &table.0;
            table.row(at.index()).map(|row| {
                let feed = table.universe.feed(row);
                (feed.scope.exchange.clone(), feed.label.clone())
            })
        })
    });
    let exchange = move || shown.with(|shown| shown.as_ref().map(|(id, _)| id.clone()));
    let label = move || shown.with(|shown| shown.as_ref().map(|(_, label)| label.clone()));
    let (logo, initials) = bound_exchange_logo(exchange, LOGO);
    let geometry = layout.geometry;
    let cell = view()
        .h_full()
        .w(move || px(geometry.with(|g| g.width(Column::Exchange))))
        .flex_none()
        .flex_row()
        .items_center()
        .justify_center()
        .overflow_hidden()
        .px(px(4.0))
        .border_color(theme::BORDER)
        .role(AccessibilityRole::Image)
        .label(move || label().unwrap_or_default())
        .child(logo)
        // The initials sit mid-height, as every cell's text does.
        .child(initials.inline_style(middle()));
    let cell = if last { cell } else { cell.border_r_1() };
    tips.watch(cell, Column::Exchange, at).into_element()
}

/// A text column's cell: the symbol (with its market id in the tooltip),
/// the quote asset, the family.
fn text_cell(
    table: Signal<Shared<Table>>,
    layout: Columns,
    column: Column,
    at: Position,
    last: bool,
    tips: &Tips,
) -> Text {
    let value = move || {
        table.with(|table| {
            let table = &table.0;
            table
                .row(at.index())
                .map(|row| columns::text_of(&table.universe, row, column))
                .unwrap_or_default()
        })
    };
    let cell = cell_text(column, last, layout, value);
    match column {
        Column::Symbol => tips.watch(
            cell.font_weight(FontWeight::MEDIUM).text_color(theme::TEXT),
            column,
            at,
        ),
        Column::Quote => cell.text_size(px(10.0)).text_color(theme::TEXT_MUTED),
        _ => cell.text_color(theme::TEXT),
    }
}

/// A number cell: the value (green or red for funding), a small `%` after
/// a funding rate, and a badge when the value is stale, all in one text;
/// the tooltip says the exact value, its unit, and where it came from.
fn number_cell(
    table: Signal<Shared<Table>>,
    layout: Columns,
    column: Column,
    at: Position,
    last: bool,
    tips: &Tips,
) -> Text {
    let shown = computed(move || {
        table.with(|table| {
            let table = &table.0;
            table.row(at.index()).map_or(NumberCell::BLANK, |row| {
                columns::number_cell(&table.universe, row, column)
            })
        })
    });
    let tone = move || match shown.with(|cell| cell.tone) {
        Tone::Plain => theme::TEXT.get(),
        Tone::Positive => theme::POSITIVE.get(),
        Tone::Negative => theme::NEGATIVE.get(),
    };
    // Spans of the one text, laid out with it: empty while they show
    // nothing, so they take no room. The badge's gap is an en space at its
    // size (4.5pt).
    let stale = text(move || {
        if shown.with(|cell| cell.stale) {
            "\u{2002}STALE"
        } else {
            ""
        }
    })
    .text_size(px(9.0))
    .font_weight(FontWeight::MEDIUM)
    .letter_spacing(px(0.3))
    .text_color(theme::WARNING);
    let cell = cell_text(column, last, layout, move || {
        shown.with(|cell| cell.text.clone())
    })
    .text_color(tone);
    // Only funding columns have the `%`.
    let cell = if column.percent() {
        cell.child(
            text(move || {
                if shown.with(|cell| cell.percent) {
                    "%"
                } else {
                    ""
                }
            })
            .text_size(px(10.0)),
        )
    } else {
        cell
    };
    tips.watch(cell.child(stale), column, at)
}

/// The cell the pointer rests on, for the table's tooltip.
#[derive(Clone, Copy)]
struct Tip {
    column: Column,
    at: Position,
    /// The cell, which the tooltip is placed against.
    cell: ElementRef,
}

/// A [`Tip`] that showed, told apart from the others by when it did.
#[derive(Clone, Copy)]
struct Shown {
    serial: u64,
    tip: Tip,
}

impl PartialEq for Shown {
    fn eq(&self, other: &Self) -> bool {
        self.serial == other.serial
    }
}

/// What the table's tooltip follows.
struct TipState {
    /// The tooltip showing, if one is.
    shown: Signal<Option<Shown>>,
    /// The pointer's wait on a cell before its tooltip shows.
    waiting: RefCell<Option<Spawned<()>>>,
    /// The cell the pointer is on.
    over: Cell<Option<NodeId>>,
    serial: Cell<u64>,
    /// The table's owner, which the waits run under.
    owner: Owner,
}

/// The table's one tooltip, which shows what a cell's own would: the
/// pointer resting on a cell for the desktops' hint delay shows it above
/// the cell, until the pointer leaves the cell or presses, or Escape. Only
/// the cells that have something to say hear the pointer
/// ([`Tips::watch`]); a listener adds no view, and the tooltip's view and
/// text are made only while it shows.
#[derive(Clone)]
struct Tips(Rc<TipState>);

impl Tips {
    fn new() -> Self {
        Self(Rc::new(TipState {
            shown: signal(None),
            waiting: RefCell::new(None),
            over: Cell::new(None),
            serial: Cell::new(0),
            owner: Owner::current(),
        }))
    }

    /// Makes `cell` show the tooltip of `column` for the row at `at`.
    fn watch<E: Interactive>(&self, cell: E, column: Column, at: Position) -> E {
        let anchor = element_ref();
        let (enter, leave) = (self.clone(), self.clone());
        cell.element_ref(&anchor)
            .on_mouse_enter(move |_: &MouseEvent| {
                enter.enter(Tip {
                    column,
                    at,
                    cell: anchor,
                })
            })
            .on_mouse_leave(move |_: &MouseEvent| {
                if leave.0.over.get() == anchor.node() {
                    leave.hide();
                }
            })
    }

    /// The pointer came over a cell: its tooltip shows once it rests there.
    fn enter(&self, tip: Tip) {
        self.hide();
        let state = &self.0;
        state.over.set(tip.cell.node());
        let serial = state.serial.get() + 1;
        state.serial.set(serial);
        let shown = state.shown;
        let wait = state.owner.run(|| {
            spawn(async move {
                timer(HINT_DELAY).await;
                shown.set(Some(Shown { serial, tip }));
            })
        });
        *state.waiting.borrow_mut() = Some(wait);
    }

    /// Hides the tooltip, and stops waiting to show one.
    fn hide(&self) {
        let state = &self.0;
        state.over.set(None);
        let wait = state.waiting.borrow_mut().take();
        if let Some(wait) = wait {
            wait.cancel();
        }
        if state.shown.with_untracked(Option::is_some) {
            state.shown.set(None);
        }
    }

    /// The tooltip, while one shows: an overlay above its cell, which says
    /// what [`columns::tooltip`] does for the row the cell shows now.
    fn tooltip(&self, table: Signal<Shared<Table>>) -> impl IntoElement + use<> {
        let shown = self.0.shown;
        let tips = self.clone();
        dynamic(move || {
            let Shown { tip, .. } = shown.get()?;
            // A cell no longer there has no tooltip.
            tip.cell.node()?;
            let says = move || {
                table.with(|table| {
                    let table = &table.0;
                    table
                        .row(tip.at.index())
                        .map(|row| columns::tooltip(&table.universe, row, tip.column))
                        .unwrap_or_default()
                })
            };
            let cancel = tips.clone();
            Some(
                overlay()
                    .anchor(&tip.cell)
                    .placement(Placement::Top)
                    .offset(px(4.0))
                    .margin(px(4.0))
                    .pointer_events(PointerEvents::None)
                    .on_cancel(move || cancel.hide())
                    .child(ui::hint(says)),
            )
        })
    }
}
