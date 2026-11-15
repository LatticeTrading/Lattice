//! The Funding Rates' settings, saved in its tab's record under the old
//! app's names: `markets` (the perpetuals watched) and a switch per column
//! (`showExchange`, `showFundingRate`, `showHourlyRate`, `showEightHourRate`,
//! `showDailyRate`, `showAnnualizedRate`, `showPaymentInterval`, `showAge`).
//!
//! Each is left out of the saved form while it holds its default (no
//! market; every column but 8H and 1D), and read leniently: a switch of the
//! wrong type reads as its default, and a market that is not a perpetual
//! with a catalog id is dropped and counted ([`FundingRatesSettings::rejected`]),
//! so the widget can say how many it left out, as the old one did. The
//! count is not saved.
//!
//! A market is told apart by its exchange and catalog id ([`PairKey`]),
//! which is what its statistics are found by.

use std::{collections::HashSet, rc::Rc};

use serde::Serialize;
use serde_json::Value;

use crate::data::ferris::{MarketSelection, MarketType};

/// What tells two watched markets apart, and finds a market's statistics:
/// its exchange and the id the catalog issued it (the old app's
/// `createCatalogMarketKey`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct PairKey(Rc<str>);

impl PairKey {
    pub fn of(market: &MarketSelection) -> Self {
        let id = market.market_id.as_deref().unwrap_or_default();
        Self(format!("{}\n{id}", market.exchange.trim().to_ascii_lowercase()).into())
    }
}

/// Whether the widget can watch `market`: a perpetual the catalog issued
/// an id.
fn watchable(market: &MarketSelection) -> bool {
    market.kind == MarketType::Perp && market.market_id.as_deref().is_some_and(|id| !id.is_empty())
}

/// Why a change to the markets was not made, which the widget says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Refused {
    /// The market cannot be watched, or the one to change is gone.
    Unavailable,
    /// The market is watched already.
    Duplicate,
}

impl Refused {
    /// The old app's words for it.
    pub const fn message(self) -> &'static str {
        match self {
            Self::Unavailable => "Market is no longer available. Select another pair.",
            Self::Duplicate => "Pair already added.",
        }
    }
}

/// A period a funding rate is restated for: a column of the table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Period {
    Hour,
    EightHours,
    Day,
    Year,
}

impl Period {
    /// Every period, in the table's order.
    pub const ALL: [Self; 4] = [Self::Hour, Self::EightHours, Self::Day, Self::Year];

    /// Its column's title.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Hour => "1H",
            Self::EightHours => "8H",
            Self::Day => "1D",
            Self::Year => "1Y",
        }
    }

    /// The switch that shows its column.
    const fn toggle(self) -> Toggle {
        match self {
            Self::Hour => Toggle::Hourly,
            Self::EightHours => Toggle::EightHour,
            Self::Day => Toggle::Daily,
            Self::Year => Toggle::Annualized,
        }
    }
}

/// One of the widget's switches: a column, or every rate column at once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Toggle {
    Exchange,
    /// The rate columns: off hides them all, whatever their own switches.
    Rates,
    Hourly,
    EightHour,
    Daily,
    Annualized,
    Interval,
    Age,
}

impl Toggle {
    /// Every switch, in the order the settings panel lists them (the old
    /// schema's).
    pub const ALL: [Self; 8] = [
        Self::Exchange,
        Self::Rates,
        Self::Hourly,
        Self::EightHour,
        Self::Daily,
        Self::Annualized,
        Self::Interval,
        Self::Age,
    ];

    /// What the settings panel calls it (the old schema's labels).
    pub const fn label(self) -> &'static str {
        match self {
            Self::Exchange => "Show Exchange",
            Self::Rates => "Show Funding Rates",
            Self::Hourly => "Show 1H",
            Self::EightHour => "Show 8H",
            Self::Daily => "Show 1D",
            Self::Annualized => "Show 1Y",
            Self::Interval => "Show Interval",
            Self::Age => "Show Age",
        }
    }

    /// What settings save it under.
    const fn name(self) -> &'static str {
        match self {
            Self::Exchange => "showExchange",
            Self::Rates => "showFundingRate",
            Self::Hourly => "showHourlyRate",
            Self::EightHour => "showEightHourRate",
            Self::Daily => "showDailyRate",
            Self::Annualized => "showAnnualizedRate",
            Self::Interval => "showPaymentInterval",
            Self::Age => "showAge",
        }
    }

    /// Whether it is on until told otherwise: all but 8H and 1D are.
    const fn default(self) -> bool {
        !matches!(self, Self::EightHour | Self::Daily)
    }
}

/// One Funding Rates widget's settings.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub(crate) struct FundingRatesSettings {
    /// The perpetuals watched, each once, in the order they were added.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    markets: Vec<MarketSelection>,
    /// How many saved markets could not be read. Not saved.
    #[serde(skip)]
    rejected: usize,
    /// Each switch: `None` is its default.
    #[serde(rename = "showExchange", skip_serializing_if = "Option::is_none")]
    exchange: Option<bool>,
    #[serde(rename = "showFundingRate", skip_serializing_if = "Option::is_none")]
    rates: Option<bool>,
    #[serde(rename = "showHourlyRate", skip_serializing_if = "Option::is_none")]
    hourly: Option<bool>,
    #[serde(rename = "showEightHourRate", skip_serializing_if = "Option::is_none")]
    eight_hour: Option<bool>,
    #[serde(rename = "showDailyRate", skip_serializing_if = "Option::is_none")]
    daily: Option<bool>,
    #[serde(rename = "showAnnualizedRate", skip_serializing_if = "Option::is_none")]
    annualized: Option<bool>,
    #[serde(
        rename = "showPaymentInterval",
        skip_serializing_if = "Option::is_none"
    )]
    interval: Option<bool>,
    #[serde(rename = "showAge", skip_serializing_if = "Option::is_none")]
    age: Option<bool>,
}

impl From<Value> for FundingRatesSettings {
    fn from(value: Value) -> Self {
        let mut settings = Self::default();
        let Value::Object(members) = value else {
            return settings;
        };
        match members.get("markets") {
            None => {}
            Some(Value::Array(list)) => {
                let mut seen = HashSet::new();
                for entry in list {
                    match MarketSelection::from_value(entry).filter(watchable) {
                        Some(market) => {
                            if seen.insert(PairKey::of(&market)) {
                                settings.markets.push(market);
                            }
                        }
                        None => settings.rejected += 1,
                    }
                }
            }
            // Not a list at all: one thing left out.
            Some(_) => settings.rejected = 1,
        }
        for toggle in Toggle::ALL {
            let on = members
                .get(toggle.name())
                .and_then(Value::as_bool)
                .unwrap_or(toggle.default());
            settings = settings.with_toggle(toggle, on);
        }
        settings
    }
}

impl FundingRatesSettings {
    /// The markets watched, in the order they were added.
    pub fn markets(&self) -> &[MarketSelection] {
        &self.markets
    }

    /// The watched market `key` names, if it is watched.
    pub fn market(&self, key: &PairKey) -> Option<&MarketSelection> {
        self.markets.iter().find(|kept| PairKey::of(kept) == *key)
    }

    /// How many saved markets were left out because they could not be read.
    pub fn rejected(&self) -> usize {
        self.rejected
    }

    /// The markets as changed by the user: nothing is left out any more.
    fn with_markets(mut self, markets: Vec<MarketSelection>) -> Self {
        self.markets = markets;
        self.rejected = 0;
        self
    }

    /// `market` when it can be added: watchable and not watched yet.
    fn admit(&self, market: MarketSelection) -> Result<MarketSelection, Refused> {
        if !watchable(&market) {
            Err(Refused::Unavailable)
        } else if self.market(&PairKey::of(&market)).is_some() {
            Err(Refused::Duplicate)
        } else {
            Ok(market)
        }
    }

    /// These settings with `market` watched last.
    pub fn adding(self, market: MarketSelection) -> Result<Self, Refused> {
        let market = self.admit(market)?;
        let mut markets = self.markets.clone();
        markets.push(market);
        Ok(self.with_markets(markets))
    }

    /// These settings with `market` in place of the market `replaced`
    /// names.
    pub fn replacing(self, replaced: &PairKey, market: MarketSelection) -> Result<Self, Refused> {
        let at = self
            .markets
            .iter()
            .position(|kept| PairKey::of(kept) == *replaced)
            .ok_or(Refused::Unavailable)?;
        let market = self.admit(market)?;
        let mut markets = self.markets.clone();
        markets[at] = market;
        Ok(self.with_markets(markets))
    }

    /// These settings no longer watching the market `key` names.
    pub fn without(self, key: &PairKey) -> Result<Self, Refused> {
        if self.market(key).is_none() {
            return Err(Refused::Unavailable);
        }
        let markets = self
            .markets
            .iter()
            .filter(|kept| PairKey::of(kept) != *key)
            .cloned()
            .collect();
        Ok(self.with_markets(markets))
    }

    fn switch(&self, toggle: Toggle) -> Option<bool> {
        match toggle {
            Toggle::Exchange => self.exchange,
            Toggle::Rates => self.rates,
            Toggle::Hourly => self.hourly,
            Toggle::EightHour => self.eight_hour,
            Toggle::Daily => self.daily,
            Toggle::Annualized => self.annualized,
            Toggle::Interval => self.interval,
            Toggle::Age => self.age,
        }
    }

    /// Whether `toggle` is on.
    pub fn shows(&self, toggle: Toggle) -> bool {
        self.switch(toggle).unwrap_or(toggle.default())
    }

    /// These settings with `toggle` on or off.
    pub fn with_toggle(mut self, toggle: Toggle, on: bool) -> Self {
        let saved = (on != toggle.default()).then_some(on);
        match toggle {
            Toggle::Exchange => self.exchange = saved,
            Toggle::Rates => self.rates = saved,
            Toggle::Hourly => self.hourly = saved,
            Toggle::EightHour => self.eight_hour = saved,
            Toggle::Daily => self.daily = saved,
            Toggle::Annualized => self.annualized = saved,
            Toggle::Interval => self.interval = saved,
            Toggle::Age => self.age = saved,
        }
        self
    }

    /// The periods whose columns show: none while the rates are off.
    pub fn periods(&self) -> Vec<Period> {
        Period::ALL
            .into_iter()
            .filter(|period| self.shows(Toggle::Rates) && self.shows(period.toggle()))
            .collect()
    }
}
