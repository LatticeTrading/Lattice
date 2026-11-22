//! The agent: a key the app makes and keeps, which the account approves to
//! trade for it ([`super::signing::approve_agent`]) and which then signs
//! every order and cancel without a wallet prompt.
//!
//! - What it can do: place and cancel orders and set leverage for the
//!   account that approved it. It cannot withdraw or transfer: those are
//!   user-signed actions, which only the account's own wallet signs.
//! - Where it is while the app runs: in memory ([`Agents`]), for the
//!   account that approved it and only while that account is the wallet's
//!   connected one (another account, or none, lets go of it).
//! - Where it is between runs: in the app's storage on this device
//!   ([`keep`], [`kept`]), by the net and the account that approved it, so
//!   that connecting the account again trades without another signature.
//!   It is saved in the clear, as everything in that storage is (the
//!   browser's local storage on the web, the app's data folder on desktop,
//!   which is also where Hyperliquid's own site keeps its agent): whatever
//!   can read it there can place and cancel orders for the account, and
//!   cannot take funds out. Disabling trading removes it ([`discard`]).
//! - Whether a kept one still holds is Hyperliquid's to say
//!   ([`ExtraAgents`]): an account has one agent of a name, so enabling
//!   trading on another device, or revoking it on Hyperliquid's site,
//!   retires this one, and one approved with an expiry runs out. A kept
//!   key is used only once Hyperliquid lists it as still valid; one it no
//!   longer lists is discarded.
//! - How it is made: 32 bytes from the system's random source (the
//!   browser's `crypto.getRandomValues` on the web) as a secp256k1 key,
//!   which signs with the `k256` alloy already brings. The key is wiped
//!   from memory when dropped.

use std::{
    cell::{Cell, RefCell},
    fmt,
};

use alloy::{
    primitives::{Address, B256, Signature, hex},
    signers::k256::ecdsa::SigningKey,
};
use nsh_core::time::unix_ms;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::{accounts::VENUE, signing::Network};
use crate::storage;

/// A key of the app's own.
pub(super) struct AgentKey {
    key: SigningKey,
    address: Address,
}

impl AgentKey {
    /// A new key from the system's random source.
    pub(super) fn random() -> Result<Self, String> {
        let mut bytes = [0_u8; 32];
        // Almost every 32 bytes are a key (all but zero and the few above
        // the curve's order); another draw for the rest.
        for _ in 0..4 {
            getrandom::getrandom(&mut bytes)
                .map_err(|error| format!("no random source to make a key from: {error}"))?;
            let key = SigningKey::from_slice(&bytes);
            bytes.fill(0);
            if let Ok(key) = key {
                return Ok(Self::of(key));
            }
        }
        Err("the random source gave no usable key".to_owned())
    }

    /// The key `bytes` are.
    pub(super) fn from_bytes(bytes: &B256) -> Result<Self, String> {
        SigningKey::from_slice(bytes.as_slice())
            .map(Self::of)
            .map_err(|error| format!("not a key: {error}"))
    }

    /// The key itself, as it is saved: for [`keep`] and nothing else.
    fn secret(&self) -> String {
        hex::encode_prefixed(self.key.to_bytes())
    }

    fn of(key: SigningKey) -> Self {
        let address = Address::from_private_key(&key);
        Self { key, address }
    }

    /// The address Hyperliquid knows the key by.
    pub(super) fn address(&self) -> Address {
        self.address
    }

    /// The key's signature of `hash`.
    pub(super) fn sign(&self, hash: &B256) -> Result<Signature, String> {
        self.key
            .sign_prehash_recoverable(hash.as_slice())
            .map(Signature::from)
            .map_err(|error| format!("the key could not sign: {error}"))
    }
}

/// Never the key itself.
impl fmt::Debug for AgentKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "AgentKey({})", self.address)
    }
}

/// `owner` as what is kept is filed under.
fn filed(owner: Address) -> String {
    format!("{owner:#x}")
}

/// The agent `saved` holds for `owner` on `network`: read leniently, so
/// what is damaged or of another shape is no agent.
fn read(saved: &Value, network: Network, owner: Address) -> Option<AgentKey> {
    let kept = saved.get(network.name())?.get(filed(owner))?;
    let key = AgentKey::from_bytes(&kept.get("key")?.as_str()?.parse().ok()?).ok()?;
    // What says which agent it is must be the key's own.
    let named = kept.get("agent").and_then(Value::as_str)?;
    (named.parse() == Ok(key.address())).then_some(key)
}

/// `saved` with `key` as `owner`'s agent on `network` (`None`: with none);
/// `None` when that leaves nothing to save.
fn written(
    saved: Option<Value>,
    network: Network,
    owner: Address,
    key: Option<&AgentKey>,
) -> Option<Value> {
    let mut nets = match saved {
        Some(Value::Object(nets)) => nets,
        _ => Map::new(),
    };
    let mut owners = match nets.remove(network.name()) {
        Some(Value::Object(owners)) => owners,
        _ => Map::new(),
    };
    match key {
        Some(key) => {
            let kept = json!({ "agent": filed(key.address()), "key": key.secret() });
            owners.insert(filed(owner), kept);
        }
        None => {
            owners.remove(&filed(owner));
        }
    }
    if !owners.is_empty() {
        nets.insert(network.name().to_owned(), Value::Object(owners));
    }
    (!nets.is_empty()).then_some(Value::Object(nets))
}

/// The agent kept for `owner` on `network` from an earlier run, if any.
pub(super) fn kept(network: Network, owner: Address) -> Option<AgentKey> {
    read(&storage::load_trading_keys(VENUE)?, network, owner)
}

/// Saves `key` (`None`: nothing) as `owner`'s agent on `network`.
fn save(network: Network, owner: Address, key: Option<&AgentKey>) {
    let saved = written(storage::load_trading_keys(VENUE), network, owner, key);
    if let Err(error) = storage::save_trading_keys(VENUE, saved.as_ref()) {
        // Trading goes on for this run; the next one enables it again.
        log::warn!("Lattice: could not save the Hyperliquid agent: {error:#}");
    }
}

/// Keeps `key` as `owner`'s agent on `network` for the next run.
pub(super) fn keep(network: Network, owner: Address, key: &AgentKey) {
    save(network, owner, Some(key));
}

/// Keeps no agent for `owner` on `network` any more.
pub(super) fn discard(network: Network, owner: Address) {
    save(network, owner, None);
}

/// The reply to [`Info::ExtraAgents`](super::Info): the agents an account
/// has approved by name, each with when it runs out.
#[derive(Debug, Deserialize)]
pub(super) struct ExtraAgents(Vec<ExtraAgent>);

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExtraAgent {
    address: String,
    /// When it stops being one, in Unix milliseconds.
    valid_until: Option<f64>,
}

impl ExtraAgents {
    /// Whether `agent` is among them and has not run out at `now_ms`.
    pub(super) fn holds(&self, agent: Address, now_ms: f64) -> bool {
        self.0.iter().any(|listed| {
            listed.address.parse() == Ok(agent)
                && listed.valid_until.is_none_or(|until| until > now_ms)
        })
    }
}

/// The agent the app holds, and the nonces of what it signs.
#[derive(Default)]
pub(super) struct Agents {
    /// The account that approved an agent, and the agent.
    approved: RefCell<Option<(Address, AgentKey)>>,
    /// The last nonce given out.
    nonce: Cell<u64>,
}

impl Agents {
    /// Keeps `key` as the agent `owner` approved, in place of any other.
    pub(super) fn approve(&self, owner: Address, key: AgentKey) {
        *self.approved.borrow_mut() = Some((owner, key));
    }

    /// Lets go of the agent held. What is kept of it for the next run
    /// stays ([`discard`] forgets that); nothing signs as it meanwhile.
    pub(super) fn forget(&self) {
        self.approved.take();
    }

    /// Whether `owner` has approved an agent that is held.
    #[cfg(test)]
    fn approved(&self, owner: Address) -> bool {
        matches!(&*self.approved.borrow(), Some((approved, _)) if *approved == owner)
    }

    /// `owner`'s agent's signature of `hash`; `None` when it has none.
    pub(super) fn sign(&self, owner: Address, hash: &B256) -> Option<Result<Signature, String>> {
        match &*self.approved.borrow() {
            Some((approved, key)) if *approved == owner => Some(key.sign(hash)),
            _ => None,
        }
    }

    /// The nonce of the next action: the time in Unix milliseconds, and
    /// always above the last one given (Hyperliquid takes a nonce once, and
    /// two actions can be made within a millisecond).
    pub(super) fn next_nonce(&self) -> u64 {
        // Whole milliseconds since 1970 fit a `u64` for good.
        #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let now = unix_ms().max(0.0) as u64;
        let nonce = now.max(self.nonce.get() + 1);
        self.nonce.set(nonce);
        nonce
    }
}

#[cfg(test)]
mod tests {
    use alloy::primitives::address;

    use super::{super::signing::tests::SDK_KEY, *};

    #[test]
    fn a_key_is_known_by_its_address_and_never_shown() {
        let key = AgentKey::from_bytes(&SDK_KEY).unwrap();
        // The address Hyperliquid's SDK has for its test key.
        assert_eq!(
            key.address(),
            address!("0x14791697260E4c9A71f18484C9f997B308e59325")
        );
        assert_eq!(
            format!("{key:?}"),
            "AgentKey(0x14791697260E4c9A71f18484C9f997B308e59325)"
        );
        assert!(AgentKey::from_bytes(&B256::ZERO).is_err());

        // Keys of the app's own differ, and each signs as itself.
        let (first, second) = (AgentKey::random().unwrap(), AgentKey::random().unwrap());
        assert_ne!(first.address(), second.address());
        let hash = B256::repeat_byte(7);
        let signature = first.sign(&hash).unwrap();
        assert_eq!(
            signature.recover_address_from_prehash(&hash).ok(),
            Some(first.address())
        );
    }

    #[test]
    fn an_agent_signs_for_the_account_that_approved_it_alone() {
        let agents = Agents::default();
        let owner = address!("0x5e9ee1089755c3435139848e47e6635505d5a13a");
        let other = address!("0x1719884eb866cb12b2287399b15f7db5e7d775ea");
        let hash = B256::repeat_byte(7);
        assert!(!agents.approved(owner));
        assert!(agents.sign(owner, &hash).is_none());

        let key = AgentKey::from_bytes(&SDK_KEY).unwrap();
        let agent = key.address();
        agents.approve(owner, key);
        assert!(agents.approved(owner) && !agents.approved(other));
        let signature = agents.sign(owner, &hash).unwrap().unwrap();
        assert_eq!(
            signature.recover_address_from_prehash(&hash).ok(),
            Some(agent)
        );
        assert!(agents.sign(other, &hash).is_none());

        agents.forget();
        assert!(!agents.approved(owner));
        assert!(agents.sign(owner, &hash).is_none());
    }

    #[test]
    fn an_agent_is_kept_by_net_and_account_and_read_leniently() {
        let owner = address!("0x5e9ee1089755c3435139848e47e6635505d5a13a");
        let other = address!("0x1719884eb866cb12b2287399b15f7db5e7d775ea");
        let key = AgentKey::from_bytes(&SDK_KEY).unwrap();

        // Kept under its net and its account, with which agent it is.
        let saved = written(None, Network::Mainnet, owner, Some(&key)).unwrap();
        assert_eq!(
            saved,
            json!({ "Mainnet": { "0x5e9ee1089755c3435139848e47e6635505d5a13a": {
                "agent": "0x14791697260e4c9a71f18484c9f997b308e59325",
                "key": "0x0123456789012345678901234567890123456789012345678901234567890123",
            } } })
        );
        let read_back = read(&saved, Network::Mainnet, owner).unwrap();
        assert_eq!(read_back.address(), key.address());
        // Another account's, or the other net's, is not this one's.
        assert!(read(&saved, Network::Mainnet, other).is_none());
        assert!(read(&saved, Network::Testnet, owner).is_none());

        // Each account keeps its own; taking one out leaves the others,
        // and the last one out leaves nothing to save.
        let second = AgentKey::random().unwrap();
        let both = written(Some(saved), Network::Mainnet, other, Some(&second)).unwrap();
        assert_eq!(
            read(&both, Network::Mainnet, other).map(|key| key.address()),
            Some(second.address())
        );
        let one = written(Some(both), Network::Mainnet, owner, None).unwrap();
        assert!(read(&one, Network::Mainnet, owner).is_none());
        assert!(read(&one, Network::Mainnet, other).is_some());
        assert_eq!(written(Some(one), Network::Mainnet, other, None), None);

        // What is damaged is no agent, and never a failure.
        let filed = filed(owner);
        for damaged in [
            json!(null),
            json!([]),
            json!({ "Mainnet": 7 }),
            json!({ "Mainnet": { filed.as_str(): "0x01" } }),
            json!({ "Mainnet": { filed.as_str(): { "key": "0x01", "agent": "0x02" } } }),
            // A key filed as another agent's.
            json!({ "Mainnet": { filed.as_str(): {
                "agent": "0x5e9ee1089755c3435139848e47e6635505d5a13a",
                "key": "0x0123456789012345678901234567890123456789012345678901234567890123",
            } } }),
        ] {
            assert!(
                read(&damaged, Network::Mainnet, owner).is_none(),
                "{damaged}"
            );
            // Saving over it starts again.
            assert!(written(Some(damaged), Network::Mainnet, owner, Some(&key)).is_some());
        }
    }

    #[test]
    fn a_kept_agent_holds_while_hyperliquid_lists_it_as_valid() {
        let agent = address!("0x14791697260E4c9A71f18484C9f997B308e59325");
        // The shape of Hyperliquid's documentation.
        let listed: ExtraAgents = serde_json::from_value(json!([
            { "name": "Mobile", "address": "0x5e9ee1089755c3435139848e47e6635505d5a13a",
              "validUntil": 1_800_000_000_000_u64 },
            { "name": "Lattice", "address": "0x14791697260e4c9a71f18484c9f997b308e59325",
              "validUntil": 1_700_000_000_000_u64 },
        ]))
        .unwrap();
        assert!(listed.holds(agent, 1_699_999_999_999.0));
        // Run out.
        assert!(!listed.holds(agent, 1_700_000_000_000.0));
        // Another's listing is not this agent's.
        let none: ExtraAgents = serde_json::from_value(json!([])).unwrap();
        assert!(!none.holds(agent, 0.0));
        // One listed with no end has none.
        let open: ExtraAgents = serde_json::from_value(json!([
            { "name": "Lattice", "address": "0x14791697260E4c9A71f18484C9f997B308e59325" }
        ]))
        .unwrap();
        assert!(open.holds(agent, 9e15));
    }

    #[test]
    fn nonces_only_go_up() {
        let agents = Agents::default();
        let first = agents.next_nonce();
        let second = agents.next_nonce();
        let third = agents.next_nonce();
        assert!(first < second && second < third);
    }
}
