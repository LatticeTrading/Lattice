//! The picker's demand and what it got: the capabilities and one shared
//! catalog watch per scope they call for, and each scope's latest result.

use std::{cell::RefCell, rc::Rc};

use nsh_core::prelude::*;

use crate::{
    data::{
        ferris::{
            Availability, Capabilities, Catalog, CatalogScope, Ferris, Requirement, catalog_scopes,
        },
        net::{ApiError, QueryState, Watch},
    },
    widgets::common::Keyed,
};

/// One catalog scope's latest result: what the listing is made of (a fetch
/// starting changes nothing).
#[derive(Clone)]
pub(super) struct Got {
    pub exchange: String,
    pub data: Option<Rc<Catalog>>,
    pub error: Option<ApiError>,
}

impl PartialEq for Got {
    fn eq(&self, other: &Self) -> bool {
        let same_data = match (&self.data, &other.data) {
            (Some(a), Some(b)) => Rc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        same_data && self.exchange == other.exchange && self.error == other.error
    }
}

/// The picker's demand: the capabilities and one catalog watch per scope
/// they call for, made under the picker's owner and let go with it.
#[derive(Clone)]
pub(super) struct Sources {
    pub capabilities: ReadSignal<QueryState<Capabilities>>,
    capabilities_watch: Rc<Watch<Capabilities>>,
    catalogs: Rc<Keyed<String, (CatalogScope, Watch<Catalog>)>>,
    /// Each scope's latest result.
    pub results: Memo<Vec<Got>>,
}

impl Sources {
    pub fn follow(ferris: &Ferris, exchange: Option<String>, requirement: Requirement) -> Self {
        let owner = Owner::current();
        let capabilities_watch = Rc::new(ferris.watch_capabilities());
        let capabilities = capabilities_watch.state();
        let catalogs: Rc<Keyed<String, (CatalogScope, Watch<Catalog>)>> = Rc::default();
        let feeds = signal(Vec::<(CatalogScope, ReadSignal<QueryState<Catalog>>)>::new());
        {
            let (ferris, catalogs, exchange) = (ferris.clone(), catalogs.clone(), exchange.clone());
            let followed = RefCell::new(None::<Rc<Capabilities>>);
            effect(move || {
                let Some(document) = capabilities.with(|state| state.data.clone()) else {
                    return;
                };
                // Refetched every few minutes: the same document changes
                // nothing.
                if followed
                    .borrow()
                    .as_ref()
                    .is_some_and(|followed| Rc::ptr_eq(followed, &document))
                {
                    return;
                }
                *followed.borrow_mut() = Some(document.clone());
                untracked(|| {
                    let scopes = catalog_scopes(&document, exchange.as_deref(), |id, product| {
                        requirement.availability(&document, id, product)
                            != Availability::Unsupported
                    });
                    // Scopes no longer called for are let go.
                    catalogs.follow(
                        scopes.into_iter().map(|scope| (scope.key.clone(), scope)),
                        |(held, _), scope| {
                            *held = scope.clone();
                            true
                        },
                        |scope| {
                            let watch =
                                owner.run(|| ferris.queries().watch(ferris.catalog_query(scope)));
                            (scope.clone(), watch)
                        },
                    );
                    feeds.set(catalogs.with(|held| {
                        held.iter()
                            .map(|(_, (scope, watch))| (scope.clone(), watch.state()))
                            .collect()
                    }));
                });
            });
        }
        let results = computed(move || {
            feeds.with(|feeds| {
                feeds
                    .iter()
                    .map(|(scope, state)| {
                        state.with(|state| Got {
                            exchange: scope.exchange.clone(),
                            data: state.data.clone(),
                            error: state.error.clone(),
                        })
                    })
                    .collect()
            })
        });
        Self {
            capabilities,
            capabilities_watch,
            catalogs,
            results,
        }
    }

    /// Asks again for what failed.
    pub fn retry(&self) {
        if self
            .capabilities
            .with_untracked(|state| state.error.is_some())
        {
            self.capabilities_watch.refresh();
        }
        self.catalogs.with(|held| {
            for (_, (_, watch)) in held {
                if watch.state().with_untracked(|state| state.error.is_some()) {
                    watch.refresh();
                }
            }
        });
    }
}
