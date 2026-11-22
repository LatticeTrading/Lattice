//! What is signed for Hyperliquid, by whom, and reading who signed it.
//!
//! Hyperliquid has two kinds of signature, and an account trades with both:
//!
//! - **User-signed actions** are what the account itself must authorize
//!   (approving an agent to trade for it, a withdrawal, a transfer):
//!   EIP-712 typed data a wallet shows and signs ([`user_signed`]).
//!   - The domain is `HyperliquidSignTransaction`, version `1`, the zero
//!     address as its contract, and for its chain *the chain the wallet is
//!     on*: a wallet refuses typed data of another chain than its own, and
//!     Hyperliquid accepts any (the action names it as `signatureChainId`).
//!   - The message's type is `HyperliquidTransaction:<Action>`, and its
//!     first member, `hyperliquidChain`, names the net (`Mainnet` or
//!     `Testnet`), so what was signed for one is not good on the other
//!     ([`Network`]).
//! - **L1 actions** are everything a trader does all day (orders, cancels,
//!   leverage): hashed ([`action_hash`]: the action's MessagePack, its
//!   nonce and whether it is for a vault), and the hash signed as the
//!   `connectionId` of an `Agent` in the domain `Exchange` on chain 1337
//!   ([`l1_signing_hash`]). No wallet signs for chain 1337 while it is on
//!   another, and nobody wants a wallet prompt per order: the account
//!   approves an *agent* once, with a user-signed action
//!   ([`approve_agent`]), and the agent's key, which the app holds
//!   ([`super::agent`]), signs the L1 actions.
//!
//! Both are checked against Hyperliquid's own SDK (the tests below and
//! `exchange.rs`'s): the same action, key and chain give the signatures
//! its `tests/signing_test.py` expects. [`signer_of`] reads who signed
//! typed data, the way Hyperliquid reads who signed an action.

use alloy::{
    dyn_abi::TypedData,
    primitives::{Address, B256, Signature, keccak256},
    sol,
    sol_types::{Eip712Domain, SolStruct, eip712_domain},
};
use serde_json::{Value, json};

use super::pack::Packed;

/// The name of the domain every user-signed action is signed in.
const DOMAIN: &str = "HyperliquidSignTransaction";

/// The domain every L1 action is signed in, whichever net it is for.
const EXCHANGE: Eip712Domain = eip712_domain! {
    name: "Exchange",
    version: "1",
    chain_id: 1337,
    verifying_contract: Address::ZERO,
};

sol! {
    /// What an L1 action's signature is of: the net it is for and the
    /// action's hash.
    struct Agent {
        string source;
        bytes32 connectionId;
    }
}

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

    /// What a user-signed action calls it (`hyperliquidChain`).
    pub fn name(self) -> &'static str {
        match self {
            Self::Mainnet => "Mainnet",
            Self::Testnet => "Testnet",
        }
    }

    /// What an L1 action's signature calls it (the agent's `source`).
    fn source(self) -> &'static str {
        match self {
            Self::Mainnet => "a",
            Self::Testnet => "b",
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

/// What an account signs to let `agent` trade for it: the user-signed
/// `ApproveAgent`, for a wallet on the chain `chain_id`. An agent places
/// and cancels orders; it cannot withdraw or transfer. `name` is what
/// Hyperliquid lists the agent under: an account has one agent of a name,
/// so approving another of the same name retires the one before. `nonce`
/// is the time in Unix milliseconds, which the posted action repeats.
pub fn approve_agent(
    network: Network,
    chain_id: u64,
    agent: Address,
    name: &str,
    nonce: u64,
) -> Result<TypedData, String> {
    user_signed(
        "HyperliquidTransaction:ApproveAgent",
        &[
            ("hyperliquidChain", "string"),
            ("agentAddress", "address"),
            ("agentName", "string"),
            ("nonce", "uint64"),
        ],
        json!({
            "hyperliquidChain": network.name(),
            "agentAddress": agent,
            "agentName": name,
            "nonce": nonce,
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

/// The hash an L1 `action` is known by: its MessagePack, then `nonce` (the
/// time it was made, in Unix milliseconds, which Hyperliquid takes once),
/// then whether it is for a `vault` and which. The members' order is part
/// of it ([`Packed`]).
pub(super) fn action_hash(action: &Packed, nonce: u64, vault: Option<Address>) -> B256 {
    let mut data = Vec::with_capacity(192);
    action.pack(&mut data);
    data.extend(nonce.to_be_bytes());
    match vault {
        None => data.push(0),
        Some(vault) => {
            data.push(1);
            data.extend(vault.as_slice());
        }
    }
    keccak256(&data)
}

/// What a key signs to authorize the L1 action hashed `action`
/// ([`action_hash`]) on `network`.
pub(super) fn l1_signing_hash(network: Network, action: B256) -> B256 {
    Agent {
        source: network.source().to_owned(),
        connectionId: action,
    }
    .eip712_signing_hash(&EXCHANGE)
}

/// `signature` as `/exchange` reads one: `r` and `s` in hexadecimal and
/// `v` as 27 or 28.
pub(super) fn wire(signature: &Signature) -> Value {
    json!({
        "r": format!("{:#x}", signature.r()),
        "s": format!("{:#x}", signature.s()),
        "v": 27 + u8::from(signature.v()),
    })
}

#[cfg(test)]
pub(super) mod tests {
    use alloy::primitives::{U256, address, b256};

    use super::{super::agent::AgentKey, *};

    /// The key Hyperliquid's Python SDK signs its tests' actions with.
    pub(in super::super) const SDK_KEY: B256 =
        b256!("0x0123456789012345678901234567890123456789012345678901234567890123");

    fn word(hex: &str) -> U256 {
        hex.parse().unwrap()
    }

    /// `action` signed for `network` by the SDK's key, as the SDK writes a
    /// signature: its `r`, its `s` and its `v`.
    pub(in super::super) fn sdk_signature(
        action: &Packed,
        nonce: u64,
        vault: Option<Address>,
        network: Network,
    ) -> (U256, U256, u8) {
        let key = AgentKey::from_bytes(&SDK_KEY).unwrap();
        let hash = l1_signing_hash(network, action_hash(action, nonce, vault));
        let signature = key.sign(&hash).unwrap();
        (signature.r(), signature.s(), 27 + u8::from(signature.v()))
    }

    /// What the SDK expects of a signature.
    pub(in super::super) fn expected(r: &str, s: &str, v: u8) -> (U256, U256, u8) {
        (word(r), word(s), v)
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
        let key = AgentKey::from_bytes(&SDK_KEY).unwrap();
        let signature = key.sign(&action.eip712_signing_hash().unwrap()).unwrap();
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

    /// `test_l1_action_signing_matches` and
    /// `test_l1_action_signing_matches_with_vault`: the SDK's dummy action
    /// (a name and a number wider than 32 bits) on both nets, for the
    /// account and for a vault.
    #[test]
    fn an_l1_action_is_hashed_and_signed_as_hyperliquids_sdk_signs_it() {
        let action = Packed::Map(vec![
            ("type", Packed::text("dummy")),
            // `float_to_int_for_hashing(1000)`.
            ("num", Packed::Uint(100_000_000_000)),
        ]);
        assert_eq!(
            sdk_signature(&action, 0, None, Network::Mainnet),
            expected(
                "0x53749d5b30552aeb2fca34b530185976545bb22d0b3ce6f62e31be961a59298",
                "0x755c40ba9bf05223521753995abb2f73ab3229be8ec921f350cb447e384d8ed8",
                27
            )
        );
        assert_eq!(
            sdk_signature(&action, 0, None, Network::Testnet),
            expected(
                "0x542af61ef1f429707e3c76c5293c80d01f74ef853e34b76efffcb57e574f9510",
                "0x17b8b32f086e8cdede991f1e2c529f5dd5297cbe8128500e00cbaf766204a613",
                28
            )
        );
        let vault = Some(address!("0x1719884eb866cb12b2287399b15f7db5e7d775ea"));
        assert_eq!(
            sdk_signature(&action, 0, vault, Network::Mainnet),
            expected(
                "0x3c548db75e479f8012acf3000ca3a6b05606bc2ec0c29c50c515066a326239",
                "0x4d402be7396ce74fbba3795769cda45aec00dc3125a984f2a9f23177b190da2c",
                28
            )
        );
        assert_eq!(
            sdk_signature(&action, 0, vault, Network::Testnet),
            expected(
                "0xe281d2fb5c6e25ca01601f878e4d69c965bb598b88fac58e475dd1f5e56c362b",
                "0x7ddad27e9a238d045c035bc606349d075d5c5cd00a6cd1da23ab5c39d4ef0f60",
                27
            )
        );
    }

    #[test]
    fn approving_an_agent_is_typed_data_in_hyperliquids_domain() {
        let agent = address!("0x5e9ee1089755c3435139848e47e6635505d5a13a");
        let message = approve_agent(
            Network::Mainnet,
            42_161,
            agent,
            "Lattice",
            1_700_000_000_000,
        )
        .unwrap();
        assert_eq!(message.domain.name.as_deref(), Some(DOMAIN));
        assert_eq!(message.domain.version.as_deref(), Some("1"));
        assert_eq!(message.domain.chain_id, Some(U256::from(42_161)));
        assert_eq!(message.domain.verifying_contract, Some(Address::ZERO));
        assert_eq!(
            message.encode_type().unwrap(),
            "HyperliquidTransaction:ApproveAgent(string hyperliquidChain,address agentAddress,\
             string agentName,uint64 nonce)"
        );
        assert_eq!(message.message["hyperliquidChain"], "Mainnet");

        // Who signed it is read from the signature; another chain's,
        // another agent's or another net's approval was signed by somebody
        // else as far as the signature says.
        let key = AgentKey::from_bytes(&SDK_KEY).unwrap();
        let signature = key.sign(&message.eip712_signing_hash().unwrap()).unwrap();
        assert_eq!(signer_of(&message, &signature), Ok(key.address()));
        for other in [
            approve_agent(Network::Mainnet, 1, agent, "Lattice", 1_700_000_000_000),
            approve_agent(
                Network::Testnet,
                42_161,
                agent,
                "Lattice",
                1_700_000_000_000,
            ),
            approve_agent(
                Network::Mainnet,
                42_161,
                key.address(),
                "Lattice",
                1_700_000_000_000,
            ),
            approve_agent(
                Network::Mainnet,
                42_161,
                agent,
                "Lattice",
                1_700_000_000_001,
            ),
        ] {
            assert_ne!(signer_of(&other.unwrap(), &signature), Ok(key.address()));
        }
    }

    #[test]
    fn a_signature_is_written_as_the_exchange_reads_it() {
        let key = AgentKey::from_bytes(&SDK_KEY).unwrap();
        let action = Packed::Map(vec![
            ("type", Packed::text("dummy")),
            ("num", Packed::Uint(100_000_000_000)),
        ]);
        let hash = l1_signing_hash(Network::Mainnet, action_hash(&action, 0, None));
        // Hexadecimal without leading zeros, as the SDK's `to_hex` writes.
        assert_eq!(
            wire(&key.sign(&hash).unwrap()),
            json!({
                "r": "0x53749d5b30552aeb2fca34b530185976545bb22d0b3ce6f62e31be961a59298",
                "s": "0x755c40ba9bf05223521753995abb2f73ab3229be8ec921f350cb447e384d8ed8",
                "v": 27,
            })
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
