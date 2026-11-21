//! The way to a wallet: each request goes down a channel to whatever reaches
//! the wallets (the page's own providers on the web, the bridge to the
//! user's browser on desktop), with where its answer goes.
//!
//! The channel is what lets one alloy transport serve both. alloy asks a
//! transport to be `Send + Sync`, which a browser's provider object and the
//! engine's `Rc`s are not; a [`Link`] is, and what answers it runs on the
//! engine thread like everything else.

use serde_json::Value;
use tokio::sync::{mpsc, oneshot};

/// A wallet's refusal of a request: EIP-1193's `ProviderRpcError`.
#[derive(Clone, Debug, PartialEq)]
pub struct Refusal {
    pub code: i64,
    pub message: String,
    pub data: Option<Value>,
}

impl Refusal {
    /// EIP-1193: the user turned the request down.
    pub const REJECTED: i64 = 4001;
    /// EIP-1193: the wallet does not know the method.
    pub const UNSUPPORTED: i64 = 4200;
    /// EIP-1193: the wallet is not connected to any chain.
    pub const DISCONNECTED: i64 = 4900;
    /// JSON-RPC: a request of the same kind is waiting in the wallet already.
    pub const PENDING: i64 = -32002;
    /// JSON-RPC: an error with no code of its own.
    pub const INTERNAL: i64 = -32603;

    pub fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }
}

/// One request for a wallet, and where its answer goes.
pub(crate) struct Call {
    /// Which of the backend's wallets it is for
    /// ([`WalletInfo::id`](super::WalletInfo::id)).
    pub(crate) wallet: String,
    pub(crate) method: String,
    /// An array or an object, as EIP-1193 takes them.
    pub(crate) params: Value,
    pub(crate) reply: oneshot::Sender<Result<Value, Refusal>>,
}

/// Why a request has no result.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum CallError {
    /// The wallet answered with an error.
    Refused(Refusal),
    /// Nothing reaches the wallet any more: the session ended.
    Gone,
}

/// The sending end: what alloy's transport holds. Cloning shares it.
#[derive(Clone, Debug)]
pub(crate) struct Link(mpsc::UnboundedSender<Call>);

impl Link {
    /// A link, and the calls made through it for a backend to answer.
    pub(crate) fn new() -> (Self, mpsc::UnboundedReceiver<Call>) {
        let (calls, receiver) = mpsc::unbounded_channel();
        (Self(calls), receiver)
    }

    /// Asks `wallet` for `method` and waits for its answer, however long the
    /// user takes over it. Dropping the future abandons the answer, not the
    /// prompt the wallet may be showing.
    pub(crate) async fn request(
        &self,
        wallet: &str,
        method: &str,
        params: Value,
    ) -> Result<Value, CallError> {
        let (reply, answer) = oneshot::channel();
        let call = Call {
            wallet: wallet.to_owned(),
            method: method.to_owned(),
            params,
            reply,
        };
        self.0.send(call).map_err(|_| CallError::Gone)?;
        match answer.await {
            Ok(Ok(result)) => Ok(result),
            Ok(Err(refusal)) => Err(CallError::Refused(refusal)),
            Err(_) => Err(CallError::Gone),
        }
    }
}
