//! DefiLlama through Ferris: protocols and what is locked in them (TVL).
//!
//! Ferris proxies DefiLlama's free API behind two cached `GET` routes and
//! projects their replies down to what a client shows; the app never asks
//! DefiLlama itself.
//!
//! - `GET /v1/defillama/protocols` ([`Ferris::defillama_protocols_query`]):
//!   protocols by TVL, largest first, filtered by `category`, `chain`,
//!   `parent` and `search` and cut to `limit` rows (500 unless asked, 10,000
//!   at most); `total` counts every match. Rows are products: a parent
//!   (`hyperliquid`) is not a row, its children name it (`parentSlug`).
//! - `GET /v1/defillama/protocol/{slug}` ([`Ferris::defillama_protocol_query`]):
//!   one protocol or parent: what it is, its TVL now, by chain
//!   (`chainTvls`, which add up to it) and apart from it (`extraTvls`:
//!   borrowed, staking, a CEX's own tokens), and its history, a point a UTC
//!   day in milliseconds, the last one the latest reading. A slug DefiLlama
//!   does not list is `404 PROTOCOL_NOT_FOUND`.
//!
//! Both replies carry `source`, `sourceUrl`, `fetchedAt` and `stale`
//! (`true` while Ferris serves its last good copy because DefiLlama did not
//! answer). DefiLlama asks to be credited: a view of this data names it and
//! opens [`Protocols::source_url`] / [`ProtocolTvl::source_url`].
//!
//! Ferris keeps each reply for 30 minutes and DefiLlama recomputes TVL
//! hourly, so a watched request is asked again every ten minutes
//! ([`REFRESH`]) and never sooner. A protocol with no TVL (one listed for
//! its volume only: `variational`) reads as no figure and an empty history,
//! not as an error. For a CEX, the "TVL" is the assets in the exchange's
//! known wallets ([`ProtocolRow::is_cex`], [`ProtocolInfo::is_cex`]); a
//! view names it so.
//!
//! Replies are read leniently, member by member: a figure of the wrong
//! type is no figure, a row without a slug or a name is left out, and only
//! a reply without its list or its protocol fails.

use std::time::Duration;

use serde_json::Value;

use super::{Ferris, VENUES, Venue};
use crate::data::net::{ApiError, QuerySpec, Subject, Watch};

/// How often a watched request is asked again.
pub const REFRESH: Duration = Duration::from_secs(10 * 60);
/// Where a view sends its reader when a reply names no page of its own.
pub const SOURCE_URL: &str = "https://defillama.com";
/// The code Ferris answers a slug DefiLlama does not list with (`404`).
pub const NOT_FOUND: &str = "PROTOCOL_NOT_FOUND";
/// The category DefiLlama files centralized exchanges under.
const CEX: &str = "CEX";

/// Which protocols a list asks for. Each member left out asks for all.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProtocolFilter {
    /// DefiLlama's category, exactly (`Derivatives`, `Liquid Staking`).
    pub category: Option<String>,
    /// A chain the protocol is on (`Solana`).
    pub chain: Option<String>,
    /// The children of this parent slug (`hyperliquid`).
    pub parent: Option<String>,
    /// What its name, slug or symbol contains.
    pub search: Option<String>,
    /// The most rows wanted, 1 to 10,000; Ferris sends 500 without it.
    pub limit: Option<u32>,
}

/// What `member` asks for, when it asks for anything.
fn asked(member: &Option<String>) -> Option<&str> {
    member
        .as_deref()
        .map(str::trim)
        .filter(|asked| !asked.is_empty())
}

impl ProtocolFilter {
    /// The most rows wanted, as Ferris takes it.
    fn rows(&self) -> Option<u32> {
        self.limit.map(|limit| limit.clamp(1, 10_000))
    }

    /// The request's path and query: members in one order and only the
    /// ones set, so equal filters make equal requests.
    pub fn path(&self) -> String {
        let mut query = url::form_urlencoded::Serializer::new(String::new());
        let members = [
            ("category", &self.category),
            ("chain", &self.chain),
            ("parent", &self.parent),
            ("search", &self.search),
        ];
        for (name, member) in members {
            if let Some(value) = asked(member) {
                query.append_pair(name, value);
            }
        }
        if let Some(limit) = self.rows() {
            query.append_pair("limit", &limit.to_string());
        }
        match query.finish() {
            query if query.is_empty() => "/v1/defillama/protocols".to_owned(),
            query => format!("/v1/defillama/protocols?{query}"),
        }
    }

    /// What the connection panel says is asked.
    fn detail(&self) -> String {
        let mut parts = vec![match self.rows() {
            Some(limit) => format!("Top {limit} by TVL"),
            None => "By TVL".to_owned(),
        }];
        let named = [&self.category, &self.chain, &self.parent];
        parts.extend(named.into_iter().filter_map(asked).map(str::to_owned));
        if let Some(search) = asked(&self.search) {
            parts.push(format!("\"{search}\""));
        }
        parts.join(" · ")
    }
}

/// A protocol list as Ferris last had it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Protocols {
    /// The rows, largest TVL first, the ones with none last.
    pub protocols: Vec<ProtocolRow>,
    /// How many protocols match the filter, of which these are the first.
    pub total: usize,
    /// When Ferris got the list from DefiLlama, in Unix milliseconds.
    pub fetched_at_ms: Option<f64>,
    /// Ferris is serving its last good copy: DefiLlama did not answer.
    pub stale: bool,
    /// The page of DefiLlama the list is credited to.
    pub source_url: String,
}

impl Protocols {
    /// The list in `reply`; an error when it holds no list.
    pub fn read(reply: &Value) -> Result<Self, ApiError> {
        let rows = reply
            .get("protocols")
            .and_then(Value::as_array)
            .ok_or_else(|| ApiError::invalid_response("the reply lists no protocols"))?;
        let protocols: Vec<ProtocolRow> = rows.iter().filter_map(ProtocolRow::read).collect();
        Ok(Self {
            total: reply
                .get("total")
                .and_then(Value::as_u64)
                .and_then(|total| usize::try_from(total).ok())
                .unwrap_or(protocols.len())
                .max(protocols.len()),
            protocols,
            fetched_at_ms: number(reply, "fetchedAt"),
            stale: reply.get("stale").and_then(Value::as_bool).unwrap_or(false),
            source_url: source_url(reply),
        })
    }
}

/// One protocol of a list.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProtocolRow {
    /// What DefiLlama's routes know it by (`hyperliquid-bridge`; a few
    /// hold more than letters, digits and dashes: `trade[xyz]`).
    pub slug: String,
    pub name: String,
    /// Its token's symbol, when it has a token.
    pub symbol: Option<String>,
    pub category: Option<String>,
    /// The chains it is on.
    pub chains: Vec<String>,
    /// Its TVL in US dollars; `None` for one listed for another measure
    /// only (its volume, its open interest).
    pub tvl: Option<f64>,
    /// How its TVL moved over the last hour, day and week, in percent.
    pub change_1h: Option<f64>,
    pub change_1d: Option<f64>,
    pub change_7d: Option<f64>,
    /// Its token's market capitalization in US dollars.
    pub mcap: Option<f64>,
    /// Its parent protocol's slug, when it is one product of several.
    pub parent_slug: Option<String>,
}

impl ProtocolRow {
    /// The row in `value`; `None` without a slug or a name.
    fn read(value: &Value) -> Option<Self> {
        Some(Self {
            slug: string(value, "slug")?,
            name: string(value, "name")?,
            symbol: string(value, "symbol"),
            category: string(value, "category"),
            chains: strings(value, "chains"),
            tvl: number(value, "tvl"),
            change_1h: number(value, "change1h"),
            change_1d: number(value, "change1d"),
            change_7d: number(value, "change7d"),
            mcap: number(value, "mcap"),
            parent_slug: string(value, "parentSlug"),
        })
    }

    /// Whether it is a centralized exchange, whose "TVL" is the assets in
    /// its known wallets.
    pub fn is_cex(&self) -> bool {
        self.category.as_deref() == Some(CEX)
    }

    /// Its token's market capitalization over its TVL; `None` without
    /// either, or with nothing locked.
    pub fn mcap_to_tvl(&self) -> Option<f64> {
        let (mcap, tvl) = (self.mcap?, self.tvl?);
        (tvl > 0.0).then(|| mcap / tvl)
    }
}

/// What a protocol is, beside its figures.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProtocolInfo {
    pub slug: String,
    pub name: String,
    pub symbol: Option<String>,
    /// DefiLlama's category; a parent has none.
    pub category: Option<String>,
    pub description: Option<String>,
    /// Its parent protocol's slug, when it is one product of several.
    pub parent_slug: Option<String>,
    /// It is the parent of several products, and its figures are theirs
    /// together with what they share counted once.
    pub is_parent: bool,
    /// Its token's market capitalization in US dollars.
    pub mcap: Option<f64>,
}

impl ProtocolInfo {
    fn read(value: &Value) -> Option<Self> {
        Some(Self {
            slug: string(value, "slug")?,
            name: string(value, "name")?,
            symbol: string(value, "symbol"),
            category: string(value, "category"),
            description: string(value, "description"),
            parent_slug: string(value, "parentSlug"),
            is_parent: value
                .get("isParent")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            mcap: number(value, "mcap"),
        })
    }

    /// Whether it is a centralized exchange, whose "TVL" is the assets in
    /// its known wallets.
    pub fn is_cex(&self) -> bool {
        self.category.as_deref() == Some(CEX)
    }
}

/// A figure under a name: a chain's share of a protocol's TVL.
#[derive(Clone, Debug, PartialEq)]
pub struct Share {
    pub name: String,
    pub usd: f64,
}

/// A protocol's TVL at one time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TvlPoint {
    /// In Unix milliseconds: a UTC day's start, or for the last point the
    /// latest reading.
    pub time_ms: f64,
    pub usd: f64,
}

/// One protocol's TVL: now, by chain and over time.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProtocolTvl {
    pub protocol: ProtocolInfo,
    /// Its TVL now, the history's last point; `None` for a protocol with
    /// no TVL.
    pub tvl: Option<f64>,
    /// Its TVL by chain, largest first; together they are [`Self::tvl`].
    pub chains: Vec<Share>,
    /// What DefiLlama counts apart from its TVL (`borrowed`, `staking`,
    /// `Binance-OwnTokens`), largest first.
    pub extras: Vec<Share>,
    /// Its TVL over time, oldest first, each time later than the last.
    pub history: Vec<TvlPoint>,
    /// When Ferris got it from DefiLlama, in Unix milliseconds.
    pub fetched_at_ms: Option<f64>,
    /// Ferris is serving its last good copy: DefiLlama did not answer.
    pub stale: bool,
    /// The protocol's page on DefiLlama, which it is credited to.
    pub source_url: String,
}

impl ProtocolTvl {
    /// The protocol in `reply`; an error when it names none.
    pub fn read(reply: &Value) -> Result<Self, ApiError> {
        let protocol = reply
            .get("protocol")
            .and_then(ProtocolInfo::read)
            .ok_or_else(|| ApiError::invalid_response("the reply names no protocol"))?;
        let mut history: Vec<TvlPoint> = Vec::new();
        let points = reply.get("tvlHistory").and_then(Value::as_array);
        for point in points.into_iter().flatten() {
            let read = |at: usize| point.get(at)?.as_f64().filter(|value| value.is_finite());
            let (Some(time_ms), Some(usd)) = (read(0), read(1)) else {
                continue;
            };
            // Oldest first, as the plot reads a series.
            if history.last().is_none_or(|last| time_ms > last.time_ms) {
                history.push(TvlPoint { time_ms, usd });
            }
        }
        Ok(Self {
            protocol,
            tvl: number(reply, "tvl"),
            chains: shares(reply, "chainTvls"),
            extras: shares(reply, "extraTvls"),
            history,
            fetched_at_ms: number(reply, "fetchedAt"),
            stale: reply.get("stale").and_then(Value::as_bool).unwrap_or(false),
            source_url: source_url(reply),
        })
    }
}

/// The finite number `value` holds as `name`.
fn number(value: &Value, name: &str) -> Option<f64> {
    value
        .get(name)?
        .as_f64()
        .filter(|number| number.is_finite())
}

/// The text `value` holds as `name`, when it says anything.
fn string(value: &Value, name: &str) -> Option<String> {
    let text = value.get(name)?.as_str()?.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

/// The texts of the list `value` holds as `name`.
fn strings(value: &Value, name: &str) -> Vec<String> {
    let items = value.get(name).and_then(Value::as_array);
    items
        .into_iter()
        .flatten()
        .filter_map(|item| item.as_str().map(str::to_owned))
        .collect()
}

/// The figures of the map `value` holds as `name`, largest first (equal
/// ones by name, so their order never depends on the reply's).
fn shares(value: &Value, name: &str) -> Vec<Share> {
    let members = value.get(name).and_then(Value::as_object);
    let mut shares: Vec<Share> = members
        .into_iter()
        .flatten()
        .filter_map(|(name, usd)| {
            let usd = usd.as_f64().filter(|usd| usd.is_finite())?;
            Some(Share {
                name: name.clone(),
                usd,
            })
        })
        .collect();
    shares.sort_by(|a, b| b.usd.total_cmp(&a.usd).then_with(|| a.name.cmp(&b.name)));
    shares
}

/// The page a reply is credited to: its `sourceUrl` while that is a page
/// of DefiLlama's site (what a press opens outside the app is never taken
/// from a reply unchecked), else the site.
fn source_url(reply: &Value) -> String {
    reply
        .get("sourceUrl")
        .and_then(Value::as_str)
        .filter(|page| {
            *page == SOURCE_URL
                || page
                    .strip_prefix(SOURCE_URL)
                    .is_some_and(|rest| rest.starts_with('/'))
        })
        .filter(|page| !page.chars().any(|c| c.is_whitespace() || c.is_control()))
        .unwrap_or(SOURCE_URL)
        .to_owned()
}

/// `slug` as one segment of a path: everything but letters, digits and
/// `-._~` percent-encoded (`trade[xyz]`, `yay!`, `dinero-(pxeth)`).
fn encoded(slug: &str) -> String {
    let mut out = String::with_capacity(slug.len());
    for byte in slug.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// A slug as requests and keys name it: DefiLlama's slugs are lower case,
/// and its routes take any case.
pub fn normalized(slug: &str) -> String {
    slug.trim().to_lowercase()
}

/// The exchanges Lattice shows that DefiLlama lists, in the venue table's
/// order, each with its slug there ([`Venue::defillama`]).
pub fn venues() -> impl Iterator<Item = (&'static Venue, &'static str)> {
    VENUES
        .iter()
        .filter_map(|venue| venue.defillama.map(|slug| (venue, slug)))
}

impl Ferris {
    /// The protocols `filter` asks for: shared by everything asking the
    /// same, and asked again every [`REFRESH`] while watched.
    pub fn defillama_protocols_query(&self, filter: &ProtocolFilter) -> QuerySpec<Protocols> {
        let (http, path) = (self.http.clone(), filter.path());
        let key = format!("ferris:defillama:{path}");
        QuerySpec::new(key, move || {
            let (http, path) = (http.clone(), path.clone());
            async move { Protocols::read(&http.get::<Value>(&path).await?) }
        })
        .refresh_every(REFRESH)
        .subject(Subject {
            exchange: String::new(),
            name: "DefiLlama protocols".to_owned(),
            detail: filter.detail(),
        })
    }

    /// Watches the protocols `filter` asks for.
    pub fn watch_defillama_protocols(&self, filter: &ProtocolFilter) -> Watch<Protocols> {
        self.queries.watch(self.defillama_protocols_query(filter))
    }

    /// The TVL of the protocol (or parent) `slug` names: shared by
    /// everything showing it, and asked again every [`REFRESH`] while
    /// watched.
    pub fn defillama_protocol_query(&self, slug: &str) -> QuerySpec<ProtocolTvl> {
        let (http, slug) = (self.http.clone(), normalized(slug));
        let path = format!("/v1/defillama/protocol/{}", encoded(&slug));
        let named = slug.clone();
        QuerySpec::new(format!("ferris:defillama:protocol:{slug}"), move || {
            let (http, path, empty) = (http.clone(), path.clone(), slug.is_empty());
            async move {
                if empty {
                    return Err(ApiError::invalid_request(
                        "no protocol to ask DefiLlama for",
                    ));
                }
                ProtocolTvl::read(&http.get::<Value>(&path).await?)
            }
        })
        .refresh_every(REFRESH)
        .subject(Subject {
            exchange: String::new(),
            name: format!("DefiLlama {named}"),
            detail: "TVL and its history".to_owned(),
        })
    }

    /// Watches the TVL of the protocol `slug` names.
    pub fn watch_defillama_protocol(&self, slug: &str) -> Watch<ProtocolTvl> {
        self.queries.watch(self.defillama_protocol_query(slug))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_filter_asks_only_for_what_it_sets() {
        assert_eq!(ProtocolFilter::default().path(), "/v1/defillama/protocols");
        let filter = ProtocolFilter {
            category: Some("Liquid Staking".to_owned()),
            search: Some(" trade[xyz] ".to_owned()),
            chain: Some(String::new()),
            limit: Some(50_000),
            ..ProtocolFilter::default()
        };
        assert_eq!(
            filter.path(),
            "/v1/defillama/protocols?category=Liquid+Staking&search=trade%5Bxyz%5D&limit=10000"
        );
        assert_eq!(
            filter.detail(),
            "Top 10000 by TVL · Liquid Staking · \"trade[xyz]\""
        );
        // A slug is one segment of its path, whatever it holds.
        assert_eq!(encoded("dinero-(pxeth)"), "dinero-%28pxeth%29");
        assert_eq!(encoded("aave-v3"), "aave-v3");
        assert_eq!(normalized(" Hyperliquid "), "hyperliquid");
    }

    #[test]
    fn a_list_reads_leniently_and_keeps_what_it_can() {
        let list = Protocols::read(&json!({
            "source": "defillama",
            "sourceUrl": "https://defillama.com",
            "fetchedAt": 1_795_273_958_390_u64,
            "stale": true,
            "total": 9086,
            "protocols": [
                {
                    "slug": "binance-cex", "name": "Binance CEX", "symbol": "BNB",
                    "category": "CEX", "chains": ["Ethereum", "Bitcoin", 7],
                    "tvl": 108_117_746_518.96, "change1h": -1.23, "change1d": "n/a",
                    "change7d": -6.75, "mcap": null, "parentSlug": null,
                },
                { "slug": "hyperliquid-perps", "name": "Hyperliquid Perps", "tvl": null,
                  "mcap": 10.0, "parentSlug": "hyperliquid" },
                { "name": "No slug" },
                "not a row",
            ],
        }))
        .unwrap();
        assert_eq!((list.total, list.protocols.len()), (9086, 2));
        assert!(list.stale);
        assert_eq!(list.fetched_at_ms, Some(1_795_273_958_390.0));
        let binance = &list.protocols[0];
        assert!(binance.is_cex());
        assert_eq!(binance.chains, ["Ethereum", "Bitcoin"]);
        assert_eq!((binance.change_1h, binance.change_1d), (Some(-1.23), None));
        assert_eq!(binance.mcap_to_tvl(), None);
        let perps = &list.protocols[1];
        assert_eq!(perps.parent_slug.as_deref(), Some("hyperliquid"));
        assert_eq!((perps.tvl, perps.mcap_to_tvl()), (None, None));
        // No list at all is the only failure; a total below the rows is not kept.
        assert!(Protocols::read(&json!({ "total": 3 })).is_err());
        let short = Protocols::read(&json!({ "total": 0, "protocols": [
            { "slug": "a", "name": "A", "tvl": 4.0, "mcap": 2.0 },
        ] }))
        .unwrap();
        assert_eq!(short.total, 1);
        assert_eq!(short.protocols[0].mcap_to_tvl(), Some(0.5));
        assert_eq!(short.source_url, SOURCE_URL);
    }

    #[test]
    fn a_protocol_reads_its_chains_largest_first_and_its_history_in_order() {
        let tvl = ProtocolTvl::read(&json!({
            "sourceUrl": "https://defillama.com/protocol/hyperliquid",
            "fetchedAt": 1_795_273_959_072_u64,
            "stale": false,
            "protocol": { "slug": "hyperliquid", "name": "Hyperliquid", "symbol": "HYPE",
                "category": null, "isParent": true, "mcap": 14_331_169_332.6 },
            "tvl": 7_151_116_329.0,
            "chainTvls": { "Arbitrum": 76_910_273.0, "Hyperliquid L1": 7_074_206_055.0, "Odd": "x" },
            "extraTvls": { "borrowed": 355_967_352.0 },
            "tvlHistory": [
                [1_686_268_800_000_u64, 3_578_746.0],
                [1_686_355_200_000_u64, 4_000_000.0],
                [1_686_355_200_000_u64, 5_000_000.0],
                ["later", 1.0],
                [1_795_268_687_000_u64, 7_151_116_329.0],
            ],
        }))
        .unwrap();
        assert!(tvl.protocol.is_parent && !tvl.protocol.is_cex());
        let chains: Vec<&str> = tvl.chains.iter().map(|chain| chain.name.as_str()).collect();
        assert_eq!(chains, ["Hyperliquid L1", "Arbitrum"]);
        assert_eq!(tvl.extras.len(), 1);
        // A point no later than the one before, or that is no pair of
        // numbers, is left out.
        assert_eq!(tvl.history.len(), 3);
        assert_eq!(tvl.history[1].usd, 4_000_000.0);
        assert_eq!(tvl.source_url, "https://defillama.com/protocol/hyperliquid");

        // One listed for its volume only: no figure, no history, no error.
        let none = ProtocolTvl::read(&json!({
            "sourceUrl": "https://example.com/protocol/variational",
            "protocol": { "slug": "variational", "name": "Variational", "category": "Derivatives" },
            "tvl": null, "chainTvls": {}, "extraTvls": {}, "tvlHistory": [],
        }))
        .unwrap();
        assert_eq!((none.tvl, none.history.len()), (None, 0));
        // A page that is not DefiLlama's is never what a press opens.
        assert_eq!(none.source_url, SOURCE_URL);
        assert!(ProtocolTvl::read(&json!({ "tvl": 1.0 })).is_err());
    }

    #[test]
    fn the_venues_defillama_lists_have_their_slug() {
        let listed: Vec<(&str, &str)> = venues().map(|(venue, slug)| (venue.id, slug)).collect();
        assert!(listed.contains(&("hyperliquid", "hyperliquid")));
        assert!(listed.contains(&("binance", "binance-cex")));
        assert!(listed.contains(&("apex", "apex-protocol")));
        assert!(listed.contains(&("lighterxyz", "lighter")));
        // A slug is as requests name it already.
        assert!(listed.iter().all(|(_, slug)| normalized(slug) == *slug));
    }
}
