//! alloy's transport over a wallet: every JSON-RPC request alloy makes
//! becomes one EIP-1193 `request({ method, params })` to the wallet, sent
//! down the [`Link`]. With it alloy's whole provider works against the
//! wallet (`RootProvider::new(RpcClient::new(transport, true))`): reads go
//! to the node the wallet uses, and what needs the user's approval
//! (accounts, signatures, transactions) prompts in the wallet.
//!
//! What differs from a node's JSON-RPC, and is evened out here:
//!
//! - A wallet takes `params` as an array or an object, never `null` or
//!   nothing, which is how alloy writes a request with no parameters.
//! - A wallet answers one request at a time: a batch is its requests one
//!   after another.
//! - Wallets read a transaction's calldata as `data`; alloy writes `input`.
//!   A transaction the wallet is given carries it as both.

use std::{
    sync::Arc,
    task::{Context, Poll},
};

use alloy::{
    rpc::json_rpc::{
        ErrorPayload, RequestPacket, Response, ResponsePacket, ResponsePayload, SerializedRequest,
    },
    transports::{TransportError, TransportErrorKind, TransportFut},
};
use serde_json::{Value, value::to_raw_value};
use tower_service::Service;

use super::link::{CallError, Link};

/// The methods whose first parameter is a transaction.
const TRANSACTION_METHODS: [&str; 3] = ["eth_sendTransaction", "eth_call", "eth_estimateGas"];

/// alloy's way to one wallet of a backend.
#[derive(Clone, Debug)]
pub(crate) struct WalletTransport {
    link: Link,
    wallet: Arc<str>,
}

impl WalletTransport {
    /// A transport to the wallet `wallet`
    /// ([`WalletInfo::id`](super::WalletInfo::id)) of `link`'s backend.
    pub(crate) fn new(link: Link, wallet: &str) -> Self {
        Self {
            link,
            wallet: wallet.into(),
        }
    }

    async fn send(&self, request: SerializedRequest) -> Result<Response, TransportError> {
        let params = match request.params() {
            Some(raw) => serde_json::from_str(raw.get()).map_err(TransportError::ser_err)?,
            None => Value::Null,
        };
        let params = wallet_params(request.method(), params);
        let payload = match self
            .link
            .request(&self.wallet, request.method(), params)
            .await
        {
            Ok(result) => {
                ResponsePayload::Success(to_raw_value(&result).map_err(TransportError::ser_err)?)
            }
            Err(CallError::Refused(refusal)) => ResponsePayload::Failure(ErrorPayload {
                code: refusal.code,
                message: refusal.message.into(),
                data: refusal.data.and_then(|data| to_raw_value(&data).ok()),
            }),
            Err(CallError::Gone) => return Err(TransportErrorKind::backend_gone()),
        };
        Ok(Response {
            id: request.id().clone(),
            payload,
        })
    }
}

impl Service<RequestPacket> for WalletTransport {
    type Response = ResponsePacket;
    type Error = TransportError;
    type Future = TransportFut<'static>;

    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, packet: RequestPacket) -> Self::Future {
        let this = self.clone();
        Box::pin(async move {
            match packet {
                RequestPacket::Single(request) => {
                    this.send(request).await.map(ResponsePacket::Single)
                }
                RequestPacket::Batch(requests) => {
                    let mut responses = Vec::with_capacity(requests.len());
                    for request in requests {
                        responses.push(this.send(request).await?);
                    }
                    Ok(ResponsePacket::Batch(responses))
                }
            }
        })
    }
}

/// `params` as a wallet takes them for `method`.
fn wallet_params(method: &str, params: Value) -> Value {
    let mut params = match params {
        Value::Null => Value::Array(Vec::new()),
        params => params,
    };
    if TRANSACTION_METHODS.contains(&method)
        && let Some(Value::Object(transaction)) = params.get_mut(0)
        && !transaction.contains_key("data")
        && let Some(input) = transaction.get("input").cloned()
    {
        transaction.insert("data".to_owned(), input);
    }
    params
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn parameters_are_written_as_a_wallet_reads_them() {
        // Nothing to pass is an empty array, never `null`.
        assert_eq!(wallet_params("eth_requestAccounts", Value::Null), json!([]));
        assert_eq!(
            wallet_params("personal_sign", json!(["0x00", "0x01"])),
            json!(["0x00", "0x01"])
        );
        // alloy's `input` is a wallet's `data` too; one that is there stays.
        assert_eq!(
            wallet_params(
                "eth_sendTransaction",
                json!([{ "to": "0x01", "input": "0xab" }])
            ),
            json!([{ "to": "0x01", "input": "0xab", "data": "0xab" }])
        );
        assert_eq!(
            wallet_params(
                "eth_call",
                json!([{ "input": "0xab", "data": "0xcd" }, "latest"])
            ),
            json!([{ "input": "0xab", "data": "0xcd" }, "latest"])
        );
        // Only where the first parameter is a transaction.
        assert_eq!(
            wallet_params("eth_getLogs", json!([{ "input": "0xab" }])),
            json!([{ "input": "0xab" }])
        );
    }
}
