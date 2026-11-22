//! The Trade Panel's settings panel, a row per setting as the other
//! widgets' panels are laid out (its name on the left, its control on the
//! right). The pane's gear opens it for the active tab; every change is
//! saved in the tab record at once.
//!
//! - Exchange: the exchange traded on, of those that can be
//!   ([`Trading::venues`](crate::data::Trading::venues)).
//! - Link Color: None or one of the five link colors; the same choice as
//!   the pane header's link button ([`crate::link`]).
//! - Confirm Orders: whether the cell that sends an order asks again
//!   before it does.
//!
//! The market is set in the panel itself, where it is typed.

use std::rc::Rc;

use nsh_base::SelectItem;
use nsh_core::prelude::*;

use super::{session::TradeSession, settings::TradePanelSettings};
use crate::{
    link::LinkColor,
    ui::{self, Side},
    widgets::TabContext,
};

/// As wide as the other widgets' panels.
const PANEL_WIDTH: Points = px(340.0);
/// What the Link Color select calls linked to no color.
const UNLINKED: &str = "none";

/// The settings panel of the Trade Panel's tab, below the pane's gear.
pub(super) fn panel(session: &Rc<TradeSession>, tab: &TabContext<TradePanelSettings>) -> Overlay {
    let saved = tab.settings;
    let confirm = {
        let tab = tab.clone();
        ui::label_edge_row("Confirm Orders").child(
            ui::check_box(
                "Confirm Orders",
                move || saved.with(TradePanelSettings::confirm),
                move || {
                    let settings = saved.get_untracked();
                    let next = !settings.confirm();
                    tab.save(&settings.with_confirm(next));
                },
            )
            .test_id("trade-panel-confirm"),
        )
    };
    let body = view()
        .test_id("trade-panel-settings")
        .flex_col()
        .py(px(4.0))
        .child(exchange_row(session, tab.clone()))
        .child(link_row(tab.clone()))
        .child(confirm);
    let close = {
        let tab = tab.clone();
        move || tab.close_settings()
    };
    ui::settings_panel(
        tab.settings_anchor(),
        Side::Right,
        PANEL_WIDTH,
        "Trade Panel settings",
        close,
        body,
    )
}

/// "Exchange": the exchange traded on.
fn exchange_row(session: &Rc<TradeSession>, tab: TabContext<TradePanelSettings>) -> View {
    let saved = tab.settings;
    let venues: Rc<[(&'static str, &'static str)]> = session
        .trading
        .venues()
        .map(|venue| (venue.venue(), venue.name()))
        .collect();
    let (listed, named, venue) = (venues.clone(), venues, session.venue);
    let select = ui::compact_dropdown(
        move || {
            listed
                .iter()
                .map(|(venue, name)| SelectItem::new(*venue, *name))
                .collect()
        },
        move || SharedString::from(venue.get()),
        move || {
            let id = venue.get();
            let name = named.iter().find(|(venue, _)| *venue == id);
            name.map_or(id.clone(), |(_, name)| (*name).to_owned())
        },
        move |id| tab.save(&saved.get_untracked().with_venue(Some(id))),
        "Exchange",
    );
    ui::label_edge_row("Exchange").child(select)
}

/// "Link Color": the link color whose picks the panel takes, or none.
fn link_row(tab: TabContext<TradePanelSettings>) -> View {
    let saved = tab.settings;
    let select = ui::compact_dropdown(
        || {
            std::iter::once(SelectItem::new(UNLINKED, "None"))
                .chain(
                    LinkColor::ALL
                        .iter()
                        .map(|color| SelectItem::new(color.id(), color.label())),
                )
                .collect()
        },
        move || {
            SharedString::from(
                saved.with(|settings| settings.link().map_or(UNLINKED, LinkColor::id)),
            )
        },
        move || saved.with(|settings| settings.link().map_or("None", LinkColor::label).to_owned()),
        move |id| {
            let color = LinkColor::from_id(id);
            if color.is_some() || id == UNLINKED {
                tab.save(&saved.get_untracked().with_link(color));
            }
        },
        "Link Color",
    );
    ui::label_edge_row("Link Color").child(select)
}
