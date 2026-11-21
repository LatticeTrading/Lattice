//! The DeFi Protocols: DefiLlama's protocols ranked by what is locked in
//! them (their total value locked), in a table: each one's rank, name and
//! token, TVL, how it moved over an hour, a day and a week, its category,
//! its chains, and its token's market capitalization alone and over its
//! TVL.
//!
//! Over the table are a search field, which asks DefiLlama's whole list
//! (not only the rows shown), and the category listed; a title orders the
//! rows by its column, and again the other way round. Under it, a strip:
//! how many protocols show of how many match and when the data is from,
//! and the credit DefiLlama asks for, which opens its site. A centralized
//! exchange's figure is the assets in its known wallets; the TVL's title
//! says so.
//!
//! Data (the widget's session, [`DefiProtocolsSession`] in [`session`]):
//!
//! - It watches Ferris's protocol list
//!   ([`Ferris::watch_defillama_protocols`](crate::data::ferris::Ferris::watch_defillama_protocols)),
//!   the first 500 by TVL of its category, one request every widget
//!   listing the same shares, asked again every ten minutes. Ferris proxies
//!   and caches DefiLlama: the app never asks DefiLlama itself.
//! - A search is one request per text, asked once typing has stopped; what
//!   it found stays until the next answer.
//! - While it shows, a new list or another order makes its table again. A
//!   hidden widget keeps its watch and does none of it.
//!
//! Settings ([`DefiProtocolsSettings`]: the category and the order) are
//! saved in its tab record; its controls are its own bar and titles, so it
//! has no settings panel.

mod session;
mod settings;
#[cfg(test)]
mod tests;
mod view;

use std::rc::Rc;

use nsh_core::prelude::*;

use self::{session::DefiProtocolsSession, settings::DefiProtocolsSettings};
use crate::{
    data::{DataServices, SessionScope},
    widgets::{Category, TabContext, host::Widget},
};

/// The DeFi Protocols, as the registry lists it.
pub(crate) struct DefiProtocols;

impl Widget for DefiProtocols {
    const ID: &'static str = "defi-protocols";
    const TITLE: &'static str = "DeFi Protocols";
    const DESCRIPTION: &'static str = "Rank DeFi protocols by total value locked, from DefiLlama.";
    const CATEGORY: Category = Category::Market;

    type Settings = DefiProtocolsSettings;
    type Session = DefiProtocolsSession;

    fn session(
        scope: SessionScope,
        services: &DataServices,
        settings: Signal<DefiProtocolsSettings>,
    ) -> DefiProtocolsSession {
        DefiProtocolsSession::new(scope, services.ferris.clone(), settings)
    }

    /// Its bar, its table and its strip, for the category and the order
    /// its tab's settings name.
    fn view(session: &Rc<DefiProtocolsSession>, tab: &TabContext<DefiProtocolsSettings>) -> View {
        view::protocols_view(session, tab)
    }
}
