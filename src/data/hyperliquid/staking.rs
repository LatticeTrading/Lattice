//! What a user has staked ([`Info::DelegatorSummary`](super::Info)) and
//! the vaults it has a share of ([`Info::UserVaultEquities`](super::Info)),
//! as Hyperliquid writes them and as the portfolio reads them ([`Staking`],
//! [`Vaults`]).
//!
//! What is staked on Hyperliquid is HYPE, apart from the user's balances:
//! delegated, staked but delegated to no validator, and on its way back
//! (`totalPendingWithdrawal`). A vault share is the vault's address and
//! what the share is worth in USDC.

use serde::Deserialize;

use crate::data::{
    net::Figure,
    portfolio::{Staking, VaultShare, Vaults},
};

/// The token Hyperliquid stakes.
const STAKED: &str = "HYPE";

/// The reply to [`Info::DelegatorSummary`](super::Info).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct DelegatorSummary {
    delegated: Option<Figure>,
    undelegated: Option<Figure>,
    total_pending_withdrawal: Option<Figure>,
}

impl From<DelegatorSummary> for Staking {
    fn from(summary: DelegatorSummary) -> Self {
        let amount = |figure: Option<Figure>| figure.map_or(0.0, |Figure(amount)| amount);
        Self {
            asset: STAKED.to_owned(),
            delegated: amount(summary.delegated),
            undelegated: amount(summary.undelegated),
            pending_withdrawal: amount(summary.total_pending_withdrawal),
        }
    }
}

/// The reply to [`Info::UserVaultEquities`](super::Info).
#[derive(Debug, Deserialize)]
pub(super) struct UserVaultEquities(Vec<VaultEquity>);

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VaultEquity {
    vault_address: String,
    equity: Figure,
    locked_until_timestamp: Option<f64>,
}

impl From<UserVaultEquities> for Vaults {
    fn from(UserVaultEquities(shares): UserVaultEquities) -> Self {
        Self(
            shares
                .into_iter()
                .map(|share| VaultShare {
                    vault: share.vault_address,
                    equity: share.equity.0,
                    locked_until_ms: share.locked_until_timestamp,
                })
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn staking_reads_as_the_hype_staked_however_it_stands() {
        let summary: DelegatorSummary = serde_json::from_value(json!({
            "delegated": "102.19644893", "undelegated": "1.0",
            "totalPendingWithdrawal": "0.5", "nPendingWithdrawals": 1
        }))
        .expect("the documented reply reads");
        let staking = Staking::from(summary);
        assert_eq!(staking.asset, "HYPE");
        assert_eq!(
            (
                staking.delegated,
                staking.undelegated,
                staking.pending_withdrawal
            ),
            (102.196_448_93, 1.0, 0.5)
        );
        assert!((staking.total() - 103.696_448_93).abs() < 1e-9);

        // Nothing staked.
        let none: DelegatorSummary = serde_json::from_value(json!({})).unwrap();
        assert_eq!(Staking::from(none).total(), 0.0);
    }

    #[test]
    fn vault_shares_read_as_their_vaults_and_worth() {
        let reply: UserVaultEquities = serde_json::from_value(json!([
            { "vaultAddress": "0xb39cecd2d493daab328e65b2eb57824a0e8d417d",
              "equity": "5968.439935", "lockedUntilTimestamp": 1_780_639_678_509_u64 },
            { "vaultAddress": "0xdfc24b077bc1425ad1dea75bcb6f8158e10df303",
              "equity": "12.5" }
        ]))
        .expect("the documented reply reads");
        let vaults = Vaults::from(reply);
        assert_eq!(vaults.0.len(), 2);
        assert_eq!(vaults.0[0].locked_until_ms, Some(1_780_639_678_509.0));
        assert_eq!(vaults.0[1].locked_until_ms, None);
        assert!((vaults.equity() - 5_980.939_935).abs() < 1e-9);
        assert_eq!(
            Vaults::from(serde_json::from_value::<UserVaultEquities>(json!([])).unwrap()),
            Vaults::default()
        );
    }
}
