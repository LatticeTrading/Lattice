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

use std::{future::Future, rc::Rc, time::Duration};

use nsh_core::time::unix_ms;

use super::{
    Api, Lighter, Request, Resolution,
    fills::TradesReply,
    performance::{History, PnlReply, performance},
    prices::AssetDetails,
    requests::{ACCOUNT_NOT_FOUND, TRADES_LIMIT},
    state::{AccountStates, AccountsReply},
};
use crate::data::{
    net::{ApiError, Queries, QuerySpec, Subject, WeakQueries},
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
const ACCOUNTS_FRESH: Duration = Duration::from_secs(30);
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
            Ok(reply) => Ok(reply.into()),
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

/// `address`'s accounts, as the hub last had them when that is no older
/// than [`ACCOUNTS_FRESH`], else asked of Lighter (once, whichever reports
/// wait for them).
async fn accounts_of(
    api: &Api,
    hub: &WeakQueries,
    address: &str,
) -> Result<Rc<AccountStates>, ApiError> {
    let hub = hub
        .upgrade()
        .ok_or_else(|| ApiError::network("the data services shut down"))?;
    let (api, user) = (api.clone(), address.to_owned());
    let query = QuerySpec::new(format!("accounts {address}"), move || {
        let (api, user) = (api.clone(), user.clone());
        async move { api.accounts(&user).await }
    });
    hub.get(
        query
            .fresh_for(ACCOUNTS_FRESH)
            .subject(subject(address, "Accounts")),
    )
    .await
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
        QuerySpec::new(format!("{name} {address}"), move || {
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
