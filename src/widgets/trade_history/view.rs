//! The Trade History's view: the frame every account widget has (the
//! accounts' strip, what could not be read, the form while no account is
//! set: [`account_frame`]) around its totals and its table.
//!
//! - The totals: how many trades are listed and since when, then their
//!   volume, the fees paid and the profit and loss they closed (green above
//!   zero, red below), side by side.
//! - The table is one grid, a row per trade under titles that stay at the
//!   top while the rows scroll: when (as the clock writes it), the market
//!   (whose it is, while several accounts show: its exchange's logo, named
//!   in its tooltip; its instrument, what the trade did, green for a buy and red for a
//!   sell, and whether it took liquidity), its price, size and worth, its
//!   fee and what it closed.
//! - A column is at least as wide as its figures and they share the room
//!   left over; a pane narrower than the columns scrolls sideways, and one
//!   shorter than the rows scrolls down under the titles.
//! - Accounts with no trades say so under the titles.
//!
//! A row is made once per trade and its cells follow its row's values, so
//! a new report rewrites text and makes no view for a trade already shown.

use nsh_core::prelude::*;

use super::{
    session::{Row, Totals, TradeHistorySession},
    settings::TradeHistorySettings,
};
use crate::{
    data::portfolio::TradeSide,
    format,
    shell::clock::{self, Clock, ClockPrefs},
    theme,
    widgets::{
        TabContext,
        common::{
            Shared,
            accounts::{Tone, account_frame, mark_view},
        },
    },
};

/// What its test ids start with.
const ID: &str = "trade-history";
/// A row's height, and a title's, each with the line under it.
const ROW_HEIGHT: f32 = 33.0;
const TITLE_HEIGHT: f32 = 29.0;
/// A cell's side padding.
const PADDING: f32 = 10.0;
/// The grid's columns: each at least as wide as its widest cell, sharing
/// the room left over.
const COLUMNS: &str = "minmax(max-content,11fr) minmax(max-content,18fr) \
                       minmax(max-content,9fr) minmax(max-content,8fr) \
                       minmax(max-content,9fr) minmax(max-content,8fr) \
                       minmax(max-content,9fr)";

/// A column of the table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Column {
    Time,
    Market,
    Price,
    Size,
    Value,
    Fee,
    Pnl,
}

impl Column {
    /// Every column, in the table's order.
    const ALL: [Self; 7] = [
        Self::Time,
        Self::Market,
        Self::Price,
        Self::Size,
        Self::Value,
        Self::Fee,
        Self::Pnl,
    ];

    fn title(self) -> &'static str {
        match self {
            Self::Time => "Time",
            Self::Market => "Market",
            Self::Price => "Price",
            Self::Size => "Size",
            Self::Value => "Value",
            Self::Fee => "Fee",
            Self::Pnl => "Closed PnL",
        }
    }

    /// What `row`'s cell of it reads; the time's and the market's are made
    /// of more.
    fn figure(self, row: &Row) -> String {
        match self {
            Self::Time | Self::Market => row.instrument.clone(),
            Self::Price => row.price.clone(),
            Self::Size => row.size.clone(),
            Self::Value => row.value.clone(),
            Self::Fee => row.fee.clone(),
            Self::Pnl => row.pnl.clone(),
        }
    }
}

/// `time_ms` as the clock writes a day and its time (`Nov 14 14:05:09`).
pub(super) fn when(time_ms: f64, prefs: ClockPrefs) -> String {
    let secs = (time_ms / 1000.0).floor() as i64;
    let date = clock::format_date(secs, prefs);
    let day = date.rsplit_once(' ').map_or(date.as_str(), |(day, _)| day);
    format!("{day} {}", clock::format_time(secs, prefs))
}

/// The Trade History of `session`, for the account its tab's settings name.
pub(super) fn history_view(
    session: &TradeHistorySession,
    tab: &TabContext<TradeHistorySettings>,
) -> View {
    let (reports, rows, totals) = (session.reports, session.rows, session.totals);
    // No trades: said once every account has arrived to say it.
    let none_made = computed(move || reports.settled() && rows.with(|rows| rows.0.is_empty()));
    let empty = view()
        .test_id(&format!("{ID}-empty"))
        .px(px(PADDING))
        .py(px(10.0))
        .text_color(theme::TEXT_MUTED)
        .role(AccessibilityRole::Text)
        .style_when(move || !none_made.get(), |s| s.hidden())
        .child(text("No trades"));

    // The pane's room below the totals, which the table fills (and scrolls
    // past sideways).
    let viewport = signal(Size::new(px(0.0), px(0.0)));
    // One selection across the table and the totals, as a page's.
    let content = view().flex_col().child(table(rows)).child(empty);
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
    let whole = view()
        .selectable(true)
        .flex_1()
        .min_h(px(0.0))
        .min_w(px(0.0))
        .flex_col()
        .child(summary(totals))
        .child(area);
    account_frame(tab, &session.portfolio, reports.standing, "trades", whole)
}

/// The totals over the table: what is listed, then what the trades come to.
fn summary(totals: Signal<Totals>) -> View {
    let prefs = clock_prefs();
    let listed = move || {
        totals.with(|totals| {
            let since = totals
                .since_ms
                .map(|since| format!(" since {}", when(since, prefs())))
                .unwrap_or_default();
            if totals.count > totals.listed {
                format!(
                    "Latest {} of {} trades{since}",
                    totals.listed,
                    format::grouped(totals.count as f64, 0)
                )
            } else {
                format!("{} trades{since}", totals.count)
            }
        })
    };
    let stat = |name: &'static str, id: &'static str, read: fn(&Totals) -> String| {
        view()
            .test_id(&format!("{ID}-{id}"))
            .flex_row()
            .items_center()
            .gap(px(6.0))
            .flex_none()
            .child(text(name).select_text().text_color(theme::TEXT_MUTED))
            .child(
                text(move || totals.with(read))
                    .tabular_nums()
                    .select_text()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(move || match id {
                        "closed-pnl" => Tone::color(totals.with(|totals| totals.tone)),
                        _ => theme::TEXT.get(),
                    }),
            )
    };
    view()
        .test_id(&format!("{ID}-totals"))
        .flex_none()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap(px(16.0))
        .px(px(PADDING))
        .py(px(7.0))
        .border_b_1()
        .border_color(theme::BORDER)
        .child(
            text(listed)
                .select_text()
                .test_id(&format!("{ID}-listed"))
                .number_of_lines(1)
                .text_color(theme::TEXT_FAINT)
                .style_when(
                    move || totals.with(|totals| totals.count == 0),
                    |s| s.hidden(),
                ),
        )
        .child(stat("Volume", "volume", |totals| totals.volume.clone()))
        .child(stat("Fees", "fees", |totals| totals.fees.clone()))
        .child(stat("Closed PnL", "closed-pnl", |totals| {
            totals.closed_pnl.clone()
        }))
}

/// How the clock reads now (tracked): the bar's, or the default.
fn clock_prefs() -> impl Fn() -> ClockPrefs + Copy + 'static {
    let clock = use_context::<Clock>();
    move || clock.map(Clock::get).unwrap_or_default()
}

/// The table: the titles, then a row per trade.
fn table(rows: Signal<Shared<Vec<Row>>>) -> View {
    let prefs = clock_prefs();
    let lines = each(
        move || rows.with(|rows| Vec::clone(&rows.0)),
        |row: &Row| row.key.clone(),
        move |row, _| {
            Column::ALL
                .iter()
                .map(|column| cell(*column, row, prefs))
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
        Column::Pnl => cell,
        _ => cell.border_r_1(),
    };
    match column {
        Column::Time | Column::Market => cell,
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
fn cell(
    column: Column,
    row: ReadSignal<Row>,
    prefs: impl Fn() -> ClockPrefs + Copy + 'static,
) -> View {
    let figure = move || {
        text(move || row.with(|row| column.figure(row)))
            .number_of_lines(1)
            .tabular_nums()
    };
    match column {
        Column::Time => boxed(ROW_HEIGHT, column).child(
            text(move || row.with(|row| when(row.time_ms, prefs())))
                .number_of_lines(1)
                .tabular_nums()
                .select_text()
                .text_color(theme::TEXT_MUTED),
        ),
        Column::Market => {
            let side = move || match row.with(|row| row.side) {
                TradeSide::Buy => theme::POSITIVE.get(),
                TradeSide::Sell => theme::NEGATIVE.get(),
            };
            // A row keeps its account: it is in its key.
            let owner = row.with_untracked(|row| row.owner.clone());
            boxed(ROW_HEIGHT, column)
                .gap(px(8.0))
                .children(owner.as_ref().map(mark_view))
                .child(
                    figure()
                        .min_w(px(0.0))
                        .truncate()
                        .select_text()
                        .font_weight(FontWeight::SEMIBOLD),
                )
                .child(
                    text(move || row.with(|row| row.action.clone()))
                        .flex_none()
                        .number_of_lines(1)
                        .text_size(px(10.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .letter_spacing(px(0.6))
                        .uppercase()
                        .text_color(side),
                )
                .child(
                    text(move || row.with(|row| row.liquidity.to_owned()))
                        .flex_none()
                        .number_of_lines(1)
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
