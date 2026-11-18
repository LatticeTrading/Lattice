//! The Positions' view: the frame every account widget has (the account's
//! strip, what could not be read, the form while no account is set:
//! [`account_frame`]) around its table.
//!
//! - The table is one grid, a row per open position under titles that stay
//!   at the top while the rows scroll: the market (its instrument, which
//!   way the position faces, green long and red short, and its leverage
//!   setting), its size, what it is worth, its entry and mark prices, its
//!   unrealized profit and loss with its return on equity (green above
//!   zero, red below), its liquidation price and the margin it holds.
//! - A column is at least as wide as its figures and they share the room
//!   left over; a pane narrower than the columns scrolls sideways, and one
//!   shorter than the rows scrolls down under the titles.
//! - An account with nothing open says so under the titles.
//!
//! A row is made once per position and its cells follow its row's values,
//! so a new report rewrites text and makes no view.

use nsh_core::prelude::*;

use super::{
    session::{PositionsSession, Row},
    settings::PositionsSettings,
};
use crate::{
    data::portfolio::Direction,
    theme,
    widgets::{
        common::{
            accounts::{account_frame, Standing, Tone},
            Shared,
        },
        TabContext,
    },
};

/// What its test ids start with.
const ID: &str = "positions";
/// A row's height, and a title's, each with the line under it.
const ROW_HEIGHT: f32 = 37.0;
const TITLE_HEIGHT: f32 = 29.0;
/// A cell's side padding.
const PADDING: f32 = 10.0;
/// The grid's columns: each at least as wide as its widest cell, sharing
/// the room left over in proportion to what a column of its kind usually
/// holds (so a pane with no room to spare is not made any wider).
const COLUMNS: &str = "minmax(max-content,17fr) minmax(max-content,7.5fr) \
                       minmax(max-content,9.5fr) minmax(max-content,8fr) \
                       minmax(max-content,8fr) minmax(max-content,15fr) \
                       minmax(max-content,8.5fr) minmax(max-content,9fr)";

/// A column of the table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Column {
    Market,
    Size,
    Value,
    Entry,
    Mark,
    Pnl,
    Liquidation,
    Margin,
}

impl Column {
    /// Every column, in the table's order.
    const ALL: [Self; 8] = [
        Self::Market,
        Self::Size,
        Self::Value,
        Self::Entry,
        Self::Mark,
        Self::Pnl,
        Self::Liquidation,
        Self::Margin,
    ];

    fn title(self) -> &'static str {
        match self {
            Self::Market => "Market",
            Self::Size => "Size",
            Self::Value => "Value",
            Self::Entry => "Entry",
            Self::Mark => "Mark",
            Self::Pnl => "PnL (ROE)",
            Self::Liquidation => "Liq. Price",
            Self::Margin => "Margin",
        }
    }

    /// What `row`'s cell of it reads; the market's cell is made of more.
    fn figure(self, row: &Row) -> String {
        match self {
            Self::Market => row.instrument.clone(),
            Self::Size => row.size.clone(),
            Self::Value => row.value.clone(),
            Self::Entry => row.entry.clone(),
            Self::Mark => row.mark.clone(),
            Self::Pnl => row.pnl.clone(),
            Self::Liquidation => row.liquidation.clone(),
            Self::Margin => row.margin.clone(),
        }
    }
}

/// The Positions of `session`, for the account its tab's settings name.
pub(super) fn positions_view(
    session: &PositionsSession,
    tab: &TabContext<PositionsSettings>,
) -> View {
    let (report, rows) = (session.report, session.rows);
    // Nothing open: said once a report has arrived to say it.
    let none_open = computed(move || {
        matches!(report.standing.get(), Standing::Ready | Standing::Stale(_))
            && rows.with(|rows| rows.0.is_empty())
    });
    let empty = view()
        .test_id(&format!("{ID}-empty"))
        .px(px(PADDING))
        .py(px(10.0))
        .text_color(theme::TEXT_MUTED)
        .role(AccessibilityRole::Text)
        .style_when(move || !none_open.get(), |s| s.hidden())
        .child(text("No open positions"));

    // The pane's room below the strip, which the table fills (and scrolls
    // past sideways).
    let viewport = signal(Size::new(px(0.0), px(0.0)));
    // One selection across the table, as a page's.
    let content = view()
        .selectable(true)
        .flex_col()
        .child(table(rows))
        .child(empty);
    let body = view()
        .flex_none()
        .min_w(move || px(viewport.get().width.0))
        .h(move || px(viewport.get().height.0))
        .child(scroll().size_full().child(content));
    let area = view()
        .flex_1()
        .min_h(px(0.0))
        .min_w(px(0.0))
        .on_layout(move |bounds: &Bounds<Points>| viewport.set(bounds.size))
        .child(scroll().horizontal().absolute().inset_0().child(body));

    account_frame(tab, &session.portfolio, report.standing, "positions", area)
}

/// The table: the titles, then a row per position.
fn table(rows: Signal<Shared<Vec<Row>>>) -> View {
    let lines = each(
        move || rows.with(|rows| Vec::clone(&rows.0)),
        |row: &Row| row.key.clone(),
        move |row, _| {
            Column::ALL
                .iter()
                .map(|column| cell(*column, row))
                .collect::<Vec<_>>()
        },
    );
    view()
        .test_id(&format!("{ID}-table"))
        .w_full()
        .grid()
        .grid_template_columns(COLUMNS)
        .font_weight(FontWeight::MEDIUM)
        .children(Column::ALL.map(title))
        .child(lines)
}

/// What every cell of the table is: as high as its row, its content in the
/// middle of its height, a line under it and, unless it is the last of its
/// row, one at its right.
fn boxed(height: f32, column: Column) -> View {
    let cell = view()
        .h(px(height))
        .flex()
        .items_center()
        .px(px(PADDING))
        .border_b_1()
        .border_color(theme::BORDER);
    let cell = match column {
        Column::Margin => cell,
        _ => cell.border_r_1(),
    };
    match column {
        Column::Market => cell,
        _ => cell.justify_end(),
    }
}

/// A column's title, which stays at the top while the rows scroll.
fn title(column: Column) -> View {
    boxed(TITLE_HEIGHT, column)
        .sticky()
        .top(px(0.0))
        .z_index(1)
        .bg(theme::SURFACE)
        .child(
            text(column.title())
                .number_of_lines(1)
                .text_size(px(10.0))
                .font_weight(FontWeight::SEMIBOLD)
                .letter_spacing(px(0.8))
                .uppercase()
                .text_color(theme::TEXT_FAINT),
        )
}

/// `row`'s cell of `column`.
fn cell(column: Column, row: ReadSignal<Row>) -> View {
    let figure = move || {
        text(move || row.with(|row| column.figure(row)))
            .number_of_lines(1)
            .tabular_nums()
    };
    match column {
        Column::Market => {
            // A row keeps its instrument and direction: they are its key.
            let direction = row.with_untracked(|row| row.direction);
            let side = match direction {
                Direction::Long => theme::POSITIVE,
                Direction::Short => theme::NEGATIVE,
            };
            boxed(ROW_HEIGHT, column)
                .gap(px(8.0))
                .child(
                    figure()
                        .min_w(px(0.0))
                        .truncate()
                        .select_text()
                        .font_weight(FontWeight::SEMIBOLD),
                )
                .child(
                    text(direction.label())
                        .flex_none()
                        .number_of_lines(1)
                        .text_size(px(10.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .letter_spacing(px(0.6))
                        .uppercase()
                        .text_color(side),
                )
                .child(
                    text(move || row.with(|row| row.leverage.clone()))
                        .flex_none()
                        .number_of_lines(1)
                        .tabular_nums()
                        .text_color(theme::TEXT_FAINT),
                )
        }
        Column::Pnl => {
            let tone = move || Tone::color(row.with(|row| row.tone));
            boxed(ROW_HEIGHT, column).child(figure().select_text().text_color(tone))
        }
        _ => boxed(ROW_HEIGHT, column).child(figure().select_text()),
    }
}
