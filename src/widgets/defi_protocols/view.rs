//! The DeFi Protocols' view, a column of full-width blocks:
//!
//! - its bar: the search field, which fills it, then the category select;
//! - what is to be said of the list, as strips hidden while there is
//!   nothing to say: that it is loading or searching, in muted text; that
//!   DefiLlama is not answering, or that a request failed, as a warning;
//! - the table: titles that order the rows by their column (the ordered
//!   one in the accent color, with which way it runs), then a virtualized
//!   list of rows. The name's column takes the room the others leave; a
//!   pane narrower than the columns scrolls sideways, and the rows scroll
//!   down as the table tells them to ([`RowScroll`]), with its scrollbar
//!   at the pane's right edge;
//! - its strip: how many protocols show of how many match and when the
//!   data is from, then the credit DefiLlama asks for.
//!
//! Only the rows near the viewport exist, and a row is cheap to bind: each
//! cell is one text sized by its column, typeset when its row changes and
//! never measured.

use std::{cell::Cell, rc::Rc};

use nsh_base::SelectItem;
use nsh_core::{LineHeight, prelude::*};

use super::{
    session::{self, DefiProtocolsSession, Standing, Table, Tone},
    settings::{Column, DefiProtocolsSettings},
};
use crate::{
    data::ferris::defillama::{Protocols, SOURCE_URL},
    format,
    shell::clock::{self, Clock, ClockPrefs},
    theme,
    ui::{self, RowScroll},
    widgets::{
        TabContext,
        common::{Shared, books::quick_select, source_credit},
    },
};

/// What its test ids start with.
const ID: &str = "defi-protocols";
/// A row's height, with the line under it.
const ROW_HEIGHT: f32 = 30.0;
/// The titles' height, and the strip's under the table: 28 and its line.
const TITLE_HEIGHT: f32 = 29.0;
const STRIP_HEIGHT: f32 = 29.0;
/// Cells' side padding.
const CELL_PADDING: f32 = 8.0;
/// The name's column at its narrowest.
const NAME_MIN: f32 = 170.0;
/// The category select's width in the bar.
const SELECT_WIDTH: f32 = 180.0;
/// The category select's choice of every category (no category is named
/// so).
const EVERY: &str = "*";

/// A column's width; `name` is the name's, which takes what the others
/// leave. The others fit their widest figure in the table's 11pt text.
fn width_of(column: Column, name: f32) -> f32 {
    match column {
        Column::Rank => 40.0,
        Column::Name => name,
        Column::Tvl | Column::Mcap | Column::McapTvl => 80.0,
        Column::Change1h | Column::Change1d | Column::Change7d => 72.0,
        Column::Category => 140.0,
        Column::Chains => 130.0,
    }
}

/// What every column but the name's takes together.
fn fixed_width() -> f32 {
    Column::ALL
        .into_iter()
        .filter(|column| *column != Column::Name)
        .map(|column| width_of(column, 0.0))
        .sum()
}

/// What the strip under the table says of `list`: how many protocols show
/// (of how many match, when more do), and when Ferris got them, on the
/// clock `prefs` set.
pub(super) fn summary(list: &Protocols, prefs: ClockPrefs) -> String {
    let count = |count: usize| format::grouped(count as f64, 0);
    let shown = list.protocols.len();
    let mut summary = match (shown, list.total) {
        (1, 1) => "1 protocol".to_owned(),
        (shown, total) if total > shown => {
            format!("{} of {} protocols", count(shown), count(total))
        }
        (shown, _) => format!("{} protocols", count(shown)),
    };
    if let Some(fetched) = list.fetched_at_ms {
        let at = clock::format_day_time((fetched / 1000.0).floor() as i64, prefs);
        summary.push_str(&format!(" · as of {at}"));
    }
    summary
}

/// The DeFi Protocols of `session`, as its tab's settings say.
pub(super) fn protocols_view(
    session: &Rc<DefiProtocolsSession>,
    tab: &TabContext<DefiProtocolsSettings>,
) -> View {
    let list = element_ref();
    let (table, standing, saved) = (session.table, session.standing, tab.settings);
    let vertical = RowScroll::new(list, ROW_HEIGHT, move || table.with(|table| table.0.len()));

    // Another category or order starts at the top, as a new search does.
    let first = Cell::new(true);
    effect(move || {
        saved.with(|_| ());
        if !first.replace(false) {
            vertical.to(0.0);
        }
    });

    let search = ui::text_field(session.search)
        .placeholder("Search protocols…")
        .label("Search protocols")
        .test_id(&format!("{ID}-search"))
        .flex_1()
        .on_change(move |_: &str| vertical.to(0.0));
    let bar = view()
        .h(ui::ROW_HEIGHT + px(1.0))
        .w_full()
        .flex_none()
        .flex_row()
        .border_b_1()
        .border_color(theme::BORDER)
        .bg(theme::SURFACE)
        .child(search)
        .child(category_select(session, tab));

    // The pane's size, which the table fills (and scrolls past sideways).
    let viewport = signal(Size::new(px(0.0), px(0.0)));
    let name_width = computed(move || (viewport.get().width.0 - fixed_width()).max(NAME_MIN));
    let body = view()
        .test_id(&format!("{ID}-table"))
        .w(move || px(fixed_width() + name_width.get()))
        .h(move || px(viewport.get().height.0))
        .flex_col()
        .child(titles(tab, name_width))
        .child(rows(table, standing, name_width, list, vertical));
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
        .child(scroll().horizontal().absolute().inset_0().child(body))
        // Beside the rows, at the pane's edge whatever the sideways scroll.
        .child(
            vertical
                .scrollbar("Scroll protocols")
                .absolute()
                .top(px(TITLE_HEIGHT))
                .bottom(px(0.0))
                .right(px(0.0)),
        );

    view()
        .test_id(ID)
        .flex_1()
        .min_h(px(0.0))
        .min_w(px(0.0))
        .flex_col()
        .overflow_hidden()
        .bg(theme::BACKGROUND)
        .text_color(theme::TEXT)
        .text_size(px(11.0))
        .child(bar)
        .child(note(move || {
            standing.with(|standing| standing.note().map(str::to_owned))
        }))
        .child(warning(move || standing.with(Standing::warning)))
        .child(area)
        .child(strip(session))
}

/// The bar's category select: "Category", then the one listed ("All" for
/// every one), which opens the list of them. It offers every category a
/// list has named so far, and the one listed whether or not one has.
fn category_select(
    session: &Rc<DefiProtocolsSession>,
    tab: &TabContext<DefiProtocolsSettings>,
) -> impl IntoElement + use<> {
    let (saved, tab, known) = (tab.settings, tab.clone(), session.categories);
    let listed = computed(move || saved.with(|settings| settings.category().map(str::to_owned)));
    let shown = text(move || listed.get().unwrap_or_else(|| "All".to_owned()))
        .flex_1()
        .min_w(px(0.0))
        .text_right()
        .truncate();
    quick_select(
        "Category",
        SELECT_WIDTH,
        shown,
        move || {
            let (known, listed) = (known.get(), listed.get());
            let unknown = listed.filter(|listed| !known.contains(listed));
            std::iter::once(SelectItem::new(EVERY, "All"))
                .chain(
                    unknown
                        .iter()
                        .chain(&known)
                        .map(|category| SelectItem::new(category.clone(), category.clone())),
                )
                .collect()
        },
        move || SharedString::from(listed.get().unwrap_or_else(|| EVERY.to_owned())),
        move |picked| {
            let category = (picked != EVERY).then_some(picked);
            tab.save(&saved.get_untracked().with_category(category));
        },
    )
}

/// A strip over the table saying `content` in small muted text, with a
/// line under it; hidden while there is nothing to say.
fn note(content: impl Fn() -> Option<String> + 'static) -> View {
    let content = computed(content);
    view()
        .flex_none()
        .w_full()
        .px(px(10.0))
        .py(px(5.0))
        .border_b_1()
        .border_color(theme::BORDER)
        .text_color(theme::TEXT_MUTED)
        .role(AccessibilityRole::Text)
        .style_when(move || content.with(Option::is_none), |s| s.hidden())
        .child(
            text(move || content.get().unwrap_or_default()).line_height(LineHeight::Multiple(1.5)),
        )
}

/// A [`note`] that warns: the warning color, on a tint of it, behind a
/// thick left edge.
fn warning(content: impl Fn() -> Option<String> + 'static) -> View {
    note(content)
        .border_l(px(3.0))
        .border_l_color(theme::WARNING)
        .bg(theme::WARNING.opacity(0.1))
        .text_color(theme::WARNING)
        .role(AccessibilityRole::Alert)
        .test_id(&format!("{ID}-warning"))
}

/// The titles, over the rows: a cell a column, as wide as its cells.
fn titles(tab: &TabContext<DefiProtocolsSettings>, name_width: Memo<f32>) -> View {
    let last = Column::ALL.len() - 1;
    let cells = Column::ALL
        .into_iter()
        .enumerate()
        .map(|(at, column)| title(tab.clone(), column, name_width, at == last));
    view()
        .h(px(TITLE_HEIGHT))
        .w_full()
        .flex_none()
        .flex_row()
        .border_b_1()
        .border_color(theme::BORDER)
        .bg(theme::SURFACE)
        .children(cells)
}

/// A column's title. A press orders the rows by it, and again the other
/// way round; while they are, it is in the accent color and says which
/// way they run.
fn title(
    tab: TabContext<DefiProtocolsSettings>,
    column: Column,
    name_width: Memo<f32>,
    last: bool,
) -> View {
    let saved = tab.settings;
    let ordered = move || saved.with(|settings| settings.sort() == column);
    let says = move || {
        saved.with(
            |settings| match (settings.sort() == column, settings.descending()) {
                (false, _) => column.title().to_owned(),
                (true, true) => format!("{} ▾", column.title()),
                (true, false) => format!("{} ▴", column.title()),
            },
        )
    };
    let cell = view()
        .h_full()
        .w(move || px(width_of(column, name_width.get())))
        .flex_none()
        .flex()
        .items_center()
        .px(px(CELL_PADDING))
        .border_color(theme::BORDER)
        .text_size(px(10.0))
        .text_color(theme::TEXT_FAINT)
        .tooltip(ui::hint(move || column.hint().to_owned()))
        .test_id(&format!("{ID}-title-{}", column.id()))
        .child(
            text(says)
                .number_of_lines(1)
                .font_weight(FontWeight::SEMIBOLD)
                .letter_spacing(px(0.4)),
        );
    let cell = if column.numeric() {
        cell.justify_end()
    } else {
        cell
    };
    let cell = if last { cell } else { cell.border_r_1() };
    if !column.sorts() {
        return cell;
    }
    cell.role(AccessibilityRole::Button)
        .label(format!("Sort by {}", column.title()))
        .accessibility_state(move || AccessibilityState {
            selected: ordered(),
            ..Default::default()
        })
        .hover(|s| s.bg(theme::PANEL))
        .active(|s| s.bg(theme::BORDER))
        .style_when(ordered, |s| s.text_color(theme::ACCENT))
        .on_press(move || tab.save(&saved.get_untracked().sorted_by(column)))
}

/// The rows, virtualized. They scroll vertically only as `vertical` says
/// (the wheel over them sets it); sideways, with the table.
fn rows(
    table: Signal<Shared<Table>>,
    standing: Memo<Standing>,
    name_width: Memo<f32>,
    list: ElementRef,
    vertical: RowScroll,
) -> VirtualList {
    // The strips say what is loading or wrong; this, that nothing matched.
    let empty = move || match standing.get() {
        Standing::Ready | Standing::Stale | Standing::Behind(_) => "No protocols match.",
        Standing::Loading | Standing::Searching | Standing::Failed(_) => "",
    };
    vertical
        .drive(virtual_list(
            move || table.with(|table| table.0.len()),
            move |at: Position| row(table, name_width, at),
        ))
        .estimated_item_size(px(ROW_HEIGHT))
        .element_ref(&list)
        .flex_1()
        .min_h(px(0.0))
        .label("Protocols")
        .empty(
            view()
                .p(px(CELL_PADDING))
                .text_color(theme::TEXT_FAINT)
                .role(AccessibilityRole::Text)
                .child(text(move || empty().to_owned())),
        )
}

/// One row: its cells, bound to the protocol at its position.
fn row(table: Signal<Shared<Table>>, name_width: Memo<f32>, at: Position) -> View {
    let last = Column::ALL.len() - 1;
    let cells = Column::ALL
        .into_iter()
        .enumerate()
        .map(|(index, column)| cell(table, name_width, column, at, index == last));
    view()
        .h(px(ROW_HEIGHT))
        .w_full()
        .flex_row()
        .border_b_1()
        .border_color(theme::BORDER)
        .hover(|s| s.bg(theme::ACCENT.opacity(0.06)))
        .children(cells)
}

/// A row's cell as one text: its column's width and the row's height, with
/// a line at its right unless it is the last. Sized by its style, it is
/// never measured: binding it to another row only typesets it. The name's
/// has the token's symbol after it, in small faint text.
fn cell(
    table: Signal<Shared<Table>>,
    name_width: Memo<f32>,
    column: Column,
    at: Position,
    last: bool,
) -> Text {
    let shown = computed(move || {
        table.with(|table| {
            table.0.row(at.index()).map_or_else(
                || (String::new(), Tone::Plain),
                |(rank, row)| session::cell_of(rank, row, column),
            )
        })
    });
    let tone = move || match shown.with(|(_, tone)| *tone) {
        Tone::Plain => theme::TEXT.get(),
        Tone::Muted => theme::TEXT_MUTED.get(),
        Tone::Faint => theme::TEXT_FAINT.get(),
        Tone::Up => theme::POSITIVE.get(),
        Tone::Down => theme::NEGATIVE.get(),
    };
    let cell = text(move || shown.with(|(text, _)| text.clone()))
        .w(move || px(width_of(column, name_width.get())))
        .h_full()
        .flex_none()
        .px(px(CELL_PADDING))
        .number_of_lines(1)
        // A cell is as tall as its row.
        .inline_style(ui::middle())
        .border_color(theme::BORDER)
        .text_color(tone);
    let cell = if last { cell } else { cell.border_r_1() };
    let cell = if column.numeric() {
        cell.text_right().tabular_nums()
    } else {
        cell
    };
    if column != Column::Name {
        return cell;
    }
    // A span of the one text, laid out with it: empty for a protocol with
    // no token, so it takes no room. Its gap is an en space at its size.
    let symbol = text(move || {
        table.with(|table| {
            table
                .0
                .row(at.index())
                .and_then(|(_, row)| row.symbol.as_ref())
                .map(|symbol| format!("\u{2002}{symbol}"))
                .unwrap_or_default()
        })
    })
    .text_size(px(10.0))
    .text_color(theme::TEXT_FAINT);
    cell.font_weight(FontWeight::MEDIUM).child(symbol)
}

/// The strip under the table: what shows of what matches and when it is
/// from, which takes what room there is, then DefiLlama's credit.
fn strip(session: &Rc<DefiProtocolsSession>) -> View {
    let table = session.table;
    let clock = use_context::<Clock>();
    let says = move || {
        let prefs = clock.map(Clock::get).unwrap_or_default();
        table.with(|table| {
            table
                .0
                .list
                .as_deref()
                .map(|list| summary(list, prefs))
                .unwrap_or_default()
        })
    };
    let page = move || {
        table.with_untracked(|table| {
            table
                .0
                .list
                .as_deref()
                .map_or_else(|| SOURCE_URL.to_owned(), |list| list.source_url.clone())
        })
    };
    view()
        .h(px(STRIP_HEIGHT))
        .w_full()
        .flex_none()
        .flex_row()
        .border_t_1()
        .border_color(theme::BORDER)
        .bg(theme::SURFACE)
        .text_color(theme::TEXT_MUTED)
        .child(
            view()
                .h_full()
                .flex_1()
                .min_w(px(0.0))
                .flex()
                .items_center()
                .px(px(10.0))
                .test_id(&format!("{ID}-summary"))
                .child(text(says).min_w(px(0.0)).truncate().tabular_nums()),
        )
        .child(source_credit("DefiLlama", page).test_id(&format!("{ID}-credit")))
}
