//! The screener's columns: what each shows, how a cell reads, and how rows
//! sort by it (the old app's `MarketScreener` `COLUMNS`, `metric` and
//! `MetricCell`).
//!
//! Values keep their native units. A cell never converts a price into
//! another basis, and a sort never ranks unlike units against each other:
//! rows sort within groups of the same unit, missing values last. Exact
//! values, units and where they came from are in each cell's tooltip
//! ([`tooltip`]).

use std::{borrow::Cow, cmp::Ordering, fmt::Write as _};

use serde::{Serialize, Serializer};

use super::universe::{Derived, Family, Row, Universe};
use crate::{
    data::ferris::{
        CapabilityState, Product,
        market_stats::{
            FieldState, Price, StatsField, StatsFieldName, StatsFields, bulk_unsupported,
        },
    },
    exchanges, format,
};

/// A column of the screener. Saved by its [`Column::id`], the old app's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(super) enum Column {
    Exchange,
    Symbol,
    Quote,
    Family,
    MarkPrice,
    LastPrice,
    IndexPrice,
    Funding1h,
    Funding8h,
    Funding1d,
    FundingApr,
    Settled1h,
    VolumeQuote,
    VolumeUsd,
    VolumeBase,
    OiBase,
    OiQuote,
}

/// A funding column's period.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Period {
    OneHour,
    EightHours,
    OneDay,
    Annualized,
}

impl Period {
    fn basis(self) -> &'static str {
        match self {
            Self::OneHour => "oneHourPercent",
            Self::EightHours => "eightHourPercent",
            Self::OneDay => "oneDayPercent",
            Self::Annualized => "annualizedPercent",
        }
    }
}

/// Which denomination of a volume or open interest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Denomination {
    Base,
    Quote,
}

/// What a column shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Exchange,
    Symbol,
    Quote,
    Family,
    Price(StatsFieldName),
    Funding(StatsFieldName, Period),
    Volume(Denomination),
    UsdVolume,
    OpenInterest(Denomination),
}

/// Every column, in the default order: exchange and symbol lead, the
/// quote asset trails.
pub(super) const ORDER: [Column; 17] = [
    Column::Exchange,
    Column::Symbol,
    Column::Family,
    Column::MarkPrice,
    Column::LastPrice,
    Column::IndexPrice,
    Column::Funding1h,
    Column::Funding8h,
    Column::Funding1d,
    Column::FundingApr,
    Column::Settled1h,
    Column::VolumeQuote,
    Column::VolumeUsd,
    Column::VolumeBase,
    Column::OiBase,
    Column::OiQuote,
    Column::Quote,
];

/// The columns the settings toggle, in their order there: every column
/// but exchange, symbol and quote, which always show.
pub(super) const METRICS: [Column; 14] = [
    Column::Family,
    Column::MarkPrice,
    Column::LastPrice,
    Column::IndexPrice,
    Column::Funding1h,
    Column::Funding8h,
    Column::Funding1d,
    Column::FundingApr,
    Column::Settled1h,
    Column::VolumeQuote,
    Column::VolumeUsd,
    Column::VolumeBase,
    Column::OiBase,
    Column::OiQuote,
];

/// The metric columns a new screener shows.
pub(super) const DEFAULT_METRICS: [Column; 5] = [
    Column::MarkPrice,
    Column::Funding8h,
    Column::VolumeQuote,
    Column::VolumeUsd,
    Column::OiBase,
];

/// What rows sort by before the user picks a column, largest first:
/// estimated USD volume when perpetuals and futures both show (their quote
/// assets differ), else quote volume, as the old app did.
pub(super) fn default_sort(products: &[Product]) -> Sort {
    Sort {
        column: if products.len() > 1 {
            Column::VolumeUsd
        } else {
            Column::VolumeQuote
        },
        descending: true,
    }
}

/// A column to sort by and its direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Sort {
    pub column: Column,
    pub descending: bool,
}

impl Sort {
    /// The sort after pressing `column`'s title: the same column ascending
    /// turns descending, anything else sorts ascending by it.
    pub fn pressed(self, column: Column) -> Self {
        Self {
            column,
            descending: self.column == column && !self.descending,
        }
    }
}

impl Serialize for Column {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.id())
    }
}

impl Column {
    /// Its saved name (the old app's column id).
    pub fn id(self) -> &'static str {
        match self {
            Self::Exchange => "exchange",
            Self::Symbol => "symbol",
            Self::Quote => "quote",
            Self::Family => "family",
            Self::MarkPrice => "markPrice",
            Self::LastPrice => "lastPrice",
            Self::IndexPrice => "indexPrice",
            Self::Funding1h => "funding1h",
            Self::Funding8h => "funding8h",
            Self::Funding1d => "funding1d",
            Self::FundingApr => "fundingApr",
            Self::Settled1h => "settled1h",
            Self::VolumeQuote => "volumeQuote",
            Self::VolumeUsd => "volumeUsd",
            Self::VolumeBase => "volumeBase",
            Self::OiBase => "oiBase",
            Self::OiQuote => "oiQuote",
        }
    }

    /// The column saved as `id`.
    pub fn from_id(id: &str) -> Option<Self> {
        ORDER.into_iter().find(|column| column.id() == id)
    }

    /// Whether it always shows (the settings do not toggle it).
    pub fn fixed(self) -> bool {
        matches!(self, Self::Exchange | Self::Symbol | Self::Quote)
    }

    fn kind(self) -> Kind {
        use {Denomination::*, Period::*, StatsFieldName as F};
        match self {
            Self::Exchange => Kind::Exchange,
            Self::Symbol => Kind::Symbol,
            Self::Quote => Kind::Quote,
            Self::Family => Kind::Family,
            Self::MarkPrice => Kind::Price(F::MarkPrice),
            Self::LastPrice => Kind::Price(F::LastPrice),
            Self::IndexPrice => Kind::Price(F::IndexPrice),
            Self::Funding1h => Kind::Funding(F::Funding, OneHour),
            Self::Funding8h => Kind::Funding(F::Funding, EightHours),
            Self::Funding1d => Kind::Funding(F::Funding, OneDay),
            Self::FundingApr => Kind::Funding(F::Funding, Annualized),
            Self::Settled1h => Kind::Funding(F::LastSettledFunding, OneHour),
            Self::VolumeQuote => Kind::Volume(Quote),
            Self::VolumeUsd => Kind::UsdVolume,
            Self::VolumeBase => Kind::Volume(Base),
            Self::OiBase => Kind::OpenInterest(Base),
            Self::OiQuote => Kind::OpenInterest(Quote),
        }
    }

    /// Its title.
    pub fn title(self) -> &'static str {
        match self {
            Self::Exchange => "Ex",
            Self::Symbol => "Symbol",
            Self::Quote => "Quote",
            Self::Family => "Family",
            Self::MarkPrice => "Mark price",
            Self::LastPrice => "Last price",
            Self::IndexPrice => "Index price",
            Self::Funding1h => "Funding 1h",
            Self::Funding8h => "Funding 8h",
            Self::Funding1d => "Funding 1d",
            Self::FundingApr => "Funding APR",
            Self::Settled1h => "Settled funding 1h",
            Self::VolumeQuote => "24h Vol (Quote)",
            Self::VolumeUsd => "24h Vol (USD)",
            Self::VolumeBase => "24h Vol (Base)",
            Self::OiBase => "OI amount",
            Self::OiQuote => "OI value",
        }
    }

    /// Its name for screen readers.
    pub fn name(self) -> &'static str {
        match self {
            Self::Exchange => "Exchange",
            other => other.title(),
        }
    }

    /// Its width before the table grows to fill the pane, in points.
    pub fn width(self) -> f32 {
        match self {
            Self::Exchange => 36.0,
            Self::Symbol => 132.0,
            Self::Quote => 60.0,
            _ => 120.0,
        }
    }

    /// Whether its cells are numbers (right-aligned).
    pub fn numeric(self) -> bool {
        !matches!(
            self.kind(),
            Kind::Exchange | Kind::Symbol | Kind::Quote | Kind::Family
        )
    }

    /// Whether its values are percentages, which a small `%` follows
    /// (funding).
    pub fn percent(self) -> bool {
        matches!(self.kind(), Kind::Funding(..))
    }

    /// Whether its unit depends on the market (volumes and open interest:
    /// base, quote, settlement asset or contracts).
    pub fn denominated(self) -> bool {
        matches!(self.kind(), Kind::Volume(_) | Kind::OpenInterest(_))
    }

    /// The statistics field it reads.
    fn field(self) -> Option<StatsFieldName> {
        match self.kind() {
            Kind::Price(field) | Kind::Funding(field, _) => Some(field),
            Kind::Volume(_) => Some(StatsFieldName::Volume24h),
            Kind::OpenInterest(_) => Some(StatsFieldName::OpenInterest),
            _ => None,
        }
    }
}

/// A number as the API sent it: decimal text (prices, funding) or a JSON
/// number (volumes, open interest).
#[derive(Clone, Copy, Debug)]
enum Amount<'a> {
    Text(&'a str),
    Number(f64),
}

impl Amount<'_> {
    fn text(self) -> Cow<'static, str> {
        match self {
            Self::Text(text) => Cow::Owned(text.to_owned()),
            Self::Number(number) => Cow::Owned(number.to_string()),
        }
    }

    fn compare(self, other: Self) -> Ordering {
        match (self, other) {
            (Self::Number(a), Self::Number(b)) => a.total_cmp(&b),
            (a, b) => format::compare_decimals(&a.text(), &b.text()),
        }
    }
}

/// A cell's value with its unit and basis, the way it sorts.
struct Metric<'a> {
    amount: Option<Amount<'a>>,
    unit: Cow<'a, str>,
    basis: &'static str,
    /// The price an open interest figure was converted with, whose
    /// freshness the cell then shares.
    conversion: Option<&'a StatsField<Price>>,
    /// What its tooltip says it is.
    detail: Detail,
}

impl Metric<'_> {
    const EMPTY: Metric<'static> = Metric {
        amount: None,
        unit: Cow::Borrowed(""),
        basis: "",
        conversion: None,
        detail: Detail::None,
    };
}

/// What a value is and where it came from, for its tooltip (written out
/// only when one is shown, not for every row a sort reads).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Detail {
    None,
    UsdVolume,
    Funding(StatsFieldName),
    Volume(Denomination),
    /// Open interest as the exchange reported it.
    OpenInterest(Denomination),
    /// Open interest converted with this price field of the market.
    Converted(Denomination, StatsFieldName),
    /// Neither denomination, nor a price to convert the other with.
    Unconvertible,
    Price(StatsFieldName),
}

/// `column`'s value for `row`.
fn metric<'a>(universe: &'a Universe, row: &'a Row, column: Column) -> Metric<'a> {
    let market = universe.market(row);
    let feed = universe.feed(row);
    let product = feed.scope.partition.product;
    let fields = &market.fields;
    let known_units = matches!(row.family, Family::Linear | Family::Inverse)
        || matches!(product, Product::Spot | Product::Option);
    let unknown = Cow::Borrowed("unknown unit");
    match column.kind() {
        Kind::UsdVolume => Metric {
            amount: (row.usd_volume.status != Derived::Unavailable)
                .then_some(row.usd_volume.value)
                .flatten()
                .map(Amount::Number),
            unit: Cow::Borrowed("USD"),
            basis: "estimated-usd-volume",
            conversion: None,
            detail: Detail::UsdVolume,
        },
        Kind::Funding(field, period) => {
            let Some(funding) = fields.funding(field).and_then(StatsField::usable) else {
                return Metric::EMPTY;
            };
            let equivalent = funding.equivalents.as_ref().and_then(|e| match period {
                Period::OneHour => e.one_hour_percent.as_ref(),
                Period::EightHours => e.eight_hour_percent.as_ref(),
                Period::OneDay => e.one_day_percent.as_ref(),
                Period::Annualized => e.annualized_percent.as_ref(),
            });
            Metric {
                amount: equivalent.map(|value| Amount::Text(value.as_str())),
                unit: Cow::Borrowed("%"),
                basis: period.basis(),
                conversion: None,
                detail: Detail::Funding(field),
            }
        }
        Kind::Volume(denomination) => {
            let Some(volume) = fields.volume24h.as_ref().and_then(StatsField::usable) else {
                return Metric::EMPTY;
            };
            let inverse = row.family == Family::Inverse;
            let (quantity, unit) = match denomination {
                Denomination::Base => (
                    volume.base_volume,
                    if inverse {
                        Cow::Borrowed("contracts")
                    } else {
                        Cow::Borrowed(market.base.as_str())
                    },
                ),
                Denomination::Quote => (
                    volume.quote_volume,
                    if inverse {
                        market
                            .settle
                            .as_deref()
                            .map_or(unknown.clone(), Cow::Borrowed)
                    } else {
                        Cow::Borrowed(market.quote.as_str())
                    },
                ),
            };
            Metric {
                amount: quantity.map(Amount::Number),
                unit: if known_units { unit } else { unknown },
                basis: "24h-volume",
                conversion: None,
                detail: Detail::Volume(denomination),
            }
        }
        Kind::OpenInterest(denomination) => {
            if bulk_unsupported(&feed.scope.exchange, StatsFieldName::OpenInterest) {
                return Metric::EMPTY;
            }
            let Some(interest) = fields.open_interest.as_ref().and_then(StatsField::usable) else {
                return Metric::EMPTY;
            };
            let base = denomination == Denomination::Base;
            let contracts = row.family == Family::Inverse || product == Product::Option;
            let quote_unit = if known_units {
                market.quote.as_str()
            } else {
                "unknown unit"
            };
            let unit = if !known_units {
                unknown
            } else if base && contracts {
                Cow::Borrowed("contracts")
            } else if base {
                Cow::Borrowed(market.base.as_str())
            } else {
                Cow::Borrowed(quote_unit)
            };
            let (mut quantity, reported) = if base {
                (interest.open_interest_amount, interest.open_interest_value)
            } else {
                (interest.open_interest_value, interest.open_interest_amount)
            };
            let mut conversion = None;
            let mut detail = Detail::OpenInterest(denomination);
            // A linear market reporting only the other denomination is
            // converted with a price of this very market; contract counts
            // and other assets (stablecoins included) never are.
            if quantity.is_none() && row.family == Family::Linear && !contracts {
                detail = Detail::Unconvertible;
            }
            if quantity.is_none()
                && row.family == Family::Linear
                && !contracts
                && let Some(reported) = reported
            {
                for name in [
                    StatsFieldName::MarkPrice,
                    StatsFieldName::LastPrice,
                    StatsFieldName::IndexPrice,
                ] {
                    let supported = feed
                        .scope
                        .capability
                        .field(product, name)
                        .is_some_and(|support| support.state == CapabilityState::Supported);
                    let Some(field) = fields.price(name) else {
                        continue;
                    };
                    let Some(price) = field.usable() else {
                        continue;
                    };
                    if !supported
                        || price.base_asset != market.base
                        || price.quote_asset != quote_unit
                    {
                        continue;
                    }
                    let Some(rate) = price
                        .amount
                        .as_str()
                        .parse::<f64>()
                        .ok()
                        .filter(|rate| rate.is_finite() && *rate > 0.0)
                    else {
                        continue;
                    };
                    let derived = if base {
                        reported / rate
                    } else {
                        reported * rate
                    };
                    if derived.is_finite() {
                        quantity = Some(derived);
                        conversion = Some(field);
                        detail = Detail::Converted(denomination, name);
                        break;
                    }
                }
            }
            Metric {
                amount: quantity.map(Amount::Number),
                unit,
                basis: "open-interest",
                conversion,
                detail,
            }
        }
        Kind::Price(field) => match fields.price(field).and_then(StatsField::usable) {
            Some(price) => Metric {
                amount: Some(Amount::Text(price.amount.as_str())),
                unit: Cow::Borrowed(price.quote_asset.as_str()),
                basis: "price",
                conversion: None,
                detail: Detail::Price(field),
            },
            None => Metric::EMPTY,
        },
        Kind::Exchange | Kind::Symbol | Kind::Quote | Kind::Family => Metric::EMPTY,
    }
}

/// How a number cell reads its sign.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Tone {
    Plain,
    Positive,
    Negative,
}

/// A number cell as it shows.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct NumberCell {
    /// The value, or `-` when there is none to show.
    pub text: String,
    pub tone: Tone,
    /// A funding percentage: a small `%` follows.
    pub percent: bool,
    /// The value is past its freshness window: a badge follows.
    pub stale: bool,
}

impl NumberCell {
    pub const BLANK: Self = Self {
        text: String::new(),
        tone: Tone::Plain,
        percent: false,
        stale: false,
    };
}

/// `column`'s state and value for `row`, as its cell shows them.
fn evaluate<'a>(universe: &'a Universe, row: &'a Row, column: Column) -> (FieldState, Metric<'a>) {
    let kind = column.kind();
    let feed = universe.feed(row);
    let product = feed.scope.partition.product;
    let state = match (kind, column.field()) {
        (Kind::UsdVolume, _) => match row.usd_volume.status {
            Derived::Available => FieldState::Available,
            Derived::Stale => FieldState::Stale,
            Derived::Unavailable => FieldState::Unavailable,
        },
        (_, Some(field)) if bulk_unsupported(&feed.scope.exchange, field) => {
            FieldState::Unsupported
        }
        (_, Some(field)) => {
            let support = feed.scope.capability.field(product, field).map(|s| s.state);
            match support {
                Some(CapabilityState::Supported) => universe.effective(row, field),
                Some(CapabilityState::NotApplicable) => FieldState::NotApplicable,
                Some(CapabilityState::Partial) => FieldState::Unavailable,
                _ => FieldState::Unsupported,
            }
        }
        (_, None) => FieldState::Unavailable,
    };
    let value = metric(universe, row, column);
    // A converted figure is as fresh as the price it was converted with.
    let state = match (state, value.conversion) {
        (FieldState::Available, Some(price))
            if feed.effective(Some(price.meta()), universe.evaluated_at) == FieldState::Stale =>
        {
            FieldState::Stale
        }
        (state, _) => state,
    };
    (state, value)
}

/// What a cell in `state` shows of `value`: nothing unless it is available
/// or stale.
fn shown<'a>(state: FieldState, value: &Metric<'a>) -> Option<Amount<'a>> {
    value
        .amount
        .filter(|_| matches!(state, FieldState::Available | FieldState::Stale))
}

/// `column`'s number cell for `row`.
pub(super) fn number_cell(universe: &Universe, row: &Row, column: Column) -> NumberCell {
    let kind = column.kind();
    let (state, value) = evaluate(universe, row, column);
    let stale = state == FieldState::Stale;
    let Some(amount) = shown(state, &value) else {
        return NumberCell {
            text: "-".to_owned(),
            stale,
            ..NumberCell::BLANK
        };
    };
    let amount = amount.text();
    match kind {
        Kind::UsdVolume => NumberCell {
            text: format!("${}", format::compact_amount(&amount)),
            stale,
            ..NumberCell::BLANK
        },
        Kind::Funding(..) => {
            let text = format::funding_rate(&amount);
            let tone = if text.starts_with('-') {
                Tone::Negative
            } else {
                Tone::Positive
            };
            NumberCell {
                text,
                tone,
                percent: true,
                stale,
            }
        }
        Kind::Volume(_) | Kind::OpenInterest(_) => NumberCell {
            text: format::compact_amount(&amount),
            stale,
            ..NumberCell::BLANK
        },
        _ => NumberCell {
            text: amount.into_owned(),
            stale,
            ..NumberCell::BLANK
        },
    }
}

/// A text column's text for `row`.
pub(super) fn text_of(universe: &Universe, row: &Row, column: Column) -> String {
    let market = universe.market(row);
    match column.kind() {
        Kind::Exchange => universe.feed(row).label.clone(),
        Kind::Symbol => market.symbol.clone(),
        Kind::Quote => market.quote.clone(),
        Kind::Family => row.family.label().to_owned(),
        _ => String::new(),
    }
}

/// What a field says about itself besides its value.
struct Envelope<'a> {
    reason: Option<&'a str>,
    source: Option<&'a str>,
    exchange_timestamp: Option<f64>,
    received_timestamp: Option<f64>,
}

impl<'a> Envelope<'a> {
    fn of<T>(field: &'a StatsField<T>) -> Self {
        Self {
            reason: field.reason.as_deref(),
            source: field.source.as_deref(),
            exchange_timestamp: field.exchange_timestamp,
            received_timestamp: field.received_timestamp,
        }
    }

    fn find(fields: &'a StatsFields, name: StatsFieldName) -> Option<Self> {
        match name {
            StatsFieldName::Funding | StatsFieldName::LastSettledFunding => {
                fields.funding(name).map(Self::of)
            }
            StatsFieldName::IndexPrice | StatsFieldName::LastPrice | StatsFieldName::MarkPrice => {
                fields.price(name).map(Self::of)
            }
            StatsFieldName::OpenInterest => fields.open_interest.as_ref().map(Self::of),
            StatsFieldName::Volume24h => fields.volume24h.as_ref().map(Self::of),
        }
    }
}

/// `column`'s tooltip for `row`: the cell's state and why, the exact value
/// and unit, how it was derived, and when and where it was received (the
/// old app's cell `title`s). Empty for a cell with none.
pub(super) fn tooltip(universe: &Universe, row: &Row, column: Column) -> String {
    let market = universe.market(row);
    let feed = universe.feed(row);
    match column.kind() {
        Kind::Exchange => return feed.label.clone(),
        Kind::Symbol => return format!("{} · Market ID: {}", market.symbol, market.market_id),
        Kind::Quote | Kind::Family => return String::new(),
        _ => {}
    }
    let product = feed.scope.partition.product;
    let (state, value) = evaluate(universe, row, column);
    let amount = shown(state, &value);
    // A field the venue serves only for selected markets says so, by name
    // ("Binance open interest is not available in bulk statistics.").
    let bulk = column
        .field()
        .filter(|&field| bulk_unsupported(&feed.scope.exchange, field))
        .map(|field| {
            format!(
                "{} {} is not available in bulk statistics.",
                exchanges::display_name(&feed.scope.exchange),
                field.label()
            )
        });
    let envelope = column
        .field()
        .filter(|_| bulk.is_none())
        .and_then(|field| Envelope::find(&market.fields, field));
    let support = bulk.as_deref().or_else(|| {
        column
            .field()
            .and_then(|field| feed.scope.capability.field(product, field))
            .and_then(|support| support.reason.as_deref())
    });
    let mut lines: Vec<String> = Vec::new();
    let label = match (state, amount.is_some()) {
        (FieldState::Stale, _) => "Stale",
        (_, true) => "",
        (FieldState::Unsupported, false) => "Unsupported",
        (FieldState::NotApplicable, false) => "Not applicable",
        _ => "Unavailable",
    };
    lines.push(label.to_owned());
    lines.push(
        envelope
            .as_ref()
            .and_then(|envelope| envelope.reason)
            .or(support)
            .unwrap_or_default()
            .to_owned(),
    );
    lines.push(
        feed.error
            .as_ref()
            .map(|error| error.message.clone())
            .unwrap_or_default(),
    );
    lines.push(detail(universe, row, &value));
    if let Some(amount) = amount {
        let mut line = format!("Value: {}", amount.text());
        if !value.unit.is_empty() {
            let _ = write!(line, " {}", value.unit);
        }
        lines.push(line);
    }
    let received = envelope.as_ref().and_then(|e| e.received_timestamp);
    let exchange = envelope.as_ref().and_then(|e| e.exchange_timestamp);
    lines.push(format!("Received: {}", format::iso_time(received)));
    lines.push(format!(
        "Exchange timestamp: {}",
        format::iso_time(exchange)
    ));
    if let Some(source) = envelope.as_ref().and_then(|e| e.source) {
        lines.push(format!("Source: {source}"));
    }
    lines.retain(|line| !line.is_empty());
    lines.join(
        "
",
    )
}

/// What `value` is and how it was derived.
fn detail(universe: &Universe, row: &Row, value: &Metric<'_>) -> String {
    let market = universe.market(row);
    let fields = &market.fields;
    let side = |denomination: Denomination| match denomination {
        Denomination::Base => "base",
        Denomination::Quote => "quote",
    };
    match value.detail {
        Detail::None => String::new(),
        Detail::UsdVolume => universe.usd_detail(row),
        Detail::Funding(field) => {
            let Some(funding) = fields.funding(field).and_then(StatsField::usable) else {
                return String::new();
            };
            let interval = funding.payment_interval_ms.map_or_else(
                || "unknown".to_owned(),
                |ms| format!("{}h", ms / 3_600_000.0),
            );
            format!(
                "Native rate: {} {}; {}; payment interval: {interval}. API-supplied equivalent; APR is simple, not compounded.",
                funding.rate.as_str(),
                funding.rate_unit.as_deref().unwrap_or("unknown unit"),
                funding.kind.as_deref().unwrap_or("unknown kind"),
            )
        }
        Detail::Volume(denomination) => format!(
            "24h {} volume; unit: {}. No denomination fallback or currency conversion.",
            side(denomination),
            value.unit
        ),
        Detail::OpenInterest(denomination) => format!(
            "Open interest {}; unit: {}. Exchange-reported.",
            match denomination {
                Denomination::Base => "amount",
                Denomination::Quote => "value",
            },
            value.unit
        ),
        Detail::Unconvertible => {
            "Missing open interest or a usable matching price for conversion.".to_owned()
        }
        Detail::Converted(denomination, name) => {
            let Some(field) = fields.price(name) else {
                return String::new();
            };
            let Some(price) = field.value.as_ref() else {
                return String::new();
            };
            format!(
                "Calculated: OI {}; {}: {} {} per {}. Price received: {}; price exchange timestamp: {}; price source: {}.",
                match denomination {
                    Denomination::Base => "value / price",
                    Denomination::Quote => "amount × price",
                },
                name.as_str(),
                price.amount.as_str(),
                price.quote_asset,
                price.base_asset,
                format::iso_time(field.received_timestamp),
                format::iso_time(field.exchange_timestamp),
                field.source.as_deref().unwrap_or("unknown"),
            )
        }
        Detail::Price(name) => fields
            .price(name)
            .and_then(StatsField::usable)
            .map(|price| format!("{} per {}.", price.quote_asset, price.base_asset))
            .unwrap_or_default(),
    }
}

/// The rows of `universe` (indices) sorted by `sort`.
///
/// A number column sorts within groups of one unit and basis (the groups
/// in a fixed order), estimated USD volume available before stale, and
/// rows without a value last, whichever the direction. A text column
/// sorts by its text. Ties fall back on the scope and market.
pub(super) fn sort_rows(universe: &Universe, rows: &mut [usize], sort: Sort) {
    let column = sort.column;
    let direct = |ordering: Ordering| {
        if sort.descending {
            ordering.reverse()
        } else {
            ordering
        }
    };
    let rows_of = &universe.rows;
    if !column.numeric() {
        let texts: Vec<String> = rows_of
            .iter()
            .map(|row| text_of(universe, row, column))
            .collect();
        rows.sort_by(|&a, &b| {
            direct(format::compare_text(&texts[a], &texts[b]))
                .then_with(|| universe.compare_keys(&rows_of[a], &rows_of[b]))
        });
        return;
    }
    let metrics: Vec<Metric<'_>> = rows_of
        .iter()
        .map(|row| metric(universe, row, column))
        .collect();
    rows.sort_by(|&a, &b| {
        let (x, y) = (&metrics[a], &metrics[b]);
        let tie = || universe.compare_keys(&rows_of[a], &rows_of[b]);
        let (Some(left), Some(right)) = (x.amount, y.amount) else {
            return match (x.amount, y.amount) {
                (None, None) => tie(),
                (None, _) => Ordering::Greater,
                _ => Ordering::Less,
            };
        };
        if column == Column::VolumeUsd {
            let (sa, sb) = (rows_of[a].usd_volume.status, rows_of[b].usd_volume.status);
            if sa != sb {
                return if sa == Derived::Available {
                    Ordering::Less
                } else {
                    Ordering::Greater
                };
            }
        }
        let group = (x.unit.as_ref(), x.basis).cmp(&(y.unit.as_ref(), y.basis));
        if group != Ordering::Equal {
            return group;
        }
        direct(left.compare(right)).then_with(tie)
    });
}
