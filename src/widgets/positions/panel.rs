//! The Positions' settings panel: the rows its accounts are listed and
//! added with (what every account widget's panel starts with:
//! [`account_panel`]), then Valued at, a select of the price the positions
//! are valued at between two reports of their account: the live price (the
//! fastest the exchange streams) or the exchange's mark. The pane's gear
//! opens it for the active tab; every change is saved in the tab record at
//! once.

use nsh_base::SelectItem;
use nsh_core::prelude::*;

use super::settings::PositionsSettings;
use crate::{
    data::{Portfolio, prices::PriceKind},
    ui,
    widgets::{TabContext, common::accounts::account_panel},
};

/// What the select is named.
const VALUED_AT: &str = "Valued at";

/// What the select calls a price.
const fn described(price: PriceKind) -> &'static str {
    match price {
        PriceKind::Live => "Live price",
        PriceKind::Mark => "Mark price",
    }
}

/// The settings panel of the Positions' tab, below the pane's gear.
pub(super) fn panel(tab: &TabContext<PositionsSettings>, portfolio: &Portfolio) -> Overlay {
    let saved = tab.settings;
    let select = {
        let tab = tab.clone();
        ui::compact_dropdown(
            || {
                PriceKind::ALL
                    .iter()
                    .map(|price| SelectItem::new(price.id(), described(*price)))
                    .collect()
            },
            move || SharedString::from(saved.with(|settings| settings.price().id())),
            move || saved.with(|settings| described(settings.price()).to_owned()),
            move |id| {
                if let Some(price) = PriceKind::from_id(id) {
                    tab.save(&saved.get_untracked().with_price(price));
                }
            },
            VALUED_AT,
        )
    };
    account_panel(
        tab,
        portfolio,
        "Positions settings",
        [ui::label_edge_row(VALUED_AT).child(select)],
    )
}
