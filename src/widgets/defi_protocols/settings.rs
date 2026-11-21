//! The DeFi Protocols' settings, saved in its tab's record:
//!
//! - `category`: the DefiLlama category listed (`"Derivatives"`); every
//!   category while left out;
//! - `sort`: the column the rows are ordered by, under its id (`"name"`,
//!   `"change7d"`); their TVL while left out;
//! - `reversed`: `true` for the column's other direction (a figure's
//!   smallest first, a name's last first); left out otherwise.
//!
//! Each is left out of the saved form while it holds its default, and read
//! leniently: a column it does not know is the TVL, a category that is not
//! text is every category.

use serde::{Serialize, Serializer};
use serde_json::Value;

/// The longest category kept: DefiLlama's run to about thirty characters.
const MAX_CATEGORY: usize = 64;

/// A column of the table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Column {
    /// The protocol's place by TVL among what is listed.
    Rank,
    Name,
    Tvl,
    Change1h,
    Change1d,
    Change7d,
    Category,
    Chains,
    Mcap,
    McapTvl,
}

impl Column {
    /// Every column, in the order the table shows them.
    pub const ALL: [Self; 10] = [
        Self::Rank,
        Self::Name,
        Self::Tvl,
        Self::Change1h,
        Self::Change1d,
        Self::Change7d,
        Self::Category,
        Self::Chains,
        Self::Mcap,
        Self::McapTvl,
    ];
    /// The one the rows are ordered by until told otherwise.
    pub const DEFAULT: Self = Self::Tvl;

    /// What settings save for it, and its test ids end with.
    pub fn id(self) -> &'static str {
        match self {
            Self::Rank => "rank",
            Self::Name => "name",
            Self::Tvl => "tvl",
            Self::Change1h => "change1h",
            Self::Change1d => "change1d",
            Self::Change7d => "change7d",
            Self::Category => "category",
            Self::Chains => "chains",
            Self::Mcap => "mcap",
            Self::McapTvl => "mcapTvl",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|column| column.id() == id)
    }

    /// Its title.
    pub fn title(self) -> &'static str {
        match self {
            Self::Rank => "#",
            Self::Name => "Name",
            Self::Tvl => "TVL",
            Self::Change1h => "1H",
            Self::Change1d => "1D",
            Self::Change7d => "7D",
            Self::Category => "Category",
            Self::Chains => "Chains",
            Self::Mcap => "Mcap",
            Self::McapTvl => "Mcap/TVL",
        }
    }

    /// What its title says on hover.
    pub fn hint(self) -> &'static str {
        match self {
            Self::Rank => "Place by TVL among the protocols listed",
            Self::Name => "The protocol, and its token's symbol",
            Self::Tvl => {
                "Total value locked, in US dollars. For a CEX: the assets in its known wallets"
            }
            Self::Change1h => "How its TVL moved over the last hour",
            Self::Change1d => "How its TVL moved over the last day",
            Self::Change7d => "How its TVL moved over the last week",
            Self::Category => "DefiLlama's category",
            Self::Chains => "The chains it is on: the first, and how many more",
            Self::Mcap => "Its token's market capitalization, in US dollars",
            Self::McapTvl => "Its token's market capitalization over its TVL",
        }
    }

    /// Whether its cells are figures: set against its right edge.
    pub fn numeric(self) -> bool {
        !matches!(self, Self::Name | Self::Category | Self::Chains)
    }

    /// Whether the rows can be ordered by it (the rank is the TVL's order
    /// already).
    pub fn sorts(self) -> bool {
        self != Self::Rank
    }

    /// Whether it is ordered largest first until reversed: a figure is
    /// (the chains by how many they are), text reads from A.
    pub fn descends(self) -> bool {
        !matches!(self, Self::Name | Self::Category)
    }
}

fn save_column<S: Serializer>(column: &Option<Column>, serializer: S) -> Result<S::Ok, S::Error> {
    match column {
        Some(column) => serializer.serialize_str(column.id()),
        None => serializer.serialize_none(),
    }
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// One DeFi Protocols' settings.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub(crate) struct DefiProtocolsSettings {
    /// `None` lists every category.
    #[serde(skip_serializing_if = "Option::is_none")]
    category: Option<String>,
    /// `None` orders by TVL.
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "save_column"
    )]
    sort: Option<Column>,
    /// The sorted column's other direction.
    #[serde(skip_serializing_if = "is_false")]
    reversed: bool,
}

impl From<Value> for DefiProtocolsSettings {
    fn from(value: Value) -> Self {
        let text = |name: &str| value.get(name).and_then(Value::as_str);
        let sort = text("sort")
            .and_then(Column::from_id)
            .filter(|column| column.sorts())
            .unwrap_or(Column::DEFAULT);
        let reversed = value.get("reversed").and_then(Value::as_bool) == Some(true);
        Self::default()
            .with_category(text("category"))
            .with_sort(sort, reversed)
    }
}

impl DefiProtocolsSettings {
    /// The category listed; `None` for every one.
    pub fn category(&self) -> Option<&str> {
        self.category.as_deref()
    }

    /// These settings listing `category`, or every one.
    #[must_use]
    pub fn with_category(mut self, category: Option<&str>) -> Self {
        self.category = category
            .map(str::trim)
            .filter(|category| !category.is_empty() && category.chars().count() <= MAX_CATEGORY)
            .map(str::to_owned);
        self
    }

    /// The column the rows are ordered by.
    pub fn sort(&self) -> Column {
        self.sort.unwrap_or(Column::DEFAULT)
    }

    /// Whether the rows run largest (or last, for text) first.
    pub fn descending(&self) -> bool {
        self.sort().descends() != self.reversed
    }

    #[must_use]
    fn with_sort(mut self, column: Column, reversed: bool) -> Self {
        self.sort = (column != Column::DEFAULT).then_some(column);
        self.reversed = reversed;
        self
    }

    /// These settings after a press on `column`'s title: ordered by it,
    /// the other way round when they already were. A column the rows
    /// cannot be ordered by changes nothing.
    #[must_use]
    pub fn sorted_by(self, column: Column) -> Self {
        if !column.sorts() {
            return self;
        }
        let reversed = self.sort() == column && !self.reversed;
        self.with_sort(column, reversed)
    }
}
