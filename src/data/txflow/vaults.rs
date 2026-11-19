//! The vaults a user has a share of ([`Info::UserVaultEquities`](super::Info)),
//! as TxFlow writes them and as the portfolio reads them ([`Vaults`]): each
//! the vault's address, what the share is worth in USDC and until when it
//! is locked.

use serde::Deserialize;

use crate::data::{
    net::Figure,
    portfolio::{VaultShare, Vaults},
};

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
    fn vault_shares_read_as_their_vaults_and_worth() {
        let reply: UserVaultEquities = serde_json::from_value(json!([
            { "vaultAddress": "0x664989a76069c2b153720745bc1c3c8eeee504c1",
              "equity": "1914.936497", "lockedUntilTimestamp": 1_792_830_521_530_u64 },
            { "vaultAddress": "0x0000000000000000000000000000000000000001", "equity": "5" }
        ]))
        .expect("the documented reply reads");
        let vaults = Vaults::from(reply);
        assert_eq!(vaults.0.len(), 2);
        assert_eq!(vaults.0[0].locked_until_ms, Some(1_792_830_521_530.0));
        assert_eq!(vaults.0[1].locked_until_ms, None);
        assert!((vaults.equity() - 1_919.936_497).abs() < 1e-9);
        assert_eq!(
            Vaults::from(serde_json::from_value::<UserVaultEquities>(json!([])).unwrap()),
            Vaults::default()
        );
    }
}
