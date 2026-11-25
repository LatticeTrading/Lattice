//! How a Lighter API key signs: everything a transaction's signature and
//! an auth token's take, in Rust, so that it runs wherever the app does
//! (the browser too).
//!
//! Lighter is a zero-knowledge rollup, and what its accounts sign is made
//! to be cheap to prove there, not what an Ethereum wallet signs: Schnorr
//! signatures ([`schnorr`]) over the curve ECgFp5 ([`curve`], [`scalar`]),
//! which is defined over the degree-five extension of the Goldilocks field
//! ([`field`]), with Poseidon2 as the hash ([`poseidon`]). Lighter
//! publishes one implementation, in Go (`github.com/elliottech/lighter-go`
//! over `github.com/elliottech/poseidon_crypto`), which its other SDKs and
//! its own site load as a compiled library; none is in Rust, so this is a
//! port of what that one computes, part for part.
//!
//! What the rest of the integration sees of it:
//!
//! - [`SecretKey`] and [`PublicKey`]: an API key, made from the system's
//!   random source, and the forty bytes Lighter knows it by;
//! - [`Members`]: what is signed, as the integers it is made of in the
//!   order Lighter hashes them, with [`Digest`] for several of them folded
//!   into one;
//! - [`Hash`]: what they hash to, which is what a key signs and what
//!   Lighter calls the transaction's hash;
//! - a signature: eighty bytes ([`SecretKey::sign`]).
//!
//! How it is known to be right, none of it being checkable by eye:
//!
//! - each layer has the reference implementation's own test vectors as its
//!   tests (the permutation, the sponge, and signatures with their nonces
//!   given), which pass only if every layer under them computes what the
//!   reference does;
//! - what is built on it ([`super::tx`]) is checked against Lighter's own
//!   signer, run as it ships (the WebAssembly build its site loads,
//!   `lighter-go` v1.0.13): the same key gives the same public key, the same
//!   transactions give the same hashes, and the signatures that signer made
//!   hold here.
//!
//! What it is not: hardened against an attacker who can time it or read the
//! process's memory. A key is wiped when it is dropped and a scalar
//! multiplication does the same steps whatever the scalar, but the field
//! arithmetic under it takes whatever time its operands make it take
//! (as the reference's does). What the keys it holds can do is said where
//! they are kept ([`super::key`]).

mod curve;
mod field;
mod poseidon;
mod scalar;
mod schnorr;

use std::fmt;

use alloy::primitives::hex;

use field::{Fp, Fp5};
#[cfg(test)]
pub(super) use schnorr::verifies;
pub(super) use schnorr::{PublicKey, SecretKey};

/// What a key signs: the hash of a transaction's members, or of a
/// message's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Hash(Fp5);

impl Hash {
    /// The hash's forty bytes.
    pub(super) fn to_bytes(self) -> [u8; 40] {
        self.0.to_bytes()
    }
}

/// As Lighter writes a transaction's: eighty hexadecimal digits.
impl fmt::Display for Hash {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&hex::encode(self.to_bytes()))
    }
}

/// Several members hashed into four, to be members of something larger
/// themselves (the orders of a group).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Digest(poseidon::Digest);

impl Digest {
    /// The digest of nothing at all: four zeros.
    pub(super) const NONE: Self = Self([Fp::ZERO; 4]);

    /// This digest and `next` hashed into one.
    pub(super) fn joined(&self, next: &Self) -> Self {
        Self(poseidon::digest_of_two(&self.0, &next.0))
    }
}

/// The members of what is hashed, in the order they are hashed in.
#[derive(Clone, Debug, Default)]
pub(super) struct Members(Vec<Fp>);

impl Members {
    /// The integers `values`, each a member.
    pub(super) fn of(values: impl IntoIterator<Item = u64>) -> Self {
        Self(values.into_iter().map(Fp::new).collect())
    }

    /// These members, then `key`'s five.
    #[must_use]
    pub(super) fn with_key(mut self, key: PublicKey) -> Self {
        self.0.extend(key.0.0);
        self
    }

    /// These members, then `digest`'s four.
    #[must_use]
    pub(super) fn with_digest(mut self, digest: &Digest) -> Self {
        self.0.extend(digest.0);
        self
    }

    /// `text` as members: its bytes eight at a time, little endian, the last
    /// ones filled up with zeros. (Eight bytes of text are always below the
    /// field's order: none of them is `0xFF`.)
    pub(super) fn text(text: &str) -> Self {
        let member = |chunk: &[u8]| {
            let mut eight = [0; 8];
            eight[..chunk.len()].copy_from_slice(chunk);
            Fp::new(u64::from_le_bytes(eight))
        };
        Self(text.as_bytes().chunks(8).map(member).collect())
    }

    /// The members hashed into four.
    pub(super) fn digest(&self) -> Digest {
        Digest(poseidon::digest(&self.0))
    }

    /// The members hashed into what is signed.
    pub(super) fn hash(&self) -> Hash {
        Hash(poseidon::hash(&self.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn members_are_hashed_in_their_order() {
        let members = Members::of([304, 14, 6]);
        assert_ne!(members.hash(), Members::of([304, 6, 14]).hash());
        assert_eq!(members.hash(), Members::of([304, 14, 6]).hash());
        assert_eq!(members.hash().to_string().len(), 80);
        assert_eq!(
            members.hash().to_string(),
            hex::encode(members.hash().to_bytes())
        );

        // A key is five members and a digest four, after what came before.
        let key = SecretKey::from_bytes(&[7; 40]).unwrap().public();
        let with_key = Members::of([1]).with_key(key);
        assert_eq!(with_key.0.len(), 6);
        assert_eq!(with_key.0[1..], key.0.0);
        let digest = members.digest();
        let with_digest = Members::of([1, 2]).with_digest(&digest);
        assert_eq!(with_digest.0[2..], digest.0);
        // Two digests fold into another, in their order.
        let other = Members::of([9]).digest();
        assert_ne!(digest.joined(&other), other.joined(&digest));
    }

    #[test]
    fn text_is_hashed_eight_bytes_at_a_time() {
        // Eight bytes, then three and five zeros.
        let members = Members::text("1795345104:");
        assert_eq!(
            members.0,
            [
                Fp::new(u64::from_le_bytes(*b"17953451")),
                Fp::new(u64::from_le_bytes(*b"04:\0\0\0\0\0")),
            ]
        );
        assert_eq!(Members::text("").0, []);
        // The highest bytes text has are still an element as they stand.
        let highest = "\u{10FFFF}\u{10FFFF}";
        let mut eight = [0; 8];
        eight.copy_from_slice(highest.as_bytes());
        assert_eq!(
            Members::text(highest).0[0].value(),
            u64::from_le_bytes(eight)
        );
    }
}
