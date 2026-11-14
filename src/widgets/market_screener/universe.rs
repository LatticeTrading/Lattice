//! The screener's markets as of one moment: every row of every scope it
//! polls, each field's effective state, and the USD rates its volume
//! estimates use (the old app's `MarketUniverseService` and
//! `AssetUsdRates`).
//!
//! A [`Universe`] is built from the feeds' latest snapshots in one pass and
//! never changes after; a new snapshot, or a value going stale, builds the
//! next one. Rows point into the snapshots rather than copying them.

use std::{cmp::Ordering, collections::HashMap, rc::Rc};

use crate::{
    data::{
        ferris::{
            MarketStatsScope, Product, SettlementVolume, StatsSnapshot, Venue,
            market_stats::{FieldMeta, FieldState, StatsFieldName, StatsMarket},
        },
        net::ApiError,
    },
    exchanges, format,
};

/// What one scope's feed had at the moment the universe was built.
pub(super) struct FeedSnap {
    pub scope: Rc<MarketStatsScope>,
    /// `Bybit · Linear perpetuals`.
    pub label: String,
    /// The latest good statistics, kept through later failures.
    pub snapshot: Option<Rc<StatsSnapshot>>,
    /// The latest fetch's error, cleared by the next success.
    pub error: Option<ApiError>,
}

impl FeedSnap {
    pub fn new(
        scope: Rc<MarketStatsScope>,
        snapshot: Option<Rc<StatsSnapshot>>,
        error: Option<ApiError>,
    ) -> Self {
        let label = format!(
            "{} · {}",
            exchanges::display_name(&scope.exchange),
            scope.partition.label
        );
        Self {
            scope,
            label,
            snapshot,
            error,
        }
    }

    /// The exchange does not have this partition after all (an inverse
    /// scope a venue lacks): it is left out entirely, not shown as an error.
    pub fn absent(&self) -> bool {
        self.snapshot.is_none() && self.error.as_ref().is_some_and(|e| e.code == "BAD_SYMBOL")
    }

    /// A field's state as shown: the API's own when it is not available;
    /// stale when the latest refresh failed or it is older than the
    /// exchange's freshness window; else available.
    pub fn effective(&self, meta: Option<FieldMeta<'_>>, now_ms: f64) -> FieldState {
        let Some(meta) = meta else {
            return FieldState::Unavailable;
        };
        if meta.state != FieldState::Available {
            return meta.state;
        }
        let refresh_failed = self.snapshot.is_some() && self.error.is_some();
        let old = meta
            .received_timestamp
            .is_some_and(|at| now_ms - at >= self.scope.capability.stale_after_ms);
        if refresh_failed || old {
            FieldState::Stale
        } else {
            FieldState::Available
        }
    }
}

/// A market's product family, from its settlement asset alone (never from
/// the venue or a guess at the quote).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Family {
    Linear,
    Inverse,
    Unknown,
    /// Spot markets have none.
    None,
}

impl Family {
    fn of(market: &StatsMarket) -> Self {
        if market.product == Product::Spot {
            return Self::None;
        }
        match market.settle.as_deref() {
            None => Self::Unknown,
            Some(settle) if settle == market.base => Self::Inverse,
            Some(settle) if settle == market.quote => Self::Linear,
            Some(_) => Self::Unknown,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Linear => "Linear",
            Self::Inverse => "Inverse",
            Self::Unknown | Self::None => "—",
        }
    }
}

/// How sure a derived figure is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Derived {
    Available,
    Stale,
    Unavailable,
}

impl Derived {
    fn of(state: FieldState) -> Self {
        match state {
            FieldState::Available => Self::Available,
            FieldState::Stale => Self::Stale,
            _ => Self::Unavailable,
        }
    }

    fn worst(self, other: Self) -> Self {
        match (self, other) {
            (Self::Unavailable, _) | (_, Self::Unavailable) => Self::Unavailable,
            (Self::Stale, _) | (_, Self::Stale) => Self::Stale,
            _ => Self::Available,
        }
    }
}

/// A row's 24h quote volume in USD, estimated through [`UsdRates`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct UsdVolume {
    pub value: Option<f64>,
    pub status: Derived,
}

impl UsdVolume {
    const UNAVAILABLE: Self = Self {
        value: None,
        status: Derived::Unavailable,
    };
}

/// One market row: a market of a feed's snapshot.
pub(super) struct Row {
    /// Its feed, an index into [`Universe::feeds`].
    pub feed: usize,
    /// Its market, an index into that feed's snapshot.
    pub market: usize,
    pub family: Family,
    pub usd_volume: UsdVolume,
}

/// Every row of every feed, as of [`Universe::evaluated_at`].
pub(super) struct Universe {
    pub feeds: Vec<FeedSnap>,
    pub rows: Vec<Row>,
    /// When the fields' states were evaluated (Unix milliseconds).
    pub evaluated_at: f64,
    /// When the next available field turns stale with no new data, if one
    /// will: the universe is built again then.
    pub next_change: Option<f64>,
    /// One line per scope with something to report (an error, incomplete
    /// coverage, failed sources).
    pub messages: Vec<String>,
    /// Some scope has neither data nor an error yet.
    pub waiting: bool,
    /// Some scope has answered (data or an error).
    pub answered: bool,
    /// The USD rates behind the rows' volume estimates.
    rates: UsdRates,
}

impl Universe {
    pub fn empty() -> Self {
        Self::build(Vec::new(), 0.0)
    }

    /// The universe of `feeds` evaluated at `now_ms`.
    pub fn build(feeds: Vec<FeedSnap>, now_ms: f64) -> Self {
        let mut rows = Vec::new();
        let mut next_change: Option<f64> = None;
        let mut messages = Vec::new();
        let (mut waiting, mut answered) = (false, false);
        for (index, feed) in feeds.iter().enumerate() {
            if feed.absent() {
                continue;
            }
            match (&feed.snapshot, &feed.error) {
                (None, None) => waiting = true,
                _ => answered = true,
            }
            if let Some(message) = status_line(feed) {
                messages.push(message);
            }
            let Some(snapshot) = &feed.snapshot else {
                continue;
            };
            let product = feed.scope.partition.product;
            let stale_after = feed.scope.capability.stale_after_ms;
            // A market listed twice keeps its first place and its last
            // statistics.
            let mut seen: HashMap<&str, usize> = HashMap::new();
            for (position, market) in snapshot.markets.iter().enumerate() {
                if !market.active || market.product != product {
                    continue;
                }
                if feed.error.is_none() {
                    for &field in &feed.scope.fields {
                        let Some(meta) = market.fields.meta(field) else {
                            continue;
                        };
                        if let (FieldState::Available, Some(at)) =
                            (meta.state, meta.received_timestamp)
                        {
                            let due = at + stale_after;
                            if due > now_ms {
                                next_change = Some(next_change.map_or(due, |next| next.min(due)));
                            }
                        }
                    }
                }
                let row = Row {
                    feed: index,
                    market: position,
                    family: Family::of(market),
                    usd_volume: UsdVolume::UNAVAILABLE,
                };
                match seen.get(market.market_id.as_str()) {
                    Some(&slot) => rows[slot] = row,
                    None => {
                        seen.insert(&market.market_id, rows.len());
                        rows.push(row);
                    }
                }
            }
        }
        let mut universe = Self {
            feeds,
            rows,
            evaluated_at: now_ms,
            next_change,
            messages,
            waiting,
            answered,
            rates: UsdRates(HashMap::new()),
        };
        let rates = UsdRates::derive(&universe);
        let volumes: Vec<UsdVolume> = universe
            .rows
            .iter()
            .map(|row| rates.volume(&universe, row))
            .collect();
        for (row, volume) in universe.rows.iter_mut().zip(volumes) {
            row.usd_volume = volume;
        }
        universe.rates = rates;
        universe
    }

    /// How a row's USD volume was estimated, or why it was not: the quote
    /// volume, the rate's route and the markets it was read from (the old
    /// app's tooltip).
    pub fn usd_detail(&self, row: &Row) -> String {
        let market = self.market(row);
        let (_, quote_volume) = quote_volume(self, row);
        let quote = normalize(&market.quote);
        let rate = self.rates.0.get(&quote);
        let via = rate
            .and_then(|rate| rate.via.as_deref())
            .and_then(|via| self.rates.0.get(via).map(|rate| (via, rate)));
        let mut lines = vec![
            "Estimated USD volume".to_owned(),
            format!(
                "Quote volume: {} {}",
                quote_volume.map_or_else(|| "unavailable".to_owned(), |v| v.to_string()),
                market.quote
            ),
        ];
        let mut route = rate
            .map_or("unavailable", |rate| rate.route.label())
            .to_owned();
        let pairs: Vec<&str> = [rate, via.map(|(_, rate)| rate)]
            .into_iter()
            .flatten()
            .filter_map(|rate| rate.source.as_ref().map(|source| source.pair.as_str()))
            .collect();
        if !pairs.is_empty() {
            route = format!("{route} · {} → {quote}/USD", pairs.join(" → "));
        }
        lines.push(format!("Route: {route}"));
        if let Some(rate) = rate {
            lines.push(rate.provenance(&quote));
        }
        if let Some((asset, rate)) = via {
            lines.push(format!("Intermediate:\n{}", rate.provenance(asset)));
        }
        let reason = if market.product != Product::Perp {
            Some("USD volume is estimated for perpetuals only.".to_owned())
        } else if !has_quote_asset_volume(market) {
            Some("Volume is reported in the settlement asset, not the quote asset.".to_owned())
        } else if quote_volume.is_none() {
            Some("No usable quote volume.".to_owned())
        } else if rate.is_none() {
            Some(format!("No USD rate for {quote}."))
        } else {
            None
        };
        lines.extend(reason);
        lines.join("\n")
    }

    pub fn feed(&self, row: &Row) -> &FeedSnap {
        &self.feeds[row.feed]
    }

    pub fn market(&self, row: &Row) -> &StatsMarket {
        let snapshot = self.feeds[row.feed]
            .snapshot
            .as_ref()
            .expect("a row's feed has its snapshot");
        &snapshot.markets[row.market]
    }

    /// The order rows fall back on when they tie: by scope, then market.
    pub fn compare_keys(&self, a: &Row, b: &Row) -> Ordering {
        self.feed(a)
            .scope
            .partition
            .id
            .cmp(&self.feed(b).scope.partition.id)
            .then_with(|| self.market(a).market_id.cmp(&self.market(b).market_id))
    }

    /// The effective state of a row's field.
    pub fn effective(&self, row: &Row, field: StatsFieldName) -> FieldState {
        self.feed(row)
            .effective(self.market(row).fields.meta(field), self.evaluated_at)
    }
}

/// What a scope reports besides its rows, if anything.
fn status_line(feed: &FeedSnap) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    if let Some(error) = &feed.error {
        parts.push(error.message.clone());
    }
    if let Some(snapshot) = &feed.snapshot {
        let coverage = &snapshot.coverage;
        if !coverage.enumeration_complete {
            parts.push("Statistics coverage incomplete.".to_owned());
        }
        for failure in &coverage.source_failures {
            let detail = if failure.message.is_empty() {
                &failure.reason
            } else {
                &failure.message
            };
            parts.push(format!("{}: {detail}", failure.source));
        }
    }
    (!parts.is_empty()).then(|| format!("{}: {}", feed.label, parts.join(" · ")))
}

// USD rates.

/// Quote assets assumed worth one dollar. Assumptions, not measured pegs:
/// lookalikes (`USD1`, `USDE`, ...) are never assumed and need a market.
const USD_ANCHORS: [&str; 4] = ["USD", "USDT", "USDC", "USDT0"];
/// The price fields a rate is read from, in order of preference.
const PRICE_PREFERENCE: [StatsFieldName; 3] = [
    StatsFieldName::LastPrice,
    StatsFieldName::MarkPrice,
    StatsFieldName::IndexPrice,
];

fn normalize(asset: &str) -> String {
    asset.trim().to_ascii_uppercase()
}

fn is_anchor(asset: &str) -> bool {
    USD_ANCHORS.contains(&asset)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Route {
    Anchor,
    Direct,
    Cross,
}

impl Route {
    fn label(self) -> &'static str {
        match self {
            Self::Anchor => "anchor",
            Self::Direct => "direct",
            Self::Cross => "cross",
        }
    }
}

/// The market a rate was read from.
#[derive(Debug)]
struct RateSource {
    /// `BTC/USDT`.
    pair: String,
    /// Its scope and symbol (`bybit:linear-perp BTC/USDT:USDT`).
    market: String,
    field: &'static str,
    price: String,
    received: Option<f64>,
}

#[derive(Clone, Debug)]
struct UsdRate {
    rate: f64,
    status: Derived,
    route: Route,
    /// Where it was read; `None` for an anchor.
    source: Option<Rc<RateSource>>,
    /// A cross's reference asset (`BTC`), whose own rate it went through.
    via: Option<String>,
}

impl UsdRate {
    /// Where `asset`'s rate came from, for a tooltip.
    fn provenance(&self, asset: &str) -> String {
        let status = match self.status {
            Derived::Available => "available",
            Derived::Stale => "stale",
            Derived::Unavailable => "unavailable",
        };
        let mut lines = vec![format!("{asset} rate: {} USD ({status})", self.rate)];
        match &self.source {
            None => lines.push("Source: assumed USD-equivalent anchor".to_owned()),
            Some(source) => {
                lines.push(format!("Source: {}", source.market));
                lines.push(format!(
                    "Price field: {}; Price: {}",
                    source.field, source.price
                ));
                lines.push(format!("Received: {}", format::iso_time(source.received)));
            }
        }
        lines.join("\n")
    }
}

/// One market's price as a candidate rate for its base asset.
struct Edge {
    asset: String,
    quote: String,
    rate: f64,
    status: Derived,
    /// The market's quote volume, to prefer the most traded market.
    volume_rank: f64,
    /// A stable tiebreak: the market and the price field.
    identity: (String, String, &'static str),
    source: Rc<RateSource>,
}

/// Whether the quote slot holds volume in the market's quote asset: some
/// venues report settlement volume there ([`Venue::settlement_volume`]:
/// Extended for every market, KuCoin for coin-margined ones).
fn has_quote_asset_volume(market: &StatsMarket) -> bool {
    let settlement_volume = match Venue::of(&market.exchange).settlement_volume {
        SettlementVolume::Never => false,
        SettlementVolume::Always => true,
        SettlementVolume::CoinMargined => market.settle.as_deref() == Some(market.base.as_str()),
    };
    !settlement_volume || market.settle.as_deref() == Some(market.quote.as_str())
}

/// A row's 24h quote volume when it is a finite, non-negative number, and
/// its effective state.
fn quote_volume(universe: &Universe, row: &Row) -> (FieldState, Option<f64>) {
    let state = universe.effective(row, StatsFieldName::Volume24h);
    let amount = universe
        .market(row)
        .fields
        .volume24h
        .as_ref()
        .and_then(|field| field.value.as_ref())
        .and_then(|volume| volume.quote_volume)
        .filter(|amount| amount.is_finite() && *amount >= 0.0);
    (state, amount)
}

/// Every asset's USD rate the universe can prove: anchors, then each
/// asset's best market quoted in an anchor, then each remaining asset's
/// best market quoted in BTC or ETH through their direct rates.
struct UsdRates(HashMap<String, UsdRate>);

impl UsdRates {
    fn derive(universe: &Universe) -> Self {
        let mut rates: HashMap<String, UsdRate> = USD_ANCHORS
            .iter()
            .map(|&anchor| {
                let rate = UsdRate {
                    rate: 1.0,
                    status: Derived::Available,
                    route: Route::Anchor,
                    source: None,
                    via: None,
                };
                (anchor.to_owned(), rate)
            })
            .collect();
        let mut edges = edges(universe);
        edges.sort_by(candidate_order);
        // Anchored quotes first: a direct rate always beats a cross.
        for edge in edges.iter().filter(|edge| is_anchor(&edge.quote)) {
            rates.entry(edge.asset.clone()).or_insert_with(|| UsdRate {
                rate: edge.rate,
                status: edge.status,
                route: Route::Direct,
                source: Some(edge.source.clone()),
                via: None,
            });
        }
        // The references are fixed before any cross is considered, so a
        // cross never feeds another (TOKEN/ETH via ETH/BTC via BTC/USDT).
        let references: HashMap<&str, (f64, Derived)> = ["BTC", "ETH"]
            .into_iter()
            .filter_map(|asset| {
                let rate = rates.get(asset)?;
                (rate.route == Route::Direct).then_some((asset, (rate.rate, rate.status)))
            })
            .collect();
        let mut crosses: Vec<Edge> = edges
            .into_iter()
            .filter_map(|edge| {
                let &(reference, status) = references.get(edge.quote.as_str())?;
                if edge.asset == edge.quote || rates.contains_key(&edge.asset) {
                    return None;
                }
                let rate = edge.rate * reference;
                (rate.is_finite() && rate > 0.0).then(|| Edge {
                    rate,
                    status: edge.status.worst(status),
                    ..edge
                })
            })
            .collect();
        crosses.sort_by(candidate_order);
        for edge in crosses {
            rates.entry(edge.asset).or_insert_with(|| UsdRate {
                rate: edge.rate,
                status: edge.status,
                route: Route::Cross,
                source: Some(edge.source),
                via: Some(edge.quote),
            });
        }
        Self(rates)
    }

    /// A row's estimated USD volume: active perpetuals only, whose volume
    /// is in the quote asset and whose quote asset has a rate.
    fn volume(&self, universe: &Universe, row: &Row) -> UsdVolume {
        let market = universe.market(row);
        if !market.active || market.product != Product::Perp || !has_quote_asset_volume(market) {
            return UsdVolume::UNAVAILABLE;
        }
        let (state, Some(amount)) = quote_volume(universe, row) else {
            return UsdVolume::UNAVAILABLE;
        };
        let volume = Derived::of(state);
        let Some(rate) = self.0.get(&normalize(&market.quote)) else {
            return UsdVolume::UNAVAILABLE;
        };
        let value = amount * rate.rate;
        if volume == Derived::Unavailable || !value.is_finite() {
            return UsdVolume::UNAVAILABLE;
        }
        UsdVolume {
            value: Some(value),
            status: volume.worst(rate.status),
        }
    }
}

/// Available before stale, then the most traded, then a stable order.
fn candidate_order(a: &Edge, b: &Edge) -> Ordering {
    let rank = |edge: &Edge| edge.status != Derived::Available;
    rank(a)
        .cmp(&rank(b))
        .then_with(|| b.volume_rank.total_cmp(&a.volume_rank))
        .then_with(|| a.identity.cmp(&b.identity))
}

/// Each active perpetual's preferred usable price, as a rate of its base.
fn edges(universe: &Universe) -> Vec<Edge> {
    let mut edges = Vec::new();
    for row in &universe.rows {
        let market = universe.market(row);
        if !market.active
            || market.product != Product::Perp
            || market.exchange.trim().is_empty()
            || market.market_id.trim().is_empty()
        {
            continue;
        }
        for field in PRICE_PREFERENCE {
            let state = universe.effective(row, field);
            let Some(price) = market
                .fields
                .price(field)
                .and_then(|price| price.value.as_ref())
            else {
                continue;
            };
            if !matches!(state, FieldState::Available | FieldState::Stale)
                || price.base_asset != market.base
                || price.quote_asset != market.quote
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
            let (volume_state, volume) = quote_volume(universe, row);
            let volume_usable = matches!(volume_state, FieldState::Available | FieldState::Stale);
            let reported = volume_usable
                && market
                    .fields
                    .volume24h
                    .as_ref()
                    .and_then(|field| field.value.as_ref())
                    .is_some_and(|value| value.quote_volume.is_some());
            // A market reporting volume in another unit, or none at all, is
            // not trusted for a rate.
            if reported && (!has_quote_asset_volume(market) || volume.is_none_or(|v| v <= 0.0)) {
                continue;
            }
            edges.push(Edge {
                asset: normalize(&market.base),
                quote: normalize(&price.quote_asset),
                rate,
                status: Derived::of(state),
                volume_rank: if has_quote_asset_volume(market) && volume_usable {
                    volume.unwrap_or(0.0)
                } else {
                    0.0
                },
                identity: (
                    universe.feed(row).scope.partition.id.clone(),
                    market.market_id.clone(),
                    field.as_str(),
                ),
                source: Rc::new(RateSource {
                    pair: format!("{}/{}", market.base, price.quote_asset),
                    market: format!(
                        "{} {}",
                        universe.feed(row).scope.partition.id,
                        market.symbol
                    ),
                    field: field.as_str(),
                    price: price.amount.as_str().to_owned(),
                    received: market
                        .fields
                        .price(field)
                        .and_then(|field| field.received_timestamp),
                }),
            });
            // The preference is per row: an unusable price falls through
            // to the next field, a usable one ends the search.
            break;
        }
    }
    edges
}
