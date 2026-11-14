//! What the picker can offer: each exchange's markets from the catalogs
//! that arrived, what is loading and what failed, and the results for a
//! tab and a search.

use std::rc::Rc;

use super::sources::Got;
use crate::{
    data::{
        ferris::{
            Availability, Capabilities, CatalogMarket, Requirement, SymbolQuery, merge_catalogs,
        },
        net::{ApiError, QueryState},
    },
    exchanges,
};

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
    pub fn new(
        capabilities: &QueryState<Capabilities>,
        got: &[Got],
        requirement: Requirement,
        only: Option<&str>,
    ) -> Self {
        let mut listing = Self::default();
        let Some(document) = capabilities.data.as_deref() else {
            listing.loading = capabilities.error.is_none();
            if let Some(error) = &capabilities.error {
                listing
                    .failed
                    .push(("the exchange list".to_owned(), error.clone()));
            }
            return listing;
        };
        let mut ids: Vec<&str> = got.iter().map(|got| got.exchange.as_str()).collect();
        ids.sort_by_key(|id| exchanges::rank(id));
        ids.dedup();
        for id in ids {
            if only.is_some_and(|only| only != id) {
                continue;
            }
            let scopes: Vec<&Got> = got.iter().filter(|got| got.exchange == id).collect();
            listing.loading |= scopes
                .iter()
                .any(|got| got.data.is_none() && got.error.is_none());
            if let Some(error) = scopes
                .iter()
                .find(|got| got.data.is_none())
                .and_then(|got| got.error.clone())
            {
                listing
                    .failed
                    .push((exchanges::display_name(id).to_owned(), error));
            }
            let markets: Vec<Rc<CatalogMarket>> = merge_catalogs(
                scopes
                    .iter()
                    .filter_map(|got| got.data.as_ref())
                    .map(|catalog| &catalog.markets),
            )
            .into_iter()
            .filter(|market| {
                requirement.availability(document, id, market.selection.kind.product())
                    != Availability::Unsupported
            })
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
