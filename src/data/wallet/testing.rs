//! A stand-in for the user's wallets: a [`Connector`] that answers as a
//! browser wallet with one account would, signing with a key of its own.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use alloy::{
    dyn_abi::TypedData,
    primitives::{Address, hex},
    signers::{SignerSync, local::PrivateKeySigner},
};
use serde_json::{Value, json};

use super::{Call, Connector, Event, Events, Link, Reach, Refusal, Session, WalletInfo};
use crate::data::net::TaskHandle;

/// The stand-in's key: the one Hyperliquid's SDK signs its tests with.
const KEY: &str = "0x0123456789012345678901234567890123456789012345678901234567890123";
/// Somebody else's key.
const OTHER_KEY: &str = "0x0000000000000000000000000000000000000000000000000000000000000002";

/// Wallets that are not there: what they answer is made here.
#[derive(Clone)]
pub(crate) struct StandIn(Rc<Inner>);

struct Inner {
    reach: Reach,
    wallets: Vec<WalletInfo>,
    key: PrivateKeySigner,
    /// The chain the wallets say they are on.
    chain_id: Cell<u64>,
    /// Whether the user turns down what the wallet asks them.
    rejecting: Cell<bool>,
    /// Whether what is signed is signed by somebody else's key.
    impostor: Cell<bool>,
    /// Every method asked, in order.
    asked: RefCell<Vec<String>>,
    /// Where the open session's events go.
    events: RefCell<Option<Events>>,
}

impl StandIn {
    /// Wallets named `names`, reached as `reach` says.
    pub(crate) fn new(reach: Reach, names: &[&str]) -> Self {
        let wallets = names
            .iter()
            .enumerate()
            .map(|(index, name)| WalletInfo {
                id: format!("w{}", index + 1),
                name: (*name).to_owned(),
            })
            .collect();
        Self(Rc::new(Inner {
            reach,
            wallets,
            key: KEY.parse().expect("the key is one"),
            chain_id: Cell::new(42_161),
            rejecting: Cell::new(false),
            impostor: Cell::new(false),
            asked: RefCell::new(Vec::new()),
            events: RefCell::new(None),
        }))
    }

    /// The account every one of the wallets shares.
    pub(crate) fn address(&self) -> Address {
        self.0.key.address()
    }

    /// Has the user turn down (or approve again) what the wallets ask them.
    pub(crate) fn reject(&self, rejecting: bool) {
        self.0.rejecting.set(rejecting);
    }

    /// Has the wallets sign with a key that is not the account's.
    pub(crate) fn sign_as_another(&self) {
        self.0.impostor.set(true);
    }

    /// The methods asked so far, in order.
    pub(crate) fn asked(&self) -> Vec<String> {
        self.0.asked.borrow().clone()
    }

    /// Has the wallets say `event`, as the open session's backend would.
    pub(crate) fn say(&self, event: Event) {
        let events = self.0.events.borrow().clone();
        if let Some(events) = events {
            events(event);
        }
    }
}

impl Connector for StandIn {
    fn reach(&self) -> Reach {
        self.0.reach
    }

    fn open(&self, events: Events) -> Result<Session, String> {
        let (link, mut calls) = Link::new();
        *self.0.events.borrow_mut() = Some(events.clone());
        let inner = self.0.clone();
        let task = TaskHandle::spawn(async move {
            if inner.reach == Reach::Browser {
                events(Event::Page("http://127.0.0.1:47821/secret".to_owned()));
            }
            events(Event::Wallets(inner.wallets.clone()));
            while let Some(call) = calls.recv().await {
                let answer = inner.answer(&call);
                let _ = call.reply.send(answer);
            }
        });
        Ok(Session::new(link, task))
    }
}

impl Inner {
    fn answer(&self, call: &Call) -> Result<Value, Refusal> {
        self.asked.borrow_mut().push(call.method.clone());
        let prompts = matches!(
            call.method.as_str(),
            "eth_requestAccounts" | "eth_signTypedData_v4" | "personal_sign"
        );
        if prompts && self.rejecting.get() {
            return Err(Refusal::new(
                Refusal::REJECTED,
                "User rejected the request.",
            ));
        }
        let unreadable = |what: &str| Refusal::new(-32602, what);
        let failed =
            |error: alloy::signers::Error| Refusal::new(Refusal::INTERNAL, error.to_string());
        // The key that signs: the account's, unless somebody else's does.
        let key = || -> PrivateKeySigner {
            if self.impostor.get() {
                OTHER_KEY.parse().expect("the key is one")
            } else {
                self.key.clone()
            }
        };
        match call.method.as_str() {
            "eth_requestAccounts" | "eth_accounts" => Ok(json!([self.key.address()])),
            "eth_chainId" => Ok(json!(format!("{:#x}", self.chain_id.get()))),
            "eth_signTypedData_v4" => {
                // As a wallet reads it: the account, then the data as text.
                let data = call.params[1]
                    .as_str()
                    .ok_or_else(|| unreadable("the typed data is not text"))?;
                let data: TypedData =
                    serde_json::from_str(data).map_err(|error| unreadable(&error.to_string()))?;
                let hash = data
                    .eip712_signing_hash()
                    .map_err(|error| unreadable(&error.to_string()))?;
                let signature = key().sign_hash_sync(&hash).map_err(failed)?;
                Ok(json!(hex::encode_prefixed(signature.as_bytes())))
            }
            "personal_sign" => {
                // As a wallet reads it: the message in hexadecimal, then the
                // account.
                let message = call.params[0]
                    .as_str()
                    .ok_or_else(|| unreadable("the message is not text"))?;
                let message =
                    hex::decode(message).map_err(|error| unreadable(&error.to_string()))?;
                let signature = key().sign_message_sync(&message).map_err(failed)?;
                Ok(json!(hex::encode_prefixed(signature.as_bytes())))
            }
            _ => Err(Refusal::new(
                Refusal::UNSUPPORTED,
                "The stand-in wallet does not know this method.",
            )),
        }
    }
}
