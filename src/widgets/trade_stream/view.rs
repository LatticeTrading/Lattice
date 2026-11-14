//! The Trade Stream's view: the markets watched and the rate, a warning
//! for markets that cannot stream, the column titles, then the prints.
//!
//! The prints are a keyed `each` in a scroll view, not a virtual list:
//! there are at most 200, so every row exists and none can show blank
//! while scrolling. A print never changes, so a row is made once, when it
//! arrives, and only its time follows the clock's offset; new prints add
//! rows at the top and the oldest go, the rest keep their views.

use std::rc::Rc;

use nsh_core::prelude::*;

use super::{
    TradeStreamSettings,
    session::{Row, Shown, Stream},
};
use crate::{
    data::ferris::{MarketSelection, Side},
    exchanges,
    shell::clock::Clock,
    theme, ui,
    widgets::common::{LogoStyle, Shared, exchange_logo},
};

/// A print's row height (the old app's).
const ROW_HEIGHT: f32 = 20.0;
/// The rows' and titles' side padding.
const PADDING: f32 = 12.0;
/// The left edge a row marks its side with while hovered.
const SIDE_EDGE: f32 = 2.0;
/// How the header and rows draw exchange logos.
const LOGO: LogoStyle = LogoStyle {
    size: 12.0,
    color: theme::TEXT_MUTED,
    initials_size: 6.0,
    boxed: true,
};

/// At or under this width the rate hides and the prints' text shrinks.
const TINY_WIDTH: f32 = 250.0;
/// At or under this width, with several markets, the Time column goes.
const NO_TIME_WIDTH: f32 = 360.0;
/// At or under this width, with three or more markets, the header's pairs
/// stack as overlapping logos.
const STACK_WIDTH: f32 = 519.0;

/// How the prints are laid out, from the market count and the widget's
/// width (the old app's container queries).
#[derive(Clone, Copy, PartialEq)]
struct Layout {
    /// Several markets show, so each print names its market.
    multiple: bool,
    /// Several markets in a narrow pane: Time is dropped.
    no_time: bool,
    /// A very narrow pane: smaller print text.
    tiny: bool,
}

/// One column: how it shares the width, its narrowest, and whether its
/// text is right-aligned (the old app's grid templates).
#[derive(Clone, Copy)]
struct Column {
    grow: f32,
    min: f32,
    right: bool,
}

/// The columns, Pair first when several markets show: Pair, Price, Qty,
/// Time (Time dropped in a narrow pane); else Price, Qty, Time in equal
/// thirds.
fn columns(layout: Layout) -> Vec<(&'static str, Column)> {
    let column = |grow, min, right| Column { grow, min, right };
    if layout.no_time {
        vec![
            ("Pair", column(1.0, 0.0, false)),
            ("Price", column(1.0, 0.0, true)),
            ("Qty", column(0.8, 0.0, true)),
        ]
    } else if layout.multiple {
        vec![
            ("Pair", column(1.4, 80.0, false)),
            ("Price", column(1.0, 45.0, true)),
            ("Qty", column(0.8, 35.0, true)),
            ("Time", column(1.0, 74.0, true)),
        ]
    } else {
        vec![
            ("Price", column(1.0, 0.0, false)),
            ("Qty", column(1.0, 0.0, true)),
            ("Time", column(1.0, 0.0, true)),
        ]
    }
}

/// The gap between columns.
fn gap(layout: Layout) -> f32 {
    if layout.no_time {
        6.0
    } else if layout.multiple {
        8.0
    } else {
        0.0
    }
}

/// `content` sized as `column`.
fn sized<E: Styled>(content: E, column: Column) -> E {
    let content = content
        .flex_1()
        .flex_grow(column.grow)
        .min_w(px(column.min))
        .truncate();
    if column.right {
        content.text_right()
    } else {
        content
    }
}

pub(super) fn stream_view(stream: &Stream, saved: Memo<TradeStreamSettings>) -> View {
    let markets = computed(move || saved.with(TradeStreamSettings::markets));
    let count = computed(move || markets.with(Vec::len));
    // Unmeasured counts as wide, so the first frame is the full layout.
    let width = signal(f32::INFINITY);
    let tiny = computed(move || width.get() <= TINY_WIDTH);
    let layout = computed(move || {
        let multiple = count.get() > 1;
        Layout {
            multiple,
            no_time: multiple && width.get() <= NO_TIME_WIDTH,
            tiny: tiny.get(),
        }
    });
    let stacked = computed(move || count.get() > 2 && width.get() <= STACK_WIDTH);
    let shown = stream.shown;
    view()
        .flex_1()
        .min_h(px(0.0))
        .min_w(px(0.0))
        .flex_col()
        .overflow_hidden()
        .bg(theme::BACKGROUND)
        .text_color(theme::TEXT)
        .text_size(px(12.0))
        .on_layout(move |bounds: &Bounds<Points>| width.set(bounds.size.width.0))
        .child(header(markets, stacked, tiny, stream.rate))
        .child(warning(shown))
        .child(dynamic(move || titles(layout.get())))
        .child(
            view()
                .flex_1()
                .min_h(px(0.0))
                .relative()
                .child(dynamic(move || list(shown, layout.get())))
                .child(empty(shown, markets)),
        )
}

/// The markets watched, then the trades-per-second rate (hidden in a very
/// narrow pane).
fn header(
    markets: Memo<Vec<MarketSelection>>,
    stacked: Memo<bool>,
    tiny: Memo<bool>,
    rate: Signal<f64>,
) -> View {
    let pairs = view()
        .flex_row()
        .items_center()
        .min_w(px(0.0))
        .overflow_hidden()
        .role(AccessibilityRole::List)
        .label("Watched trade pairs")
        .child(dynamic(move || {
            if stacked.get() {
                return markets.with(|markets| stack(markets));
            }
            markets.with(|markets| {
                markets
                    .iter()
                    .enumerate()
                    .map(|(index, market)| pair(market, index > 0))
                    .fold(view().flex_row().items_center(), View::child)
            })
        }));
    let tps = view()
        .flex_none()
        .flex_row()
        .items_baseline()
        .gap(px(4.0))
        .child(
            text("TPS")
                .text_size(px(10.0))
                .font_weight(FontWeight::MEDIUM)
                .letter_spacing(px(0.5))
                .text_color(theme::TEXT_FAINT),
        )
        .child(
            text(move || rate_text(rate.get()))
                .min_w(px(32.0))
                .text_right()
                .tabular_nums()
                .text_size(px(11.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme::TEXT_MUTED),
        )
        .label(move || format!("Trades per second: {}", rate_text(rate.get())))
        .style_when(move || tiny.get(), |s| s.hidden());
    view()
        .flex_none()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(12.0))
        .px(px(PADDING))
        .py(px(6.0))
        .bg(theme::SURFACE)
        .border_b_1()
        .border_color(theme::BORDER)
        .child(pairs)
        .child(tps)
}

/// The rate as the old app showed it: two decimals under ten.
fn rate_text(rate: f64) -> String {
    if rate < 10.0 {
        format!("{rate:.2}")
    } else {
        format!("{rate:.0}")
    }
}

/// `Bybit BTC/USDT:USDT (perp)`: a market's full name.
fn full_name(market: &MarketSelection) -> String {
    format!(
        "{} {} ({})",
        exchanges::display_name(&market.exchange),
        market.request_symbol,
        market.kind.as_str()
    )
}

/// One watched market in the header: its logo and pair, divided from the
/// one before.
fn pair(market: &MarketSelection, divided: bool) -> View {
    let chip = view()
        .flex_none()
        .flex_row()
        .items_center()
        .gap(px(6.0))
        .label(full_name(market))
        .child(exchange_logo(&market.exchange, LOGO))
        .child(
            text(market.symbol.clone())
                .text_size(px(11.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme::TEXT_MUTED),
        );
    if divided {
        chip.ml(px(10.0))
            .pl(px(10.0))
            .border_l_1()
            .border_color(theme::BORDER)
    } else {
        chip
    }
}

/// Three or more watched markets in a narrow pane: their logos stacked,
/// with every market's full name, one per line, in the stack's tooltip.
fn stack(markets: &[MarketSelection]) -> View {
    let names = markets.iter().map(full_name).collect::<Vec<_>>().join("\n");
    markets
        .iter()
        .enumerate()
        .map(|(index, market)| stacked_pair(market, index > 0))
        .fold(view().flex_row().items_center(), View::child)
        .tooltip(ui::hint(move || names.clone()))
}

/// One watched market in a stack: its logo in a round chip overlapping
/// the one before.
fn stacked_pair(market: &MarketSelection, overlapped: bool) -> View {
    let chip = view()
        .flex_none()
        .p(px(2.0))
        .border_1()
        .border_color(theme::SURFACE)
        .rounded_full()
        .bg(theme::SURFACE)
        .label(full_name(market))
        .child(exchange_logo(&market.exchange, LOGO));
    if overlapped { chip.ml(px(-5.0)) } else { chip }
}

/// The markets the server refused, while there are any; the others keep
/// streaming.
fn warning(shown: Signal<Shared<Shown>>) -> View {
    let message = computed(move || {
        shown.with(|shown| {
            let failed = &shown.0.failed;
            (!failed.is_empty()).then(|| {
                let names: Vec<String> = failed
                    .iter()
                    .map(|market| {
                        format!(
                            "{} {}",
                            exchanges::display_name(&market.exchange),
                            market.symbol
                        )
                    })
                    .collect();
                format!(
                    "Unable to connect: {}. Other pairs continue streaming.",
                    names.join(", ")
                )
            })
        })
    });
    view()
        .flex_none()
        .px(px(PADDING))
        .py(px(4.0))
        .text_size(px(11.0))
        .text_color(theme::WARNING)
        .style_when(move || message.with(Option::is_none), |s| s.hidden())
        .child(text(move || message.get().unwrap_or_default()))
}

/// The column titles.
fn titles(layout: Layout) -> View {
    columns(layout).into_iter().fold(
        view()
            .flex_none()
            .flex_row()
            .gap(px(gap(layout)))
            .px(px(PADDING))
            .py(px(4.0))
            .bg(theme::SURFACE)
            .border_b_1()
            .border_color(theme::BORDER),
        |titles, (name, column)| {
            titles.child(sized(
                text(name)
                    .uppercase()
                    .text_size(px(10.0))
                    .font_weight(FontWeight::MEDIUM)
                    .letter_spacing(px(0.5))
                    .text_color(theme::TEXT_FAINT),
                column,
            ))
        },
    )
}

/// The prints, newest first.
fn list(shown: Signal<Shared<Shown>>, layout: Layout) -> impl IntoElement {
    let offset = use_context::<Clock>().map(|clock| move || clock.get().offset);
    let offset: Rc<dyn Fn() -> i16> = match offset {
        Some(offset) => Rc::new(offset),
        None => Rc::new(|| 0),
    };
    scroll()
        .absolute()
        .inset_0()
        .child(view().flex_col().child(each(
            move || shown.with(|shown| shown.0.rows.clone()),
            Row::key,
            move |row, _| print_row(&row.get_untracked(), layout, offset.clone()),
        )))
}

/// One print: its market when several show, then price (green for a buy,
/// red for a sell), amount and time (unless the layout drops it); smaller
/// in a very narrow pane.
fn print_row(row: &Row, layout: Layout, offset: Rc<dyn Fn() -> i16>) -> View {
    let trade = &row.trade.trade;
    let side = match trade.side {
        Side::Buy => theme::BID,
        Side::Sell => theme::ASK,
    };
    let (figures, clock) = if layout.tiny {
        (10.0, 9.0)
    } else {
        (11.0, 10.0)
    };
    let columns = columns(layout);
    let mut cells = columns.iter().map(|&(_, column)| column);
    let mut next = || {
        cells.next().unwrap_or(Column {
            grow: 1.0,
            min: 0.0,
            right: true,
        })
    };
    let mut line = view()
        .h(px(ROW_HEIGHT))
        .flex_none()
        .flex_row()
        .items_center()
        .gap(px(gap(layout)))
        .pl(px(PADDING - SIDE_EDGE))
        .pr(px(PADDING))
        .border_l(px(SIDE_EDGE))
        .border_color(theme::BACKGROUND)
        .hover(move |s| s.bg(theme::ACCENT.opacity(0.06)).border_color(side));
    if layout.multiple {
        line = line.child(market_cell(&row.market, next()));
    }
    let line = line
        .child(sized(
            text(price(trade.price))
                .text_size(px(figures))
                .font_weight(FontWeight::SEMIBOLD)
                .tabular_nums()
                .text_color(side),
            next(),
        ))
        .child(sized(
            text(quantity(trade.amount))
                .text_size(px(figures))
                .tabular_nums()
                .text_color(theme::TEXT),
            next(),
        ));
    if layout.no_time {
        return line;
    }
    let timestamp = trade.timestamp;
    line.child(sized(
        text(move || time(timestamp, offset()))
            .text_size(px(clock))
            .tabular_nums()
            .text_color(theme::TEXT_MUTED),
        next(),
    ))
}

/// A print's market: logo, pair and tags, its full name on hover.
fn market_cell(market: &MarketSelection, column: Column) -> View {
    view()
        .flex_1()
        .flex_grow(column.grow)
        .min_w(px(column.min))
        .flex_row()
        .items_center()
        .gap(px(5.0))
        .overflow_hidden()
        .label(full_name(market))
        .child(exchange_logo(&market.exchange, LOGO))
        .child(
            text(market.symbol.clone())
                .min_w(px(0.0))
                .truncate()
                .text_size(px(11.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme::TEXT_MUTED),
        )
        .child(
            text(market.tags())
                .flex_none()
                .text_size(px(9.0))
                .text_color(theme::TEXT_FAINT),
        )
}

/// What shows instead of prints: a prompt to add a market, connecting, or
/// waiting for the first print.
fn empty(shown: Signal<Shared<Shown>>, markets: Memo<Vec<MarketSelection>>) -> View {
    let message = computed(move || {
        if markets.with(Vec::is_empty) {
            return Some("Add a pair in Settings to start watching trades.");
        }
        shown.with(|shown| {
            let shown = &shown.0;
            if !shown.rows.is_empty() {
                None
            } else if shown.loading {
                Some("Connecting to trade stream…")
            } else {
                Some("Waiting for trades…")
            }
        })
    });
    view()
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .p(px(12.0))
        .text_center()
        .text_size(px(11.0))
        .text_color(theme::TEXT_MUTED)
        .style_when(move || message.with(Option::is_none), |s| s.hidden())
        .child(text(move || message.get().unwrap_or_default()))
}

/// A price as the old app showed it: fewer decimals the larger it is.
fn price(price: f64) -> String {
    if price >= 10_000.0 {
        format!("{price:.1}")
    } else if price >= 100.0 {
        format!("{price:.2}")
    } else if price >= 1.0 {
        format!("{price:.3}")
    } else {
        format!("{price:.4}")
    }
}

/// An amount as the old app showed it.
fn quantity(amount: f64) -> String {
    if amount >= 1_000.0 {
        format!("{amount:.1}")
    } else if amount >= 1.0 {
        format!("{amount:.3}")
    } else {
        format!("{amount:.4}")
    }
}

/// `14:05:09.123`: when it printed, at the clock's UTC offset (minutes).
fn time(timestamp: f64, offset: i16) -> String {
    let ms = (timestamp as i64).saturating_add(i64::from(offset) * 60_000);
    let of_day = ms.rem_euclid(86_400_000);
    format!(
        "{:02}:{:02}:{:02}.{:03}",
        of_day / 3_600_000,
        of_day / 60_000 % 60,
        of_day / 1_000 % 60,
        of_day % 1_000
    )
}
