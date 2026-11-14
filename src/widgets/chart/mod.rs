//! The Chart: one market's candles at a timeframe, or its trades as a tick
//! line, drawn on the GPU (the old app's `AdvancedChart` and its WebGL
//! `chart-gl` engine, moved to WebGPU's shading language; [`shader`] says
//! how NSH runs it).
//!
//! Data (the widget's session, [`ChartSession`] in [`session`]):
//!
//! - It holds one lease on what its settings chart: the market's shared
//!   candle feed for the timeframe
//!   ([`Ferris::candles`](crate::data::ferris::Ferris::candles)), or for a
//!   tick chart its shared trade feed
//!   ([`Ferris::trades`](crate::data::ferris::Ferris::trades)), whose
//!   prints it keeps as ticks. Two charts of one market and timeframe
//!   share one topic and one history. A hidden chart keeps streaming, so
//!   showing it again is instant.
//! - Feeds tell it of each change from the socket's callback; it only
//!   marks itself dirty there. While it shows, it makes the [`scene`] its
//!   view draws at most once a frame, and at once when the [`viewport`]
//!   moves or the plot is resized. A hidden chart does none of it.
//! - The viewport lives in the session, so a chart is where it was left
//!   when its page shows again; it starts over when the market or the
//!   timeframe changes. Panning near the oldest candle asks the feed for
//!   the page before it.
//!
//! Settings ([`ChartSettings`]: the market, the timeframe, the grid and the
//! hovered bar's prices) are saved in its tab record; the pane's gear opens
//! its settings panel ([`panel`]).
//!
//! Not ported yet from the old chart: color-linked markets, indicators and
//! drawings (neither existed there beyond a roadmap), and the placeholder
//! space it reserved while older candles loaded.

mod panel;
mod scene;
mod session;
mod settings;
mod shader;
#[cfg(test)]
mod tests;
mod view;
mod viewport;

use std::rc::Rc;

use nsh_core::prelude::*;

use self::{session::ChartSession, settings::ChartSettings};
use crate::{
    data::{DataServices, SessionScope},
    widgets::{Category, TabContext, host::Widget},
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

    /// Its plot, for the market and timeframe its tab's settings chart.
    fn view(session: &Rc<ChartSession>, tab: &TabContext<ChartSettings>) -> View {
        view::chart_view(session, tab.settings)
    }

    fn panel(_session: &Rc<ChartSession>, tab: &TabContext<ChartSettings>) -> Option<Overlay> {
        Some(panel::panel(tab.clone()))
    }
}
