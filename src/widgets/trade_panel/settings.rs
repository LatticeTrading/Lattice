//! The Trade Panel's settings, saved in its tab's record:
//!
//! - `venue`: the exchange traded on (the first there is while left out);
//! - `instrument`: what is traded there, by the exchange's own name for it
//!   (`BTC` while left out);
//! - `linkColor`: the link color whose picks it takes, as the Chart saves
//!   its own (blue while left out, `""` for none);
//! - `confirmOrders`: `false` sends an order at the first press (asked
//!   again in place while left out).
//!
//! Each is left out of the saved form while it holds its default, and read
//! leniently: a member of the wrong type is its default.
//!
//! A market sent to its link color is traded by its base asset
//! ([`TradePanelSettings::with_linked_market`]): whichever exchange the
//! market picked is on, the panel trades that asset where it trades, so a
//! chart of Bybit's BTC/USDT and a panel on Hyperliquid follow one pick. An
//! asset the exchange does not list is said so in the panel, and nothing
//! can be sent for it.

use serde::{Serialize, Serializer};
use serde_json::Value;

use crate::{data::ferris::MarketSelection, link::LinkColor};

/// What is traded until something else is picked.
pub(super) const DEFAULT_INSTRUMENT: &str = "BTC";
/// The longest an instrument's name is kept, in characters.
const INSTRUMENT_LIMIT: usize = 32;

/// One Trade Panel's settings.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub(crate) struct TradePanelSettings {
    /// The exchange traded on; `None` is the first there is.
    #[serde(skip_serializing_if = "Option::is_none")]
    venue: Option<String>,
    /// What is traded; `None` is [`DEFAULT_INSTRUMENT`].
    #[serde(skip_serializing_if = "Option::is_none")]
    instrument: Option<String>,
    /// The link color whose picks the panel takes: `None` is the default
    /// one ([`LinkColor::DEFAULT`]), `Some(None)` no color (saved as `""`).
    #[serde(
        rename = "linkColor",
        skip_serializing_if = "Option::is_none",
        serialize_with = "save_link"
    )]
    link: Option<Option<LinkColor>>,
    /// `Some(false)` sends an order at the first press; `None` asks again.
    #[serde(rename = "confirmOrders", skip_serializing_if = "Option::is_none")]
    confirm: Option<bool>,
}

/// A link that is not the default one, as it is saved: its color's id, or
/// `""` for none.
fn save_link<S: Serializer>(
    link: &Option<Option<LinkColor>>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(link.flatten().map_or("", LinkColor::id))
}

/// `name` as an instrument is kept: without the space around it, cut to a
/// name's length; `None` for nothing.
fn named(name: &str) -> Option<String> {
    let name: String = name.trim().chars().take(INSTRUMENT_LIMIT).collect();
    (!name.is_empty()).then_some(name)
}

impl From<Value> for TradePanelSettings {
    fn from(value: Value) -> Self {
        let Value::Object(members) = value else {
            return Self::default();
        };
        let text = |name: &str| members.get(name).and_then(Value::as_str);
        Self::default()
            .with_venue(text("venue"))
            .with_instrument(text("instrument").unwrap_or_default())
            .with_link(match members.get("linkColor") {
                Some(Value::Null) => None,
                Some(Value::String(id)) if id.is_empty() => None,
                Some(Value::String(id)) => LinkColor::from_id(id).or(Some(LinkColor::DEFAULT)),
                _ => Some(LinkColor::DEFAULT),
            })
            .with_confirm(
                members
                    .get("confirmOrders")
                    .and_then(Value::as_bool)
                    .unwrap_or(true),
            )
    }
}

impl TradePanelSettings {
    /// The exchange traded on, when one was picked.
    pub fn venue(&self) -> Option<&str> {
        self.venue.as_deref()
    }

    /// What is traded, by the exchange's name for it.
    pub fn instrument(&self) -> &str {
        self.instrument.as_deref().unwrap_or(DEFAULT_INSTRUMENT)
    }

    /// The link color whose picks the panel takes, if any.
    pub fn link(&self) -> Option<LinkColor> {
        self.link.unwrap_or(Some(LinkColor::DEFAULT))
    }

    /// Whether an order is asked for again before it is sent.
    pub fn confirm(&self) -> bool {
        self.confirm.unwrap_or(true)
    }

    /// These settings trading on `venue` (`None`: the first there is).
    pub fn with_venue(mut self, venue: Option<&str>) -> Self {
        self.venue = venue.and_then(named);
        self
    }

    /// These settings trading `instrument` (nothing: the default one).
    pub fn with_instrument(mut self, instrument: &str) -> Self {
        self.instrument = named(instrument).filter(|name| name != DEFAULT_INSTRUMENT);
        self
    }

    /// These settings linked to `color` (`None`: to no color).
    pub fn with_link(mut self, color: Option<LinkColor>) -> Self {
        self.link = (color != Some(LinkColor::DEFAULT)).then_some(color);
        self
    }

    /// These settings trading the asset of `market`, which the panel's
    /// link color was sent.
    pub fn with_linked_market(self, market: MarketSelection) -> Self {
        self.with_instrument(&market.base)
    }

    pub fn with_confirm(mut self, confirm: bool) -> Self {
        self.confirm = (!confirm).then_some(false);
        self
    }
}
