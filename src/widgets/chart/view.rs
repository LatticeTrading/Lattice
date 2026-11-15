//! The chart's view: the drawing toolbar down its left side while it is
//! open ([`drawings::toolbar`]), and beside it its plot, which is the
//! shared one ([`plot_view`]: the tiles, the price and time axes, the
//! newest price's tag, the status, the crosshair and the pointer's
//! handling), with what is the chart's own over it: its drawings
//! ([`drawings::layer`]), whose editor hears the plot's pointer first
//! ([`Editor`]), the header (the market charted and its interval) and the
//! note that a page of older candles is on its way. While the toolbar is
//! closed the plot is all of it, and the badge that opens the toolbar
//! floats on the plot ([`drawings::toolbar_badge`]).

use std::rc::Rc;

use nsh_core::prelude::*;

use super::{
    ChartSettings,
    drawings::{self, Editor},
    session::ChartSession,
};
use crate::{
    exchanges, theme,
    widgets::{
        TabContext,
        common::plot::{self, PlotLook, plot_view},
    },
};

/// The chart of `session`, as its tab's settings say.
pub(super) fn chart_view(session: &Rc<ChartSession>, tab: &TabContext<ChartSettings>) -> View {
    let saved = tab.settings;
    let status = session.status;
    let editor = Editor::new(session, tab);
    let look = PlotLook {
        id: "chart",
        grid: computed(move || saved.with(ChartSettings::grid)),
        bar_prices: computed(move || saved.with(ChartSettings::ohlc)),
        status: computed(move || status.get().text()),
        pointer: Some(editor.clone()),
    };
    let over = view()
        .absolute()
        .size_full()
        .child(drawings::layer(&editor))
        .child(header(saved))
        .child(loading_older(session.loading_older));
    // The plot, and laid over its view (after it, so over its surfaces)
    // the badge that opens the toolbar.
    let plot = view()
        .flex_1()
        .min_h(px(0.0))
        .min_w(px(0.0))
        .relative()
        .flex_col()
        .child(plot_view(&session.plot, look, over))
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

/// The market charted and its interval, in the plot's top left corner.
fn header(saved: Memo<ChartSettings>) -> Text {
    plot::header(move || {
        saved.with(|settings| {
            let market = settings.market();
            format!(
                "{}  ·  {}  ·  {}",
                market.symbol,
                settings.interval().label(),
                exchanges::display_name(&market.exchange)
            )
        })
    })
    .test_id("chart-header")
}

/// "Loading older candles..." under the header, while a page of history is
/// on its way.
fn loading_older(loading: Signal<bool>) -> Text {
    text("Loading older candles...")
        .absolute()
        .left(px(14.0))
        .top(px(32.0))
        .number_of_lines(1)
        .text_size(px(10.0))
        .text_color(theme::TEXT_FAINT)
        .style_when(move || !loading.get(), |s| s.hidden())
}
