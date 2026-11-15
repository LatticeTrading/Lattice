//! The Funding Rates' view, laid out as the old app's `FundingRates` and
//! its `FundingRates.css` drew it: what could not be read or changed in
//! warning strips, then the table (a row per watched perpetual, titles that
//! stay at the top while the rows scroll), and "Add pair" under the rows.
//!
//! - The table is one grid, so a column is as wide as its widest cell and
//!   the titles line up with the rows. As a browser lays an auto table
//!   out, the room left over is shared between the columns in proportion to
//!   their content (`minmax(max-content, <content width>fr)`): the content
//!   widths are worked out from the text, which is exact enough in the
//!   app's monospace face, and only weigh the spare room, so a cell is
//!   never narrower than what it shows. The × column keeps its 42pt.
//! - A pane narrower than the columns scrolls sideways, and one shorter
//!   than the rows scrolls down under the titles.
//! - A pair's cell opens the market picker to change it (its Clear removes
//!   it), × removes it, and "Add pair" opens the picker to add one. The
//!   picker offers the perpetuals of the exchanges that report funding. A
//!   change that cannot be made says why in a warning strip.
//!
//! A row is made once per market and its cells follow its row's values, so
//! each second's new ages rewrite text and make no view.

use nsh_core::{LineHeight, element::ImageSrc, prelude::*};

use super::{
    session::{Funding, NONE, Row, Sign, Status},
    settings::{FundingRatesSettings, PairKey, Period, Refused, Toggle},
};
use crate::{
    data::ferris::{MarketSelection, StatsFieldName},
    exchanges, theme, ui,
    widgets::{
        TabContext,
        common::{LogoStyle, MarketPicker, Shared, exchange_logo, tag_text},
    },
};

/// A title's height: 18pt of text, 7pt above and below, and its border.
const TITLE_HEIGHT: f32 = 33.0;
/// A row's height, and "Add pair"'s: the old 42pt buttons and their
/// cell's border.
const ROW_HEIGHT: f32 = 43.0;
/// A cell's side padding, and a button's.
const CELL_PADDING: f32 = 8.0;
const BUTTON_PADDING: f32 = 12.0;
/// The × column's width.
const REMOVE_WIDTH: f32 = 42.0;
/// The table's text size, and a tag's.
const TEXT_SIZE: f32 = 12.0;
const TAG_SIZE: f32 = 9.0;
/// How wide a character of the app's face (IBM Plex Mono) is, per point of
/// text size.
const ADVANCE: f32 = 0.6;
/// How the table draws exchange logos.
const LOGO: LogoStyle = LogoStyle {
    size: 14.0,
    color: theme::TEXT,
    initials_size: 7.0,
    boxed: true,
};
/// The gaps in a pair's button, and between an exchange's logo and name
/// (a space of the table's text).
const BUTTON_GAP: f32 = 6.0;
const NAME_GAP: f32 = 7.0;

/// A column of the table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Column {
    Exchange,
    Pair,
    Rate(Period),
    Interval,
    Age,
    Remove,
}

impl Column {
    /// The columns `settings` show, in order: the pair and × always.
    fn shown(settings: &FundingRatesSettings) -> Vec<Self> {
        let mut columns = Vec::new();
        if settings.shows(Toggle::Exchange) {
            columns.push(Self::Exchange);
        }
        columns.push(Self::Pair);
        columns.extend(settings.periods().into_iter().map(Self::Rate));
        if settings.shows(Toggle::Interval) {
            columns.push(Self::Interval);
        }
        if settings.shows(Toggle::Age) {
            columns.push(Self::Age);
        }
        columns.push(Self::Remove);
        columns
    }

    fn title(self) -> &'static str {
        match self {
            Self::Exchange => "Exchange",
            Self::Pair => "Pair",
            Self::Rate(period) => period.label(),
            Self::Interval => "Interval",
            Self::Age => "Age",
            Self::Remove => "X",
        }
    }

    /// What its title says on hover, for a rate.
    fn hint(self) -> Option<&'static str> {
        match self {
            Self::Rate(Period::Year) => {
                Some("API-supplied annualized funding rate (simple APR, not compounded APY)")
            }
            Self::Rate(_) => Some("API-supplied funding percentage for this period"),
            _ => None,
        }
    }

    /// How wide `row`'s cell of it is, borders and padding included.
    fn cell_width(self, row: &Row) -> f32 {
        let text = |text: &str| text.chars().count() as f32 * TEXT_SIZE * ADVANCE;
        let market = &row.market;
        match self {
            Self::Exchange => {
                2.0 * CELL_PADDING
                    + LOGO.size
                    + NAME_GAP
                    + text(exchanges::display_name(&market.exchange))
            }
            Self::Pair => {
                // A tag's letters are spaced 0.4pt apart.
                let tag = |tag: &str| tag.chars().count() as f32 * (TAG_SIZE * ADVANCE + 0.4);
                let detail = market.detail().map_or(0.0, |detail| 4.0 + tag(&detail));
                2.0 * BUTTON_PADDING
                    + LOGO.size
                    + 2.0 * BUTTON_GAP
                    + text(&market.symbol)
                    + tag(market.kind.label())
                    + detail
            }
            Self::Rate(period) => {
                2.0 * CELL_PADDING + text(row.rate(period).map_or(NONE, |rate| &rate.text))
            }
            Self::Interval => 2.0 * CELL_PADDING + text(&row.interval),
            Self::Age => 2.0 * CELL_PADDING + text(&row.age),
            Self::Remove => REMOVE_WIDTH,
        }
    }

    /// Its content's width: its widest cell's, or its title's.
    fn width(self, rows: &[Row]) -> f32 {
        let title = 2.0 * CELL_PADDING + self.title().chars().count() as f32 * TEXT_SIZE * ADVANCE;
        rows.iter()
            .map(|row| self.cell_width(row))
            .fold(title, f32::max)
    }
}

/// The grid's columns: each at least as wide as its content, sharing the
/// room left over in proportion to it, and the × column fixed.
fn tracks(columns: &[Column], rows: &[Row]) -> String {
    columns
        .iter()
        .map(|column| match column {
            Column::Remove => format!("{REMOVE_WIDTH}px"),
            column => format!("minmax(max-content,{:.1}fr)", column.width(rows)),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// What the market picker is open for.
#[derive(Clone, Debug, PartialEq)]
enum Picking {
    /// A market to add.
    Add,
    /// A market to put in place of the one this names.
    Change(PairKey),
}

/// What the table's buttons and the picker do: change the tab's markets,
/// open and close the picker, and say why a change was not made.
#[derive(Clone)]
struct Controls {
    tab: TabContext<FundingRatesSettings>,
    picking: Signal<Option<Picking>>,
    /// Why the last change was not made, until the next one.
    refusal: Signal<Option<&'static str>>,
}

impl Controls {
    /// Opens the picker for `picking`.
    fn open(&self, picking: Picking) {
        self.refusal.set(None);
        self.picking.set(Some(picking));
    }

    /// Saves a change to the markets, or says why it was refused.
    fn change(
        &self,
        change: impl FnOnce(FundingRatesSettings) -> Result<FundingRatesSettings, Refused>,
    ) {
        match change(self.tab.settings.get_untracked()) {
            Ok(settings) => {
                self.tab.save(&settings);
                self.refusal.set(None);
            }
            Err(refused) => self.refusal.set(Some(refused.message())),
        }
    }

    /// Stops watching the market `key` names, closing the picker.
    fn remove(&self, key: &PairKey) {
        self.change(|settings| settings.without(key));
        self.picking.set(None);
    }
}

pub(super) fn funding_view(funding: &Funding, tab: &TabContext<FundingRatesSettings>) -> View {
    let saved = tab.settings;
    let controls = Controls {
        tab: tab.clone(),
        picking: signal(None),
        refusal: signal(None),
    };
    let (picking, refusal) = (controls.picking, controls.refusal);
    // A tab that is not shown has no picker open (the old app closed it).
    let shown = tab.shown;
    effect(move || {
        if !shown.get() && picking.with_untracked(Option::is_some) {
            picking.set(None);
        }
    });

    let rejected = computed(move || saved.with(FundingRatesSettings::rejected));
    let unread = warning(move || {
        let count = rejected.get();
        (count > 0).then(|| {
            format!("Saved-selection warning: {count} invalid saved selection(s) excluded.")
        })
    });
    let refused = warning(move || refusal.get().map(str::to_owned));

    let status = funding.status;
    let loading = note(move || {
        (status.get() == Status::Loading).then(|| "Loading funding rates…".to_owned())
    });
    let failed = warning(move || match status.get() {
        Status::Failed(message) => Some(message),
        _ => None,
    });
    let unsupported = note(move || {
        (status.get() == Status::Unsupported).then(|| "No supported funding markets".to_owned())
    });

    // The columns shown, and whether there is a row to title.
    let shape = computed(move || {
        saved.with(|settings| (Column::shown(settings), !settings.markets().is_empty()))
    });
    let rows = funding.rows;
    let table = {
        let controls = controls.clone();
        dynamic(move || {
            let (columns, titled) = shape.get();
            table(&columns, titled, rows, &controls)
        })
    };
    let picker = {
        let controls = controls.clone();
        dynamic(move || picking.get().map(|target| picker(target, &controls)))
    };

    // The pane's room, which the table fills (and scrolls past sideways).
    let viewport = signal(Size::new(px(0.0), px(0.0)));
    let content = view()
        .flex_col()
        .child(loading)
        .child(failed)
        .child(unsupported)
        .child(table);
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

    view()
        .test_id("funding-rates")
        .flex_1()
        .min_h(px(0.0))
        .min_w(px(0.0))
        .flex_col()
        .overflow_hidden()
        .bg(theme::BACKGROUND)
        .text_color(theme::TEXT)
        .text_size(px(TEXT_SIZE))
        .child(unread)
        .child(refused)
        .child(area)
        .child(picker)
}

/// A strip above the table saying `content` in small muted text, with a
/// line under it; hidden while there is nothing to say.
fn note(content: impl Fn() -> Option<String> + 'static) -> View {
    let content = computed(content);
    view()
        .flex_none()
        .px(px(CELL_PADDING))
        .py(px(5.0))
        .border_b_1()
        .border_color(theme::BORDER)
        .text_size(px(11.0))
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
}

/// The table: the titles (while there is a row), a row per watched market,
/// then "Add pair" across every column.
fn table(
    columns: &[Column],
    titled: bool,
    rows: Signal<Shared<Vec<Row>>>,
    controls: &Controls,
) -> View {
    let last = columns.len() - 1;
    let template = {
        let columns = columns.to_vec();
        computed(move || rows.with(|rows| tracks(&columns, &rows.0)))
    };
    let titles = columns
        .iter()
        .enumerate()
        .filter(|_| titled)
        .map(|(at, column)| title(*column, at == last));
    let lines = {
        let (columns, controls) = (columns.to_vec(), controls.clone());
        each(
            move || rows.with(|rows| Vec::clone(&rows.0)),
            |row: &Row| row.key.clone(),
            move |row, _| {
                columns
                    .iter()
                    .enumerate()
                    .map(|(at, column)| cell(*column, at == last, row, &controls))
                    .collect::<Vec<_>>()
            },
        )
    };
    view()
        .test_id("funding-rates-table")
        .w_full()
        .grid()
        .grid_template_columns(move || template.get())
        .children(titles)
        .child(lines)
        .child(add_pair(controls))
}

/// What every cell of the table is: as high as its row, its content in the
/// middle of its height, a line under it and, unless it is the last of its
/// row, one at its right.
fn boxed(height: f32, last: bool) -> View {
    let cell = view()
        .h(px(height))
        .flex()
        .items_center()
        .border_b_1()
        .border_color(theme::BORDER);
    if last { cell } else { cell.border_r_1() }
}

/// A column's title, which stays at the top while the rows scroll.
fn title(column: Column, last: bool) -> View {
    let name = text(column.title())
        .number_of_lines(1)
        .font_weight(FontWeight::MEDIUM)
        .text_color(theme::TEXT_FAINT);
    let cell = boxed(TITLE_HEIGHT, last)
        .sticky()
        .top(px(0.0))
        .z_index(1)
        .px(px(CELL_PADDING))
        .bg(theme::SURFACE)
        .test_id(&format!("funding-rates-title-{}", column.title()))
        .child(name);
    let cell = if column == Column::Remove {
        cell.justify_center()
    } else {
        cell
    };
    match column.hint() {
        Some(hint) => cell.tooltip(ui::hint(move || hint.to_owned())),
        None => cell,
    }
}

/// `row`'s cell of `column`.
fn cell(column: Column, last: bool, row: ReadSignal<Row>, controls: &Controls) -> View {
    let cell = boxed(ROW_HEIGHT, last);
    // A row keeps its market for as long as it is watched.
    let market = row.with_untracked(|row| row.market.clone());
    let exchange = exchanges::display_name(&market.exchange).to_owned();
    let figure = |content: Text| content.number_of_lines(1).tabular_nums();
    match column {
        Column::Exchange => cell
            .px(px(CELL_PADDING))
            .gap(px(NAME_GAP))
            .child(exchange_logo(&market.exchange, LOGO))
            .child(text(exchange).number_of_lines(1)),
        Column::Pair => {
            let (key, controls) = (PairKey::of(&market), controls.clone());
            pair(cell, &market, &exchange)
                .on_press(move || controls.open(Picking::Change(key.clone())))
        }
        Column::Rate(period) => {
            let rate = move |row: &Row| row.rate(period).cloned();
            let tone = move || match row.with(|row| rate(row).map(|rate| rate.sign)) {
                Some(Sign::Positive) => theme::POSITIVE.get(),
                Some(Sign::Negative) => theme::NEGATIVE.get(),
                Some(Sign::Zero) | None => theme::TEXT.get(),
            };
            let missing = computed(move || row.with(|row| rate(row).is_none()));
            cell.relative()
                .px(px(CELL_PADDING))
                .child(
                    figure(text(move || {
                        row.with(|row| rate(row).map_or_else(|| NONE.to_owned(), |rate| rate.text))
                    }))
                    .select_text()
                    .text_color(tone),
                )
                // Over the whole cell while it has no rate, to say so.
                .child(dynamic(move || {
                    missing.get().then(|| {
                        view().absolute().inset_0().tooltip(ui::hint(|| {
                            "Funding percentage unavailable for this period".to_owned()
                        }))
                    })
                }))
        }
        Column::Interval => cell
            .px(px(CELL_PADDING))
            .child(figure(text(move || row.with(|row| row.interval.clone())))),
        Column::Age => cell
            .px(px(CELL_PADDING))
            .tooltip(ui::hint(move || {
                row.with(|row| format!("Received: {}", row.received))
            }))
            .child(figure(text(move || row.with(|row| row.age.clone())))),
        Column::Remove => {
            let (key, controls) = (PairKey::of(&market), controls.clone());
            cell.justify_center()
                .text_color(theme::TEXT_FAINT)
                .role(AccessibilityRole::Button)
                .label(format!("Remove pair: {exchange} {}", market.symbol))
                .transition_colors()
                .hover(|s| s.bg(theme::ACCENT.opacity(0.06)).text_color(theme::DANGER))
                .on_press(move || controls.remove(&key))
                .child(
                    svg(ImageSrc::svg_markup(include_str!(
                        "../../../assets/icons/remove.svg"
                    )))
                    .size(px(12.0)),
                )
        }
    }
}

/// A pair's cell, a button: its exchange's logo, its pair and its tags
/// (bare text here, as the old table drew them).
fn pair(cell: View, market: &MarketSelection, exchange: &str) -> View {
    let tags = view()
        .flex_none()
        .flex_row()
        .items_center()
        .gap(px(4.0))
        .child(tag_text(text(market.kind.label())).text_color(theme::TEXT_FAINT))
        .children(
            market
                .detail()
                .map(|detail| tag_text(text(detail)).text_color(theme::WARNING)),
        );
    cell.px(px(BUTTON_PADDING))
        .gap(px(BUTTON_GAP))
        .role(AccessibilityRole::Button)
        .label(format!("Change pair: {exchange} {}", market.symbol))
        .hover(|s| s.bg(theme::ACCENT.opacity(0.06)))
        .child(exchange_logo(&market.exchange, LOGO))
        .child(text(market.symbol.clone()).number_of_lines(1))
        .child(tags)
}

/// "Add pair", under the rows and across every column.
fn add_pair(controls: &Controls) -> View {
    let controls = controls.clone();
    boxed(ROW_HEIGHT, true)
        .col_span_full()
        .justify_center()
        .gap(px(BUTTON_GAP))
        .px(px(BUTTON_PADDING))
        .text_color(theme::TEXT_MUTED)
        .role(AccessibilityRole::Button)
        .label("Add pair")
        .test_id("funding-rates-add-pair")
        .hover(|s| s.bg(theme::ACCENT.opacity(0.06)).text_color(theme::TEXT))
        .on_press(move || controls.open(Picking::Add))
        .child(
            svg(ImageSrc::svg_markup(include_str!(
                "../../../assets/icons/plus.svg"
            )))
            .size(px(14.0)),
        )
        .child(text("Add pair").number_of_lines(1))
}

/// The market picker, open for `target`: a pick is saved (or refused, which
/// the widget then says), and Clear removes the market being changed.
fn picker(target: Picking, controls: &Controls) -> View {
    let current = match &target {
        Picking::Add => None,
        Picking::Change(key) => controls
            .tab
            .settings
            .with_untracked(|settings| settings.market(key).cloned()),
    };
    let pick = {
        let (target, controls) = (target.clone(), controls.clone());
        move |picked: MarketSelection| {
            controls.change(|settings| match &target {
                Picking::Add => settings.adding(picked),
                Picking::Change(replaced) => settings.replacing(replaced, picked),
            });
        }
    };
    let mut picker = MarketPicker::new(pick)
        .with_statistics(StatsFieldName::Funding)
        .current(current);
    if let Picking::Change(key) = target {
        let controls = controls.clone();
        picker = picker.on_clear("Remove pair", move || controls.remove(&key));
    }
    let picking = controls.picking;
    picker.view(move || picking.set(None))
}
