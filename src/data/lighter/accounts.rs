//! Lighter's accounts as the portfolio asks for them ([`AccountSource`]):
//! how an address reads, and one shared request per report.
//!
//! - An account is an L1 address: `0x` and forty hexadecimal digits, written
//!   in lower case so one account is one request however it was typed.
//! - The address holds several accounts on Lighter, each known by its
//!   index: its main one and its sub-accounts
//!   ([`AccountStates`](super::state)). A report is of all of them
//!   together. An address Lighter knows no account of reads as an empty
//!   account, not as a failure.
//! - Every report starts from the address's accounts
//!   ([`Request::Accounts`]), which is one request however many reports are
//!   read from it: a query of its own in the hub, which each report's fetch
//!   asks the hub for and which is asked of Lighter again once it is
//!   [`ACCOUNTS_FRESH`] old. The margin and the balances are read from it
//!   outright; the history takes two more requests of each account (a week
//!   of hours and every day) and the trades one.
//! - Every report is refetched every five minutes ([`REFRESH`]) while
//!   watched. An address with one account whose every report is watched
//!   spends 5 requests a refresh (its accounts, 2 for its history, its
//!   trades and the prices), one with a sub-account 8.
//! - What it pays to trade, has staked and holds of pools are not
//!   reported: the widgets that show them leave them out.
//! - What changes an address's accounts between two refreshes (an order of
//!   its own, a trade its stream tells of) has them asked for again a
//!   moment later ([`refresh_soon`]), with the reports read from them:
//!   once however much changed in that moment. What the stream tells of
//!   does so no oftener than every [`SPACING`] however busy the account,
//!   so one that trades all the time does not spend the minute's requests
//!   on itself; what the user did here is never kept waiting for that.

use std::{future::Future, rc::Rc, time::Duration};

use nsh_core::{
    prelude::*,
    time::{Instant, now, unix_ms},
};

use super::{
    Api, Kept, Lighter, Request, Resolution,
    fills::TradesReply,
    performance::{History, PnlReply, performance},
    prices::AssetDetails,
    requests::{ACCOUNT_NOT_FOUND, TRADES_LIMIT},
    state::{AccountStates, AccountsReply},
};
use crate::data::{
    net::{ApiError, Queries, QuerySpec, Subject, TaskHandle, WeakQueries},
    portfolio::{
        AccountSource, AddressError, Balances, Fills, MarginAccount, Performance, Prices,
        evm_address, short_address,
    },
};

/// The venue's id: the exchange id Lighter's name and logo are filed
/// under.
pub(super) const VENUE: &str = "lighterxyz";

/// How often every report is asked for again while it is watched.
const REFRESH: Duration = Duration::from_secs(5 * 60);
/// How old an address's accounts may be and still be what a report is read
/// from: the reports of one refresh, which start within moments of each
/// other, share one request.
const ACCOUNTS_FRESH: Duration = Duration::from_secs(10);
/// How long after something changed an address's accounts they are asked
/// for again: long enough for Lighter to have run what changed them, and
/// for several changes at once to be one asking.
const SETTLE: Duration = Duration::from_millis(700);
/// The least time between two such askings for one address, for those the
/// stream sets off.
const SPACING: Duration = Duration::from_secs(5);

/// What sets off an asking again ([`refresh_soon`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Cause {
    /// Something the user did here: asked for a moment later, whatever was
    /// asked for before.
    Own,
    /// Something the account's stream told of: no oftener than every
    /// [`SPACING`].
    Stream,
}
/// How far back the hours of a history are asked for, in seconds: a week
/// (Lighter keeps ten days of them).
const HOURS_SPAN: u64 = 7 * 86_400;
/// From when the days of a history are asked for, in Unix seconds: the
/// start of 2025, the earliest Lighter keeps them from.
const DAYS_FROM: u64 = 1_735_689_600;

/// `input` as a Lighter address: `0x` and forty hexadecimal digits, in
/// lower case.
pub(super) fn parse_address(input: &str) -> Result<String, AddressError> {
    const INVALID: AddressError =
        AddressError("Enter the account's L1 address: 0x followed by 40 hexadecimal characters.");
    evm_address(input).ok_or(INVALID)
}

/// What the connection panel calls a request about `address`.
fn subject(address: &str, detail: &str) -> Subject {
    Subject {
        exchange: VENUE.to_owned(),
        name: short_address(address),
        detail: detail.to_owned(),
    }
}

impl Api {
    /// `address`'s own accounts; none for an address Lighter knows no
    /// account of.
    async fn accounts(&self, address: &str) -> Result<AccountStates, ApiError> {
        let request = Request::Accounts {
            l1_address: address.to_owned(),
        };
        match self.get::<AccountsReply>(&request).await {
            Ok(reply) => {
                let accounts = AccountStates::from(reply);
                // Every entry names its market's index beside its symbol.
                for (symbol, id) in accounts.markets() {
                    self.directory.note(symbol, id);
                }
                Ok(accounts)
            }
            Err(error) if error.code == ACCOUNT_NOT_FOUND => Ok(AccountStates::default()),
            Err(error) => Err(error),
        }
    }

    /// The history of the accounts `indexes` together, as it stands at
    /// `now` (Unix seconds).
    async fn performance(&self, indexes: &[u64], now: u64) -> Result<Performance, ApiError> {
        let mut histories = Vec::with_capacity(indexes.len());
        for account_index in indexes.iter().copied() {
            let asked = |resolution, start| Request::Pnl {
                account_index,
                resolution,
                start,
                end: now,
            };
            let hours = asked(Resolution::Hour, now.saturating_sub(HOURS_SPAN));
            let days = asked(Resolution::Day, DAYS_FROM);
            histories.push(History {
                hourly: self.get::<PnlReply>(&hours).await?.entries(),
                daily: self.get::<PnlReply>(&days).await?.entries(),
            });
        }
        Ok(performance(histories, now as f64))
    }

    /// The latest trades of the accounts `indexes` together, newest first:
    /// as many of each as one request returns.
    async fn fills(&self, indexes: &[u64]) -> Result<Fills, ApiError> {
        let mut parts = Vec::with_capacity(indexes.len());
        for account_index in indexes.iter().copied() {
            let request = Request::Trades {
                account_index,
                limit: TRADES_LIMIT,
            };
            let reply = self.get::<TradesReply>(&request).await?;
            parts.push(Fills(reply.fills(account_index)));
        }
        Ok(Fills::combined(&parts))
    }
}

/// The request for `address`'s accounts, which every report of it is read
/// from.
fn accounts_query(api: &Api, address: &str) -> QuerySpec<AccountStates> {
    let (api, user) = (api.clone(), address.to_owned());
    let query = QuerySpec::new(format!("accounts {address}"), move || {
        let (api, user) = (api.clone(), user.clone());
        async move { api.accounts(&user).await }
    });
    query.subject(subject(address, "Accounts"))
}

/// `address`'s accounts, as the hub last had them when that is no older
/// than [`ACCOUNTS_FRESH`], else asked of Lighter (once, whichever reports
/// wait for them).
pub(super) async fn accounts_of(
    api: &Api,
    hub: &WeakQueries,
    address: &str,
) -> Result<Rc<AccountStates>, ApiError> {
    let hub = hub
        .upgrade()
        .ok_or_else(|| ApiError::network("the data services shut down"))?;
    hub.get(accounts_query(api, address).fresh_for(ACCOUNTS_FRESH))
        .await
}

/// The key of the report of `address` named `name` ([`Lighter::report`]).
pub(super) fn report_key(name: &str, address: &str) -> String {
    format!("{name} {address}")
}

/// An asking again that waits a moment, for one address
/// ([`Lighter::refresh_soon`]).
#[derive(Default)]
pub(super) struct Refreshing {
    /// The asking that waits, while one does.
    waiting: Option<TaskHandle>,
    /// The other requests asked for again once the accounts are in.
    also: Vec<String>,
    /// When the address's accounts were last asked for this way.
    last: Option<Instant>,
}

impl Lighter {
    /// A report of `address` that `read` makes of its accounts (asking
    /// Lighter for more when it takes more), keyed `name` and called
    /// `detail` in the connection panel, refetched every [`REFRESH`] while
    /// watched.
    fn report<T, F, Fut>(&self, address: &str, name: &str, detail: &str, read: F) -> QuerySpec<T>
    where
        T: 'static,
        F: Fn(Api, Rc<AccountStates>) -> Fut + 'static,
        Fut: Future<Output = Result<T, ApiError>> + 'static,
    {
        let (api, hub) = (self.api.clone(), self.queries.downgrade());
        let (user, read) = (address.to_owned(), Rc::new(read));
        QuerySpec::new(report_key(name, address), move || {
            let (api, hub) = (api.clone(), hub.clone());
            let (user, read) = (user.clone(), read.clone());
            async move {
                let accounts = accounts_of(&api, &hub, &user).await?;
                read(api, accounts).await
            }
        })
        .refresh_every(REFRESH)
        .subject(subject(address, detail))
    }

    /// `address`'s accounts, no older than a report's may be.
    pub(super) async fn accounts(&self, address: &str) -> Result<Rc<AccountStates>, ApiError> {
        accounts_of(&self.api, &self.queries.downgrade(), address).await
    }

    /// [`refresh_soon`] for `address` after something the user did here,
    /// with this client's own parts.
    pub(super) fn refresh_soon(&self, address: &str, also: &[String]) {
        refresh_soon(
            &self.api,
            &self.queries.downgrade(),
            &self.kept,
            address,
            also,
            Cause::Own,
        );
    }
}

/// Asks again, a moment from now, for `address`'s accounts and the reports
/// read from them (its margin and its balances, and `also`, the keys of
/// other requests that read them): after a transaction of its own, or when
/// its stream says something changed (`cause`). Several within the moment
/// are one asking, and what the stream sets off is spaced out
/// ([`Cause::Stream`]). `kept` holds what waits.
pub(super) fn refresh_soon(
    api: &Api,
    hub: &WeakQueries,
    kept: &Rc<Kept>,
    address: &str,
    also: &[String],
    cause: Cause,
) {
    let mut refreshes = kept.refreshes.borrow_mut();
    let refreshing = refreshes.entry(address.to_owned()).or_default();
    for key in also {
        if !refreshing.also.contains(key) {
            refreshing.also.push(key.clone());
        }
    }
    let wait = match cause {
        // In place of one that waits for its turn: this one is not kept
        // waiting.
        Cause::Own => SETTLE,
        Cause::Stream if refreshing.waiting.is_some() => return,
        Cause::Stream => refreshing.last.map_or(SETTLE, |last| {
            (last + SPACING)
                .saturating_duration_since(now())
                .max(SETTLE)
        }),
    };
    let (api, hub, kept) = (api.clone(), hub.clone(), Rc::downgrade(kept));
    let user = address.to_owned();
    refreshing.waiting = Some(TaskHandle::spawn(async move {
        timer(wait).await;
        // Its own handle is held until it is done: dropped any sooner, it
        // would end at its next wait.
        let (own, also) = {
            let Some(kept) = kept.upgrade() else {
                return;
            };
            let mut refreshes = kept.refreshes.borrow_mut();
            let Some(refreshing) = refreshes.get_mut(&user) else {
                return;
            };
            refreshing.last = Some(now());
            (
                refreshing.waiting.take(),
                std::mem::take(&mut refreshing.also),
            )
        };
        let Some(hub) = hub.upgrade() else {
            return;
        };
        let anew = accounts_query(&api, &user).fresh_for(Duration::ZERO);
        if hub.get(anew).await.is_ok() {
            let reports = ["margin", "balances"].map(|name| report_key(name, &user));
            for key in reports.iter().chain(&also) {
                hub.refresh(key);
            }
        }
        drop(own);
    }));
}

impl AccountSource for Lighter {
    fn venue(&self) -> &'static str {
        VENUE
    }

    fn name(&self) -> &'static str {
        "Lighter"
    }

    fn address_hint(&self) -> &'static str {
        "0x… L1 address"
    }

    fn parse_address(&self, input: &str) -> Result<String, AddressError> {
        parse_address(input)
    }

    fn queries(&self) -> &Queries {
        self.queries()
    }

    fn margin_query(&self, address: &str) -> Option<QuerySpec<MarginAccount>> {
        Some(self.report(
            address,
            "margin",
            "Perpetuals account",
            |_, accounts| async move { Ok(accounts.margin()) },
        ))
    }

    fn balances_query(&self, address: &str) -> Option<QuerySpec<Balances>> {
        Some(self.report(
            address,
            "balances",
            "Token balances",
            |_, accounts| async move { Ok(accounts.balances()) },
        ))
    }

    fn performance_query(&self, address: &str) -> Option<QuerySpec<Performance>> {
        Some(self.report(
            address,
            "performance",
            "Account history",
            |api, accounts| async move {
                let now = (unix_ms() / 1000.0).floor().max(0.0) as u64;
                api.performance(&accounts.indexes(), now).await
            },
        ))
    }

    fn prices_query(&self) -> Option<QuerySpec<Prices>> {
        let api = self.api.clone();
        let query = QuerySpec::new("prices", move || {
            let api = api.clone();
            async move {
                let reply = api.get::<AssetDetails>(&Request::AssetDetails).await?;
                Ok(Prices::from(reply))
            }
        });
        Some(query.refresh_every(REFRESH).subject(Subject {
            exchange: VENUE.to_owned(),
            name: "Assets".to_owned(),
            detail: "Token prices".to_owned(),
        }))
    }

    fn fills_query(&self, address: &str) -> Option<QuerySpec<Fills>> {
        Some(
            self.report(address, "fills", "Trades", |api, accounts| async move {
                api.fills(&accounts.indexes()).await
            }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_address_is_0x_and_forty_hex_digits_in_lower_case() {
        let address = "0x3f4ec7684f679f83c782e485b358a2d43045d6a2";
        assert_eq!(parse_address(address).as_deref(), Ok(address));
        assert_eq!(
            parse_address(" 0x3f4ec7684F679F83c782e485b358A2D43045d6A2 ").as_deref(),
            Ok(address)
        );
        for invalid in [
            "",
            "0x",
            "702384",
            "0x3f4ec7684f679f83c782e485b358a2d43045d6a",
        ] {
            assert!(parse_address(invalid).is_err(), "{invalid}");
        }
    }
}
