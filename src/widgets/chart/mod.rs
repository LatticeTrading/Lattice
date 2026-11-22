//! The Chart: one market's candles at a timeframe, or its trades as a tick
//! line, drawn on the GPU (the old app's `AdvancedChart` and its WebGL
//! `chart-gl` engine, moved to WebGPU's shading language). The plot itself
//! (its scene, shader, viewport and view) is the shared one
//! ([`crate::widgets::common::plot`]), which the Premium/Arb Chart draws
//! with too; this folder is what charts one market with it.
//!
//! Data (the widget's session, [`ChartSession`] in [`session`]):
//!
//! - It holds one lease on what its settings chart: the market's shared
//!   candle feed for the timeframe
//!   ([`Ferris::candles`](crate::data::ferris::Ferris::candles)), or for a
//!   tick chart its shared trade feed
//!   ([`Ferris::trades`](crate::data::ferris::Ferris::trades)), whose
//!   prints it keeps as ticks; in fast mode a candle chart holds both
//!   ([`fast`]). Two charts of one market and timeframe
//!   share one topic and one history. A hidden chart keeps streaming, so
//!   showing it again is instant.
//! - Feeds tell it of each change from the socket's callback; it only
//!   marks its plot there. While it shows, it makes the scene its view
//!   draws at most once a frame, and at once when the viewport moves or the
//!   plot is resized. A hidden chart does none of it.
//! - The viewport lives in the session, so a chart is where it was left
//!   when its page shows again; it starts over when the market or the
//!   timeframe changes. Panning near the oldest candle asks the feed for
//!   the page before it.
//!
//! Settings ([`ChartSettings`]: the market, the timeframe, the grid, the
//! hovered bar's prices, the link color, its drawings, whether their
//! toolbar is open, its indicators, and fast mode) are saved in its tab
//! record; the pane's gear opens its settings panel ([`panel`]).
//!
//! Drawings ([`drawings`]): a toolbar down the plot's left side has the
//! tools (a trend line, a horizontal line, a rectangle, a Fibonacci
//! retracement, a measurement), and what is drawn with them is fixed to
//! its times and prices on the market it
//! was drawn on, over the plot. Closed, the toolbar takes no room: a thin
//! badge floating on the plot, just inside its left edge and halfway down
//! it, brings it out again. The plot is still the shared one: it asks the
//! chart's editor about the pointer before it pans.
//!
//! The timeframe ([`timeframe`]): the plot's header says the market and
//! its exchange, and the first badge floating beside it ([`badges`]) says
//! the interval charted and opens the menu of them, so it is changed
//! without the settings panel.
//!
//! Indicators ([`indicators`]): the badge right of the timeframe's, which
//! says `INDICATORS`, opens the menu of them (a simple moving average,
//! an exponential one, a triple exponential one, Bollinger Bands, a
//! volume-weighted average price). Each one added is computed from the
//! bars charted, whatever the market, and drawn over
//! them in its color; the prices fitted to the bars make room for its
//! lines. A legend under the header lists them, each with its values at
//! the bar under the pointer and, with the pointer over its row, its
//! controls: hiding it, its settings (its inputs and its color), removing
//! it. The session computes them with the scene, again only when a bar or
//! an indicator changed.
//!
//! Fast mode ([`fast`]): the badge right of the indicators', which says
//! `FAST MODE`, turns it on and off. On, a candle chart leases its
//! market's trade feed beside its candle feed and makes its newest candle
//! from the prints, which come sooner and more often than the venue's
//! candle does: its high, low and close follow each print, and a candle
//! the venue has not opened yet is made of its prints alone. Older candles
//! stay the venue's.
//!
//! The chart can be linked ([`crate::link`]): its pane header's link button
//! picks a color (blue at first), and a market picked in the Everything
//! Bar's search while the bar has that color becomes the chart's market
//! setting, which the session follows as any other change of it.
//!
//! Not ported yet from the old chart: the placeholder space it reserved
//! while older candles loaded. Its indicators are Lattice's own, as its
//! drawings are: the old chart had neither beyond a roadmap. Ones that
//! need a plot of their own under the bars (an oscillator, volume) have
//! none yet.

mod badges;
mod drawings;
mod fast;
mod indicators;
mod panel;
mod session;
mod settings;
#[cfg(test)]
mod tests;
mod timeframe;
mod view;

use std::rc::Rc;

use nsh_core::prelude::*;

use self::{session::ChartSession, settings::ChartSettings};
use crate::{
    data::{DataServices, SessionScope},
    widgets::{Category, Linking, TabContext, host::Widget},
};

/// The Chart, as the registry lists it.
pub(crate) struct Chart;

impl Widget for Chart {
    const ID: &'static str = "chart";
    const TITLE: &'static str = "Chart";
    const DESCRIPTION: &'static str =
        "Follow price action with candles and configurable timeframes.";
    const CATEGORY: Category = Category::Charts;

    type Settings = ChartSettings;
    type Session = ChartSession;

    fn session(
        scope: SessionScope,
        services: &DataServices,
        settings: Signal<ChartSettings>,
    ) -> ChartSession {
        ChartSession::new(scope, services.ferris.clone(), settings)
    }

    /// Its plot, for the market and timeframe its tab's settings chart,
    /// with its indicators and its drawing tools.
    fn view(session: &Rc<ChartSession>, tab: &TabContext<ChartSettings>) -> View {
        view::chart_view(session, tab)
    }

    fn panel(_session: &Rc<ChartSession>, tab: &TabContext<ChartSettings>) -> Option<Overlay> {
        Some(panel::panel(tab.clone()))
    }

    const LINKING: Option<Linking<ChartSettings>> = Some(Linking {
        link: ChartSettings::link,
        with_link: ChartSettings::with_link,
        with_market: ChartSettings::with_linked_market,
    });
}
