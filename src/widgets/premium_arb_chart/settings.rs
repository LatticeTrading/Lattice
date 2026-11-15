//! The Premium/Arb Chart's settings, saved in its tab's record under the
//! old app's names (`premiumArbChartSettingsSchema`): `primaryMarket` and
//! `secondaryMarket` (the two markets compared, [`Pair`]), `displayMode`
//! (`raw` or `percent`, [`Display`]) and `showGrid`.
//!
//! Each is left out of the saved form while it holds its default (Bybit's
//! BTC/USDT perpetual against Binance's, the raw difference, the grid
//! shown) and read leniently:
//!
//! - a market saved as `null` was cleared on purpose and is none; one that
//!   cannot be read is none too, as the old app cleared it (the default
//!   market in its place would chart a spread nobody asked for). With a
//!   market missing the widget asks for it instead of charting;
//! - a display mode Lattice does not know is the raw difference;
//! - a grid member that is not a boolean shows the grid.

use std::rc::Rc;

use serde::{Serialize, Serializer};
use serde_json::Value;

use crate::{
    data::ferris::{MarketSelection, order_book},
    widgets::common::books::{Layer, Pair},
};

/// What the chart plots of its two prices (the old `displayMode`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Display {
    /// The second market's price less the primary's.
    #[default]
    Raw,
    /// That difference as a percentage of the primary's price.
    Percent,
}

impl Display {
    /// Both modes, in the order the settings list them (the old app's).
    pub const ALL: [Self; 2] = [Self::Raw, Self::Percent];

    /// What settings save for it.
    pub const fn id(self) -> &'static str {
        match self {
            Self::Raw => "raw",
            Self::Percent => "percent",
        }
    }

    /// Its name (the old app's).
    pub const fn label(self) -> &'static str {
        match self {
            Self::Raw => "Raw Delta",
            Self::Percent => "% Delta",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|display| display.id() == id)
    }
}

impl Serialize for Display {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.id())
    }
}

/// A pair's market until told otherwise (the old app's
/// `DEFAULT_BYBIT_BTC_USDT_MARKET` and `DEFAULT_BINANCE_BTC_USDT_MARKET`).
fn default_market(pair: Pair) -> MarketSelection {
    match pair {
        Pair::Primary => MarketSelection::bybit_btc_usdt(),
        Pair::Secondary => MarketSelection::binance_btc_usdt(),
    }
}

/// A market as it is kept: `None` is the pair's default one, `Some(None)`
/// no market (saved as `null`). Boxed: a selection is several times the
/// size of the other settings.
type Kept = Option<Option<Box<MarketSelection>>>;

/// One Premium/Arb Chart's settings.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub(crate) struct PremiumArbSettings {
    #[serde(rename = "primaryMarket", skip_serializing_if = "Option::is_none")]
    primary: Kept,
    #[serde(rename = "secondaryMarket", skip_serializing_if = "Option::is_none")]
    secondary: Kept,
    /// `None` is the raw difference.
    #[serde(rename = "displayMode", skip_serializing_if = "Option::is_none")]
    display: Option<Display>,
    /// `Some(false)` hides the grid; `None` shows it.
    #[serde(rename = "showGrid", skip_serializing_if = "Option::is_none")]
    show_grid: Option<bool>,
}

impl From<Value> for PremiumArbSettings {
    fn from(value: Value) -> Self {
        let Value::Object(members) = value else {
            return Self::default();
        };
        let market = |pair: Pair, name: &str| match members.get(name) {
            None => Some(default_market(pair)),
            Some(saved) => MarketSelection::from_value(saved),
        };
        Self::default()
            .with_market(Pair::Primary, market(Pair::Primary, "primaryMarket"))
            .with_market(Pair::Secondary, market(Pair::Secondary, "secondaryMarket"))
            .with_display(
                members
                    .get("displayMode")
                    .and_then(Value::as_str)
                    .and_then(Display::from_id)
                    .unwrap_or_default(),
            )
            .with_grid(
                members
                    .get("showGrid")
                    .and_then(Value::as_bool)
                    .unwrap_or(true),
            )
    }
}

impl PremiumArbSettings {
    fn kept(&self, pair: Pair) -> &Kept {
        match pair {
            Pair::Primary => &self.primary,
            Pair::Secondary => &self.secondary,
        }
    }

    /// `pair`'s market, if it has one.
    pub fn market(&self, pair: Pair) -> Option<MarketSelection> {
        match self.kept(pair) {
            None => Some(default_market(pair)),
            Some(market) => market.as_deref().cloned(),
        }
    }

    /// These settings with `market` as `pair`'s (`None`: with none).
    pub fn with_market(mut self, pair: Pair, market: Option<MarketSelection>) -> Self {
        let kept = match market {
            Some(market) if market == default_market(pair) => None,
            market => Some(market.map(Box::new)),
        };
        match pair {
            Pair::Primary => self.primary = kept,
            Pair::Secondary => self.secondary = kept,
        }
        self
    }

    /// Whether both markets are set: the chart has a pair to compare.
    pub fn paired(&self) -> bool {
        Pair::ALL
            .into_iter()
            .all(|pair| !matches!(self.kept(pair), Some(None)))
    }

    /// The two markets compared, the primary first; none while one is
    /// missing.
    pub fn markets(&self) -> Option<(MarketSelection, MarketSelection)> {
        Some((self.market(Pair::Primary)?, self.market(Pair::Secondary)?))
    }

    /// The markets set, each as the book its price is read from (one level
    /// a side): what the notices of indicative books name.
    pub fn layers(&self) -> Vec<Layer> {
        Pair::ALL
            .into_iter()
            .filter_map(|pair| {
                let market = self.market(pair)?;
                Some(Layer {
                    pair,
                    key: order_book::stream_key(&market, 1).unwrap_or_default(),
                    market: Rc::new(market),
                })
            })
            .collect()
    }

    pub fn display(&self) -> Display {
        self.display.unwrap_or_default()
    }

    pub fn with_display(mut self, display: Display) -> Self {
        self.display = (display != Display::default()).then_some(display);
        self
    }

    /// Whether the plot draws its grid.
    pub fn grid(&self) -> bool {
        self.show_grid.unwrap_or(true)
    }

    pub fn with_grid(mut self, shown: bool) -> Self {
        self.show_grid = (!shown).then_some(false);
        self
    }
}
