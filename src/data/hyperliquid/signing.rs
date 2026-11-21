//! What a wallet signs for Hyperliquid, and reading who signed it.
//!
//! Hyperliquid takes what an account itself must authorize (approving an
//! agent to trade for it, a withdrawal, a transfer) as "user-signed
//! actions": EIP-712 typed data a wallet shows and signs.
//!
//! - The domain is `HyperliquidSignTransaction`, version `1`, the zero
//!   address as its contract, and for its chain *the chain the wallet is
//!   on*: a wallet refuses typed data of another chain than its own, and
//!   Hyperliquid accepts any (the action names it as `signatureChainId`).
//! - The message's type is `HyperliquidTransaction:<Action>`, and its first
//!   member, `hyperliquidChain`, names the net (`Mainnet` or `Testnet`), so
//!   what was signed for one is not good on the other ([`Network`]).
//!
//! [`user_signed`] makes that typed data for any action; checked against
//! Hyperliquid's own SDK's signature of a transfer (the test below). Orders
//! are not signed this way: they are signed by an agent's key on chain 1337,
//! which no wallet would sign for, and that agent is approved with one of
//! these.
//!
//! No action is signed or sent yet. What is signed is the connection test
//! ([`connection_test`]): a message in Hyperliquid's domain of a type
//! Hyperliquid has no action for, so its signature shows that the wallet
//! signs for Hyperliquid and authorizes nothing there. [`signer_of`] reads
//! who signed it, the way Hyperliquid reads who signed an action.

use alloy::{
    dyn_abi::TypedData,
    primitives::{Address, Signature},
};
use serde_json::{Value, json};

/// The name of the domain every user-signed action is signed in.
const DOMAIN: &str = "HyperliquidSignTransaction";

/// The type of the connection test's message. Not `HyperliquidTransaction:`
/// anything: Hyperliquid reads no action from it.
const TEST_TYPE: &str = "Lattice:ConnectionTest";

/// Which Hyperliquid a message is signed for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Network {
    Mainnet,
    Testnet,
}

impl Network {
    /// The net of the API at `http_base`: Hyperliquid's testnet is at a
    /// host named for it, and everything else is taken for the mainnet.
    pub(super) fn of(http_base: &str) -> Self {
        if http_base.contains("testnet") {
            Self::Testnet
        } else {
            Self::Mainnet
        }
    }

    /// What a signed message calls it (`hyperliquidChain`).
    pub fn name(self) -> &'static str {
        match self {
            Self::Mainnet => "Mainnet",
            Self::Testnet => "Testnet",
        }
    }
}

/// The typed data of a user-signed action: a `message` of the type
/// `primary_type`, whose members are `fields` (each a name and its Solidity
/// type, in the order Hyperliquid hashes them), for a wallet on the chain
/// `chain_id` to sign.
pub fn user_signed(
    primary_type: &str,
    fields: &[(&str, &str)],
    message: Value,
    chain_id: u64,
) -> Result<TypedData, String> {
    let member = |(name, kind): &(&str, &str)| json!({ "name": name, "type": kind });
    let fields: Vec<Value> = fields.iter().map(member).collect();
    serde_json::from_value(json!({
        "domain": {
            "name": DOMAIN,
            "version": "1",
            "chainId": chain_id,
            "verifyingContract": Address::ZERO,
        },
        "types": {
            "EIP712Domain": [
                { "name": "name", "type": "string" },
                { "name": "version", "type": "string" },
                { "name": "chainId", "type": "uint256" },
                { "name": "verifyingContract", "type": "address" },
            ],
            primary_type: fields,
        },
        "primaryType": primary_type,
        "message": message,
    }))
    .map_err(|error| format!("unusable typed data: {error}"))
}

/// The connection test's message, for a wallet on the chain `chain_id`:
/// signed in Hyperliquid's domain as an action is, and no action. `time_ms`
/// (Unix milliseconds) makes each test's message its own.
pub fn connection_test(network: Network, chain_id: u64, time_ms: u64) -> Result<TypedData, String> {
    user_signed(
        TEST_TYPE,
        &[
            ("hyperliquidChain", "string"),
            ("app", "string"),
            ("purpose", "string"),
            ("time", "uint64"),
        ],
        json!({
            "hyperliquidChain": network.name(),
            "app": "Lattice",
            "purpose": "Connection test. This signature authorizes nothing.",
            "time": time_ms,
        }),
        chain_id,
    )
}

/// The address whose key made `signature` of `message`. A contract
/// wallet's signature has no such key: it reads as some other address, as
/// it would to Hyperliquid.
pub fn signer_of(message: &TypedData, signature: &Signature) -> Result<Address, String> {
    let hash = message
        .eip712_signing_hash()
        .map_err(|error| format!("unusable typed data: {error}"))?;
    signature
        .recover_address_from_prehash(&hash)
        .map_err(|error| format!("unreadable signature: {error}"))
}

#[cfg(test)]
mod tests {
    use alloy::{
        primitives::{U256, address},
        signers::{SignerSync, local::PrivateKeySigner},
    };

    use super::*;

    /// The key Hyperliquid's Python SDK signs its tests' actions with.
    const SDK_KEY: &str = "0x0123456789012345678901234567890123456789012345678901234567890123";

    fn word(hex: &str) -> U256 {
        hex.parse().unwrap()
    }

    /// `test_sign_usd_transfer_action` of hyperliquid-python-sdk
    /// (`tests/signing_test.py`): the same action, key and chain give the
    /// signature the SDK expects, so the typed data is what Hyperliquid
    /// verifies.
    #[test]
    fn a_user_signed_action_is_signed_as_hyperliquids_sdk_signs_it() {
        let action = user_signed(
            "HyperliquidTransaction:UsdSend",
            &[
                ("hyperliquidChain", "string"),
                ("destination", "string"),
                ("amount", "string"),
                ("time", "uint64"),
            ],
            json!({
                "hyperliquidChain": Network::Testnet.name(),
                "destination": "0x5e9ee1089755c3435139848e47e6635505d5a13a",
                "amount": "1",
                "time": 1_687_816_341_423_u64,
            }),
            // The SDK's `signatureChainId`, 0x66eee.
            421_614,
        )
        .unwrap();
        let key: PrivateKeySigner = SDK_KEY.parse().unwrap();
        let signature = key
            .sign_hash_sync(&action.eip712_signing_hash().unwrap())
            .unwrap();
        assert_eq!(
            signature.r(),
            word("0x637b37dd731507cdd24f46532ca8ba6eec616952c56218baeff04144e4a77073")
        );
        assert_eq!(
            signature.s(),
            word("0x11a6a24900e6e314136d2592e2f8d502cd89b7c15b198e1bee043c9589f9fad7")
        );
        // The SDK's `v` of 27.
        assert!(!signature.v());
        assert_eq!(signer_of(&action, &signature), Ok(key.address()));
    }

    #[test]
    fn the_connection_test_is_in_hyperliquids_domain_and_is_no_action() {
        let test = connection_test(Network::Mainnet, 42_161, 1_700_000_000_000).unwrap();
        assert_eq!(test.domain.name.as_deref(), Some(DOMAIN));
        assert_eq!(test.domain.version.as_deref(), Some("1"));
        assert_eq!(test.domain.chain_id, Some(U256::from(42_161)));
        assert_eq!(test.domain.verifying_contract, Some(Address::ZERO));
        assert_eq!(test.primary_type, TEST_TYPE);
        assert!(!test.primary_type.starts_with("HyperliquidTransaction:"));
        assert_eq!(test.message["hyperliquidChain"], "Mainnet");
        assert_eq!(
            test.encode_type().unwrap(),
            "Lattice:ConnectionTest(string hyperliquidChain,string app,string purpose,uint64 time)"
        );

        // Who signed it is read from the signature; another chain's or
        // another time's message was signed by somebody else as far as
        // the signature says.
        let key: PrivateKeySigner = SDK_KEY.parse().unwrap();
        let signature = key
            .sign_hash_sync(&test.eip712_signing_hash().unwrap())
            .unwrap();
        assert_eq!(signer_of(&test, &signature), Ok(key.address()));
        for other in [
            connection_test(Network::Mainnet, 1, 1_700_000_000_000).unwrap(),
            connection_test(Network::Mainnet, 42_161, 1_700_000_000_001).unwrap(),
            connection_test(Network::Testnet, 42_161, 1_700_000_000_000).unwrap(),
        ] {
            assert_ne!(signer_of(&other, &signature), Ok(key.address()));
        }
        assert_ne!(
            key.address(),
            address!("0x5e9ee1089755c3435139848e47e6635505d5a13a")
        );
    }

    #[test]
    fn the_net_is_read_from_where_the_api_is() {
        assert_eq!(Network::of("https://api.hyperliquid.xyz"), Network::Mainnet);
        assert_eq!(
            Network::of("https://api.hyperliquid-testnet.xyz"),
            Network::Testnet
        );
    }
}
