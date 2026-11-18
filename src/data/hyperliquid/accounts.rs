//! Hyperliquid's accounts as the portfolio asks for them
//! ([`AccountSource`]): how an address reads, and one shared request per
//! report.
//!
//! - An account is an address: `0x` and forty hexadecimal digits, written
//!   in lower case so one account is one request however it was typed. It
//!   must be the account's own (a master or sub-account): an agent wallet's
//!   address reads as an empty account.
//! - Margin is the perpetuals account ([`super::perps`]) of Hyperliquid's
//!   own perpetuals dex; positions on builder-deployed dexes are a request
//!   per dex and are not asked for yet.
//! - Every report is refetched every five minutes ([`REFRESH`]) while
//!   watched, so an account costs Hyperliquid little however many widgets
//!   show it: the two account states weigh 2 each, the others 20, so one
//!   account's reports spend 84 every five minutes (about 17 a minute), and
//!   the token prices, one request for every account, 20 more. Its trades
//!   weigh 20 and 1 more for every 20 returned: up to 120 for the 2000
//!   Hyperliquid returns at most.

use std::time::Duration;

use super::{
    fees::UserFees,
    fills::UserFills,
    performance::PortfolioReply,
    perps::ClearinghouseState,
    prices::SpotMetaAndAssetCtxs,
    spot::SpotClearinghouseState,
    staking::{DelegatorSummary, UserVaultEquities},
    Hyperliquid, Info,
};
use crate::data::{
    net::{Queries, QuerySpec, Subject},
    portfolio::{
        short_address, AccountSource, AddressError, Balances, Fees, Fills, MarginAccount,
        Performance, Prices, Staking, Vaults,
    },
};

/// The venue's id: the exchange id Hyperliquid's name and logo are filed
/// under.
pub(super) const VENUE: &str = "hyperliquid";

/// How often every report is asked for again while it is watched: the
/// account's perpetuals, balances, history, staking, vaults and fees, and
/// the tokens' prices.
const REFRESH: Duration = Duration::from_secs(5 * 60);

/// `input` as a Hyperliquid address: `0x` and forty hexadecimal digits, in
/// lower case.
pub(super) fn parse_address(input: &str) -> Result<String, AddressError> {
    const INVALID: AddressError =
        AddressError("Enter the account's address: 0x followed by 40 hexadecimal characters.");
    let address = input.trim().to_ascii_lowercase();
    let digits = address.strip_prefix("0x").ok_or(INVALID)?;
    if digits.len() == 40 && digits.bytes().all(|digit| digit.is_ascii_hexdigit()) {
        Ok(address)
    } else {
        Err(INVALID)
    }
}

/// What the connection panel calls a request about `address`.
fn subject(address: &str, detail: &str) -> Subject {
    Subject {
        exchange: VENUE.to_owned(),
        name: short_address(address),
        detail: detail.to_owned(),
    }
}

impl AccountSource for Hyperliquid {
    fn venue(&self) -> &'static str {
        VENUE
    }

    fn name(&self) -> &'static str {
        "Hyperliquid"
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
            dex: None,
        };
        Some(self.info_query(
            request,
            REFRESH,
            subject(address, "Perpetuals account"),
            |state: ClearinghouseState| state.into(),
        ))
    }

    fn balances_query(&self, address: &str) -> Option<QuerySpec<Balances>> {
        let request = Info::SpotClearinghouseState {
            user: address.to_owned(),
        };
        Some(self.info_query(
            request,
            REFRESH,
            subject(address, "Token balances"),
            |state: SpotClearinghouseState| state.into(),
        ))
    }

    fn performance_query(&self, address: &str) -> Option<QuerySpec<Performance>> {
        let request = Info::Portfolio {
            user: address.to_owned(),
        };
        Some(self.info_query(
            request,
            REFRESH,
            subject(address, "Account history"),
            |reply: PortfolioReply| reply.into(),
        ))
    }

    fn prices_query(&self) -> Option<QuerySpec<Prices>> {
        let subject = Subject {
            exchange: VENUE.to_owned(),
            name: "Spot".to_owned(),
            detail: "Token prices".to_owned(),
        };
        Some(self.info_query(
            Info::SpotMetaAndAssetCtxs,
            REFRESH,
            subject,
            |reply: SpotMetaAndAssetCtxs| reply.into(),
        ))
    }

    fn staking_query(&self, address: &str) -> Option<QuerySpec<Staking>> {
        let request = Info::DelegatorSummary {
            user: address.to_owned(),
        };
        Some(self.info_query(
            request,
            REFRESH,
            subject(address, "Staking"),
            |reply: DelegatorSummary| reply.into(),
        ))
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
        };
        Some(self.info_query(
            request,
            REFRESH,
            subject(address, "Trading fees"),
            |reply: UserFees| reply.into(),
        ))
    }

    fn fills_query(&self, address: &str) -> Option<QuerySpec<Fills>> {
        let request = Info::UserFills {
            user: address.to_owned(),
            aggregate_by_time: true,
        };
        Some(self.info_list_query(
            request,
            REFRESH,
            subject(address, "Trades"),
            |reply: &UserFills| reply.0.len(),
            |reply: UserFills| reply.into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_address_is_0x_and_forty_hex_digits_in_lower_case() {
        let address = "0x0123456789abcdef0123456789abcdef01234567";
        assert_eq!(parse_address(address).as_deref(), Ok(address));
        assert_eq!(
            parse_address("  0X0123456789ABCDEF0123456789abcdef01234567 ").as_deref(),
            Ok(address)
        );
        for invalid in [
            "",
            "0x",
            "0123456789abcdef0123456789abcdef01234567",
            "0x0123456789abcdef0123456789abcdef0123456",
            "0x0123456789abcdef0123456789abcdef012345678",
            "0x0123456789abcdef0123456789abcdef0123456g",
            "vitalik.eth",
        ] {
            assert!(parse_address(invalid).is_err(), "{invalid}");
        }
    }
}
