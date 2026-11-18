//! The Trade History's settings, saved in its tab's record: the account it
//! watches (`account`, what every widget that shows an account saves:
//! [`Watched`]), left out while it watches none and read leniently.

use serde::Serialize;
use serde_json::Value;

use crate::widgets::common::accounts::{AccountSettings, Watched};

/// One Trade History's settings.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub(crate) struct TradeHistorySettings {
    /// The account whose trades show.
    #[serde(flatten)]
    watched: Watched,
}

impl From<Value> for TradeHistorySettings {
    fn from(value: Value) -> Self {
        let Value::Object(members) = value else {
            return Self::default();
        };
        Self {
            watched: Watched::read(&members),
        }
    }
}

impl AccountSettings for TradeHistorySettings {
    const TEST_ID: &'static str = "trade-history";

    fn watched(&self) -> &Watched {
        &self.watched
    }

    fn with_watched(mut self, watched: Watched) -> Self {
        self.watched = watched;
        self
    }
}
