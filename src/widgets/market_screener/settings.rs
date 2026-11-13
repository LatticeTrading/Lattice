//! The screener's settings, saved in its tab's record (the old app's
//! `scope`, `visibleProducts`, `columns`, `columnOrder` and `columnWidths`
//! tab settings, under the same names).
//!
//! Every member is optional and left out of the saved form while it holds
//! its default, so a tab nobody configured saves exactly as before settings
//! existed. Reading is lenient member by member: a member that is missing,
//! of the wrong type or names nothing known reads as its default (unknown
//! column ids are dropped, widths are clamped), so a damaged setting never
//! fails the workspace it is in.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::columns::{Column, DEFAULT_METRICS, METRICS, ORDER};
use crate::data::ferris::Product;

/// The products the screener can show, with their names in the settings.
/// Spot and options have their own metrics and get screeners of their own.
pub(super) const PRODUCT_CHOICES: [(Product, &str); 2] =
    [(Product::Perp, "Perps"), (Product::Future, "Futures")];

/// A column's narrowest and widest width, in points (the old app's).
pub(super) const MIN_WIDTH: f32 = 24.0;
pub(super) const MAX_WIDTH: f32 = 4000.0;

/// One screener's settings.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "Value")]
pub(crate) struct ScreenerSettings {
    /// The one scope shown (a partition id, `bybit:linear-perp`); `None`
    /// shows every exchange.
    #[serde(skip_serializing_if = "Option::is_none")]
    scope: Option<String>,
    /// The products shown, in [`PRODUCT_CHOICES`] order; `None` is both.
    /// None at all is a real choice: the screener shows no rows.
    #[serde(rename = "visibleProducts", skip_serializing_if = "Option::is_none")]
    products: Option<Vec<Product>>,
    /// The metric columns shown, in [`METRICS`] order, never empty; `None`
    /// is [`DEFAULT_METRICS`].
    #[serde(skip_serializing_if = "Option::is_none")]
    columns: Option<Vec<Column>>,
    /// Every column's place, hidden ones included so showing one again puts
    /// it back where it was; `None` is [`ORDER`].
    #[serde(rename = "columnOrder", skip_serializing_if = "Option::is_none")]
    order: Option<Vec<Column>>,
    /// Widths the user set, in whole points. Once there are any, the table
    /// no longer grows to fill its pane.
    #[serde(rename = "columnWidths", skip_serializing_if = "BTreeMap::is_empty")]
    widths: BTreeMap<Column, u16>,
}

impl From<Value> for ScreenerSettings {
    fn from(value: Value) -> Self {
        let mut settings = Self::default();
        let Value::Object(members) = value else {
            return settings;
        };
        if let Some(Value::String(scope)) = members.get("scope")
            && scope != "all"
            && !scope.is_empty()
        {
            settings.scope = Some(scope.clone());
        }
        if let Some(Value::Array(products)) = members.get("visibleProducts") {
            let named = |product: Product| {
                products.iter().any(|value| {
                    serde_json::from_value::<Product>(value.clone()).ok() == Some(product)
                })
            };
            // An explicit empty choice is kept; one naming nothing known is
            // the default, as the old app read it.
            let chosen: Vec<Product> = choices().filter(|&product| named(product)).collect();
            if products.is_empty() || !chosen.is_empty() {
                settings = settings.with_products(chosen);
            }
        }
        if let Some(Value::Array(columns)) = members.get("columns") {
            let named = known_columns(columns);
            let metrics: Vec<Column> = METRICS.into_iter().filter(|c| named.contains(c)).collect();
            settings.columns =
                (!metrics.is_empty() && metrics.as_slice() != DEFAULT_METRICS).then_some(metrics);
        }
        if let Some(Value::Array(order)) = members.get("columnOrder") {
            settings.order = Some(complete_order(known_columns(order)))
                .filter(|order| order.as_slice() != ORDER);
        }
        if let Some(Value::Object(widths)) = members.get("columnWidths") {
            for (id, width) in widths {
                if let (Some(column), Some(width)) = (Column::from_id(id), width.as_f64())
                    && width.is_finite()
                    && width > 0.0
                {
                    settings.widths.insert(column, clamp_width(width as f32));
                }
            }
        }
        settings
    }
}

/// The columns `ids` name, each once, in their order.
fn known_columns(ids: &[Value]) -> Vec<Column> {
    let mut columns = Vec::new();
    for column in ids.iter().filter_map(|id| Column::from_id(id.as_str()?)) {
        if !columns.contains(&column) {
            columns.push(column);
        }
    }
    columns
}

/// `order` followed by every column it leaves out, in [`ORDER`].
fn complete_order(mut order: Vec<Column>) -> Vec<Column> {
    for column in ORDER {
        if !order.contains(&column) {
            order.push(column);
        }
    }
    order
}

fn clamp_width(width: f32) -> u16 {
    width.round().clamp(MIN_WIDTH, MAX_WIDTH) as u16
}

fn choices() -> impl Iterator<Item = Product> {
    PRODUCT_CHOICES.into_iter().map(|(product, _)| product)
}

/// Which side of a column another one is dropped on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Side {
    Before,
    After,
}

impl ScreenerSettings {
    /// Nothing set: what a new screener has, and what is not saved.
    pub(crate) fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// The scope shown, `None` for every exchange.
    pub(super) fn scope(&self) -> Option<&str> {
        self.scope.as_deref()
    }

    pub(super) fn with_scope(&self, scope: Option<String>) -> Self {
        Self {
            scope,
            ..self.clone()
        }
    }

    /// The products shown, in [`PRODUCT_CHOICES`] order.
    pub(super) fn products(&self) -> Vec<Product> {
        self.products.clone().unwrap_or_else(|| choices().collect())
    }

    /// Shows `products` (any of [`PRODUCT_CHOICES`], none included).
    pub(super) fn with_products(&self, products: Vec<Product>) -> Self {
        let products: Vec<Product> = choices().filter(|p| products.contains(p)).collect();
        let all = products.len() == PRODUCT_CHOICES.len();
        Self {
            products: (!all).then_some(products),
            ..self.clone()
        }
    }

    /// The metric columns shown, in [`METRICS`] order.
    pub(super) fn metrics(&self) -> Vec<Column> {
        self.columns
            .clone()
            .unwrap_or_else(|| DEFAULT_METRICS.to_vec())
    }

    /// `column` shown or hidden; `None` when that would hide the last
    /// metric column, or `column` always shows.
    pub(super) fn toggled(&self, column: Column) -> Option<Self> {
        if column.fixed() {
            return None;
        }
        let mut metrics = self.metrics();
        if metrics.contains(&column) {
            metrics.retain(|&shown| shown != column);
        } else {
            metrics.push(column);
        }
        if metrics.is_empty() {
            return None;
        }
        let metrics: Vec<Column> = METRICS
            .into_iter()
            .filter(|c| metrics.contains(c))
            .collect();
        Some(Self {
            columns: (metrics.as_slice() != DEFAULT_METRICS).then_some(metrics),
            ..self.clone()
        })
    }

    /// Every column's place, hidden ones included.
    fn order(&self) -> Vec<Column> {
        self.order.clone().unwrap_or_else(|| ORDER.to_vec())
    }

    /// The columns shown, in order: exchange, symbol and quote, and the
    /// metric columns chosen.
    pub(super) fn visible(&self) -> Vec<Column> {
        let metrics = self.metrics();
        self.order()
            .into_iter()
            .filter(|column| column.fixed() || metrics.contains(column))
            .collect()
    }

    /// `source` moved to `side` of `target`; `None` when nothing moves.
    pub(super) fn moved(&self, source: Column, target: Column, side: Side) -> Option<Self> {
        let order = self.order();
        if source == target || !order.contains(&target) {
            return None;
        }
        let mut next: Vec<Column> = order.iter().copied().filter(|&c| c != source).collect();
        let at = next.iter().position(|&c| c == target)? + usize::from(side == Side::After);
        next.insert(at, source);
        (next != order).then(|| Self {
            order: (next.as_slice() != ORDER).then_some(next),
            ..self.clone()
        })
    }

    /// `column`'s width as the user set it, if they did.
    pub(super) fn width(&self, column: Column) -> Option<f32> {
        self.widths.get(&column).map(|&width| f32::from(width))
    }

    /// Whether the user set any width: the table then keeps the widths as
    /// they are instead of growing to fill its pane.
    pub(super) fn sized(&self) -> bool {
        !self.widths.is_empty()
    }

    /// Every width the user set.
    pub(super) fn widths(&self) -> impl Iterator<Item = (Column, f32)> + '_ {
        self.widths
            .iter()
            .map(|(&column, &width)| (column, f32::from(width)))
    }

    /// The widths set to `widths` (clamped, whole points); a column listed
    /// twice takes its last width.
    pub(super) fn with_widths(&self, widths: impl IntoIterator<Item = (Column, f32)>) -> Self {
        Self {
            widths: widths
                .into_iter()
                .map(|(column, width)| (column, clamp_width(width)))
                .collect(),
            ..self.clone()
        }
    }
}

/// `width` kept within [`MIN_WIDTH`] and [`MAX_WIDTH`], as saved.
pub(super) fn clamped(width: f32) -> f32 {
    f32::from(clamp_width(width))
}
