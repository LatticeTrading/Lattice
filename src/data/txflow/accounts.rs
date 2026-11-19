//! TxFlow's accounts as the portfolio asks for them ([`AccountSource`]):
//! how an address reads, and one shared request per report.
//!
//! - An account is an address: `0x` and forty hexadecimal digits, written
//!   in lower case so one account is one request however it was typed. It
//!   must be the account's own: an agent key's address reads as an empty
//!   account.
//! - Margin is the perpetuals sub-account ([`super::perps`]); the Outcome
//!   sub-account's positions are not asked for.
//! - The history ([`super::performance`]) takes six requests: the day, the
//!   week and the month, of the perpetuals sub-account and of it with the
//!   Outcome one.
//! - The account's margin is refetched every minute ([`MARGIN_REFRESH`]),
//!   its other reports every five ([`REFRESH`]), while watched: an account
//!   whose every report is watched spends 1 request a minute and 9 more
//!   every five, of the 50 a second TxFlow allows.
//! - Token balances, prices and staking are not reported: the widgets that
//!   show them leave them out.

use std::time::Duration;

use nsh_core::time::unix_ms;

use super::{
    Info, OverviewPeriod, TxFlow,
    fees::{FEES_ASSET, UserFees},
    fills::UserFills,
    info::FILLS_LIMIT,
    performance::Overview,
    perps::ClearinghouseState,
    vaults::UserVaultEquities,
};
use crate::data::{
    net::{Queries, QuerySpec, Subject},
    portfolio::{
        AccountSource, AddressError, Balances, Fees, Fills, MarginAccount, Performance, Scope,
        Vaults, evm_address, short_address,
    },
};

/// The venue's id: the exchange id TxFlow's name and logo are filed under.
pub(super) const VENUE: &str = "txflow";

/// How often the margin is asked for again while it is watched.
const MARGIN_REFRESH: Duration = Duration::from_secs(60);
/// How often every other report is.
const REFRESH: Duration = Duration::from_secs(5 * 60);

/// `input` as a TxFlow address: `0x` and forty hexadecimal digits, in lower
/// case.
pub(super) fn parse_address(input: &str) -> Result<String, AddressError> {
    const INVALID: AddressError =
        AddressError("Enter the account's address: 0x followed by 40 hexadecimal characters.");
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

/// The sub-accounts a history of `scope` counts, as TxFlow names them;
/// `None` is the perpetuals one alone.
fn sub_accounts(scope: Scope) -> Option<Vec<&'static str>> {
    match scope {
        Scope::Perps => None,
        Scope::Total => Some(vec!["perps", "outcome"]),
    }
}

impl AccountSource for TxFlow {
    fn venue(&self) -> &'static str {
        VENUE
    }

    fn name(&self) -> &'static str {
        "TxFlow"
    }

    fn address_hint(&self) -> &'static str {
        "0x… account address"
    }

    fn parse_address(&self, input: &str) -> Result<String, AddressError> {
        parse_address(input)
    }

    fn queries(&self) -> &Queries {
        self.queries()
    }

    fn margin_query(&self, address: &str) -> Option<QuerySpec<MarginAccount>> {
        let request = Info::ClearinghouseState {
            user: address.to_owned(),
            account_type: "perp",
        };
        Some(self.info_query(
            request,
            MARGIN_REFRESH,
            subject(address, "Perpetuals account"),
            |state: ClearinghouseState| state.into(),
        ))
    }

    fn balances_query(&self, _address: &str) -> Option<QuerySpec<Balances>> {
        None
    }

    fn performance_query(&self, address: &str) -> Option<QuerySpec<Performance>> {
        let (api, user) = (self.api.clone(), address.to_owned());
        let query = QuerySpec::new(format!("performance {address}"), move || {
            let (api, user) = (api.clone(), user.clone());
            async move {
                let now_ms = unix_ms();
                let mut windows = Vec::new();
                for scope in Scope::ALL {
                    for period in OverviewPeriod::ALL {
                        let request = Info::UserOverview {
                            user_address: user.clone(),
                            period,
                            account_type: sub_accounts(scope),
                        };
                        let overview = api.info::<Overview>(&request).await?;
                        windows.push(overview.window(period, scope, now_ms));
                    }
                }
                Ok(Performance(windows))
            }
        });
        Some(
            query
                .refresh_every(REFRESH)
                .subject(subject(address, "Account history")),
        )
    }

    fn vaults_query(&self, address: &str) -> Option<QuerySpec<Vaults>> {
        let request = Info::UserVaultEquities {
            user: address.to_owned(),
        };
        Some(self.info_query(
            request,
            REFRESH,
            subject(address, "Vault shares"),
            |reply: UserVaultEquities| reply.into(),
        ))
    }

    fn fees_query(&self, address: &str) -> Option<QuerySpec<Fees>> {
        let request = Info::UserFees {
            user: address.to_owned(),
            asset: FEES_ASSET.to_owned(),
        };
        Some(self.info_query(
            request,
            REFRESH,
            subject(address, "Trading fees"),
            |reply: UserFees| reply.fees(unix_ms()),
        ))
    }

    fn fills_query(&self, address: &str) -> Option<QuerySpec<Fills>> {
        let request = Info::UserFills {
            user: address.to_owned(),
            limit: FILLS_LIMIT,
        };
        Some(self.info_query(
            request,
            REFRESH,
            subject(address, "Trades"),
            |reply: UserFills| reply.into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_address_is_0x_and_forty_hex_digits_in_lower_case() {
        let address = "0x386967b3b81807968d0f114e824bba3f7de9c1dc";
        assert_eq!(parse_address(address).as_deref(), Ok(address));
        assert_eq!(
            parse_address(" 0x386967B3B81807968D0F114E824BBA3F7DE9C1DC ").as_deref(),
            Ok(address)
        );
        for invalid in [
            "",
            "0x",
            "nope",
            "0x386967b3b81807968d0f114e824bba3f7de9c1d",
        ] {
            assert!(parse_address(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn a_history_counts_the_outcome_sub_account_in_the_total_alone() {
        assert_eq!(sub_accounts(Scope::Perps), None);
        assert_eq!(sub_accounts(Scope::Total), Some(vec!["perps", "outcome"]));
    }
}
