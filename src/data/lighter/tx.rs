//! Lighter's transactions: everything that changes an account, as an API
//! key signs it and as it is posted ([`super::requests`]).
//!
//! - A transaction is its kind's number and its members ([`Transaction`]),
//!   with who signs it and until when it may be taken ([`Envelope`]): the
//!   chain, the account and its key's index, the key's next nonce, and a
//!   time after which Lighter refuses it.
//! - What is signed is the Poseidon2 hash of those integers in Lighter's
//!   order ([`Transaction::hash`]), which is also what Lighter calls the
//!   transaction's hash. The orders of a group are each hashed first and
//!   folded into one digest.
//! - What is posted is the transaction as JSON with its signature
//!   ([`Transaction::signed`]): the members as Lighter's Go signer names and
//!   orders them, byte slices in base64, and the attributes a transaction
//!   may carry (an integrator's fee, a nonce to skip) as `null`: none is
//!   ever sent.
//! - Registering a key is the one transaction the account's wallet signs
//!   too ([`register_message`]): the key signs the transaction, and the
//!   wallet a line of text naming the key, which proves the account wants
//!   it.
//! - An auth token is not a transaction: a deadline, the account and the
//!   key's index as text, with the key's signature of it ([`auth_token`]).
//!   It reads what Lighter tells only an account's own key (its open
//!   orders).
//!
//! Checked against Lighter's own signer (the WebAssembly build its site
//! loads, `lighter-go` v1.0.13, run on 2026-11-22): the tests give that
//! signer's public key for its private key, its hash and its JSON for each
//! kind of transaction made here, and find its signatures good.
//!
//! Not made: cancelling every order at once, modifying an order, adding
//! margin to an isolated position, transfers and withdrawals (which the
//! wallet signs, not a key), sub-accounts and pools.

use alloy::primitives::hex;
use serde::Serialize;

use super::crypto::{Digest, Hash, Members, PublicKey, SecretKey};

/// Which Lighter an API is: its chain's id is part of what is signed, so
/// what is signed for one is nothing on the other.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Network {
    Mainnet,
    Testnet,
}

impl Network {
    /// The net the API at `http_base` is: the mainnet's is the one that
    /// says so, and anything else (the testnet, a stand-in) is signed for
    /// as the testnet.
    pub fn of(http_base: &str) -> Self {
        let host = url::Url::parse(http_base)
            .ok()
            .and_then(|url| url.host_str().map(str::to_ascii_lowercase));
        match host {
            Some(host) if host.starts_with("mainnet.") => Self::Mainnet,
            _ => Self::Testnet,
        }
    }

    /// The chain's id, as transactions name it.
    pub const fn chain_id(self) -> u32 {
        match self {
            Self::Mainnet => 304,
            Self::Testnet => 300,
        }
    }

    /// What it is called where a key is kept.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Mainnet => "Mainnet",
            Self::Testnet => "Testnet",
        }
    }
}

/// Who signs a transaction and until when it may be taken.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Envelope {
    pub network: Network,
    /// The account's index.
    pub account: u64,
    /// Which of the account's API keys signs.
    pub key_index: u8,
    /// The key's nonce: one more than its last transaction's.
    pub nonce: u64,
    /// When Lighter stops taking the transaction, in Unix milliseconds.
    pub expires_at: u64,
}

/// What kind an order is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum OrderType {
    Limit,
    /// At what the book gives, no worse than its price.
    Market,
    /// At the market's price once its trigger is reached, with a loss.
    StopLoss,
    /// The same, with a gain.
    TakeProfit,
}

impl OrderType {
    const fn number(self) -> u8 {
        match self {
            Self::Limit => 0,
            Self::Market => 1,
            Self::StopLoss => 2,
            Self::TakeProfit => 4,
        }
    }
}

/// How long an order stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Standing {
    /// What does not trade at once is cancelled.
    ImmediateOrCancel,
    /// It rests until its expiry.
    GoodTillTime,
    /// It only ever rests.
    PostOnly,
}

impl Standing {
    const fn number(self) -> u8 {
        match self {
            Self::ImmediateOrCancel => 0,
            Self::GoodTillTime => 1,
            Self::PostOnly => 2,
        }
    }
}

/// How the orders of a group stand to each other.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Grouping {
    /// An order and its one exit: the first, once it trades, places the
    /// second (Lighter's "one triggers the other").
    WithExit,
    /// The two exits of a position: whichever trades cancels the other
    /// ("one cancels the other").
    Exits,
    /// An order and both its exits: the first, once it trades, places the
    /// other two, each of which cancels the other ("one triggers a one
    /// cancels the other").
    WithExits,
}

impl Grouping {
    const fn number(self) -> u8 {
        match self {
            Self::WithExit => 1,
            Self::Exits => 2,
            Self::WithExits => 3,
        }
    }
}

/// An order, in Lighter's integers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Order {
    /// The market's index.
    pub market: u64,
    /// The client's own number for it; zero for none.
    pub client_index: u64,
    /// How much, in the market's smallest size. Zero, for an order that
    /// only reduces, is the whole of the position whatever that comes to.
    pub base_amount: u64,
    /// Its price, in the market's smallest price: the limit, or the worst
    /// price it may trade at.
    pub price: u32,
    /// Whether it sells.
    pub ask: bool,
    pub kind: OrderType,
    pub standing: Standing,
    pub reduce_only: bool,
    /// The price it waits for, in the market's smallest price; zero for one
    /// that waits for none.
    pub trigger_price: u32,
    /// When it stops resting or waiting, in Unix milliseconds; zero for one
    /// that does neither.
    pub expires_at: u64,
}

impl Order {
    /// The order's own members, in the order they are hashed and written.
    fn members(&self) -> [u64; 10] {
        [
            self.market,
            self.client_index,
            self.base_amount,
            u64::from(self.price),
            u64::from(self.ask),
            u64::from(self.kind.number()),
            u64::from(self.standing.number()),
            u64::from(self.reduce_only),
            u64::from(self.trigger_price),
            self.expires_at,
        ]
    }

    fn wire(&self) -> WireOrder {
        WireOrder {
            market_index: self.market,
            client_order_index: self.client_index,
            base_amount: self.base_amount,
            price: self.price,
            is_ask: u8::from(self.ask),
            kind: self.kind.number(),
            time_in_force: self.standing.number(),
            reduce_only: u8::from(self.reduce_only),
            trigger_price: self.trigger_price,
            order_expiry: self.expires_at,
        }
    }
}

/// Something that changes an account.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Transaction {
    /// Registers `key` as the API key at the envelope's index, in place of
    /// any there. `wallet_signature` is the account's wallet's of
    /// [`register_message`], as hexadecimal text.
    ChangePubKey {
        key: PublicKey,
        wallet_signature: String,
    },
    CreateOrder(Order),
    /// Several orders of one market that stand to each other as `grouping`
    /// says: two or three.
    CreateGroupedOrders {
        grouping: Grouping,
        orders: Vec<Order>,
    },
    /// Cancels the order `order` (its index) of `market`.
    CancelOrder {
        market: u64,
        order: u64,
    },
    /// Sets how the account trades `market`: the initial margin its
    /// positions take, in hundredths of a percent of their worth (`500` is
    /// 20x), and whether that margin is each position's own.
    UpdateLeverage {
        market: u64,
        margin_fraction: u16,
        isolated: bool,
    },
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct WireOrder {
    market_index: u64,
    client_order_index: u64,
    base_amount: u64,
    price: u32,
    is_ask: u8,
    #[serde(rename = "Type")]
    kind: u8,
    time_in_force: u8,
    reduce_only: u8,
    trigger_price: u32,
    order_expiry: u64,
}

/// What every transaction's JSON starts with.
#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct Head {
    account_index: u64,
    api_key_index: u8,
}

/// What every transaction's JSON ends with.
#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct Tail {
    expired_at: u64,
    nonce: u64,
    /// The key's signature, in base64.
    sig: String,
    /// The attributes it carries: none.
    #[serde(rename = "L2TxAttributes")]
    attributes: Option<()>,
}

/// A transaction's JSON: its head, its own members, its tail.
#[derive(Serialize)]
struct Wire<T: Serialize> {
    #[serde(flatten)]
    head: Head,
    #[serde(flatten)]
    body: T,
    #[serde(flatten)]
    tail: Tail,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct WireChangePubKey<'a> {
    /// The key, in base64.
    pub_key: String,
    #[serde(rename = "L1Sig")]
    wallet_signature: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct WireGroup {
    grouping_type: u8,
    orders: Vec<WireOrder>,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct WireCancel {
    market_index: u64,
    index: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct WireLeverage {
    market_index: u64,
    initial_margin_fraction: u16,
    margin_mode: u8,
}

/// A transaction ready to post.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Signed {
    /// The kind's number (`tx_type`).
    pub kind: u8,
    /// The transaction as JSON, signature and all (`tx_info`).
    pub info: String,
    /// What was signed: the hash Lighter knows the transaction by.
    pub hash: Hash,
}

impl Transaction {
    /// The kind's number.
    pub(super) const fn kind(&self) -> u8 {
        match self {
            Self::ChangePubKey { .. } => 8,
            Self::CreateOrder(_) => 14,
            Self::CancelOrder { .. } => 15,
            Self::UpdateLeverage { .. } => 20,
            Self::CreateGroupedOrders { .. } => 28,
        }
    }

    /// What is signed: the hash of the envelope's members and the
    /// transaction's own, in Lighter's order.
    pub(super) fn hash(&self, envelope: &Envelope) -> Hash {
        let head = [
            u64::from(envelope.network.chain_id()),
            u64::from(self.kind()),
            envelope.nonce,
            envelope.expires_at,
            envelope.account,
            u64::from(envelope.key_index),
        ];
        let with = |own: &[u64]| Members::of(head.into_iter().chain(own.iter().copied()));
        match self {
            Self::ChangePubKey { key, .. } => with(&[]).with_key(*key).hash(),
            Self::CreateOrder(order) => with(&order.members()).hash(),
            Self::CreateGroupedOrders { grouping, orders } => {
                let digest = |order: &Order| Members::of(order.members()).digest();
                let folded = orders
                    .iter()
                    .map(digest)
                    .reduce(|all, next| all.joined(&next));
                // A group of none is nothing Lighter takes; it is hashed
                // as Lighter would hash it all the same.
                with(&[u64::from(grouping.number())])
                    .with_digest(&folded.unwrap_or(Digest::NONE))
                    .hash()
            }
            Self::CancelOrder { market, order } => with(&[*market, *order]).hash(),
            Self::UpdateLeverage {
                market,
                margin_fraction,
                isolated,
            } => with(&[*market, u64::from(*margin_fraction), u64::from(*isolated)]).hash(),
        }
    }

    /// The transaction as JSON with `signature` as its key's.
    fn info(&self, envelope: &Envelope, signature: &[u8; 80]) -> String {
        let head = Head {
            account_index: envelope.account,
            api_key_index: envelope.key_index,
        };
        let tail = Tail {
            expired_at: envelope.expires_at,
            nonce: envelope.nonce,
            sig: base64(signature),
            attributes: None,
        };
        fn written<T: Serialize>(head: Head, body: T, tail: Tail) -> String {
            // Numbers and text alone: there is nothing in it to fail on.
            serde_json::to_string(&Wire { head, body, tail }).unwrap_or_default()
        }
        match self {
            Self::ChangePubKey {
                key,
                wallet_signature,
            } => written(
                head,
                WireChangePubKey {
                    pub_key: base64(&key.to_bytes()),
                    wallet_signature,
                },
                tail,
            ),
            Self::CreateOrder(order) => written(head, order.wire(), tail),
            Self::CreateGroupedOrders { grouping, orders } => written(
                head,
                WireGroup {
                    grouping_type: grouping.number(),
                    orders: orders.iter().map(Order::wire).collect(),
                },
                tail,
            ),
            Self::CancelOrder { market, order } => written(
                head,
                WireCancel {
                    market_index: *market,
                    index: *order,
                },
                tail,
            ),
            Self::UpdateLeverage {
                market,
                margin_fraction,
                isolated,
            } => written(
                head,
                WireLeverage {
                    market_index: *market,
                    initial_margin_fraction: *margin_fraction,
                    margin_mode: u8::from(*isolated),
                },
                tail,
            ),
        }
    }

    /// The transaction signed by `key` for `envelope`, ready to post.
    pub(super) fn signed(&self, envelope: &Envelope, key: &SecretKey) -> Signed {
        let hash = self.hash(envelope);
        let signature = key.sign(hash).to_bytes();
        Signed {
            kind: self.kind(),
            info: self.info(envelope, &signature),
            hash,
        }
    }
}

/// What the account's wallet signs to register `key` at `envelope`'s index
/// (a message, `personal_sign`): the text Lighter makes of the same members
/// and recovers the account's address from. Every number is sixteen
/// hexadecimal digits.
pub(super) fn register_message(key: PublicKey, envelope: &Envelope) -> String {
    format!(
        "Register Lighter Account\n\npubkey: 0x{key}\nnonce: 0x{:016x}\naccount index: \
         0x{:016x}\napi key index: 0x{:016x}\nOnly sign this message for a trusted client!",
        envelope.nonce, envelope.account, envelope.key_index
    )
}

/// An auth token of the key `key`, the one at `key_index` of `account`,
/// good until `deadline` (Unix seconds): what a request or a subscription
/// that reads the account's own carries.
pub(super) fn auth_token(key: &SecretKey, account: u64, key_index: u8, deadline: u64) -> String {
    let message = format!("{deadline}:{account}:{key_index}");
    let signature = key.sign(Members::text(&message).hash());
    format!("{message}:{}", hex::encode(signature.to_bytes()))
}

/// `bytes` in base64, as Go writes a byte slice in JSON: the standard
/// alphabet, padded.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut written = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let three = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let sixes = [
            three[0] >> 2,
            (three[0] & 0x03) << 4 | three[1] >> 4,
            (three[1] & 0x0F) << 2 | three[2] >> 6,
            three[2] & 0x3F,
        ];
        for (place, six) in sixes.into_iter().enumerate() {
            // A byte gives two digits, two give three.
            if place <= chunk.len() {
                written.push(char::from(ALPHABET[usize::from(six)]));
            } else {
                written.push('=');
            }
        }
    }
    written
}

/// What Lighter's own signer made, and what is made here, side by side.
///
/// The signer is the WebAssembly build Lighter's site loads
/// (`lighter-signer-web.wasm`, a release asset of `lighter-go`), run under
/// Node with Go's `wasm_exec.js`: `_createClientByPrv(key, 304, account,
/// nonce, index)` for the public key and the text a wallet signs, then
/// `_signCreateOrder`, `_signCreateGroupedOrders`, `_signCancelOrder`,
/// `_signUpdateLeverage`, `_signChangePubKey` and `_createAuthToken`, each
/// of which answers its transaction's JSON and hash. Its signatures differ
/// from run to run (a nonce each); its hashes and its JSON do not.
#[cfg(test)]
pub(super) mod tests {
    use super::{super::crypto::verifies, *};

    /// The key Lighter's signer was given.
    pub(in crate::data::lighter) const SIGNER_KEY: &str =
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef0011223344556677";
    /// The public key it made of it.
    const SIGNER_PUBLIC: &str =
        "921babf1db8b9e8a2deb30afdb0d36d8bd57664e4d351ee65f3345ae51acd147b0fa1903f6cc2ba6";
    /// The account and the key's index it signed as, on the mainnet.
    const ACCOUNT: u64 = 281_474_976_710;
    const KEY_INDEX: u8 = 7;
    /// When its orders stop resting.
    const ORDER_EXPIRY: u64 = 1_795_000_000_000;

    pub(in crate::data::lighter) fn signer_key() -> SecretKey {
        let mut bytes = [0; 40];
        hex::decode_to_slice(SIGNER_KEY, &mut bytes).unwrap();
        SecretKey::from_bytes(&bytes).unwrap()
    }

    fn envelope(nonce: u64, expires_at: u64) -> Envelope {
        Envelope {
            network: Network::Mainnet,
            account: ACCOUNT,
            key_index: KEY_INDEX,
            nonce,
            expires_at,
        }
    }

    /// A limit buy of 0.0015 BTC at 61,125.7 that rests.
    fn limit() -> Order {
        Order {
            market: 1,
            client_index: 0,
            base_amount: 150,
            price: 611_257,
            ask: false,
            kind: OrderType::Limit,
            standing: Standing::GoodTillTime,
            reduce_only: false,
            trigger_price: 0,
            expires_at: ORDER_EXPIRY,
        }
    }

    /// An exit of a whole long: sold at the market once `trigger` is
    /// reached, no worse than `price`.
    fn exit(kind: OrderType, trigger: u32, price: u32) -> Order {
        Order {
            market: 1,
            client_index: 0,
            base_amount: 0,
            price,
            ask: true,
            kind,
            standing: Standing::ImmediateOrCancel,
            reduce_only: true,
            trigger_price: trigger,
            expires_at: ORDER_EXPIRY,
        }
    }

    /// What Lighter's signer made of `transaction` at `nonce`: the hash, and
    /// the JSON with its signature, which holds for the key.
    fn check(
        transaction: &Transaction,
        nonce: u64,
        expires_at: u64,
        hash: &str,
        signature: &str,
        info: &str,
    ) {
        let envelope = envelope(nonce, expires_at);
        let hashed = transaction.hash(&envelope);
        assert_eq!(hashed.to_string(), hash, "{transaction:?}");
        let mut signed = [0; 80];
        hex::decode_to_slice(signature, &mut signed).unwrap();
        assert_eq!(transaction.info(&envelope, &signed), info);
        let key = signer_key();
        assert!(verifies(key.public(), hashed, &signed), "{transaction:?}");

        // And what is made here holds too, for the same hash.
        let made = transaction.signed(&envelope, &key);
        assert_eq!((made.kind, made.hash), (transaction.kind(), hashed));
        let written: serde_json::Value = serde_json::from_str(&made.info).unwrap();
        assert_eq!(written["Nonce"], nonce);
        assert_eq!(written["L2TxAttributes"], serde_json::Value::Null);
    }

    #[test]
    fn a_key_is_the_public_key_lighters_signer_makes_of_it() {
        assert_eq!(signer_key().public().to_string(), SIGNER_PUBLIC);
    }

    #[test]
    fn orders_are_hashed_and_written_as_lighters_signer_does() {
        check(
            &Transaction::CreateOrder(limit()),
            6,
            1_795_342_103_866,
            "e836f5a342d94f167c372b39a90290b71cd02594728aec20aad75194fd790b05aa7a64a575fe1b58",
            "1eb3a81046a09092fc12a932e3126eea82aa3ef71d6f0052664fb3d3d85909fc2edf5b2dd1219d0f\
             309f6fe73c75d2e64d97d01e4b782ec196386ade4fb536e30eef951e5dceed2f44dd2d2fe462fa2f",
            r#"{"AccountIndex":281474976710,"ApiKeyIndex":7,"MarketIndex":1,"ClientOrderIndex":0,"BaseAmount":150,"Price":611257,"IsAsk":0,"Type":0,"TimeInForce":1,"ReduceOnly":0,"TriggerPrice":0,"OrderExpiry":1795000000000,"ExpiredAt":1795342103866,"Nonce":6,"Sig":"HrOoEEagkJL8Eqky4xJu6oKqPvcdbwBSZk+z09hZCfwu31st0SGdDzCfb+c8ddLmTZfQHkt4LsGWOGreT7U24w7vlR5dzu0vRN0tL+Ri+i8=","L2TxAttributes":null}"#,
        );
        // A market sell that only reduces, with a client's number.
        let market = Order {
            client_index: 77,
            price: 641_820,
            ask: true,
            kind: OrderType::Market,
            standing: Standing::ImmediateOrCancel,
            reduce_only: true,
            expires_at: 0,
            ..limit()
        };
        check(
            &Transaction::CreateOrder(market),
            7,
            1_795_342_103_869,
            "38076da7093b20ae6a246a77d7c6622dfff23dc7525a86f1244b55130f9878f7e6ab05882d23a5c3",
            "a0ebe111b4265fe2558a9af4999aa209bae636a85334bdb2a2b16f07b5ac701db36c43e39c6ff318\
             56f985b38ec8b19e8ac33b3099bb9479fd310764b368d76d7da56cbb9f660eb9cb3800bbe7a16b4d",
            r#"{"AccountIndex":281474976710,"ApiKeyIndex":7,"MarketIndex":1,"ClientOrderIndex":77,"BaseAmount":150,"Price":641820,"IsAsk":1,"Type":1,"TimeInForce":0,"ReduceOnly":1,"TriggerPrice":0,"OrderExpiry":0,"ExpiredAt":1795342103869,"Nonce":7,"Sig":"oOvhEbQmX+JVipr0mZqiCbrmNqhTNL2yorFvB7WscB2zbEPjnG/zGFb5hbOOyLGeisM7MJm7lHn9MQdks2jXbX2lbLufZg65yzgAu+eha00=","L2TxAttributes":null}"#,
        );
        // A stop loss of a whole position: no size of its own.
        check(
            &Transaction::CreateOrder(exit(OrderType::StopLoss, 580_000, 550_131)),
            8,
            1_795_342_103_870,
            "2229aa92cfbfe3e774584429b9f7589d34c60dab046e1ffd800a512f162fbccbcb13bbb8494b49c9",
            "7101fbb02a34ef6339af535fb0b0c3324fa62074038f22e3a09ba71a2a94b5b3547f8cf4d9020405\
             594d69a2d534f2de5509ef44a3b356efdd71eeca810f63242339b247400005fe184ee29e3457e262",
            r#"{"AccountIndex":281474976710,"ApiKeyIndex":7,"MarketIndex":1,"ClientOrderIndex":0,"BaseAmount":0,"Price":550131,"IsAsk":1,"Type":2,"TimeInForce":0,"ReduceOnly":1,"TriggerPrice":580000,"OrderExpiry":1795000000000,"ExpiredAt":1795342103870,"Nonce":8,"Sig":"cQH7sCo072M5r1NfsLDDMk+mIHQDjyLjoJunGiqUtbNUf4z02QIEBVlNaaLVNPLeVQnvRKOzVu/dce7KgQ9jJCM5skdAAAX+GE7injRX4mI=","L2TxAttributes":null}"#,
        );
    }

    #[test]
    fn grouped_orders_are_each_hashed_and_folded_into_one() {
        let take_profit = exit(OrderType::TakeProfit, 650_000, 585_000);
        let stop_loss = exit(OrderType::StopLoss, 580_000, 522_000);
        // An order with both its exits.
        check(
            &Transaction::CreateGroupedOrders {
                grouping: Grouping::WithExits,
                orders: vec![limit(), take_profit, stop_loss],
            },
            11,
            1_795_342_103_876,
            "64723f5daefcac97cc7bca4c69ae7ee608a481bd25589e80f75dea1a349819f76c22145f6d17fdb7",
            "26126dae6b2036f6d2955841bd66b1ec3bd3aa441d673f5841a577cd7d42e54b7417d40fa1426515\
             2caa9ce1183228fb7923363c605adaba75d15532ae35bfa557213b112c3addfae4bec195e53d4a01",
            r#"{"AccountIndex":281474976710,"ApiKeyIndex":7,"GroupingType":3,"Orders":[{"MarketIndex":1,"ClientOrderIndex":0,"BaseAmount":150,"Price":611257,"IsAsk":0,"Type":0,"TimeInForce":1,"ReduceOnly":0,"TriggerPrice":0,"OrderExpiry":1795000000000},{"MarketIndex":1,"ClientOrderIndex":0,"BaseAmount":0,"Price":585000,"IsAsk":1,"Type":4,"TimeInForce":0,"ReduceOnly":1,"TriggerPrice":650000,"OrderExpiry":1795000000000},{"MarketIndex":1,"ClientOrderIndex":0,"BaseAmount":0,"Price":522000,"IsAsk":1,"Type":2,"TimeInForce":0,"ReduceOnly":1,"TriggerPrice":580000,"OrderExpiry":1795000000000}],"ExpiredAt":1795342103876,"Nonce":11,"Sig":"JhJtrmsgNvbSlVhBvWax7DvTqkQdZz9YQaV3zX1C5Ut0F9QPoUJlFSyqnOEYMij7eSM2PGBa2rp10VUyrjW/pVchOxEsOt365L7BleU9SgE=","L2TxAttributes":null}"#,
        );
        // The two exits of a position held.
        check(
            &Transaction::CreateGroupedOrders {
                grouping: Grouping::Exits,
                orders: vec![take_profit, stop_loss],
            },
            12,
            1_795_342_103_879,
            "8e98e4d618dacd6b4e58edc41db39a1383d15729d7e7544923991ca15c8885f83dad476a9fb88325",
            "333c90745c576eb49c3e60a49d53bd86dddf32e463edf3ab046c2ddedb9ba7ed09bed5c5e813a567\
             57194b6375d91a152f8a19ddbbf4e5bb3bd491ef748f97e93d3cee995986d43bfa9359ea74797040",
            r#"{"AccountIndex":281474976710,"ApiKeyIndex":7,"GroupingType":2,"Orders":[{"MarketIndex":1,"ClientOrderIndex":0,"BaseAmount":0,"Price":585000,"IsAsk":1,"Type":4,"TimeInForce":0,"ReduceOnly":1,"TriggerPrice":650000,"OrderExpiry":1795000000000},{"MarketIndex":1,"ClientOrderIndex":0,"BaseAmount":0,"Price":522000,"IsAsk":1,"Type":2,"TimeInForce":0,"ReduceOnly":1,"TriggerPrice":580000,"OrderExpiry":1795000000000}],"ExpiredAt":1795342103879,"Nonce":12,"Sig":"MzyQdFxXbrScPmCknVO9ht3fMuRj7fOrBGwt3tubp+0JvtXF6BOlZ1cZS2N12RoVL4oZ3bv05bs71JHvdI+X6T087plZhtQ7+pNZ6nR5cEA=","L2TxAttributes":null}"#,
        );
    }

    #[test]
    fn a_cancel_and_a_leverage_are_hashed_and_written_as_lighters_signer_does() {
        check(
            &Transaction::CancelOrder {
                market: 1,
                order: 281_474_977_354_074,
            },
            9,
            1_795_342_103_872,
            "73ce8090d212f247a192370eebf1c9f4ef514923d609547f88a2ade67d8323e3b19a7a718286ff76",
            "1a00f31e132f9ffe86abce4a5c02c5b721f0fda4255bd35ab871114a065c6f0c9a261b299815a014\
             a869c419b0a84f699af5dc1057a742c56d15fcea1fa9429bb07c340a418ddd423e161f7b8b4c8213",
            r#"{"AccountIndex":281474976710,"ApiKeyIndex":7,"MarketIndex":1,"Index":281474977354074,"ExpiredAt":1795342103872,"Nonce":9,"Sig":"GgDzHhMvn/6Gq85KXALFtyHw/aQlW9NauHERSgZcbwyaJhspmBWgFKhpxBmwqE9pmvXcEFenQsVtFfzqH6lCm7B8NApBjd1CPhYfe4tMghM=","L2TxAttributes":null}"#,
        );
        // 20x, with each position's own margin.
        check(
            &Transaction::UpdateLeverage {
                market: 1,
                margin_fraction: 500,
                isolated: true,
            },
            10,
            1_795_342_103_874,
            "1012a04ec76fc2252b9d87c118c4d6bcfc0b685736da655b9086c4046163c8f03df10cd606016436",
            "58e53cc625031ee1842ec681252ff8cb6b5d4d21fa690eb2c29680b081feb83435a1c21c9c104d67\
             30786d3d76a274b76aa0707e08ebb45807f98c86f8185bc55892d81de461626e8b9dbe9d7dc67268",
            r#"{"AccountIndex":281474976710,"ApiKeyIndex":7,"MarketIndex":1,"InitialMarginFraction":500,"MarginMode":1,"ExpiredAt":1795342103874,"Nonce":10,"Sig":"WOU8xiUDHuGELsaBJS/4y2tdTSH6aQ6ywpaAsIH+uDQ1ocIcnBBNZzB4bT12onS3aqBwfgjrtFgH+YyG+BhbxViS2B3kYWJui52+nX3Gcmg=","L2TxAttributes":null}"#,
        );
    }

    #[test]
    fn a_key_is_registered_by_itself_and_by_the_accounts_wallet() {
        let key = signer_key().public();
        // What the wallet is shown and signs.
        assert_eq!(
            register_message(key, &envelope(5, 0)),
            "Register Lighter Account\n\npubkey: \
             0x921babf1db8b9e8a2deb30afdb0d36d8bd57664e4d351ee65f3345ae51acd147b0fa1903f6cc2ba6\n\
             nonce: 0x0000000000000005\naccount index: 0x0000004189374bc6\napi key index: \
             0x0000000000000007\nOnly sign this message for a trusted client!"
        );
        // The transaction, with a wallet's signature beside the key's.
        let wallet_signature = format!("0x{}1b", "ab".repeat(64));
        check(
            &Transaction::ChangePubKey {
                key,
                wallet_signature,
            },
            5,
            1_795_342_103_859,
            "00e135ed3cd1515d4c08384c04a5503499204f728bd6062caf7175096222ad63a6d105c41c198cab",
            "51a2449cbcbc4baab0f36d5b6520aae11bddaab44823b65ab95c96aa65ba6c67423b92cecd5df637\
             25330d7655b8e902bd197d7ede0eb6ba65bee4720a5f7a0807acb422c75c2a438bee69ab499b6634",
            r#"{"AccountIndex":281474976710,"ApiKeyIndex":7,"PubKey":"khur8duLnoot6zCv2w022L1XZk5NNR7mXzNFrlGs0Uew+hkD9swrpg==","L1Sig":"0xabababababababababababababababababababababababababababababababababababababababababababababababababababababababababababababababab1b","ExpiredAt":1795342103859,"Nonce":5,"Sig":"UaJEnLy8S6qw821bZSCq4RvdqrRII7ZauVyWqmW6bGdCO5LOzV32NyUzDXZVuOkCvRl9ft4OtrplvuRyCl96CAestCLHXCpDi+5pq0mbZjQ=","L2TxAttributes":null}"#,
        );
    }

    #[test]
    fn an_auth_token_is_a_deadline_signed_by_the_key() {
        let key = signer_key();
        // The token Lighter's signer made for the same key and deadline: its
        // text is hashed here to what that signature holds for.
        let message = "1795345104:281474976710:7";
        let mut theirs = [0; 80];
        hex::decode_to_slice(
            "a930290452569274c9534cc8ed04b3bbf1910d4858cb3eb2a1cc03e61a98cded3c17239396df967d\
             fc48cc8f9a173377dbb6397ef2d8b9fb9377f8eadd9589ffc944630b2b41c2ee064a98d14000d05e",
            &mut theirs,
        )
        .unwrap();
        let hashed = Members::text(message).hash();
        assert!(verifies(key.public(), hashed, &theirs));

        // The one made here: the same text, then a signature that holds.
        let token = auth_token(&key, ACCOUNT, KEY_INDEX, 1_795_345_104);
        let (said, signature) = token.rsplit_once(':').unwrap();
        assert_eq!(said, message);
        let mut ours = [0; 80];
        hex::decode_to_slice(signature, &mut ours).unwrap();
        assert!(verifies(key.public(), hashed, &ours));
        assert!(!verifies(
            key.public(),
            Members::text("1795345105:281474976710:7").hash(),
            &ours
        ));
    }

    #[test]
    fn the_net_is_the_apis_and_names_its_chain() {
        assert_eq!(
            Network::of("https://mainnet.zklighter.elliot.ai"),
            Network::Mainnet
        );
        for other in [
            "https://testnet.zklighter.elliot.ai",
            "https://lighter.test",
            "http://127.0.0.1:3001",
            "not a url",
        ] {
            assert_eq!(Network::of(other), Network::Testnet, "{other}");
        }
        assert_eq!(Network::Mainnet.chain_id(), 304);
        assert_eq!(Network::Testnet.chain_id(), 300);
        // What is signed for one net is nothing on the other.
        let transaction = Transaction::CancelOrder {
            market: 1,
            order: 2,
        };
        let testnet = Envelope {
            network: Network::Testnet,
            ..envelope(1, 2)
        };
        assert_ne!(
            transaction.hash(&envelope(1, 2)),
            transaction.hash(&testnet)
        );
    }

    #[test]
    fn bytes_are_written_in_base64_as_go_writes_them() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64(&[0xFB, 0xFF, 0xBE]), "+/++");
    }
}
