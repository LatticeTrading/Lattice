//! The chart's view: its plot, which is the shared one
//! ([`plot_view`]: the tiles, the price and time axes, the newest price's
//! tag, the status, the crosshair and the pointer's handling), and over it
//! what is the chart's own: the header (the market charted and its
//! interval) and the note that a page of older candles is on its way.

use std::rc::Rc;

use nsh_core::prelude::*;

use super::{ChartSettings, session::ChartSession};
use crate::{
    exchanges, theme,
    widgets::common::plot::{self, PlotLook, plot_view},
};

/// The chart of `session`, as its tab's settings say.
pub(super) fn chart_view(session: &Rc<ChartSession>, saved: Memo<ChartSettings>) -> View {
    let status = session.status;
    let look = PlotLook {
        id: "chart",
        grid: computed(move || saved.with(ChartSettings::grid)),
        bar_prices: computed(move || saved.with(ChartSettings::ohlc)),
        status: computed(move || status.get().text()),
    };
    let over = view()
        .absolute()
        .size_full()
        .child(header(saved))
        .child(loading_older(session.loading_older));
    plot_view(&session.plot, look, over).test_id("chart")
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
