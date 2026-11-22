//! The Positions' view: the frame every account widget has (the accounts'
//! strip, what could not be read, the form while no account is set:
//! [`account_frame`]) around its table.
//!
//! - The table is one grid, a row per open position under titles that stay
//!   at the top while the rows scroll: the market (whose it is, while
//!   several accounts show: its exchange's logo, named in its tooltip; its
//!   instrument, which way the position faces, green long and red short,
//!   and its leverage setting), its size, what it is worth, its entry and mark prices, its
//!   unrealized profit and loss with its return on equity (green above
//!   zero, red below), its liquidation price and the margin it holds. The
//!   price, the worth and the profit move with the price as the exchange
//!   says it, between two reports of the account: the live price (the
//!   column is then titled `Price`) or the exchange's mark (`Mark`), as
//!   the settings say.
//! - Over the table, while there is a row: how many positions show, what
//!   they are worth together and what they have made, which move with the
//!   rows.
//! - A column is at least as wide as its figures and they share the room
//!   left over; a pane narrower than the columns scrolls sideways, and one
//!   shorter than the rows scrolls down under the titles.
//! - Accounts with nothing open say so under the titles.
//! - While a row is the connected wallet's own, the table has two more
//!   columns. The first says the take profit and the stop loss that wait
//!   on the position (`65000.0 / 58000.0`, a dash for one it has none of).
//!   The last is [`CLOSE_WIDTH`] wide whatever it holds: `Reduce` and
//!   `Close` as two halves, which turn the cell into the size to close
//!   (half the position, or all of it, to be typed over), the cell that
//!   sends it at the market's price (`Sell` for a long, `Buy` for a
//!   short, in the book's colors) and × which lets it go. Nothing is sent
//!   by the first press. Rows that are somebody else's have both cells
//!   empty.
//! - What closing one came to, or why nothing was sent, is said in a strip
//!   over the table, with × to take it away.
//!
//! A row is made once per position and its cells follow its row's values,
//! so a new report rewrites text and makes no view.

use std::rc::Rc;

use nsh_core::prelude::*;

use super::{
    session::{PositionsSession, Row, Take, Totals, closing_order},
    settings::PositionsSettings,
};
use crate::{
    data::{portfolio::Direction, prices::PriceKind, trading::Side},
    theme,
    ui::{self, Tone as Reads},
    widgets::{
        TabContext,
        common::{
            access::{Said, strip},
            accounts::{Tone, account_frame, mark_view},
            orders::order_line,
        },
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
/// The width of the column that closes a row's position: the same for its
/// two buttons and for the size, the cell that sends it and ×, so starting
/// to close one moves nothing.
const CLOSE_WIDTH: f32 = 168.0;
/// The widths of the cell that sends and of × beside it; the size has the
/// rest.
const SEND_WIDTH: f32 = 48.0;
const CANCEL_WIDTH: f32 = 28.0;

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
    /// The exits that wait on the row's position, while a row is the
    /// connected wallet's own.
    Exits,
    /// What closes the row's position, while one of the rows can be.
    Close,
}

impl Column {
    /// Whether it is one of the two a table has only while a row is the
    /// connected wallet's own.
    const fn own(self) -> bool {
        matches!(self, Self::Exits | Self::Close)
    }

    /// Every column, in the table's order.
    const ALL: [Self; 10] = [
        Self::Market,
        Self::Size,
        Self::Value,
        Self::Entry,
        Self::Mark,
        Self::Pnl,
        Self::Liquidation,
        Self::Margin,
        Self::Exits,
        Self::Close,
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
            Self::Exits => "TP / SL",
            Self::Close => "Close at market",
        }
    }

    /// What `row`'s cell of it reads; the market's cell is made of more,
    /// and the closing one of controls.
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
            Self::Exits => row.exits.clone().unwrap_or_default(),
            Self::Close => String::new(),
        }
    }
}

/// The grid's columns, with the exits' and the one that closes a position
/// while a row is `closable`.
fn template(closable: bool) -> String {
    if closable {
        format!("{COLUMNS} minmax(max-content,11fr) {CLOSE_WIDTH}px")
    } else {
        COLUMNS.to_owned()
    }
}

/// The Positions of `session`, for the account its tab's settings name.
pub(super) fn positions_view(
    session: &Rc<PositionsSession>,
    tab: &TabContext<PositionsSettings>,
) -> View {
    let (reports, rows) = (session.reports, session.rows);
    // Nothing open: said once every account has arrived to say it.
    let none_open = computed(move || reports.settled() && rows.with(|rows| rows.0.is_empty()));
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
        .child(table(session))
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
    let content = view()
        .flex_1()
        .min_h(px(0.0))
        .min_w(px(0.0))
        .flex_col()
        .child(said_strip(session.said))
        .child(totals_bar(session.totals))
        .child(area);

    account_frame(
        tab,
        &session.portfolio,
        reports.standing,
        "positions",
        content,
    )
}

/// What closing a position came to, or why nothing was sent, over the
/// table, with × which takes it away; hidden with nothing to say.
fn said_strip(said: Signal<Option<Said>>) -> View {
    let dismiss = ui::button("×", Reads::Quiet)
        .w(px(CANCEL_WIDTH))
        .flex_none()
        .justify_center()
        .border_l_1()
        .border_color(theme::BORDER)
        .label("Dismiss")
        .test_id(&format!("{ID}-said-dismiss"))
        .on_press(move || said.set(None));
    view()
        .flex_none()
        .flex_row()
        .border_b_1()
        .border_color(theme::BORDER)
        .test_id(&format!("{ID}-said"))
        .style_when(move || said.with(Option::is_none), |s| s.hidden())
        .child(view().flex_1().min_w(px(0.0)).child(dynamic(move || {
            said.get().map(|said| strip(said.text, said.voice))
        })))
        .child(dismiss)
}

/// What the positions come to together, over the table: how many there
/// are, what they are worth and what they have made (toned), which move
/// with the prices as the rows do. Hidden with none.
fn totals_bar(totals: Signal<Totals>) -> View {
    let figure = |name: &'static str, id: &str, read: fn(&Totals) -> String| {
        view()
            .h_full()
            .flex_none()
            .flex_row()
            .items_center()
            .gap(px(8.0))
            .px(px(PADDING))
            .border_r_1()
            .border_color(theme::BORDER)
            .test_id(&format!("{ID}-total-{id}"))
            .child(
                text(name)
                    .number_of_lines(1)
                    .text_size(px(10.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .letter_spacing(px(0.8))
                    .uppercase()
                    .text_color(theme::TEXT_FAINT),
            )
            .child(
                text(move || totals.with(read))
                    .number_of_lines(1)
                    .tabular_nums()
                    .font_weight(FontWeight::MEDIUM),
            )
    };
    let tone = move || Tone::color(totals.with(|totals| totals.tone));
    view()
        .h(px(TITLE_HEIGHT))
        .flex_none()
        .flex_row()
        .border_b_1()
        .border_color(theme::BORDER)
        .test_id(&format!("{ID}-totals"))
        .style_when(
            move || totals.with(|totals| totals.count == 0),
            |s| s.hidden(),
        )
        .child(figure("Positions", "count", |totals| {
            totals.count.to_string()
        }))
        .child(figure("Total value", "value", |totals| {
            totals.value.clone()
        }))
        .child(figure("Total PnL", "pnl", |totals| totals.pnl.clone()).text_color(tone))
}

/// The table: the titles, then a row per position.
fn table(session: &Rc<PositionsSession>) -> View {
    let (rows, pricing) = (session.rows, session.pricing);
    // The last column is there while there is a row to close.
    let closable = computed(move || rows.with(|rows| rows.0.iter().any(|row| row.close.is_some())));
    let lines = {
        let session = session.clone();
        each(
            move || rows.with(|rows| Vec::clone(&rows.0)),
            |row: &Row| row.key.clone(),
            move |row, _| {
                Column::ALL
                    .iter()
                    .map(|column| match column {
                        Column::Close => close_cell(&session, row, closable),
                        Column::Exits => {
                            cell(*column, row).style_when(move || !closable.get(), |s| s.hidden())
                        }
                        _ => cell(*column, row),
                    })
                    .collect::<Vec<_>>()
            },
        )
    };
    view()
        .test_id(&format!("{ID}-table"))
        .w_full()
        .grid()
        .grid_template_columns(move || template(closable.get()))
        .font_weight(FontWeight::MEDIUM)
        .children(Column::ALL.map(|column| title(column, closable, pricing)))
        .child(lines)
}

/// What every cell of the table is: as high as its row, its content in the
/// middle of its height, a line under it and one between it and the next
/// (that of the two columns a table has only for the wallet's own rows is
/// each one's own left edge, so the table ends without one when they are
/// not there).
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
        Column::Exits | Column::Close => cell.border_l_1(),
        _ => cell.border_r_1(),
    };
    match column {
        Column::Market | Column::Close => cell,
        _ => cell.justify_end(),
    }
}

/// A column's title, which stays at the top while the rows scroll; those of
/// the wallet's own two columns are there while a row is `closable`, and
/// the price's is `Price` while the rows are valued at the live price and
/// `Mark` while at the exchange's mark (`pricing`).
fn title(column: Column, closable: Memo<bool>, pricing: Memo<PriceKind>) -> View {
    // The price's column is named for the price the rows are valued at.
    let named = move || match (column, pricing.get()) {
        (Column::Mark, PriceKind::Live) => "Price".to_owned(),
        _ => column.title().to_owned(),
    };
    let title = boxed(TITLE_HEIGHT, column)
        .sticky()
        .top(px(0.0))
        .z_index(1)
        .bg(theme::SURFACE)
        .child(
            text(named)
                .number_of_lines(1)
                .text_size(px(10.0))
                .font_weight(FontWeight::SEMIBOLD)
                .letter_spacing(px(0.8))
                .uppercase()
                .text_color(theme::TEXT_FAINT),
        );
    if column.own() {
        title.style_when(move || !closable.get(), |s| s.hidden())
    } else {
        title
    }
}

/// `row`'s cell of `column`, one of the columns that say something.
fn cell(column: Column, row: ReadSignal<Row>) -> View {
    let figure = move || {
        text(move || row.with(|row| column.figure(row)))
            .number_of_lines(1)
            .tabular_nums()
    };
    match column {
        Column::Market => {
            // A row keeps its account, its instrument and its direction:
            // they are its key.
            let (direction, owner) = row.with_untracked(|row| (row.direction, row.owner.clone()));
            let side = match direction {
                Direction::Long => theme::POSITIVE,
                Direction::Short => theme::NEGATIVE,
            };
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

/// `row`'s cell of the closing column, there while a row of the table is
/// `closable`: empty for a position that is not the wallet's to close;
/// else `Reduce` and `Close` as two halves, and in their place, while its
/// position is the one being closed, what sends the order ([`editor`]).
fn close_cell(session: &Rc<PositionsSession>, row: ReadSignal<Row>, closable: Memo<bool>) -> View {
    // A row keeps its key, its instrument and its direction.
    let (key, instrument, direction) =
        row.with_untracked(|row| (row.key.clone(), row.instrument.clone(), row.direction));
    let closing = session.closing;
    let own = computed(move || row.with(|row| row.close.is_some()));
    let open = {
        let key = key.clone();
        computed(move || closing.with(|closing| closing.as_deref() == Some(&*key)))
    };
    let facing = direction.label().to_lowercase();
    let starts = |label: &'static str, take: Take| {
        let (session, key) = (session.clone(), key.clone());
        ui::button(label, Reads::Normal)
            .flex_1()
            .justify_center()
            .label(format!("{label} the {instrument} {facing}"))
            .on_press(move || session.start(&key, take))
    };
    let buttons = view()
        .flex_1()
        .h_full()
        .min_w(px(0.0))
        .flex_row()
        .style_when(move || !own.get() || open.get(), |s| s.hidden())
        .child(starts("Reduce", Take::Half))
        .child(
            starts("Close", Take::Whole)
                .border_l_1()
                .border_color(theme::BORDER),
        );
    let editing = {
        let session = session.clone();
        when(open, move || editor(&session, row))
    };
    view()
        .h(px(ROW_HEIGHT))
        .flex_row()
        .border_b_1()
        .border_l_1()
        .border_color(theme::BORDER)
        .test_id(&format!("{ID}-close"))
        .style_when(move || !closable.get(), |s| s.hidden())
        .child(buttons)
        .child(editing)
}

/// What the closing cell of the row being closed holds: how much to close
/// (a field, which takes the keyboard), the cell that sends the order (it
/// says the side, and in its label and its tooltip the whole order), and
/// × which lets it go.
fn editor(session: &Rc<PositionsSession>, row: ReadSignal<Row>) -> View {
    let (amount, sending) = (session.amount, session.sending);
    let (instrument, direction) = row.with_untracked(|row| (row.instrument.clone(), row.direction));
    let side = Side::closing(direction);
    let color = match side {
        Side::Buy => theme::BID,
        Side::Sell => theme::ASK,
    };
    let field = {
        let session = session.clone();
        ui::text_field(amount)
            .flex_1()
            .placeholder("Size")
            .label(format!("Size of {instrument} to close"))
            .keyboard_type(KeyboardType::Decimal)
            .auto_focus(true)
            .test_id(&format!("{ID}-close-size"))
            .on_submit(move |_: &str| session.send())
    };
    // What sending would send, for the size typed so far.
    let says = move || match row.with(|row| closing_order(row, &amount.get())) {
        Ok(order) => order_line(&order),
        Err(_) => format!("{} {instrument} at market", side.label()),
    };
    let send = {
        let session = session.clone();
        ui::button(side.label(), Reads::Normal)
            .w(px(SEND_WIDTH))
            .flex_none()
            .justify_center()
            .border_l_1()
            .border_color(theme::BORDER)
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(color)
            .label(says.clone())
            .tooltip(ui::hint(says))
            .test_id(&format!("{ID}-close-send"))
            .disabled(move || sending.get())
            .on_press(move || session.send())
    };
    let cancel = {
        let session = session.clone();
        ui::button("×", Reads::Quiet)
            .w(px(CANCEL_WIDTH))
            .flex_none()
            .justify_center()
            .border_l_1()
            .border_color(theme::BORDER)
            .label("Do not close")
            .test_id(&format!("{ID}-close-cancel"))
            .on_press(move || session.cancel())
    };
    view()
        .flex_1()
        .h_full()
        .min_w(px(0.0))
        .flex_row()
        .child(field)
        .child(send)
        .child(cancel)
}
