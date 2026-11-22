//! How the wallet service reads what wallets send and says what they
//! refuse. Its flow through the bar's panel, over a stand-in wallet, is
//! tested there (`shell/everything_bar/tests.rs`); the desktop bridge over a
//! real connection in `bridge/tests.rs`.

use alloy::{
    primitives::address,
    rpc::json_rpc::ErrorPayload,
    transports::{TransportError, TransportErrorKind},
};
use serde_json::json;

use super::*;

#[test]
fn accounts_and_chains_are_read_as_wallets_send_them() {
    let account = address!("0x5e9ee1089755c3435139848e47e6635505d5a13a");
    // Checksummed or lower case; what is no address is left out.
    assert_eq!(
        addresses(&json!([
            "0x5E9ee1089755c3435139848e47e6635505d5A13a",
            "nonsense",
            7
        ])),
        [account]
    );
    assert_eq!(
        addresses(&json!(["0x5e9ee1089755c3435139848e47e6635505d5a13a"])),
        [account]
    );
    assert!(addresses(&json!([])).is_empty());
    assert!(addresses(&json!(null)).is_empty());

    assert_eq!(chain_id(&json!("0xa4b1")), Some(42_161));
    assert_eq!(chain_id(&json!("42161")), Some(42_161));
    assert_eq!(chain_id(&json!(42_161)), Some(42_161));
    assert_eq!(chain_id(&json!("0xnothing")), None);
    assert_eq!(chain_id(&json!(null)), None);
}

/// A wallet's error reply as alloy hands it on.
fn refusal(code: i64, message: &str) -> TransportError {
    let payload: ErrorPayload = ErrorPayload {
        code,
        message: message.to_owned().into(),
        data: None,
    };
    TransportError::err_resp(payload)
}

#[test]
fn what_a_wallet_refuses_is_said_in_words() {
    let refused = |code: i64, message: &'static str| describe(&refusal(code, message));
    assert_eq!(
        refused(Refusal::REJECTED, "User rejected the request."),
        "Rejected in the wallet."
    );
    assert!(refused(Refusal::PENDING, "Already processing").contains("already has a request"));
    // The wallet's own words where it has some, its code where it has none.
    assert_eq!(refused(-32000, "insufficient funds"), "insufficient funds");
    assert_eq!(refused(-32000, " "), "The wallet refused (code -32000).");
    assert_eq!(
        describe(&TransportErrorKind::backend_gone()),
        "The wallet can no longer be reached."
    );

    // A signature's refusal is the same refusal, under alloy's signer error.
    let rejected = refusal(Refusal::REJECTED, "User rejected the request.");
    assert_eq!(
        describe_signing(&alloy::signers::Error::other(rejected)),
        "Rejected in the wallet."
    );
}

#[test]
fn what_a_wallet_says_is_cleaned_and_cut_before_it_is_shown() {
    // Its name: no control characters, no longer than a name is, never
    // nothing.
    assert_eq!(named("Rabby Wallet"), "Rabby Wallet");
    assert_eq!(named(" Rabby\u{7}\nWallet "), "Rabby  Wallet");
    assert_eq!(named("\u{0}"), "Wallet");
    assert_eq!(named(&"w".repeat(500)).chars().count(), NAME_LIMIT);

    // What it refuses with, the same.
    assert_eq!(
        describe(&refusal(-32000, "execution\nreverted\u{1b}[31m")),
        "execution reverted [31m"
    );
    let long = describe(&refusal(-32000, &"x".repeat(1_000)));
    assert_eq!(long.chars().count(), SAID_LIMIT);
    assert_eq!(
        describe(&refusal(-32000, "\u{0}")),
        "The wallet refused (code -32000)."
    );
}

#[test]
fn an_address_is_written_as_exchanges_are_asked_about_it() {
    let account = Account {
        wallet: WalletInfo {
            id: "w1".to_owned(),
            name: "Test Wallet".to_owned(),
        },
        address: address!("0x5E9ee1089755c3435139848e47e6635505d5A13a"),
        chain_id: 1,
    };
    assert_eq!(
        account.address_text(),
        "0x5e9ee1089755c3435139848e47e6635505d5a13a"
    );
}
