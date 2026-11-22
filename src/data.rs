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
//! - [`hyperliquid`] is the second: Hyperliquid's own API, asked by the app
//!   itself for what Ferris does not carry (accounts), within the user's
//!   own rate limit there.
//! - [`lighter`] is the third: Lighter's own API, asked the same way for
//!   the same reason.
//! - [`txflow`] is the fourth: TxFlow's own API, asked the same way again.
//! - [`portfolio`] is what an account holds and has made, in one shape
//!   whatever its exchange, and the exchanges that report accounts
//!   ([`Portfolio`]): what the portfolio widgets read.
//! - [`prices`] is what an instrument is priced at on an exchange now,
//!   followed live, whichever stream prices that exchange ([`Prices`]):
//!   what values positions between reports and what a ticket shows.
//! - [`session`] gives each widget a session that owns its data for as long
//!   as the widget is in the workspace, whether or not it is on screen.
//! - [`markets`] is the app's one directory of every market Ferris lists,
//!   which the Everything Bar's search and the market picker both read.
//! - [`wallet`] is the user's browser wallet: the account it shares and
//!   what it signs, asked through alloy on both builds.
//! - [`trading`] is placing and cancelling orders for that account, in one
//!   shape whatever its exchange, and the exchanges that can be traded on
//!   ([`Trading`]): what the trading widgets ask.
//!
//! The shell makes one [`DataServices`] and provides it as context; a
//! widget reaches it with [`DataServices::get`]. Nothing connects until
//! something asks for data, and the last consumer leaving lets the
//! connection go.

pub mod ferris;
pub mod hyperliquid;
mod keyed;
pub mod lighter;
pub mod markets;
pub mod net;
pub mod portfolio;
pub mod prices;
pub mod session;
mod shared;
pub mod trading;
pub mod txflow;
pub mod wallet;

pub(crate) use keyed::Keyed;
pub use markets::Markets;
pub use portfolio::Portfolio;
pub use prices::Prices;
pub use session::{SessionScope, Sessions, WidgetId, WidgetView};
pub use shared::Shared;
pub use trading::Trading;
pub use wallet::Wallet;

use std::rc::Rc;

use nsh_core::prelude::*;

/// Where each API the app talks to is: production's, a build's overrides, or
/// a test's stand-ins.
#[derive(Clone, Debug, Default)]
pub struct DataConfig {
    pub ferris: ferris::FerrisConfig,
    pub hyperliquid: hyperliquid::HyperliquidConfig,
    pub lighter: lighter::LighterConfig,
    pub txflow: txflow::TxFlowConfig,
    pub wallet: wallet::WalletConfig,
    /// One more exchange than the build's, for a test.
    #[cfg(test)]
    pub(crate) extra: ExtraExchange,
}

/// What makes one more exchange than the build's, for a test: given the
/// services' status board, how it reports accounts and how it is traded
/// on. It is listed after the build's own in both.
#[cfg(test)]
pub(crate) type MakesExchange = dyn Fn(
    net::StatusBoard,
) -> (
    Rc<dyn portfolio::AccountSource>,
    Rc<dyn trading::TradingVenue>,
);

/// [`DataConfig::extra`]: none, unless a test names one.
#[cfg(test)]
#[derive(Clone, Default)]
pub(crate) struct ExtraExchange(pub(crate) Option<Rc<MakesExchange>>);

#[cfg(test)]
impl std::fmt::Debug for ExtraExchange {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(if self.0.is_some() { "one more" } else { "none" })
    }
}

/// A Ferris of one's own, with everything else as the build has it.
impl From<ferris::FerrisConfig> for DataConfig {
    fn from(ferris: ferris::FerrisConfig) -> Self {
        Self {
            ferris,
            ..Self::default()
        }
    }
}

/// The app's data services, shared by every widget. Cloning is cheap: every
/// clone is the same services.
#[derive(Clone)]
pub struct DataServices {
    /// What each source reports for the connection indicator.
    pub status: net::StatusBoard,
    /// Lattice's market data backend.
    pub ferris: ferris::Ferris,
    /// Hyperliquid's own API, which the app asks itself.
    pub hyperliquid: hyperliquid::Hyperliquid,
    /// Lighter's own API, which the app asks itself.
    pub lighter: lighter::Lighter,
    /// TxFlow's own API, which the app asks itself.
    pub txflow: txflow::TxFlow,
    /// The accounts widgets watch, whichever exchange each is on.
    pub portfolio: Portfolio,
    /// What instruments are priced at now, whichever exchange each is on.
    pub prices: Prices,
    /// Every widget's session (what a widget streams and keeps).
    pub sessions: Sessions,
    /// Every market Ferris lists, shared by whatever offers or ranks them.
    pub markets: Markets,
    /// The user's wallet, once they connect one.
    pub wallet: Wallet,
    /// Orders for the wallet's account, whichever exchange they go to.
    pub trading: Trading,
}

impl DataServices {
    /// Makes the services under the current owner (the shell) and provides
    /// them to everything made under it from now on. They shut down with
    /// that owner.
    pub fn provide() -> Self {
        Self::provide_with(DataConfig::default())
    }

    /// [`Self::provide`] with the APIs at `config` (a test's backends; a
    /// [`FerrisConfig`](ferris::FerrisConfig) alone is that Ferris with the
    /// rest as built).
    pub fn provide_with(config: impl Into<DataConfig>) -> Self {
        let config = config.into();
        let status = net::StatusBoard::new();
        let ferris = ferris::Ferris::new(config.ferris, status).unwrap_or_else(|error| {
            log::error!("Lattice: invalid Ferris configuration ({error}); using production");
            ferris::Ferris::new(ferris::FerrisConfig::production(), status)
                .expect("the production Ferris configuration is valid")
        });
        let hyperliquid = hyperliquid::Hyperliquid::new(&config.hyperliquid, status)
            .unwrap_or_else(|error| {
                log::error!(
                    "Lattice: invalid Hyperliquid configuration ({error}); using production"
                );
                let production = hyperliquid::HyperliquidConfig::production();
                hyperliquid::Hyperliquid::new(&production, status)
                    .expect("the production Hyperliquid configuration is valid")
            });
        let lighter = lighter::Lighter::new(&config.lighter, status).unwrap_or_else(|error| {
            log::error!("Lattice: invalid Lighter configuration ({error}); using production");
            lighter::Lighter::new(&lighter::LighterConfig::production(), status)
                .expect("the production Lighter configuration is valid")
        });
        let txflow = txflow::TxFlow::new(&config.txflow, status).unwrap_or_else(|error| {
            log::error!("Lattice: invalid TxFlow configuration ({error}); using production");
            txflow::TxFlow::new(&txflow::TxFlowConfig::production(), status)
                .expect("the production TxFlow configuration is valid")
        });
        // The exchanges that report accounts, in the order widgets offer
        // them: a new one is its integration above and its line here.
        let sources: [Rc<dyn portfolio::AccountSource>; 3] = [
            Rc::new(hyperliquid.clone()),
            Rc::new(lighter.clone()),
            Rc::new(txflow.clone()),
        ];
        // The exchanges that can be traded on, in the order widgets offer
        // them: a new one is its integration above and its line here.
        let venues: [Rc<dyn trading::TradingVenue>; 1] = [Rc::new(hyperliquid.clone())];
        // A test's one more, after the build's own.
        #[cfg(test)]
        let extra = config.extra.0.as_ref().map(|make| make(status));
        #[cfg(not(test))]
        let extra: Option<(
            Rc<dyn portfolio::AccountSource>,
            Rc<dyn trading::TradingVenue>,
        )> = None;
        let (more_sources, more_venues) = extra.unzip();
        let sources = sources.into_iter().chain(more_sources);
        let venues = venues.into_iter().chain(more_venues);
        let portfolio = Portfolio::new(sources);
        // What prices each exchange's instruments, live: a new one is its
        // line here, the exchange's own integration or another stream of
        // its markets.
        let prices = Prices::new([Rc::new(hyperliquid.clone()) as Rc<dyn prices::PriceSource>]);
        let sessions = Sessions::new();
        let markets = Markets::new(ferris.clone());
        let wallet = Wallet::new(config.wallet);
        let trading = Trading::new(wallet.clone(), venues);
        let services = Self {
            status,
            ferris,
            hyperliquid,
            lighter,
            txflow,
            portfolio,
            prices,
            sessions,
            markets,
            wallet,
            trading,
        };
        provide_context(services.clone());
        let shutdown = services.clone();
        cleanup(move || {
            // Sessions first: they hold the subscriptions and watches.
            shutdown.sessions.shutdown();
            shutdown.trading.shutdown();
            shutdown.wallet.shutdown();
            shutdown.ferris.shutdown();
            shutdown.hyperliquid.shutdown();
            shutdown.lighter.shutdown();
            shutdown.txflow.shutdown();
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
        let mut sources = self.ferris.sources();
        sources.extend(self.hyperliquid.sources());
        sources.extend(self.lighter.sources());
        sources.extend(self.txflow.sources());
        sources
    }
}
