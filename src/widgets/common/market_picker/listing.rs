//! What the picker can offer: of the app's market directory, each
//! exchange's markets its requirement allows, what is loading and what
//! failed, and the results for a tab and a search.

use std::rc::Rc;

use crate::{
    data::{
        ferris::{Availability, CatalogMarket, Product, Requirement, SymbolQuery},
        markets::Directory,
        net::ApiError,
    },
    exchanges,
};

/// The products a catalog can list (options are never asked for).
const PRODUCTS: [Product; 3] = [Product::Perp, Product::Future, Product::Spot];

/// What the picker can offer, from what has arrived.
#[derive(Default)]
pub(super) struct Listing {
    /// Each exchange with markets on offer, in the old app's order, and
    /// its markets in list order.
    pub exchanges: Vec<(String, Vec<Rc<CatalogMarket>>)>,
    /// Every exchange's markets, exchange after exchange.
    pub all: Vec<Rc<CatalogMarket>>,
    /// Something has not answered yet.
    pub loading: bool,
    /// What could not load and why: exchange names, or the capabilities.
    pub failed: Vec<(String, ApiError)>,
}

impl Listing {
    /// What `directory` has for a picker of `only` (one exchange, or
    /// every one) that needs `requirement`: markets whose exchange and
    /// product are denied it are left out, and so is an exchange with none
    /// of its products on offer (what it is loading or failed to load is
    /// not this picker's business).
    pub fn new(directory: &Directory, requirement: Requirement, only: Option<&str>) -> Self {
        let mut listing = Self::default();
        let Some(document) = directory.capabilities.as_deref() else {
            listing.loading = directory.error.is_none();
            if let Some(error) = &directory.error {
                listing
                    .failed
                    .push(("the exchange list".to_owned(), error.clone()));
            }
            return listing;
        };
        for listed in &directory.exchanges {
            let id = listed.exchange.as_str();
            if only.is_some_and(|only| only != id) {
                continue;
            }
            let offered = |product: Product| {
                requirement.availability(document, id, product) != Availability::Unsupported
            };
            if !PRODUCTS.into_iter().any(offered) {
                continue;
            }
            listing.loading |= listed.loading;
            if let Some(error) = &listed.error {
                listing
                    .failed
                    .push((exchanges::display_name(id).to_owned(), error.clone()));
            }
            let markets: Vec<Rc<CatalogMarket>> = listed
                .markets
                .iter()
                .filter(|market| offered(market.selection.kind.product()))
                .cloned()
                .collect();
            if !markets.is_empty() {
                listing.all.extend(markets.iter().cloned());
                listing.exchanges.push((id.to_owned(), markets));
            }
        }
        listing
    }

    /// The results for `tab` (`None`: every exchange) and `query`: every
    /// market in list order with nothing typed, else those matching, best
    /// first, ties in list order.
    pub fn shown(&self, tab: Option<&str>, query: &SymbolQuery) -> Vec<Rc<CatalogMarket>> {
        let markets = match tab {
            None => &self.all,
            Some(tab) => match self.exchanges.iter().find(|(id, _)| id == tab) {
                Some((_, markets)) => markets,
                None => &self.all,
            },
        };
        if query.is_empty() {
            return markets.clone();
        }
        let mut scored: Vec<(u8, &Rc<CatalogMarket>)> = markets
            .iter()
            .map(|market| (query.score(market), market))
            .filter(|(score, _)| *score > 0)
            .collect();
        // Stable: equal scores keep list order.
        scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
        scored
            .into_iter()
            .map(|(_, market)| market.clone())
            .collect()
    }
}
