//! The Trade Stream: the latest prints of one or more markets, newest
//! first, with each market's buys and sells and the trades-per-second
//! rate (the old app's `TradeStream` and `TradeStreamController`).
//!
//! Data (the widget's session, [`Stream`] in [`session`]):
//!
//! - It holds one lease per market its settings watch on the market's
//!   shared trade feed ([`Ferris::trades`](crate::data::ferris::Ferris::trades)): two streams of one market
//!   share one topic, one snapshot and one buffer. A market the settings
//!   drop is let go; one they keep keeps its feed and prints. A hidden
//!   stream keeps streaming, so showing it again is instant.
//! - Feeds tell it of each change from the socket's callback; it only
//!   marks itself dirty there. While it shows, it merges the feeds' prints
//!   into the rows its view shows at most once a frame, and samples the
//!   rate four times a second. A hidden stream does none of it.
//!
//! Settings ([`TradeStreamSettings`]: the markets, how many prints show)
//! are saved in its tab record; the session keeps a copy the host brings
//! up to date, so what it streams follows them while no view shows it.
//! The pane's gear opens its settings panel ([`panel`]), whose markets
//! come from the market picker.

mod panel;
mod session;
mod settings;
mod view;

use std::rc::Rc;

use nsh_core::prelude::*;

use self::{session::Stream, settings::TradeStreamSettings};
use crate::{
    data::{DataServices, SessionScope},
    widgets::{Category, TabContext, host::Widget},
};

/// The Trade Stream, as the registry lists it.
pub(crate) struct TradeStream;

impl Widget for TradeStream {
    const ID: &'static str = "trade-stream";
    const TITLE: &'static str = "Trade Stream";
    const DESCRIPTION: &'static str = "Watch recent buys and sells across your selected markets.";
    const CATEGORY: Category = Category::OrderFlow;

    type Settings = TradeStreamSettings;
    type Session = Stream;

    fn session(
        scope: SessionScope,
        services: &DataServices,
        settings: TradeStreamSettings,
    ) -> Stream {
        Stream::new(scope, services.ferris.clone(), settings)
    }

    fn settings(stream: &Stream) -> Signal<TradeStreamSettings> {
        stream.settings
    }

    /// Its rows, for the markets its tab's settings watch.
    fn view(stream: &Rc<Stream>, tab: &TabContext<TradeStreamSettings>) -> View {
        view::stream_view(stream, tab.settings)
    }

    fn panel(_stream: &Rc<Stream>, tab: &TabContext<TradeStreamSettings>) -> Option<Overlay> {
        Some(panel::panel(tab.clone()))
    }
}
