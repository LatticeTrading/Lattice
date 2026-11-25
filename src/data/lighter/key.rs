//! The API key: a key the app makes and keeps, which the account registers
//! on Lighter ([`super::tx::register_message`]) and which then signs every
//! order and cancel without a wallet prompt.
//!
//! - What it is: a Lighter account has up to 255 API keys, each at an index.
//!   Lighter keeps the first few for its own site and apps
//!   ([`FIRST_INDEX`] is the first that is anybody's), and a key registered
//!   at an index takes the place of the one there. So the app takes an
//!   index nothing is registered at ([`free_index`]), from the top down:
//!   the ones people give their own bots are the low ones.
//! - What it can do: place and cancel orders and set leverage for the
//!   account it is registered for, and read what Lighter tells only the
//!   account (its open orders) with the auth tokens it signs. Lighter also
//!   lets a key ask for a withdrawal, which it pays to the address that
//!   owns the account and nowhere else; nothing here asks for one. It
//!   cannot send funds to anybody else, nor register or replace a key:
//!   those take the wallet's own signature.
//! - Where it is while the app runs: in memory ([`Keys`]), for the account
//!   that registered it and only while that account is the wallet's
//!   connected one, with the nonce its next transaction takes.
//! - Where it is between runs: in the app's storage on this device
//!   ([`keep`], [`kept`]), by the net and the address that registered it,
//!   in the clear, as Hyperliquid's agent is
//!   ([`crate::data::hyperliquid`]): whatever can read it there can trade
//!   for the account, and can take funds nowhere but back to the account's
//!   own wallet. Disabling trading removes the key and remembers its index
//!   ([`retire`]), which enabling again takes back instead of another.
//! - Whether a kept one still holds is Lighter's to say: it lists the
//!   public key at each index, and a key is used only while the one at its
//!   index is its own.
//!
//! A key's transactions carry a nonce each, one more than the last, and
//! Lighter takes them in that order only. The next one is asked of Lighter
//! once and counted here from then on ([`Keys::nonce`],
//! [`Keys::set_nonce`]); whatever Lighter refuses forgets the count, so the
//! next transaction asks again. Transactions wait for one another ([`Keys::turn`]), so two sent
//! at once still go in the order of their nonces.

use std::{cell::RefCell, fmt};

use alloy::primitives::hex;
use serde_json::{Map, Value, json};
use tokio::sync::{Mutex, MutexGuard};

use super::{
    accounts::VENUE,
    crypto::{PublicKey, SecretKey},
    tx::{self, Envelope, Network, Signed, Transaction},
};
use crate::storage;

/// The first index that is not kept for Lighter's own site and apps.
pub(super) const FIRST_INDEX: u8 = 4;
/// The index the app tries first, and goes down from: the highest but one,
/// the highest being what other software takes when told nothing.
const PREFERRED_INDEX: u8 = 253;
/// How long an auth token is good for, in seconds: an hour, as the tokens
/// Lighter's own site makes are.
const TOKEN_LIFE: u64 = 60 * 60;
/// How long before a token runs out another is made, in seconds.
const TOKEN_MARGIN: u64 = 5 * 60;

/// The index a new key is registered at, `taken` being the ones that hold a
/// key: the preferred one, or the nearest free one under it. `None` when
/// every one is taken.
pub(super) fn free_index(taken: &[u8]) -> Option<u8> {
    (FIRST_INDEX..=PREFERRED_INDEX)
        .rev()
        .find(|index| !taken.contains(index))
}

/// A key of the app's own, registered (or about to be) at an index of an
/// account.
pub(super) struct ApiKey {
    secret: SecretKey,
    public: PublicKey,
    /// The account's index.
    pub(super) account: u64,
    /// The key's own, among the account's.
    pub(super) index: u8,
}

/// Never the key itself.
impl fmt::Debug for ApiKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "ApiKey({} at {} of account {})",
            self.public, self.index, self.account
        )
    }
}

impl ApiKey {
    /// A new key from the system's random source, for the index `index` of
    /// the account `account`.
    pub(super) fn random(account: u64, index: u8) -> Result<Self, String> {
        SecretKey::random().map(|secret| Self::of(secret, account, index))
    }

    fn of(secret: SecretKey, account: u64, index: u8) -> Self {
        Self {
            public: secret.public(),
            secret,
            account,
            index,
        }
    }

    /// The key Lighter lists it as.
    pub(super) fn public(&self) -> PublicKey {
        self.public
    }

    /// Whether `listed` (a public key as Lighter writes one) is this key's.
    pub(super) fn is(&self, listed: &str) -> bool {
        let listed = listed.trim();
        let listed = listed.strip_prefix("0x").unwrap_or(listed);
        listed.eq_ignore_ascii_case(&self.public.to_string())
    }

    /// Who signs a transaction of the key's with `nonce`, good until
    /// `expires_at` (Unix milliseconds).
    pub(super) fn envelope(&self, network: Network, nonce: u64, expires_at: u64) -> Envelope {
        Envelope {
            network,
            account: self.account,
            key_index: self.index,
            nonce,
            expires_at,
        }
    }

    /// `transaction` signed by the key for `envelope`.
    pub(super) fn sign(&self, transaction: &Transaction, envelope: &Envelope) -> Signed {
        transaction.signed(envelope, &self.secret)
    }

    /// The key itself, as it is saved: for [`keep`] and nothing else.
    fn secret(&self) -> String {
        hex::encode_prefixed(self.secret.to_bytes())
    }
}

/// The key `saved` holds for `owner` on `network`: read leniently, so what
/// is damaged or of another shape is no key.
fn read(saved: &Value, network: Network, owner: &str) -> Option<ApiKey> {
    let kept = saved.get(network.name())?.get(owner)?;
    let mut bytes = [0; 40];
    hex::decode_to_slice(kept.get("key")?.as_str()?, &mut bytes).ok()?;
    let secret = SecretKey::from_bytes(&bytes);
    bytes.fill(0);
    let account = kept.get("account")?.as_u64()?;
    let index = u8::try_from(kept.get("index")?.as_u64()?).ok()?;
    let key = ApiKey::of(secret?, account, index);
    // What says which key it is must be the key's own.
    key.is(kept.get("public")?.as_str()?).then_some(key)
}

/// The index `saved` remembers for `owner`'s account `account` on `network`,
/// and the public key that was registered there: a key kept, or one retired.
fn slot_in(saved: &Value, network: Network, owner: &str, account: u64) -> Option<(u8, String)> {
    let kept = saved.get(network.name())?.get(owner)?;
    if kept.get("account")?.as_u64()? != account {
        return None;
    }
    let index = u8::try_from(kept.get("index")?.as_u64()?).ok()?;
    let public = kept.get("public")?.as_str()?.to_owned();
    (index >= FIRST_INDEX).then_some((index, public))
}

/// `saved` with `entry` as what is kept for `owner` on `network` (`None`:
/// with nothing); `None` when that leaves nothing to save.
fn written(
    saved: Option<Value>,
    network: Network,
    owner: &str,
    entry: Option<Value>,
) -> Option<Value> {
    let mut nets = match saved {
        Some(Value::Object(nets)) => nets,
        _ => Map::new(),
    };
    let mut owners = match nets.remove(network.name()) {
        Some(Value::Object(owners)) => owners,
        _ => Map::new(),
    };
    match entry {
        Some(entry) => {
            owners.insert(owner.to_owned(), entry);
        }
        None => {
            owners.remove(owner);
        }
    }
    if !owners.is_empty() {
        nets.insert(network.name().to_owned(), Value::Object(owners));
    }
    (!nets.is_empty()).then_some(Value::Object(nets))
}

/// What is saved of `key`: where it is registered, which key it is, and the
/// key (`with_secret`), or only the first two for one that was retired.
fn entry(key: &ApiKey, with_secret: bool) -> Value {
    let mut entry = json!({
        "account": key.account,
        "index": key.index,
        "public": key.public.to_string(),
    });
    if with_secret {
        entry["key"] = Value::String(key.secret());
    }
    entry
}

/// The key kept for `owner` (an address, in lower case) on `network` from
/// an earlier run, if any.
pub(super) fn kept(network: Network, owner: &str) -> Option<ApiKey> {
    read(&storage::load_trading_keys(VENUE)?, network, owner)
}

/// The index remembered for `owner`'s account `account` on `network`, with
/// the public key registered there: where a key is registered again.
pub(super) fn slot(network: Network, owner: &str, account: u64) -> Option<(u8, String)> {
    slot_in(&storage::load_trading_keys(VENUE)?, network, owner, account)
}

/// Saves `entry` (`None`: nothing) as what is kept for `owner` on `network`.
fn save(network: Network, owner: &str, entry: Option<Value>) {
    let saved = written(storage::load_trading_keys(VENUE), network, owner, entry);
    if let Err(error) = storage::save_trading_keys(VENUE, saved.as_ref()) {
        // Trading goes on for this run; the next one enables it again.
        log::warn!("Lattice: could not save the Lighter API key: {error:#}");
    }
}

/// Keeps `key` as `owner`'s on `network` for the next run.
pub(super) fn keep(network: Network, owner: &str, key: &ApiKey) {
    save(network, owner, Some(entry(key, true)));
}

/// Keeps `key` no more, and remembers where it was registered: nothing
/// signs with it again, and enabling trading again takes its index back.
pub(super) fn retire(network: Network, owner: &str, key: &ApiKey) {
    save(network, owner, Some(entry(key, false)));
}

/// Keeps nothing for `owner` on `network` any more: for a key Lighter no
/// longer lists at its index, which is somebody else's now.
pub(super) fn discard(network: Network, owner: &str) {
    save(network, owner, None);
}

/// The key the app holds, and what it has signed with it.
#[derive(Default)]
pub(super) struct Keys {
    held: RefCell<Option<Held>>,
    /// Held by a transaction from its nonce to Lighter's answer.
    order: Mutex<()>,
}

struct Held {
    /// The address that registered the key, in lower case.
    owner: String,
    key: ApiKey,
    /// The nonce the key's next transaction takes, once Lighter said it.
    nonce: Option<u64>,
    /// The auth token last made, and when it runs out (Unix seconds).
    token: Option<(String, u64)>,
}

impl Keys {
    /// Keeps `key` as the one `owner` registered, in place of any other.
    pub(super) fn hold(&self, owner: &str, key: ApiKey) {
        *self.held.borrow_mut() = Some(Held {
            owner: owner.to_owned(),
            key,
            nonce: None,
            token: None,
        });
    }

    /// Lets go of the key held. What is kept of it for the next run stays;
    /// nothing signs with it meanwhile.
    pub(super) fn forget(&self) {
        self.held.take();
    }

    /// Lets go of the key held and gives it, when it is `owner`'s: for
    /// [`retire`].
    pub(super) fn take(&self, owner: &str) -> Option<ApiKey> {
        let mut held = self.held.borrow_mut();
        if held.as_ref().is_some_and(|held| held.owner == owner) {
            held.take().map(|held| held.key)
        } else {
            None
        }
    }

    /// The account `owner`'s key is registered for; `None` when none of
    /// `owner`'s is held.
    pub(super) fn account(&self, owner: &str) -> Option<u64> {
        self.with(owner, |held| held.key.account)
    }

    /// What `owner`'s key is, for a transaction to be made with: its
    /// account and its index.
    pub(super) fn signer(&self, owner: &str) -> Option<(u64, u8)> {
        self.with(owner, |held| (held.key.account, held.key.index))
    }

    /// Waits for the transactions before this one, and holds the turn until
    /// the guard drops: a key's transactions go to Lighter in the order of
    /// their nonces.
    pub(super) async fn turn(&self) -> MutexGuard<'_, ()> {
        self.order.lock().await
    }

    /// The nonce `owner`'s key's next transaction takes, when it is known.
    pub(super) fn nonce(&self, owner: &str) -> Option<u64> {
        self.with(owner, |held| held.nonce).flatten()
    }

    /// Notes that `owner`'s key's next transaction takes `nonce` (`None`:
    /// that Lighter is to be asked again).
    pub(super) fn set_nonce(&self, owner: &str, nonce: Option<u64>) {
        self.with_mut(owner, |held| held.nonce = nonce);
    }

    /// `transaction` signed by `owner`'s key on `network` with `nonce`, good
    /// until `expires_at` (Unix milliseconds); `None` when none of `owner`'s
    /// is held.
    pub(super) fn sign(
        &self,
        owner: &str,
        network: Network,
        transaction: &Transaction,
        nonce: u64,
        expires_at: u64,
    ) -> Option<Signed> {
        self.with(owner, |held| {
            let envelope = held.key.envelope(network, nonce, expires_at);
            held.key.sign(transaction, &envelope)
        })
    }

    /// An auth token of `owner`'s key that is good at `now` (Unix seconds)
    /// and for a while yet: the one last made, or a new one.
    pub(super) fn token(&self, owner: &str, now: u64) -> Option<String> {
        self.with_mut(owner, |held| held.token_at(now))
    }

    /// [`Self::token`] of the key held for the account `account`, whoever
    /// registered it: what a subscription of that account's carries.
    pub(super) fn token_of(&self, account: u64, now: u64) -> Option<String> {
        let mut held = self.held.borrow_mut();
        let held = held.as_mut().filter(|held| held.key.account == account)?;
        Some(held.token_at(now))
    }

    fn with<T>(&self, owner: &str, read: impl FnOnce(&Held) -> T) -> Option<T> {
        let held = self.held.borrow();
        held.as_ref().filter(|held| held.owner == owner).map(read)
    }

    fn with_mut<T>(&self, owner: &str, change: impl FnOnce(&mut Held) -> T) -> Option<T> {
        let mut held = self.held.borrow_mut();
        held.as_mut().filter(|held| held.owner == owner).map(change)
    }
}

impl Held {
    fn token_at(&mut self, now: u64) -> String {
        if let Some((token, until)) = &self.token
            && *until > now + TOKEN_MARGIN
        {
            return token.clone();
        }
        let until = now + TOKEN_LIFE;
        let key = &self.key;
        let token = tx::auth_token(&key.secret, key.account, key.index, until);
        self.token = Some((token.clone(), until));
        token
    }
}

#[cfg(test)]
mod tests {
    use super::{super::tx::tests::signer_key, *};

    const OWNER: &str = "0x5e9ee1089755c3435139848e47e6635505d5a13a";
    const OTHER: &str = "0x1719884eb866cb12b2287399b15f7db5e7d775ea";
    /// The public key of the key Lighter's signer was given.
    const PUBLIC: &str =
        "921babf1db8b9e8a2deb30afdb0d36d8bd57664e4d351ee65f3345ae51acd147b0fa1903f6cc2ba6";

    fn key() -> ApiKey {
        ApiKey::of(signer_key(), 2_180, 253)
    }

    #[test]
    fn a_key_takes_an_index_nothing_is_registered_at() {
        assert_eq!(free_index(&[]), Some(253));
        // Lighter's own, a bot's, and other software's default.
        assert_eq!(free_index(&[0, 1, 4, 254]), Some(253));
        assert_eq!(free_index(&[253, 252, 0]), Some(251));
        // Never one Lighter keeps for itself.
        let all: Vec<u8> = (FIRST_INDEX..=253).collect();
        assert_eq!(free_index(&all), None);
    }

    #[test]
    fn a_key_is_known_by_its_public_key_and_never_shown() {
        let key = key();
        assert_eq!(key.public().to_string(), PUBLIC);
        assert!(key.is(PUBLIC));
        assert!(key.is(&format!(" 0x{} ", PUBLIC.to_uppercase())));
        assert!(!key.is(&PUBLIC.replace('9', "8")));
        let shown = format!("{key:?}");
        assert_eq!(shown, format!("ApiKey({PUBLIC} at 253 of account 2180)"));
        assert!(!shown.contains(key.secret().trim_start_matches("0x")));

        // Keys of the app's own differ.
        let (first, second) = (
            ApiKey::random(1, 253).unwrap(),
            ApiKey::random(1, 253).unwrap(),
        );
        assert_ne!(first.public(), second.public());
    }

    #[test]
    fn a_key_is_kept_by_net_and_address_and_read_leniently() {
        let key = key();
        let saved = written(None, Network::Mainnet, OWNER, Some(entry(&key, true))).unwrap();
        assert_eq!(
            saved,
            json!({ "Mainnet": { OWNER: {
                "account": 2_180,
                "index": 253,
                "public": PUBLIC,
                "key": format!("0x{}", super::super::tx::tests::SIGNER_KEY),
            } } })
        );
        let read_back = read(&saved, Network::Mainnet, OWNER).unwrap();
        assert_eq!(read_back.public(), key.public());
        assert_eq!((read_back.account, read_back.index), (2_180, 253));
        // Another address's, or the other net's, is not this one's.
        assert!(read(&saved, Network::Mainnet, OTHER).is_none());
        assert!(read(&saved, Network::Testnet, OWNER).is_none());
        assert_eq!(
            slot_in(&saved, Network::Mainnet, OWNER, 2_180),
            Some((253, PUBLIC.to_owned()))
        );

        // Each address keeps its own; taking one out leaves the others, and
        // the last one out leaves nothing to save.
        let second = ApiKey::random(7, 252).unwrap();
        let both = written(
            Some(saved),
            Network::Mainnet,
            OTHER,
            Some(entry(&second, true)),
        )
        .unwrap();
        assert_eq!(
            read(&both, Network::Mainnet, OTHER).map(|key| key.public()),
            Some(second.public())
        );
        let one = written(Some(both), Network::Mainnet, OWNER, None).unwrap();
        assert!(read(&one, Network::Mainnet, OWNER).is_none());
        assert!(read(&one, Network::Mainnet, OTHER).is_some());
        assert_eq!(written(Some(one), Network::Mainnet, OTHER, None), None);

        // What is damaged is no key, and never a failure.
        for damaged in [
            json!(null),
            json!([]),
            json!({ "Mainnet": 7 }),
            json!({ "Mainnet": { OWNER: "0x01" } }),
            json!({ "Mainnet": { OWNER: { "account": 1, "index": 253, "public": PUBLIC,
                                          "key": "0x01" } } }),
            // An index no key has.
            json!({ "Mainnet": { OWNER: { "account": 1, "index": 300, "public": PUBLIC,
                                          "key": key.secret() } } }),
            // A key filed as another's.
            json!({ "Mainnet": { OWNER: { "account": 1, "index": 253,
                                          "public": second.public().to_string(),
                                          "key": key.secret() } } }),
        ] {
            assert!(
                read(&damaged, Network::Mainnet, OWNER).is_none(),
                "{damaged}"
            );
            // Saving over it starts again.
            let entry = Some(entry(&key, true));
            assert!(written(Some(damaged), Network::Mainnet, OWNER, entry).is_some());
        }
    }

    #[test]
    fn a_retired_key_is_gone_and_its_index_remembered() {
        let key = key();
        let saved = written(None, Network::Mainnet, OWNER, Some(entry(&key, false))).unwrap();
        // No key to sign with, and where one was registered.
        assert!(read(&saved, Network::Mainnet, OWNER).is_none());
        assert!(!saved.to_string().contains("key\""));
        assert_eq!(
            slot_in(&saved, Network::Mainnet, OWNER, 2_180),
            Some((253, PUBLIC.to_owned()))
        );
        // Another account of the address's, or another address: no index.
        assert_eq!(slot_in(&saved, Network::Mainnet, OWNER, 9), None);
        assert_eq!(slot_in(&saved, Network::Mainnet, OTHER, 2_180), None);
        assert_eq!(slot_in(&saved, Network::Testnet, OWNER, 2_180), None);
        // An index that is Lighter's own is nobody's to take back.
        let low =
            json!({ "Mainnet": { OWNER: { "account": 2_180, "index": 2, "public": PUBLIC } } });
        assert_eq!(slot_in(&low, Network::Mainnet, OWNER, 2_180), None);
    }

    #[test]
    fn a_key_signs_for_the_address_that_registered_it_alone() {
        let keys = Keys::default();
        let cancel = Transaction::CancelOrder {
            market: 1,
            order: 2,
        };
        let sign = |owner: &str| keys.sign(owner, Network::Mainnet, &cancel, 5, 1_000);
        assert!(sign(OWNER).is_none());
        assert_eq!((keys.signer(OWNER), keys.account(OWNER)), (None, None));

        keys.hold(OWNER, key());
        assert_eq!(keys.signer(OWNER), Some((2_180, 253)));
        assert_eq!(keys.account(OWNER), Some(2_180));
        let signed = sign(OWNER).unwrap();
        assert_eq!(signed.kind, 15);
        let written: Value = serde_json::from_str(&signed.info).unwrap();
        assert_eq!(
            (
                &written["AccountIndex"],
                &written["ApiKeyIndex"],
                &written["Nonce"]
            ),
            (&json!(2_180), &json!(253), &json!(5))
        );
        assert!(sign(OTHER).is_none());
        assert_eq!(keys.signer(OTHER), None);

        // Its nonce is counted for it alone, and asked for again when
        // forgotten.
        assert_eq!(keys.nonce(OWNER), None);
        keys.set_nonce(OWNER, Some(6));
        keys.set_nonce(OTHER, Some(9));
        assert_eq!((keys.nonce(OWNER), keys.nonce(OTHER)), (Some(6), None));
        keys.set_nonce(OWNER, None);
        assert_eq!(keys.nonce(OWNER), None);

        // Taken by its owner alone, and gone once taken.
        assert!(keys.take(OTHER).is_none());
        assert_eq!(keys.take(OWNER).map(|key| key.index), Some(253));
        assert!(sign(OWNER).is_none());
        keys.hold(OWNER, key());
        keys.forget();
        assert!(sign(OWNER).is_none());
    }

    #[test]
    fn an_auth_token_is_made_once_and_again_before_it_runs_out() {
        let keys = Keys::default();
        assert_eq!(keys.token(OWNER, 1_000), None);
        keys.hold(OWNER, key());
        let token = keys.token(OWNER, 1_000).unwrap();
        // The deadline an hour on, the account and the key's index.
        assert!(token.starts_with("4600:2180:253:"), "{token}");
        // The same one while it has a while to go, by the address or by its
        // account; another's account has none.
        assert_eq!(
            keys.token(OWNER, 1_000 + TOKEN_LIFE - TOKEN_MARGIN - 1),
            Some(token.clone())
        );
        assert_eq!(keys.token_of(2_180, 1_500), Some(token.clone()));
        assert_eq!(keys.token_of(7, 1_500), None);
        assert_eq!(keys.token(OTHER, 1_500), None);
        // Another once it is about to run out.
        let next = keys
            .token(OWNER, 1_000 + TOKEN_LIFE - TOKEN_MARGIN)
            .unwrap();
        assert_ne!(next, token);
        assert!(next.starts_with(&format!(
            "{}:2180:253:",
            1_000 + 2 * TOKEN_LIFE - TOKEN_MARGIN
        )));
    }
}
