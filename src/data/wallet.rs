//! The user's wallet: a browser wallet (Rabby, MetaMask, anything that
//! speaks EIP-1193) whose account the app asks for and which signs for it.
//! The wallet's key is never in the app: what needs it is asked of the
//! wallet, which asks its user.
//!
//! One connection, the same on both builds, in three layers:
//!
//! - What reaches the wallets ([`Connector`]). On the web it is the page
//!   itself (`web.rs`): the wallets announce themselves (EIP-6963), or
//!   there is `window.ethereum`. A desktop window has no extensions, so
//!   there the app serves a small page on this machine and opens it in the
//!   user's browser, where the wallet is; the page hands the app's requests
//!   to the wallet and its answers back (`bridge.rs`, `bridge.html`).
//!   Either one answers the requests that come down a [`Link`] and reports
//!   what the wallets say of themselves ([`Event`]).
//! - alloy over that link: [`WalletTransport`] is alloy's transport, so
//!   alloy's provider works against the wallet as it does against a node,
//!   and [`WalletSigner`] is alloy's `Signer`, for what the wallet signs.
//! - [`Wallet`], the service the app sees: what the connection is doing
//!   ([`WalletState`], a signal) and what the user can do about it
//!   (connect, choose a wallet, disconnect).
//!
//! Connecting: the wallets are looked for; one alone is asked for its
//! accounts at once (`eth_requestAccounts`, which prompts there), several
//! are offered to choose from; the first account it shares and the chain it
//! is on are the connection. The wallet's own `accountsChanged` and
//! `chainChanged` keep it current, and an empty `accountsChanged` (the
//! user disconnected the site in the wallet) ends it. Nothing is restored
//! on the next start: a connection lasts while the app runs.
//!
//! The wallet knows no exchange. What it signs for one is asked by that
//! exchange's side of trading ([`super::trading`]), which is handed the
//! connected account's signer ([`Wallet::signer`]): the wallet's user is
//! shown what is signed and approves it there, each time.

#[cfg(not(target_arch = "wasm32"))]
mod bridge;
mod link;
mod signer;
#[cfg(test)]
pub(crate) mod testing;
#[cfg(test)]
mod tests;
mod transport;
#[cfg(target_arch = "wasm32")]
mod web;

use std::{any::Any, cell::RefCell, fmt, rc::Rc};

use alloy::{
    primitives::Address,
    providers::{Provider, RootProvider},
    rpc::client::{NoParams, RpcClient},
    transports::TransportError,
};
use nsh_core::prelude::*;
use serde::Deserialize;
use serde_json::Value;

pub(crate) use self::link::{Call, Link};
use self::transport::WalletTransport;
pub use self::{link::Refusal, signer::WalletSigner};
use super::net::{TaskHandle, sanitize};

/// The longest a wallet's name is kept and shown, in characters, and the
/// longest of what it says when it refuses.
const NAME_LIMIT: usize = 64;
const SAID_LIMIT: usize = 240;

/// A wallet there is to connect: one that announced itself (EIP-6963), or
/// the browser's `window.ethereum`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct WalletInfo {
    /// What tells it from the others: the name it registered (EIP-6963's
    /// `rdns`, `io.rabby`), which a reloaded page finds it by again; its
    /// announcement's own id where it registered none.
    pub id: String,
    /// What it calls itself ("Rabby Wallet").
    pub name: String,
}

/// Where the wallets are, which is what the user is told to expect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reach {
    /// In this page: the web build.
    Page,
    /// In the user's browser, on a page the app opens there: the desktop
    /// build.
    Browser,
}

/// What a backend reports of the wallets, always from a task of its own,
/// never from inside [`Connector::open`].
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Event {
    /// The page the user's browser is to open: the desktop bridge's.
    #[cfg_attr(
        target_arch = "wasm32",
        allow(dead_code, reason = "the web build's wallets are in its own page")
    )]
    Page(String),
    /// The wallets there are, once they were looked for, and again
    /// whenever that changes.
    Wallets(Vec<WalletInfo>),
    /// The wallet `wallet`'s `accountsChanged`, as it came.
    Accounts { wallet: String, accounts: Value },
    /// The wallet `wallet`'s `chainChanged`, as it came.
    Chain { wallet: String, chain: Value },
    /// The wallets cannot be reached any more, and why.
    #[cfg_attr(
        target_arch = "wasm32",
        allow(dead_code, reason = "the web build's wallets are in its own page")
    )]
    Lost(String),
}

/// Where a backend's [`Event`]s go.
pub(crate) type Events = Rc<dyn Fn(Event)>;

/// A backend at work: the link its wallets are asked through, and what
/// keeps it running. Dropping it ends it.
pub(crate) struct Session {
    link: Link,
    _running: Box<dyn Any>,
}

impl Session {
    /// A session asked through `link` that lasts as long as `running`.
    pub(crate) fn new(link: Link, running: impl Any) -> Self {
        Self {
            link,
            _running: Box::new(running),
        }
    }
}

/// What reaches the wallets: the platform's, or a test's stand-in.
pub(crate) trait Connector: 'static {
    fn reach(&self) -> Reach;

    /// Starts looking for wallets. What it finds comes through `events`
    /// until the session drops.
    fn open(&self, events: Events) -> Result<Session, String>;
}

/// Which wallets the app's are.
#[derive(Clone, Default)]
pub struct WalletConfig {
    /// A stand-in for the platform's wallets (a test's).
    pub(crate) connector: Option<Rc<dyn Connector>>,
}

impl fmt::Debug for WalletConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let connector = if self.connector.is_some() {
            "a stand-in"
        } else {
            "the platform's"
        };
        formatter
            .debug_struct("WalletConfig")
            .field("connector", &connector)
            .finish()
    }
}

/// The platform's connector: the page's wallets on the web, the bridge to
/// the user's browser everywhere else.
fn platform_connector() -> Rc<dyn Connector> {
    #[cfg(target_arch = "wasm32")]
    return Rc::new(web::PageWallets);
    #[cfg(not(target_arch = "wasm32"))]
    return Rc::new(bridge::Bridge::default());
}

/// The connected account.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    pub wallet: WalletInfo,
    pub address: Address,
    /// The chain the wallet is on, which typed data it signs must name.
    pub chain_id: u64,
}

impl Account {
    /// The address as exchanges are asked about it: `0x` and forty
    /// hexadecimal digits in lower case.
    pub fn address_text(&self) -> String {
        format!("{:#x}", self.address)
    }
}

/// How far a connection has come.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Phase {
    #[default]
    Disconnected,
    /// Looking for wallets.
    Finding,
    /// There is more than one wallet, or the one there is was not
    /// connected: the user chooses.
    Choosing(Vec<WalletInfo>),
    /// The wallet was asked for its accounts and is asking its user.
    Approving(WalletInfo),
    Connected(Account),
}

/// Everything the app shows of the wallet.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WalletState {
    pub phase: Phase,
    /// The page the user's browser has open, or is to open, while the
    /// wallets are reached through it ([`Reach::Browser`]).
    pub page: Option<String>,
    /// What last went wrong, until something else is tried.
    pub failure: Option<String>,
}

/// The wallet service. Cloning is cheap: every clone is the same wallet.
#[derive(Clone)]
pub struct Wallet(Rc<Inner>);

struct Inner {
    connector: Rc<dyn Connector>,
    state: Signal<WalletState>,
    /// The backend, from Connect until the connection ends.
    session: RefCell<Option<Session>>,
    /// The wallets the backend last reported.
    wallets: RefCell<Vec<WalletInfo>>,
    /// alloy's provider over the chosen wallet.
    provider: RefCell<Option<RootProvider>>,
    /// The request for the wallet's accounts, while it waits for the user.
    approving: RefCell<Option<TaskHandle>>,
}

impl Wallet {
    /// A wallet service under the current owner (the shell), reaching the
    /// wallets as `config` says. Nothing is looked for until the user
    /// connects.
    pub(crate) fn new(config: WalletConfig) -> Self {
        Self(Rc::new(Inner {
            connector: config.connector.unwrap_or_else(platform_connector),
            state: signal(WalletState::default()),
            session: RefCell::new(None),
            wallets: RefCell::new(Vec::new()),
            provider: RefCell::new(None),
            approving: RefCell::new(None),
        }))
    }

    /// Where the wallets are.
    pub fn reach(&self) -> Reach {
        self.0.connector.reach()
    }

    /// Reads the state (tracked).
    pub fn with<R>(&self, read: impl FnOnce(&WalletState) -> R) -> R {
        self.0.state.with(read)
    }

    /// The connected account, if there is one (tracked).
    pub fn account(&self) -> Option<Account> {
        self.with(|state| match &state.phase {
            Phase::Connected(account) => Some(account.clone()),
            _ => None,
        })
    }

    /// What signs for the connected account as it is now (its address and
    /// the chain its wallet is on): every signature it makes is the user's
    /// approval in the wallet. `None` while nothing is connected.
    pub(crate) fn signer(&self) -> Option<WalletSigner> {
        let account = self.connected()?;
        let provider = self.0.provider.borrow().clone()?;
        Some(WalletSigner::new(
            provider,
            account.address,
            Some(account.chain_id),
        ))
    }

    /// Starts connecting: looks for the wallets. Does nothing while a
    /// connection is being made or stands.
    pub fn connect(&self) {
        if self.phase() != Phase::Disconnected {
            return;
        }
        let inner = Rc::downgrade(&self.0);
        let events: Events = Rc::new(move |event| {
            if let Some(inner) = inner.upgrade() {
                Self(inner).handle(event);
            }
        });
        match self.0.connector.open(events) {
            Ok(session) => {
                *self.0.session.borrow_mut() = Some(session);
                self.0.state.set(WalletState {
                    phase: Phase::Finding,
                    ..WalletState::default()
                });
            }
            Err(failure) => self.end(Some(failure)),
        }
    }

    /// Asks the wallet `id`, one of those being chosen from, for its
    /// accounts.
    pub fn choose(&self, id: &str) {
        let Phase::Choosing(wallets) = self.phase() else {
            return;
        };
        if let Some(wallet) = wallets.into_iter().find(|wallet| wallet.id == id) {
            self.approve(wallet);
        }
    }

    /// Ends the connection, or the making of one. The wallet still lists
    /// the app among its connected sites, as it does any site: only the
    /// wallet forgets that.
    pub fn disconnect(&self) {
        self.end(None);
    }

    /// Opens the bridge's page in the user's browser again (it was closed,
    /// or opened in a browser without the wallet).
    pub fn reopen_page(&self) {
        if let Some(page) = self.0.state.with_untracked(|state| state.page.clone()) {
            // Whether it opened changes nothing here.
            drop(open_url(page));
        }
    }

    /// Ends everything in flight (the app is going away).
    pub(crate) fn shutdown(&self) {
        self.0.approving.take();
        self.0.provider.take();
        self.0.session.take();
    }

    fn phase(&self) -> Phase {
        self.0.state.with_untracked(|state| state.phase.clone())
    }

    /// The connected account as it is now, if there is one (untracked).
    pub(crate) fn connected(&self) -> Option<Account> {
        match self.phase() {
            Phase::Connected(account) => Some(account),
            _ => None,
        }
    }

    /// Back to disconnected, saying `failure` if that is why.
    fn end(&self, failure: Option<String>) {
        self.shutdown();
        self.0.wallets.borrow_mut().clear();
        self.0.state.set(WalletState {
            failure,
            ..WalletState::default()
        });
    }

    /// Offers the wallets there are again, saying `failure`: what follows
    /// a connection that was not made.
    fn offer(&self, failure: String) {
        let wallets = self.0.wallets.borrow().clone();
        self.0.state.update(|state| {
            state.phase = if wallets.is_empty() {
                Phase::Finding
            } else {
                Phase::Choosing(wallets)
            };
            state.failure = Some(failure);
        });
    }

    fn handle(&self, event: Event) {
        match event {
            Event::Page(page) => {
                self.0.state.update(|state| state.page = Some(page.clone()));
                // Whether it opened changes nothing here: the panel shows
                // the link to open by hand.
                drop(open_url(page));
            }
            Event::Wallets(wallets) => self.found(wallets),
            Event::Accounts { wallet, accounts } => {
                let Some(account) = self.account_of(&wallet) else {
                    return;
                };
                match addresses(&accounts).first() {
                    None => self.end(Some(format!(
                        "{} disconnected Lattice.",
                        account.wallet.name
                    ))),
                    Some(&address) if address != account.address => {
                        // Another account: what the old one allowed (an
                        // exchange's trading) is not this one's, which
                        // trading sees from the state.
                        self.0.state.update(|state| {
                            state.phase = Phase::Connected(Account { address, ..account });
                        });
                    }
                    Some(_) => {}
                }
            }
            Event::Chain { wallet, chain } => {
                let (Some(account), Some(chain_id)) = (self.account_of(&wallet), chain_id(&chain))
                else {
                    return;
                };
                if chain_id != account.chain_id {
                    self.0.state.update(|state| {
                        state.phase = Phase::Connected(Account {
                            chain_id,
                            ..account
                        });
                    });
                }
            }
            Event::Lost(reason) => self.end(Some(reason)),
        }
    }

    /// The connected account when it is the wallet `wallet`'s.
    fn account_of(&self, wallet: &str) -> Option<Account> {
        self.connected()
            .filter(|account| account.wallet.id == wallet)
    }

    /// The backend's wallets, new or changed.
    fn found(&self, mut wallets: Vec<WalletInfo>) {
        for wallet in &mut wallets {
            wallet.name = named(&wallet.name);
        }
        self.0.wallets.borrow_mut().clone_from(&wallets);
        match self.phase() {
            Phase::Finding => match wallets.as_slice() {
                [] => {
                    let failure = match self.reach() {
                        Reach::Page => {
                            "No wallet was found in this browser. Install or unlock one \
                             (Rabby, MetaMask), then connect again."
                        }
                        Reach::Browser => {
                            "No wallet was found in the browser that opened. Open the link \
                             in the browser your wallet is installed in."
                        }
                    };
                    self.0
                        .state
                        .update(|state| state.failure = Some(failure.to_owned()));
                }
                [only] => self.approve(only.clone()),
                _ => self.0.state.update(|state| {
                    state.phase = Phase::Choosing(wallets);
                    state.failure = None;
                }),
            },
            Phase::Choosing(_) if !wallets.is_empty() => {
                self.0
                    .state
                    .update(|state| state.phase = Phase::Choosing(wallets));
            }
            _ => {}
        }
    }

    /// Asks `wallet` for its accounts, which prompts its user, and connects
    /// to the first it shares.
    fn approve(&self, wallet: WalletInfo) {
        let Some(link) = self
            .0
            .session
            .borrow()
            .as_ref()
            .map(|session| session.link.clone())
        else {
            return;
        };
        // Not "local": alloy polls a local node four times a second where
        // it polls at all (for a transaction's receipt), and what answers
        // here is the wallet's node, by way of the wallet.
        let client = RpcClient::new(WalletTransport::new(link, &wallet.id), false);
        let provider: RootProvider = RootProvider::new(client);
        self.0.state.update(|state| {
            state.phase = Phase::Approving(wallet.clone());
            state.failure = None;
        });
        let this = self.clone();
        let task = TaskHandle::spawn(async move {
            let asked = async {
                let accounts: Vec<Address> = provider
                    .raw_request("eth_requestAccounts".into(), NoParams::default())
                    .await?;
                let chain_id = provider.get_chain_id().await?;
                Ok::<_, TransportError>((accounts, chain_id))
            };
            match asked.await {
                Ok((accounts, chain_id)) => match accounts.first() {
                    Some(&address) => {
                        *this.0.provider.borrow_mut() = Some(provider);
                        this.0.state.update(|state| {
                            state.phase = Phase::Connected(Account {
                                wallet,
                                address,
                                chain_id,
                            });
                            state.failure = None;
                        });
                    }
                    None => this.offer(format!("{} shared no account.", wallet.name)),
                },
                Err(error) if error.is_transport_error() => this.end(Some(describe(&error))),
                Err(error) => this.offer(describe(&error)),
            }
        });
        *self.0.approving.borrow_mut() = Some(task);
    }
}

/// A wallet's name as the app shows it. A wallet names itself, so its name
/// is text from outside the app: cleaned, cut, and never nothing.
fn named(name: &str) -> String {
    let name = sanitize(name, NAME_LIMIT);
    if name.is_empty() {
        "Wallet".to_owned()
    } else {
        name
    }
}

/// The addresses of an `accountsChanged` (or an `eth_accounts`): a list of
/// addresses as text. What is no address is left out.
fn addresses(accounts: &Value) -> Vec<Address> {
    accounts
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|account| account.as_str()?.parse().ok())
        .collect()
}

/// The chain of a `chainChanged`: its id in hexadecimal text (`0xa4b1`), or
/// as some wallets send it, a number or decimal text.
fn chain_id(chain: &Value) -> Option<u64> {
    match chain {
        Value::Number(number) => number.as_u64(),
        Value::String(text) => match text.strip_prefix("0x") {
            Some(digits) => u64::from_str_radix(digits, 16).ok(),
            None => text.parse().ok(),
        },
        _ => None,
    }
}

/// What a request the wallet did not grant says to the user. The wallet's
/// own words are text from outside the app: cleaned and cut ([`sanitize`]).
fn describe(error: &TransportError) -> String {
    let Some(refusal) = error.as_error_resp() else {
        return if error.is_transport_error() {
            "The wallet can no longer be reached.".to_owned()
        } else {
            // alloy's error quotes the answer it could not read.
            let unread = format!("The wallet's answer could not be read: {error}");
            sanitize(&unread, SAID_LIMIT)
        };
    };
    let said = sanitize(&refusal.message, SAID_LIMIT);
    match refusal.code {
        Refusal::REJECTED => "Rejected in the wallet.".to_owned(),
        Refusal::PENDING => {
            "The wallet already has a request waiting. Open the wallet and answer it first."
                .to_owned()
        }
        Refusal::UNSUPPORTED => "The wallet does not support this request.".to_owned(),
        code if said.is_empty() => format!("The wallet refused (code {code})."),
        _ => said,
    }
}

/// [`describe`] for a signature that was not made.
pub(crate) fn describe_signing(error: &alloy::signers::Error) -> String {
    match error {
        alloy::signers::Error::Other(source) => match source.downcast_ref::<TransportError>() {
            Some(error) => describe(error),
            None => sanitize(&source.to_string(), SAID_LIMIT),
        },
        error => {
            let unread = format!("The wallet's signature could not be read: {error}");
            sanitize(&unread, SAID_LIMIT)
        }
    }
}
