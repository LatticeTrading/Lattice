//! alloy's `Signer` for an account of the wallet: what anything in alloy
//! that signs can be given.
//!
//! A wallet signs what it can show its user, so:
//!
//! - typed data (EIP-712) is `eth_signTypedData_v4`, the data as JSON text
//!   (what every wallet reads; some refuse an object), always with its
//!   `EIP712Domain` type written out: a wallet hashes the domain by the type
//!   it is given and takes a missing one to have no members;
//! - a message is `personal_sign`;
//! - a bare hash is not signed at all (`eth_sign` is switched off in every
//!   current wallet), so `sign_hash` is unsupported, and with it alloy's
//!   `sign_typed_data` of a `sol!` struct, which hashes first. Make the
//!   struct a `TypedData` (`TypedData::from_struct`) and use
//!   `sign_dynamic_typed_data`.
//!
//! Transactions are not signed here either: a wallet signs and sends in one
//! step (`eth_sendTransaction`), which is the provider's `send_transaction`.

use alloy::{
    dyn_abi::TypedData,
    primitives::{Address, B256, ChainId, Signature, hex},
    providers::{Provider, RootProvider},
    signers::{Error, Result, Signer, UnsupportedSignerOperation},
};
use async_trait::async_trait;

/// One account of a wallet, signing through the wallet's own prompts.
#[derive(Clone, Debug)]
pub struct WalletSigner {
    provider: RootProvider,
    address: Address,
    chain_id: Option<ChainId>,
}

impl WalletSigner {
    /// The account `address` of the wallet behind `provider`, which is on
    /// the chain `chain_id`.
    pub(crate) fn new(provider: RootProvider, address: Address, chain_id: Option<ChainId>) -> Self {
        Self {
            provider,
            address,
            chain_id,
        }
    }

    /// Asks the wallet for `method`'s signature and reads it.
    async fn signature(&self, method: &'static str, params: (String, String)) -> Result<Signature> {
        let signature: String = self
            .provider
            .raw_request(method.into(), params)
            .await
            .map_err(Error::other)?;
        Ok(signature.parse()?)
    }
}

#[cfg_attr(target_family = "wasm", async_trait(?Send))]
#[cfg_attr(not(target_family = "wasm"), async_trait)]
impl Signer for WalletSigner {
    async fn sign_hash(&self, _hash: &B256) -> Result<Signature> {
        Err(Error::UnsupportedOperation(
            UnsupportedSignerOperation::SignHash,
        ))
    }

    async fn sign_message(&self, message: &[u8]) -> Result<Signature> {
        let params = (hex::encode_prefixed(message), self.address.to_string());
        self.signature("personal_sign", params).await
    }

    async fn sign_dynamic_typed_data(&self, payload: &TypedData) -> Result<Signature> {
        let params = (self.address.to_string(), typed_data_text(payload)?);
        self.signature("eth_signTypedData_v4", params).await
    }

    fn address(&self) -> Address {
        self.address
    }

    fn chain_id(&self) -> Option<ChainId> {
        self.chain_id
    }

    fn set_chain_id(&mut self, chain_id: Option<ChainId>) {
        self.chain_id = chain_id;
    }
}

/// `payload` as the JSON text a wallet is given: with the domain's type
/// among its types, and the domain's chain as a plain number, which is how
/// the sites wallets are built against write it (alloy writes hexadecimal
/// text, which not everything behind a wallet reads).
fn typed_data_text(payload: &TypedData) -> Result<String> {
    const DOMAIN: &str = "EIP712Domain";
    let mut data = if payload.resolver.contains_type_name(DOMAIN) {
        serde_json::to_value(payload)
    } else {
        let mut payload = payload.clone();
        payload
            .resolver
            .ingest_string(payload.domain.encode_type())?;
        serde_json::to_value(&payload)
    }
    .map_err(Error::other)?;
    if let Some(chain) = payload.domain.chain_id
        && let Ok(chain) = u64::try_from(chain)
    {
        data["domain"]["chainId"] = chain.into();
    }
    Ok(data.to_string())
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;

    #[test]
    fn typed_data_goes_to_the_wallet_with_its_domain_type() {
        let types = json!({ "Note": [{ "name": "text", "type": "string" }] });
        let payload: TypedData = serde_json::from_value(json!({
            "domain": { "name": "Lattice", "chainId": 42161 },
            "types": types,
            "primaryType": "Note",
            "message": { "text": "hello" },
        }))
        .unwrap();
        let sent: Value = serde_json::from_str(&typed_data_text(&payload).unwrap()).unwrap();
        assert_eq!(
            sent["types"]["EIP712Domain"],
            json!([
                { "name": "name", "type": "string" },
                { "name": "chainId", "type": "uint256" },
            ])
        );
        assert_eq!(sent["types"]["Note"], types["Note"]);
        assert_eq!(
            sent["domain"],
            json!({ "name": "Lattice", "chainId": 42_161 })
        );
        assert_eq!(sent["primaryType"], "Note");
        assert_eq!(sent["message"], json!({ "text": "hello" }));
        // What is hashed here is what the wallet hashes: the type added
        // changes nothing of it.
        let again: TypedData = serde_json::from_value(sent).unwrap();
        assert_eq!(
            again.eip712_signing_hash().unwrap(),
            payload.eip712_signing_hash().unwrap()
        );
    }
}
