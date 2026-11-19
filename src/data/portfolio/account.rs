//! Which account: an address on a venue.

use serde::Serialize;
use serde_json::Value;

/// An account a widget watches: an address on a venue. Public data: the
/// address is what the venue reports the account by, and nothing here can
/// act for it.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
pub struct Account {
    /// The venue's id (`hyperliquid`): its
    /// [`AccountSource`](super::AccountSource)'s, and the exchange id its
    /// name and logo are filed under.
    pub venue: String,
    /// The address, as the venue's source writes it.
    pub address: String,
}

impl Account {
    /// The account a saved member names (`{ "venue", "address" }`); `None`
    /// unless it names both. The address is kept as saved: the venue's
    /// source wrote it.
    pub fn from_value(value: &Value) -> Option<Self> {
        let member = |name: &str| {
            let text = value.get(name)?.as_str()?.trim();
            (!text.is_empty()).then(|| text.to_owned())
        };
        Some(Self {
            venue: member("venue")?,
            address: member("address")?,
        })
    }

    /// The address in a short form, for where the whole of it has no room
    /// (`0x1234…cdef`): its first six and last four characters.
    pub fn short_address(&self) -> String {
        short_address(&self.address)
    }
}

/// `address` in a short form (`0x1234…cdef`), whole when it is short
/// already.
pub fn short_address(address: &str) -> String {
    let characters: Vec<char> = address.chars().collect();
    if characters.len() <= 13 {
        return address.to_owned();
    }
    let (head, tail) = (&characters[..6], &characters[characters.len() - 4..]);
    head.iter().chain(['…'].iter()).chain(tail).collect()
}

/// `input` as an Ethereum address (what the venues that settle on it
/// report an account by): `0x` and forty hexadecimal digits, in lower
/// case, so one account is one request however it was typed. `None` when
/// it is not one.
pub fn evm_address(input: &str) -> Option<String> {
    let address = input.trim().to_ascii_lowercase();
    let digits = address.strip_prefix("0x")?;
    (digits.len() == 40 && digits.bytes().all(|digit| digit.is_ascii_hexdigit())).then_some(address)
}

/// Why what was typed is not an address of a venue, in words for the
/// person who typed it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AddressError(pub &'static str);

impl AddressError {
    /// No source reports the venue asked for.
    pub const UNKNOWN_VENUE: Self = Self("This exchange's accounts are not available.");

    pub const fn message(self) -> &'static str {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn an_account_reads_from_what_it_saves_and_shortens_its_address() {
        let account = Account {
            venue: "hyperliquid".to_owned(),
            address: "0x0123456789abcdef0123456789abcdef01234567".to_owned(),
        };
        let saved = serde_json::to_value(&account).unwrap();
        assert_eq!(
            saved,
            json!({ "venue": "hyperliquid", "address": account.address })
        );
        assert_eq!(Account::from_value(&saved), Some(account.clone()));
        assert_eq!(account.short_address(), "0x0123…4567");
        assert_eq!(short_address("0xabc"), "0xabc");

        // Both members, as text, or no account.
        for damaged in [
            json!(null),
            json!("0x01"),
            json!({ "venue": "hyperliquid" }),
            json!({ "venue": "", "address": "0x01" }),
            json!({ "venue": "hyperliquid", "address": 7 }),
        ] {
            assert_eq!(Account::from_value(&damaged), None, "{damaged}");
        }
    }
}
