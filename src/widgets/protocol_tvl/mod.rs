//! The Protocol TVL: what is locked in a DeFi protocol (its total value
//! locked, as DefiLlama counts it) over a month, a quarter, a year or all
//! of its history, as a line over time. It is drawn with the GPU plot the
//! charts share ([`crate::widgets::common::plot`]).
//!
//! What it plots: the protocol's daily TVL in US dollars, one range of it
//! at a time ([`settings`]), as a line green above what the range began at
//! and red below it, with the area under it filled or not. Over the plot
//! are the protocol, which opens the picker of them ([`picker`]), and the
//! range, which opens the menu of them; under those the latest figure and
//! how far it moved over the range. Under the plot, a strip: the chains
//! the value is on with each one's share, and the credit DefiLlama asks
//! for, which opens the protocol's page there. For a centralized exchange
//! the figure is the assets in its known wallets, and is named so.
//!
//! Data (the widget's session, [`ProtocolTvlSession`] in [`session`]):
//!
//! - It watches its protocol's TVL through Ferris
//!   ([`Ferris::watch_defillama_protocol`](crate::data::ferris::Ferris::watch_defillama_protocol)),
//!   one request every widget charting the protocol shares, asked again
//!   every ten minutes. One reply holds the whole history, so another
//!   range asks for nothing. The watch follows the settings: another
//!   protocol is watched before the last one is let go. A hidden widget
//!   keeps its watch, so showing it again is instant.
//! - The picker's search asks Ferris's protocol list once typing has
//!   stopped, and lists a parent protocol before its products; before
//!   anything is typed it offers the exchanges Lattice shows.
//! - While it shows, new data or another range marks its plot, which makes
//!   the scene its view draws at most once a frame. A hidden widget does
//!   none of it. The viewport lives in the session.
//!
//! Settings ([`ProtocolTvlSettings`]) are saved in its tab record; the
//! pane's gear opens them ([`panel`]).

mod panel;
mod picker;
mod session;
mod settings;
#[cfg(test)]
mod tests;
mod view;

use std::rc::Rc;

use nsh_core::prelude::*;

use self::{session::ProtocolTvlSession, settings::ProtocolTvlSettings};
use crate::{
    data::{DataServices, SessionScope},
    widgets::{Category, TabContext, host::Widget},
};

/// The Protocol TVL, as the registry lists it.
pub(crate) struct ProtocolTvl;

impl Widget for ProtocolTvl {
    const ID: &'static str = "protocol-tvl";
    const TITLE: &'static str = "Protocol TVL";
    const DESCRIPTION: &'static str =
        "Chart a DeFi protocol's total value locked over time, from DefiLlama.";
    const CATEGORY: Category = Category::Charts;

    type Settings = ProtocolTvlSettings;
    type Session = ProtocolTvlSession;

    fn session(
        scope: SessionScope,
        services: &DataServices,
        settings: Signal<ProtocolTvlSettings>,
    ) -> ProtocolTvlSession {
        ProtocolTvlSession::new(scope, services.ferris.clone(), settings)
    }

    /// Its plot and its strip, for the protocol and the range its tab's
    /// settings name.
    fn view(session: &Rc<ProtocolTvlSession>, tab: &TabContext<ProtocolTvlSettings>) -> View {
        view::tvl_view(session, tab)
    }

    fn panel(
        _session: &Rc<ProtocolTvlSession>,
        tab: &TabContext<ProtocolTvlSettings>,
    ) -> Option<Overlay> {
        Some(panel::panel(tab))
    }
}
