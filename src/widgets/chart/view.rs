//! The chart's view: the drawing toolbar down its left side while it is
//! open ([`drawings::toolbar`]), and beside it its plot, which is the
//! shared one ([`plot_view`]: the tiles, the price and time axes, the
//! newest price's tag, the status, the crosshair and the pointer's
//! handling), with what is the chart's own over it: its indicators' lines
//! ([`indicators::layer`]), its drawings ([`drawings::layer`]), whose
//! editor hears the plot's pointer first ([`Editor`]), the header (the
//! market charted and its exchange) and the note that a page of older
//! candles is on its way. Over the plot's view, where a press is their
//! own, float the badges beside the header ([`badges`]: the one that says
//! the interval charted and opens the menu of them, [`timeframe::badge`],
//! the one that opens the indicators' menu, [`indicators::badge`], and the
//! one that turns fast mode on and off, [`fast::badge`]),
//! the indicators' legend under the header
//! ([`indicators::legend`]) and, while the toolbar is closed and the plot
//! is all of the chart, the badge that opens it
//! ([`drawings::toolbar_badge`]).

use std::rc::Rc;

use nsh_core::prelude::*;

use super::{
    ChartSettings, badges,
    drawings::{self, Editor},
    fast, indicators,
    session::ChartSession,
    timeframe,
};
use crate::{
    exchanges, theme,
    widgets::{
        TabContext,
        common::plot::{self, PlotLook, Times, plot_view},
    },
};

/// The chart of `session`, as its tab's settings say.
pub(super) fn chart_view(session: &Rc<ChartSession>, tab: &TabContext<ChartSettings>) -> View {
    let saved = tab.settings;
    let status = session.status;
    let editor = Editor::new(session, tab);
    let title = computed(move || saved.with(heading));
    // How far under the header the indicators' rows reach.
    let legend =
        computed(move || indicators::legend_height(saved.with(|chart| chart.indicators().len())));
    let look = PlotLook {
        id: "chart",
        grid: computed(move || saved.with(ChartSettings::grid)),
        bar_prices: computed(move || saved.with(ChartSettings::ohlc)),
        status: computed(move || status.get().text()),
        pointer: Some(editor.clone()),
        legend,
        times: computed(Times::default),
    };
    let over = view()
        .absolute()
        .size_full()
        .child(indicators::layer(session))
        .child(drawings::layer(&editor))
        .child(plot::header(move || title.get()).test_id("chart-header"))
        .child(loading_older(session.loading_older, legend));
    // Where the badges beside the header start: the timeframe's, as wide
    // as the interval it says, then the indicators', then fast mode's.
    let interval = computed(move || saved.with(ChartSettings::interval));
    let lefts = badges::lefts(&session.plot, title, move || {
        [
            timeframe::badge_width(interval.get()),
            indicators::BADGE_WIDTH,
            fast::BADGE_WIDTH,
        ]
    });
    // The plot, and laid over its view (after it, so over its surfaces)
    // the indicators' legend, the badges beside the header and the one
    // that opens the toolbar.
    let plot = view()
        .flex_1()
        .min_h(px(0.0))
        .min_w(px(0.0))
        .relative()
        .flex_col()
        .child(plot_view(&session.plot, look, over))
        .child(indicators::legend(session, tab))
        .child(timeframe::badge(tab, move || lefts.get()[0]))
        .child(indicators::badge(tab, move || lefts.get()[1]))
        .child(fast::badge(tab, move || lefts.get()[2]))
        .child(drawings::toolbar_badge(tab, &session.plot));
    view()
        .flex_1()
        .min_h(px(0.0))
        .min_w(px(0.0))
        .flex_row()
        .test_id("chart")
        .child(drawings::toolbar(&editor, tab))
        .child(plot)
}

/// What the plot's header says, in its top left corner: the market
/// charted and its exchange. The interval is the badge's beside it.
fn heading(settings: &ChartSettings) -> String {
    let market = settings.market();
    format!(
        "{}  ·  {}",
        market.symbol,
        exchanges::display_name(&market.exchange)
    )
}

/// "Loading older candles..." under the header and the indicators' rows
/// (`legend` points of them), while a page of history is on its way.
fn loading_older(loading: Signal<bool>, legend: Memo<f32>) -> Text {
    let (left, top) = plot::UNDER_HEADER;
    text("Loading older candles...")
        .absolute()
        .left(px(left))
        .top(move || px(top + legend.get()))
        .number_of_lines(1)
        .text_size(px(10.0))
        .text_color(theme::TEXT_FAINT)
        .style_when(move || !loading.get(), |s| s.hidden())
}
