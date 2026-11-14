//! Every exchange Lattice knows, one record each ([`Venue`]), by Ferris's
//! exchange id (`bybit`): its name, place and logo, and the rules its
//! catalogs, topics and statistics follow. Code that differs by exchange
//! reads a record's flags instead of matching names; an exchange Lattice
//! does not know gets [`Venue::of`]'s defaults.
//!
//! [`VENUES`] is in the old app's order (`SUPPORTED_EXCHANGES`), which is
//! where lists of exchanges put them. Logos are the old app's artwork
//! (`exchangeLogoAssets.ts`), one SVG per exchange in `assets/exchanges/`,
//! inlined at compile time.
//!
//! A symbol rule that is truly one venue's quirk stays a function (in
//! [`super::market`]), which the venue's record names.

use std::time::Duration;

use super::{
    MarketType, Product, StatsFieldName,
    market::{self, Derived, Requested},
};

/// One catalog request profile: its `category` (none for `{}`) and the
/// products its reply lists.
pub type CatalogProfile = (Option<&'static str>, &'static [Product]);

/// A statistics partition of the fallback table: `<exchange>:<id>`, its
/// label, product and statistics parameters.
pub type StatsProfile = (
    &'static str,
    &'static str,
    Product,
    &'static [(&'static str, &'static str)],
);

/// Where an exchange's statistics put a market's 24h volume.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettlementVolume {
    /// In the quote slot, in the quote asset.
    Never,
    /// In the quote slot, but in the settlement asset, for every market.
    Always,
    /// In the settlement asset for coin-margined markets (settled in their
    /// base).
    CoinMargined,
}

/// One exchange and its rules.
#[derive(Debug)]
pub struct Venue {
    /// Ferris's id, lower case.
    pub id: &'static str,
    /// Other ids Ferris has served it under: its data rules apply to them
    /// too (not its name, place or logo).
    pub aliases: &'static [&'static str],
    /// Its name for people.
    pub name: &'static str,
    /// Its logo's SVG markup. Marks drawn in `currentColor` follow the text
    /// color they are drawn in; the others keep their brand colors.
    pub logo: &'static str,
    /// Every id containing its own shares its logo (`binanceusdm`,
    /// `hyperliquid-testnet`).
    pub logo_family: bool,
    /// Short names its logo is also filed under (`hl`).
    pub logo_aliases: &'static [&'static str],

    /// Its catalog's request profiles. Most list every product with `{}`;
    /// a few only list some unless asked for others by `category`.
    pub catalog_profiles: &'static [CatalogProfile],
    /// A catalog request's deadline.
    pub catalog_timeout: Duration,
    /// Catalog choices are told apart by catalog id, not by route
    /// (Hyperliquid: deployers can share a route).
    pub catalog_keys_by_id: bool,

    /// The products whose topics pick them by `type` (`{"type":"perp"}`);
    /// on such a venue the category never picks a product, and spot needs
    /// none. Elsewhere the category does (`{"category":"linear"}`).
    pub routes_by_type: &'static [MarketType],
    /// A REST snapshot is asked for by catalog id, not by symbol.
    pub rest_by_market_id: bool,
    /// Its derivatives come in linear and inverse families: their category
    /// is their family, and their symbols are settled by it
    /// (`BASE/QUOTE:SETTLE`, dated futures `-YYMMDD`).
    pub linear_inverse: bool,
    /// A dated future streams only with its exact day in the symbol (a
    /// month-code contract has none, so it is not offered).
    pub futures_need_day: bool,
    /// Its symbols carry no settlement (`SOL/USD`): a plain-pair perpetual
    /// is not given one.
    pub unsettled_symbols: bool,
    /// Its own request symbol for a catalog market, before the general
    /// rules; `None` from it falls through to them.
    pub(super) derive_symbol: Option<fn(&Derived<'_>) -> Option<String>>,
    /// Its repair of a request symbol's aliases; `None` from it keeps the
    /// symbol as requested.
    pub(super) repair_symbol: Option<fn(&Requested<'_>) -> Option<String>>,

    /// Its statistics partitions until the backend sends `partitions`.
    pub stats_profiles: &'static [StatsProfile],
    /// The statistics params sent for a scope that has none.
    pub stats_default_params: &'static [(&'static str, &'static str)],
    /// Fields it advertises but serves only for selected markets, never in
    /// bulk.
    pub stats_bulk_unsupported: &'static [StatsFieldName],
    /// A typed statistics scope (`{"type":..}`) is large whatever it asks
    /// for.
    pub stats_typed_scopes_large: bool,
    /// A large statistics request's deadline.
    pub stats_large_timeout: Duration,
    /// Where its statistics put 24h volume.
    pub settlement_volume: SettlementVolume,
}

impl Venue {
    /// The rules of `exchange` (any case, or one of its aliases); one
    /// Lattice does not know gets the defaults.
    pub fn of(exchange: &str) -> &'static Self {
        let id = exchange.trim();
        VENUES
            .iter()
            .find(|venue| {
                venue.id.eq_ignore_ascii_case(id)
                    || venue
                        .aliases
                        .iter()
                        .any(|alias| alias.eq_ignore_ascii_case(id))
            })
            .unwrap_or(&DEFAULT)
    }

    /// The venue whose id is exactly `id`, when Lattice knows it.
    pub fn listed(id: &str) -> Option<&'static Self> {
        VENUES.iter().find(|venue| venue.id == id)
    }
}

const fn secs(secs: u64) -> Duration {
    Duration::from_secs(secs)
}

/// An exchange Lattice does not know, and every record's starting point.
const DEFAULT: Venue = Venue {
    id: "",
    aliases: &[],
    name: "",
    logo: "",
    logo_family: false,
    logo_aliases: &[],
    catalog_profiles: &[(None, &[Product::Spot, Product::Perp, Product::Future])],
    catalog_timeout: secs(20),
    catalog_keys_by_id: false,
    routes_by_type: &[],
    rest_by_market_id: false,
    linear_inverse: false,
    futures_need_day: false,
    unsettled_symbols: false,
    derive_symbol: None,
    repair_symbol: None,
    stats_profiles: &[],
    stats_default_params: &[],
    stats_bulk_unsupported: &[],
    stats_typed_scopes_large: false,
    stats_large_timeout: secs(20),
    settlement_volume: SettlementVolume::Never,
};

macro_rules! logo {
    ($id:literal) => {
        include_str!(concat!("../../../assets/exchanges/", $id, ".svg"))
    };
}

const TYPE_PERP: &[(&str, &str)] = &[("type", "perp")];
const TYPE_FUTURE: &[(&str, &str)] = &[("type", "future")];
const TYPE_SPOT: &[(&str, &str)] = &[("type", "spot")];
const TYPE_OPTION: &[(&str, &str)] = &[("type", "option")];
const INVERSE_PERP: &[(&str, &str)] = &[("category", "inverse"), ("type", "perp")];
const INVERSE_FUTURE: &[(&str, &str)] = &[("category", "inverse"), ("type", "future")];

/// Statistics profiles (ported from the old app's `marketPartitions.ts`):
/// complete profiles, not selectors to merge into another profile.
const LINEAR_INVERSE: &[StatsProfile] = &[
    ("linear-perp", "Linear perpetuals", Product::Perp, TYPE_PERP),
    (
        "linear-future",
        "Linear futures",
        Product::Future,
        TYPE_FUTURE,
    ),
    (
        "inverse-perp",
        "Inverse perpetuals",
        Product::Perp,
        INVERSE_PERP,
    ),
    (
        "inverse-future",
        "Inverse futures",
        Product::Future,
        INVERSE_FUTURE,
    ),
    ("spot", "Spot", Product::Spot, TYPE_SPOT),
    ("option", "Options", Product::Option, TYPE_OPTION),
];
const PERP_SPOT: &[StatsProfile] = &[
    ("perp", "Perpetuals", Product::Perp, TYPE_PERP),
    ("spot", "Spot", Product::Spot, TYPE_SPOT),
];
const PERP: &[StatsProfile] = &[("perp", "Perpetuals", Product::Perp, TYPE_PERP)];
/// Perpl and Phoenix take no parameters: Phoenix rejects `type`, and both
/// catalogs are perpetuals only.
const UNTYPED_PERP: &[StatsProfile] = &[("perp", "Perpetuals", Product::Perp, &[])];

/// Every exchange Lattice knows, in the old app's order.
pub static VENUES: &[Venue] = &[
    Venue {
        id: "bybit",
        name: "Bybit",
        logo: logo!("bybit"),
        linear_inverse: true,
        futures_need_day: true,
        stats_profiles: LINEAR_INVERSE,
        // Bybit without parameters means its linear category, as the old
        // app sent.
        stats_default_params: &[("category", "linear")],
        ..DEFAULT
    },
    Venue {
        id: "binance",
        name: "Binance",
        logo: logo!("binance"),
        logo_family: true,
        // `{}` is USDT-M perpetuals and futures (checked live 2026-11-13).
        catalog_profiles: &[
            (None, &[Product::Perp, Product::Future]),
            (Some("inverse"), &[Product::Perp, Product::Future]),
            (Some("spot"), &[Product::Spot]),
        ],
        linear_inverse: true,
        futures_need_day: true,
        stats_profiles: LINEAR_INVERSE,
        // A bulk request for open interest is a `VALIDATION_ERROR`
        // ("requires selected marketIds").
        stats_bulk_unsupported: &[StatsFieldName::OpenInterest],
        ..DEFAULT
    },
    Venue {
        id: "hyperliquid",
        name: "Hyperliquid",
        logo: logo!("hyperliquid"),
        logo_family: true,
        logo_aliases: &["hl"],
        // Its catalog took 10-18s live (2026-11-13); the old app allowed
        // 20s for every catalog.
        catalog_timeout: secs(30),
        catalog_keys_by_id: true,
        derive_symbol: Some(market::hyperliquid_symbol),
        stats_profiles: &[
            ("linear-perp", "Linear perpetuals", Product::Perp, TYPE_PERP),
            ("spot", "Spot", Product::Spot, TYPE_SPOT),
        ],
        stats_typed_scopes_large: true,
        stats_large_timeout: secs(30),
        ..DEFAULT
    },
    Venue {
        id: "lighterxyz",
        aliases: &["lighter"],
        name: "Lighter",
        logo: logo!("lighterxyz"),
        logo_family: true,
        stats_profiles: PERP_SPOT,
        ..DEFAULT
    },
    Venue {
        id: "aster",
        name: "Aster",
        logo: logo!("aster"),
        // `{}` is perpetuals.
        catalog_profiles: &[(None, &[Product::Perp]), (Some("spot"), &[Product::Spot])],
        stats_profiles: PERP_SPOT,
        ..DEFAULT
    },
    Venue {
        id: "extended",
        name: "Extended",
        logo: logo!("extended"),
        derive_symbol: Some(market::extended_symbol),
        stats_profiles: PERP,
        settlement_volume: SettlementVolume::Always,
        ..DEFAULT
    },
    Venue {
        id: "apex",
        name: "Apex",
        logo: logo!("apex"),
        repair_symbol: Some(market::apex_symbol),
        stats_profiles: PERP,
        ..DEFAULT
    },
    Venue {
        id: "kucoin",
        name: "KuCoin",
        logo: logo!("kucoin"),
        logo_aliases: &["kucoinfutures"],
        routes_by_type: &[MarketType::Spot, MarketType::Perp, MarketType::Future],
        rest_by_market_id: true,
        futures_need_day: true,
        repair_symbol: Some(market::kucoin_symbol),
        stats_profiles: &[
            ("linear-perp", "Linear perpetuals", Product::Perp, TYPE_PERP),
            ("future", "Futures", Product::Future, TYPE_FUTURE),
            ("spot", "Spot", Product::Spot, TYPE_SPOT),
        ],
        settlement_volume: SettlementVolume::CoinMargined,
        ..DEFAULT
    },
    Venue {
        id: "bitfinex",
        name: "Bitfinex",
        logo: logo!("bitfinex"),
        routes_by_type: &[MarketType::Spot, MarketType::Perp],
        rest_by_market_id: true,
        repair_symbol: Some(market::bitfinex_symbol),
        stats_profiles: PERP_SPOT,
        ..DEFAULT
    },
    Venue {
        id: "nado",
        name: "Nado",
        logo: logo!("nado"),
        stats_profiles: PERP_SPOT,
        ..DEFAULT
    },
    Venue {
        id: "variational",
        name: "Variational",
        logo: logo!("variational"),
        stats_profiles: PERP,
        ..DEFAULT
    },
    Venue {
        id: "phoenix",
        name: "Phoenix",
        logo: logo!("phoenix"),
        unsettled_symbols: true,
        stats_profiles: UNTYPED_PERP,
        ..DEFAULT
    },
    Venue {
        id: "perpl",
        name: "Perpl",
        logo: logo!("perpl"),
        stats_profiles: UNTYPED_PERP,
        ..DEFAULT
    },
];
