//! The Positions' settings, saved in its tab's record: the accounts it
//! watches (`account` or `accounts`, what every widget that shows
//! accounts saves: [`Watched`]), left out while it watches none and read
//! leniently.

use serde::Serialize;
use serde_json::Value;

use crate::widgets::common::accounts::{AccountSettings, Watched};

/// One Positions widget's settings.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub(crate) struct PositionsSettings {
    /// The accounts whose positions show.
    #[serde(flatten)]
    watched: Watched,
}

impl From<Value> for PositionsSettings {
    fn from(value: Value) -> Self {
        let Value::Object(members) = value else {
            return Self::default();
        };
        Self {
            watched: Watched::read(&members),
        }
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
