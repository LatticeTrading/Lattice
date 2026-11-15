//! The Price Delta Monitor's view, laid out as the old app's
//! `PriceDeltaMonitor` and its `PriceDeltaMonitor.css` drew it:
//!
//! - the base: its exchange, pair and tags, a button that opens the market
//!   picker to change it, and its price in large figures at the right;
//! - the table: a row per compared market (the market, its price, and its
//!   difference from the base's as a figure and as a percentage, green
//!   above the base and red below it), under titles that stay at the top
//!   while the rows scroll, then "Add comparison market". A pane narrower
//!   than the columns scrolls sideways;
//! - a row's market opens the picker to change it (its Clear removes it),
//!   × removes it, and "Add comparison market" opens the picker to add one.
//!   The picker offers the markets whose exchange serves a book both ways,
//!   as the old one did: the prices are read from books.
//!
//! What is not the old widget's:
//!
//! - a market's tags are the segments the settings panels' market box and
//!   the Everything Bar's results draw ([`tag_segment`]): flush with the
//!   row, a divider before each, the type last so it lines up down the
//!   rows; the old widget drew each in a box of its own;
//! - there is no "Indicative book" tag beside a market.
//!
//! A row is made once per market and its cells follow its row's values, so
//! a new price rewrites text and makes no view.

use nsh_core::{element::ImageSrc, prelude::*};

use super::{
    session::{Monitor, Price, Row, Tone, price_text},
    settings::PriceDeltaSettings,
};
use crate::{
    data::ferris::{Channel, MarketKey, MarketSelection, Operation, Requirement},
    exchanges, theme, ui,
    widgets::{
        TabContext,
        common::{LogoStyle, MarketPicker, Shared, exchange_logo, tag_segment},
    },
};

/// What its test ids start with: the old app's widget id.
const ID: &str = "price-delta-monitor";
/// A row's height, the base's and "Add comparison market"'s: the old 40pt
/// and the line under it.
const ROW_HEIGHT: f32 = 41.0;
/// A title's height: the old 28pt and its line.
const TITLE_HEIGHT: f32 = 29.0;
/// A cell's side padding.
const PADDING: f32 = 12.0;
/// The × column's width, and the "+" box's.
const REMOVE_WIDTH: f32 = 40.0;
/// A type tag's segment: as wide as the longest type, so the types line up
/// down the rows (the Everything Bar's results' width for it).
const TYPE_WIDTH: f32 = 52.0;
/// The table's text size.
const TEXT_SIZE: f32 = 12.0;
/// The grid's columns (the old `grid-template-columns`): each at least
/// wide enough for its content, sharing the room left over.
const COLUMNS: &str =
    "minmax(190px,1.6fr) minmax(110px,1fr) minmax(100px,0.9fr) minmax(100px,0.9fr) 40px";
/// How the widget draws exchange logos.
const LOGO: LogoStyle = LogoStyle {
    size: 14.0,
    color: theme::TEXT,
    initials_size: 7.0,
    boxed: true,
};

/// A column of the table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Column {
    Market,
    Price,
    Difference,
    Percent,
    Remove,
}

impl Column {
    /// Every column, in the table's order.
    const ALL: [Self; 5] = [
        Self::Market,
        Self::Price,
        Self::Difference,
        Self::Percent,
        Self::Remove,
    ];

    fn title(self) -> &'static str {
        match self {
            Self::Market => "Market",
            Self::Price => "Price",
            Self::Difference => "Diff",
            Self::Percent => "Diff %",
            Self::Remove => "",
        }
    }
}

/// What the market picker is open for.
#[derive(Clone, Debug, PartialEq)]
enum Picking {
    /// Another base.
    Base,
    /// A market to compare.
    Add,
    /// A market to put in place of the compared one this names.
    Change(MarketKey),
}

/// What the widget's buttons and the picker do: change the tab's markets,
/// and open and close the picker.
#[derive(Clone)]
struct Controls {
    tab: TabContext<PriceDeltaSettings>,
    picking: Signal<Option<Picking>>,
}

impl Controls {
    /// Opens the picker for `picking`.
    fn open(&self, picking: Picking) {
        self.picking.set(Some(picking));
    }

    /// Saves a change to the markets (one that changes nothing saves
    /// nothing).
    fn change(&self, change: impl FnOnce(PriceDeltaSettings) -> PriceDeltaSettings) {
        self.tab.save(&change(self.tab.settings.get_untracked()));
    }

    /// Stops comparing the market `key` names, closing the picker.
    fn remove(&self, key: &MarketKey) {
        self.change(|settings| settings.without(key));
        self.picking.set(None);
    }
}

pub(super) fn monitor_view(monitor: &Monitor, tab: &TabContext<PriceDeltaSettings>) -> View {
    let controls = Controls {
        tab: tab.clone(),
        picking: signal(None),
    };
    let picking = controls.picking;
    // A tab that is not shown has no picker open (the old app closed it).
    let shown = tab.shown;
    effect(move || {
        if !shown.get() && picking.with_untracked(Option::is_some) {
            picking.set(None);
        }
    });
    let picker = {
        let controls = controls.clone();
        dynamic(move || picking.get().map(|target| picker(target, &controls)))
    };

    // The pane's room below the base, which the table fills (and scrolls
    // past sideways).
    let viewport = signal(Size::new(px(0.0), px(0.0)));
    let body = view()
        .flex_none()
        .min_w(move || px(viewport.get().width.0))
        .h(move || px(viewport.get().height.0))
        .child(scroll().size_full().child(table(monitor.rows, &controls)));
    let area = view()
        .flex_1()
        .min_h(px(0.0))
        .min_w(px(0.0))
        .on_layout(move |bounds: &Bounds<Points>| viewport.set(bounds.size))
        .child(scroll().horizontal().absolute().inset_0().child(body));

    view()
        .test_id(ID)
        .flex_1()
        .min_h(px(0.0))
        .min_w(px(0.0))
        .flex_col()
        .overflow_hidden()
        .bg(theme::BACKGROUND)
        .text_color(theme::TEXT)
        .text_size(px(TEXT_SIZE))
        .font_weight(FontWeight::MEDIUM)
        .child(base(tab.settings, monitor.base, &controls))
        .child(area)
        .child(picker)
}

/// A line under what it is as high as and, unless it is the last of its
/// row, one at its right: what every cell of the widget is.
fn lined(height: f32, last: bool) -> View {
    let cell = view()
        .h(px(height))
        .border_b_1()
        .border_color(theme::BORDER);
    if last { cell } else { cell.border_r_1() }
}

/// A [`lined`] cell with its content in the middle of its height.
fn boxed(height: f32, last: bool) -> View {
    lined(height, last).flex().items_center()
}

/// `market`'s tags after what `row` holds, as segments: its detail when
/// it has one (`USDT-M`, an expiry), then its type.
fn tagged(row: View, market: &MarketSelection) -> View {
    row.children(
        market
            .detail()
            .map(|detail| tag_segment(detail, theme::WARNING)),
    )
    .child(
        tag_segment(market.kind.label().to_owned(), theme::TEXT_FAINT)
            .w(px(TYPE_WIDTH))
            .justify_center(),
    )
}

/// The base, over the table: the button that changes it, and its price.
fn base(saved: Memo<PriceDeltaSettings>, price: Signal<Price>, controls: &Controls) -> View {
    let market = computed(move || saved.with(PriceDeltaSettings::base));
    let selector = {
        let controls = controls.clone();
        dynamic(move || {
            let controls = controls.clone();
            market
                .with(base_selector)
                .on_press(move || controls.open(Picking::Base))
        })
    };
    let figure = text(move || match price.get() {
        Price::At(price) => price_text(price),
        Price::Waiting => "Waiting for live price".to_owned(),
        Price::Unavailable => "Unavailable".to_owned(),
    })
    .number_of_lines(1)
    .tabular_nums()
    .text_size(px(18.0))
    .font_weight(FontWeight::BOLD);
    view()
        .flex_none()
        .flex_row()
        .bg(theme::SURFACE)
        .child(selector)
        .child(
            boxed(ROW_HEIGHT, true)
                .flex_1()
                .min_w(px(0.0))
                .justify_end()
                .px(px(PADDING))
                .overflow_hidden()
                .test_id(&format!("{ID}-base-price"))
                .child(figure),
        )
}

/// The base's button: its exchange's logo and name, its pair and its tags.
fn base_selector(market: &MarketSelection) -> View {
    let exchange = exchanges::display_name(&market.exchange);
    let name = view()
        .flex_none()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .px(px(PADDING))
        .child(exchange_logo(&market.exchange, LOGO))
        .child(text(exchange.to_owned()).number_of_lines(1))
        .child(
            text(market.symbol.clone())
                .number_of_lines(1)
                .font_weight(FontWeight::SEMIBOLD),
        );
    tagged(
        lined(ROW_HEIGHT, false).flex_none().flex_row().child(name),
        market,
    )
    .role(AccessibilityRole::Button)
    .label(format!(
        "Change base market: {exchange} {} {}",
        market.symbol,
        market.tags()
    ))
    .test_id(&format!("{ID}-base"))
    .tooltip(ui::hint(|| "Change base market".to_owned()))
    .hover(|s| s.bg(theme::ACCENT.opacity(0.06)))
}

/// The table: the titles, a row per compared market, then "Add comparison
/// market" across every column.
fn table(rows: Signal<Shared<Vec<Row>>>, controls: &Controls) -> View {
    let lines = {
        let controls = controls.clone();
        each(
            move || rows.with(|rows| Vec::clone(&rows.0)),
            |row: &Row| row.key.clone(),
            move |row, _| {
                Column::ALL
                    .iter()
                    .map(|column| cell(*column, row, &controls))
                    .collect::<Vec<_>>()
            },
        )
    };
    view()
        .test_id(&format!("{ID}-table"))
        .w_full()
        .grid()
        .grid_template_columns(COLUMNS)
        .children(Column::ALL.map(title))
        .child(lines)
        .child(add_comparison(controls))
}

/// A column's title, which stays at the top while the rows scroll.
fn title(column: Column) -> View {
    boxed(TITLE_HEIGHT, column == Column::Remove)
        .sticky()
        .top(px(0.0))
        .z_index(1)
        .px(px(PADDING))
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
fn cell(column: Column, row: ReadSignal<Row>, controls: &Controls) -> View {
    // A row keeps its market for as long as it is compared.
    let (key, market) = row.with_untracked(|row| (row.key.clone(), row.market.clone()));
    let exchange = exchanges::display_name(&market.exchange).to_owned();
    let figure = |content: Text| content.number_of_lines(1).tabular_nums();
    match column {
        Column::Market => {
            let controls = controls.clone();
            let hint = format!("Change comparison market: {exchange} {}", market.symbol);
            let unavailable = text("Unavailable")
                .flex_none()
                .number_of_lines(1)
                .text_color(theme::TEXT_FAINT)
                .style_when(
                    move || row.with(|row| row.price != Price::Unavailable),
                    |s| s.hidden(),
                );
            let name = view()
                .flex_1()
                .min_w(px(0.0))
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .px(px(PADDING))
                .child(exchange_logo(&market.exchange, LOGO))
                .child(
                    text(market.symbol.clone())
                        .min_w(px(0.0))
                        .truncate()
                        .tabular_nums(),
                )
                .child(unavailable);
            tagged(lined(ROW_HEIGHT, false).flex_row().child(name), &market)
                .role(AccessibilityRole::Button)
                .label(format!("{hint} {}", market.tags()))
                .tooltip(ui::hint(move || hint.clone()))
                .hover(|s| s.bg(theme::ACCENT.opacity(0.06)))
                .on_press(move || controls.open(Picking::Change(key.clone())))
        }
        Column::Price => boxed(ROW_HEIGHT, false)
            .justify_end()
            .px(px(PADDING))
            .child(figure(text(move || row.with(Row::price_text))).select_text()),
        Column::Difference | Column::Percent => {
            let tone = move || match row.with(Row::tone) {
                Some(Tone::Above) => theme::POSITIVE.get(),
                Some(Tone::Below) => theme::NEGATIVE.get(),
                Some(Tone::Level) => theme::TEXT_MUTED.get(),
                None => theme::TEXT.get(),
            };
            let shown = move || {
                row.with(|row| match column {
                    Column::Percent => row.percent_text(),
                    _ => row.difference_text(),
                })
            };
            boxed(ROW_HEIGHT, false)
                .justify_end()
                .px(px(PADDING))
                .child(figure(text(shown)).select_text().text_color(tone))
        }
        Column::Remove => {
            let controls = controls.clone();
            boxed(ROW_HEIGHT, true)
                .justify_center()
                .text_color(theme::TEXT_FAINT)
                .role(AccessibilityRole::Button)
                .label(format!("Remove {exchange} {}", market.symbol))
                .tooltip(ui::hint(|| "Remove comparison".to_owned()))
                .transition_colors()
                .hover(|s| s.bg(theme::ACCENT.opacity(0.06)).text_color(theme::DANGER))
                .on_press(move || controls.remove(&key))
                .child(
                    svg(ImageSrc::svg_markup(include_str!(
                        "../../../assets/icons/remove.svg"
                    )))
                    .size(px(10.0)),
                )
        }
    }
}

/// "Add comparison market", under the rows and across every column.
fn add_comparison(controls: &Controls) -> View {
    let controls = controls.clone();
    lined(ROW_HEIGHT, true)
        .col_span_full()
        .flex_row()
        .text_color(theme::TEXT_MUTED)
        .role(AccessibilityRole::Button)
        .label("Add comparison market")
        .test_id(&format!("{ID}-add"))
        .hover(|s| s.bg(theme::ACCENT.opacity(0.06)))
        .on_press(move || controls.open(Picking::Add))
        .child(
            view()
                .w(px(REMOVE_WIDTH))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .border_r_1()
                .border_color(theme::BORDER)
                .child(
                    text("+")
                        .text_size(px(16.0))
                        .font_weight(FontWeight::BOLD)
                        .text_color(theme::TEXT),
                ),
        )
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .flex()
                .items_center()
                .px(px(PADDING))
                .child(text("Add comparison market").number_of_lines(1)),
        )
}

/// The market picker, open for `target`: a pick is saved, and Clear
/// removes the compared market being changed.
fn picker(target: Picking, controls: &Controls) -> View {
    let settings = controls.tab.settings.get_untracked();
    let current = match &target {
        Picking::Base => Some(settings.base()),
        Picking::Add => None,
        Picking::Change(key) => settings.comparison(key),
    };
    let pick = {
        let (target, controls) = (target.clone(), controls.clone());
        move |picked: MarketSelection| {
            controls.change(|settings| match &target {
                Picking::Base => settings.with_base(picked),
                Picking::Add => settings.adding(picked),
                Picking::Change(replaced) => settings.replacing(replaced, picked),
            });
        }
    };
    let mut picker = MarketPicker::new(pick)
        .requires(Requirement {
            snapshot: Some(Operation::FetchOrderBook),
            realtime: Some(Channel::OrderBook),
        })
        .current(current);
    if let Picking::Change(key) = target {
        let controls = controls.clone();
        picker = picker.on_clear("Remove comparison", move || controls.remove(&key));
    }
    let picking = controls.picking;
    picker.view(move || picking.set(None))
}
