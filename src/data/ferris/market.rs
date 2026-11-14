//! One market as a widget saves it, and the Ferris topics it streams and
//! fetches on (the old app's `MarketSelection` and the routing half of
//! `marketIdentity.ts`).
//!
//! A selection is saved under the old app's member names. Its request
//! symbol is what Ferris is asked for; a few venues' saved symbols are
//! known aliases of what the server streams under (Apex `BTC/USDT` for
//! `BTC/USDT:USDT`, KuCoin and Bitfinex pairs without their settlement),
//! and are repaired here, where the market is selected, never by loosening
//! the socket's routing.
//!
//! A catalog market (`fetchMarkets`) becomes a selection through
//! [`MarketSelection::from_catalog`] (the old `normalizeMarketRow` and
//! `deriveMarketRequestSymbol`, with the corrections live Ferris called
//! for). Not ported: the old app's legacy selections
//! (`createLegacyMarketSelection`); Lattice never saved any.
//!
//! - [`selection`]: the saved selection (`MarketSelection`, `MarketType`).
//! - [`catalog_row`]: a catalog row to a selection (`from_catalog`).
//! - [`routing`]: topics, params, route keys and the venues' symbol
//!   repairs.
//! - [`expiry`]: date and symbol text helpers.

mod catalog_row;
mod expiry;
mod routing;
mod selection;

pub(super) use catalog_row::{Derived, extended_symbol, hyperliquid_symbol};
pub(super) use routing::{Requested, apex_symbol, bitfinex_symbol, kucoin_symbol};
pub use selection::{MarketKey, MarketSelection, MarketType};
