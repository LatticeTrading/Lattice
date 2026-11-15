//! The data layer: the external APIs Lattice talks to, and the shared
//! plumbing that lets many widgets use one connection.
//!
//! - [`net`] knows nothing about any one API: an HTTP client with deadlines
//!   and one error model, one multiplexed WebSocket per API shared by every
//!   subscriber ([`net::SharedSocket`]), shared keyed polling and caching
//!   ([`net::Queries`]), the status board the connection indicator reads
//!   ([`net::StatusBoard`]), and who asked for what ([`net::Consumer`]),
//!   which the connection panel lists ([`DataServices::connections`]).
//! - [`ferris`] is the first integration built on it: Lattice's own market
//!   data backend, treated like any other external API.
//! - [`session`] gives each widget a session that owns its data for as long
//!   as the widget is in the workspace, whether or not it is on screen.
//! - [`markets`] is the app's one directory of every market Ferris lists,
//!   which the Everything Bar's search and the market picker both read.
//!
//! The shell makes one [`DataServices`] and provides it as context; a
//! widget reaches it with [`DataServices::get`]. Nothing connects until
//! something asks for data, and the last consumer leaving lets the
//! connection go.

pub mod ferris;
mod keyed;
pub mod markets;
pub mod net;
pub mod session;
mod shared;

pub(crate) use keyed::Keyed;
pub use markets::Markets;
pub use session::{SessionScope, Sessions, WidgetId, WidgetView};
pub use shared::Shared;

use nsh_core::prelude::*;

/// The app's data services, shared by every widget. Cloning is cheap: every
/// clone is the same services.
#[derive(Clone)]
pub struct DataServices {
    /// What each source reports for the connection indicator.
    pub status: net::StatusBoard,
    /// Lattice's market data backend.
    pub ferris: ferris::Ferris,
    /// Every widget's session (what a widget streams and keeps).
    pub sessions: Sessions,
    /// Every market Ferris lists, shared by whatever offers or ranks them.
    pub markets: Markets,
}

impl DataServices {
    /// Makes the services under the current owner (the shell) and provides
    /// them to everything made under it from now on. They shut down with
    /// that owner.
    pub fn provide() -> Self {
        Self::provide_with(ferris::FerrisConfig::default())
    }

    /// [`Self::provide`] with Ferris at `config` (a test's backend).
    pub fn provide_with(config: ferris::FerrisConfig) -> Self {
        let status = net::StatusBoard::new();
        let ferris = ferris::Ferris::new(config, status).unwrap_or_else(|error| {
            log::error!("Lattice: invalid Ferris configuration ({error}); using production");
            ferris::Ferris::new(ferris::FerrisConfig::production(), status)
                .expect("the production Ferris configuration is valid")
        });
        let sessions = Sessions::new();
        let markets = Markets::new(ferris.clone());
        let services = Self {
            status,
            ferris,
            sessions,
            markets,
        };
        provide_context(services.clone());
        let shutdown = services.clone();
        cleanup(move || {
            // Sessions first: they hold the subscriptions and watches.
            shutdown.sessions.shutdown();
            shutdown.ferris.shutdown();
        });
        services
    }

    /// The services the shell provided. Every component under the shell can
    /// call it.
    pub fn get() -> Self {
        Self::try_get().expect("DataServices::provide runs in the shell, above every widget")
    }

    /// The services, or `None` outside the shell (a component rendered on
    /// its own, as some tests do).
    pub fn try_get() -> Option<Self> {
        use_context::<Self>()
    }

    /// Every source in full, as it stands now: what the connection panel
    /// lists while it is open. [`Self::status`] says when a source's health
    /// changed; this reads what each carries and for whom, from the sources
    /// themselves. A new integration adds its sources here.
    pub fn connections(&self) -> Vec<net::SourceDetails> {
        self.ferris.sources()
    }
}
