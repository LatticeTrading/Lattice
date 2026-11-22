//! The Positions' settings, saved in its tab's record:
//!
//! - `account` or `accounts`: the accounts it watches (what every widget
//!   that shows accounts saves: [`Watched`]);
//! - `price`: the price its positions are valued at between two reports of
//!   their account ([`PriceKind`]): `mark` for the exchange's mark; left
//!   out for the live price, the fastest the exchange streams.
//!
//! Each is left out of the saved form while it holds its default, and read
//! leniently: a price that names neither is the live one.

use serde::{Serialize, Serializer};
use serde_json::Value;

use crate::{
    data::prices::PriceKind,
    widgets::common::accounts::{AccountSettings, Watched},
};

/// One Positions widget's settings.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub(crate) struct PositionsSettings {
    /// The accounts whose positions show.
    #[serde(flatten)]
    watched: Watched,
    /// The price they are valued at since their report.
    #[serde(skip_serializing_if = "is_live", serialize_with = "by_id")]
    price: PriceKind,
}

fn is_live(price: &PriceKind) -> bool {
    *price == PriceKind::Live
}

fn by_id<S: Serializer>(price: &PriceKind, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(price.id())
}

impl From<Value> for PositionsSettings {
    fn from(value: Value) -> Self {
        let Value::Object(members) = value else {
            return Self::default();
        };
        let price = members
            .get("price")
            .and_then(Value::as_str)
            .and_then(PriceKind::from_id);
        Self {
            watched: Watched::read(&members),
            price: price.unwrap_or_default(),
        }
    }
}

impl PositionsSettings {
    /// The price the positions are valued at since their report.
    pub fn price(&self) -> PriceKind {
        self.price
    }

    /// These settings with the positions valued at `price`.
    #[must_use]
    pub fn with_price(mut self, price: PriceKind) -> Self {
        self.price = price;
        self
    }
}

impl AccountSettings for PositionsSettings {
    const TEST_ID: &'static str = "positions";

    fn watched(&self) -> &Watched {
        &self.watched
    }

    fn with_watched(mut self, watched: Watched) -> Self {
        self.watched = watched;
        self
    }
}
