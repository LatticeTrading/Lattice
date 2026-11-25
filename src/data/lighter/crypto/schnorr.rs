//! Schnorr signatures over ECgFp5 ([`super::curve`]) with Poseidon2 as
//! their hash ([`super::poseidon`]): how a Lighter API key signs.
//!
//! - A private key is a scalar `d` that is not zero ([`SecretKey`]); its
//!   public key is the encoding of `d·G` ([`PublicKey`]), forty bytes.
//! - A signature of the message `m` (an element of the extension field: a
//!   hash already) is made with a nonce `k`: with `r` the encoding of
//!   `k·G`, the challenge is `e = H(r ‖ m)` read as a scalar and the answer
//!   `s = k - e·d`. It is `s` then `e`, eighty bytes ([`Signature`]), and
//!   holds when `s·G + e·P` encodes to an `r` that hashes to `e` again.
//! - The nonce is the one thing that must never repeat or be guessed: two
//!   signatures with one nonce give the key away. It is a hash of the key,
//!   the message and fresh random bytes ([`nonce`]), so it differs for
//!   every message whatever the random source is worth, and is drawn again
//!   until it is a scalar below the order (so none is likelier than
//!   another).
//!
//! Checked against the reference implementation
//! (`github.com/elliottech/poseidon_crypto`, `signature/schnorr`): its test
//! vectors, with their nonces, give its signatures here.

use std::fmt;

use alloy::primitives::keccak256;

use super::{Hash, curve::Point, field::Fp5, poseidon, scalar::Scalar};

/// What the nonce's hash starts with, so that it is no other hash of the
/// same bytes.
const NONCE_DOMAIN: &[u8] = b"Lattice Lighter Schnorr nonce v1";

/// A private key.
pub(in crate::data::lighter) struct SecretKey(Scalar);

/// Never the key itself.
impl fmt::Debug for SecretKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "SecretKey({})", self.public())
    }
}

/// The key is wiped from memory when dropped.
impl Drop for SecretKey {
    fn drop(&mut self) {
        self.0 = Scalar::ZERO;
        // Kept from being optimized away as a write nothing reads.
        std::hint::black_box(&self.0);
    }
}

/// A public key: what Lighter knows an API key by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::data::lighter) struct PublicKey(pub(super) Fp5);

impl PublicKey {
    /// The key's forty bytes.
    pub(in crate::data::lighter) fn to_bytes(self) -> [u8; 40] {
        self.0.to_bytes()
    }
}

/// As Lighter writes one: eighty hexadecimal digits.
impl fmt::Display for PublicKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&alloy::primitives::hex::encode(self.to_bytes()))
    }
}

/// A signature: the answer, then the challenge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::data::lighter) struct Signature {
    s: Scalar,
    e: Scalar,
}

impl Signature {
    /// The signature's eighty bytes.
    pub(in crate::data::lighter) fn to_bytes(self) -> [u8; 80] {
        let mut bytes = [0; 80];
        bytes[..40].copy_from_slice(&self.s.to_bytes());
        bytes[40..].copy_from_slice(&self.e.to_bytes());
        bytes
    }
}

/// The challenge of a signature whose commitment encodes to `commitment`,
/// of `message`.
fn challenge(commitment: Fp5, message: Fp5) -> Scalar {
    let mut hashed = commitment.0.to_vec();
    hashed.extend(message.0);
    Scalar::from_element(poseidon::hash(&hashed))
}

/// Forty bytes as a scalar that is not zero, when they are below the
/// order with their top bit cleared: each scalar is as likely as another.
fn drawn(mut bytes: [u8; 40]) -> Option<Scalar> {
    // The order is a little under `2^319`.
    bytes[39] &= 0x7F;
    let scalar = Scalar::canonical(&bytes).filter(|scalar| !scalar.is_zero());
    bytes.fill(0);
    scalar
}

/// The nonce `secret` signs `message` with.
fn nonce(secret: Scalar, message: Fp5) -> Scalar {
    let mut fresh = [0_u8; 32];
    if getrandom::getrandom(&mut fresh).is_err() {
        // A random source that fails leaves the nonce to the key and the
        // message alone, which is still one no other message shares.
        fresh.fill(0);
    }
    let mut seed = Vec::with_capacity(NONCE_DOMAIN.len() + 2 + 32 + 40 + 40);
    seed.extend_from_slice(NONCE_DOMAIN);
    seed.extend_from_slice(&[0, 0]);
    seed.extend_from_slice(&fresh);
    seed.extend_from_slice(&secret.to_bytes());
    seed.extend_from_slice(&message.to_bytes());
    let (draw, half) = (NONCE_DOMAIN.len(), NONCE_DOMAIN.len() + 1);
    loop {
        // Forty bytes of it: a hash, and eight bytes of another.
        let mut bytes = [0_u8; 40];
        seed[half] = 0;
        bytes[..32].copy_from_slice(keccak256(&seed).as_slice());
        seed[half] = 1;
        bytes[32..].copy_from_slice(&keccak256(&seed)[..8]);
        let nonce = drawn(bytes);
        bytes.fill(0);
        if let Some(nonce) = nonce {
            seed.fill(0);
            return nonce;
        }
        // One draw in a billion is not below the order: another.
        seed[draw] = seed[draw].wrapping_add(1);
    }
}

impl SecretKey {
    /// A new key from the system's random source.
    pub(in crate::data::lighter) fn random() -> Result<Self, String> {
        let mut bytes = [0_u8; 40];
        for _ in 0..8 {
            getrandom::getrandom(&mut bytes)
                .map_err(|error| format!("no random source to make a key from: {error}"))?;
            let key = drawn(bytes);
            bytes.fill(0);
            if let Some(key) = key {
                return Ok(Self(key));
            }
        }
        Err("the random source gave no usable key".to_owned())
    }

    /// The key `bytes` are (forty, little endian); `None` for what is no
    /// key: zero, or not below the order.
    pub(in crate::data::lighter) fn from_bytes(bytes: &[u8; 40]) -> Option<Self> {
        Scalar::canonical(bytes)
            .filter(|scalar| !scalar.is_zero())
            .map(Self)
    }

    /// The key itself, as it is saved: for that and nothing else.
    pub(in crate::data::lighter) fn to_bytes(&self) -> [u8; 40] {
        self.0.to_bytes()
    }

    /// The key Lighter knows this one by.
    pub(in crate::data::lighter) fn public(&self) -> PublicKey {
        PublicKey(Point::GENERATOR.mul(&self.0).encode())
    }

    /// The key's signature of `hash`.
    pub(in crate::data::lighter) fn sign(&self, hash: Hash) -> Signature {
        self.signed(hash.0, nonce(self.0, hash.0))
    }

    /// The signature of `message` made with `nonce`.
    fn signed(&self, message: Fp5, nonce: Scalar) -> Signature {
        let commitment = Point::GENERATOR.mul(&nonce).encode();
        let e = challenge(commitment, message);
        Signature {
            s: nonce.sub(e.mul(self.0)),
            e,
        }
    }
}

/// Whether `signature` (its eighty bytes) is `key`'s of `hash`: what
/// Lighter checks, here to check what is made against what Lighter's own
/// signer makes.
#[cfg(test)]
pub(in crate::data::lighter) fn verifies(key: PublicKey, hash: Hash, signature: &[u8; 80]) -> bool {
    let message = hash.0;
    let (mut s, mut e) = ([0; 40], [0; 40]);
    s.copy_from_slice(&signature[..40]);
    e.copy_from_slice(&signature[40..]);
    let (Some(s), Some(e)) = (Scalar::canonical(&s), Scalar::canonical(&e)) else {
        return false;
    };
    let Some(key) = Point::decode(key.0) else {
        return false;
    };
    let commitment = Point::GENERATOR.mul(&s).add(&key.mul(&e));
    !commitment.is_neutral() && challenge(commitment.encode(), message) == e
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `TestComparativeSchnorrSignAndVerify` of the reference
    /// implementation: a key, a message and a nonce, and the signature they
    /// make there.
    struct Vector {
        key: [u64; 5],
        message: [u64; 5],
        nonce: [u64; 5],
        s: [u64; 5],
        e: [u64; 5],
    }

    const VECTORS: [Vector; 3] = [
        Vector {
            key: [
                12_235_002_942_052_073_545,
                1_175_977_464_658_719_998,
                8_536_934_969_147_463_310,
                6_524_687_619_313_720_391,
                2_922_072_024_880_609_112,
            ],
            message: [
                8_398_652_514_106_806_347,
                11_069_112_711_939_986_896,
                9_732_488_227_085_561_369,
                18_076_754_337_204_438_535,
                17_155_407_358_725_346_236,
            ],
            nonce: [
                5_245_666_847_777_449_560,
                15_178_169_970_799_106_939,
                4_403_065_012_435_293_749,
                15_306_540_389_399_388_999,
                8_935_555_081_913_173_844,
            ],
            s: [
                6_950_590_877_883_398_434,
                17_178_336_263_794_770_543,
                11_012_823_478_139_181_320,
                16_445_091_359_523_510_936,
                5_882_925_226_143_600_273,
            ],
            e: [
                4_544_744_459_434_870_309,
                4_180_764_085_957_612_004,
                3_024_669_018_778_978_615,
                15_433_417_688_859_446_606,
                6_775_027_260_348_937_828,
            ],
        },
        Vector {
            key: [
                14_609_471_659_974_493_146,
                15_558_617_123_161_593_410,
                853_367_204_868_339_037,
                17_594_253_198_278_631_904,
                368_396_584_122_947_478,
            ],
            message: [
                14_569_490_467_507_212_064,
                2_707_063_505_563_578_676,
                7_506_743_487_465_742_335,
                12_569_771_346_154_554_175,
                4_305_083_698_940_175_790,
            ],
            nonce: [
                1_980_123_857_560_067_020,
                10_696_795_398_834_097_509,
                3_211_831_869_376_171_671,
                6_194_822_139_276_031_840,
                3_482_023_782_412_490_864,
            ],
            s: [
                15_189_311_883_262_425_203,
                16_924_634_885_527_914_505,
                11_098_200_095_411_565_797,
                11_441_434_601_417_451_505,
                2_245_797_172_600_273_048,
            ],
            e: [
                4_905_460_437_060_282_008,
                9_275_377_852_059_362_729,
                10_383_772_785_796_962_929,
                6_858_067_464_918_579_610,
                7_078_247_668_913_970_626,
            ],
        },
        Vector {
            key: [
                846_395_111_423_676_945,
                1_354_180_063_821_346_280,
                5_751_371_120_309_175_011,
                4_898_038_106_472_090_654,
                1_076_345_918_732_914_302,
            ],
            message: [
                17_529_153_479_246_803_593,
                1_743_712_677_205_511_695,
                4_834_285_972_617_397_460,
                5_486_672_566_342_530_358,
                7_254_989_001_695_704_129,
            ],
            nonce: [
                10_299_597_990_997_564_957,
                8_547_298_489_021_408_803,
                12_250_978_550_108_858_722,
                5_282_281_975_236_198_197,
                5_328_603_554_431_393_061,
            ],
            s: [
                1_747_989_245_728_027_396,
                18_083_435_619_737_379_521,
                18_276_259_610_811_995_786,
                15_101_757_397_705_334_408,
                5_007_814_817_019_340_642,
            ],
            e: [
                4_911_725_746_357_568_132,
                12_205_663_641_120_664_338,
                16_433_506_899_074_513_700,
                14_763_562_571_101_437_023,
                2_547_950_465_160_283_358,
            ],
        },
    ];

    #[test]
    fn a_signature_is_the_reference_implementations() {
        for vector in VECTORS {
            let key = SecretKey(Scalar::of(vector.key));
            let message = Fp5::new(vector.message);
            let signature = key.signed(message, Scalar::of(vector.nonce));
            assert_eq!(signature.s.limbs(), vector.s);
            assert_eq!(signature.e.limbs(), vector.e);
            assert!(verifies(key.public(), Hash(message), &signature.to_bytes()));
        }
    }

    #[test]
    fn a_signature_holds_for_its_key_and_its_message_alone() {
        let key = SecretKey::random().unwrap();
        let other = SecretKey::random().unwrap();
        assert_ne!(key.to_bytes(), other.to_bytes());
        let message = Hash(Fp5::new([1, 2, 3, 4, 5]));
        let signature = key.sign(message).to_bytes();
        assert!(verifies(key.public(), message, &signature));
        assert!(!verifies(other.public(), message, &signature));
        let another = Hash(Fp5::new([1, 2, 3, 4, 6]));
        assert!(!verifies(key.public(), another, &signature));
        let mut altered = signature;
        altered[0] ^= 1;
        assert!(!verifies(key.public(), message, &altered));
        // What is not below the order is no signature.
        assert!(!verifies(key.public(), message, &[0xFF; 80]));
        // Each signature has a nonce of its own, so no two are alike.
        assert_ne!(key.sign(message).to_bytes(), signature);
    }

    #[test]
    fn a_key_is_forty_bytes_and_never_shown() {
        let key = SecretKey::random().unwrap();
        let read = SecretKey::from_bytes(&key.to_bytes()).unwrap();
        assert_eq!(read.public(), key.public());
        // Zero and what is not below the order are no key.
        assert!(SecretKey::from_bytes(&[0; 40]).is_none());
        assert!(SecretKey::from_bytes(&[0xFF; 40]).is_none());
        let shown = format!("{key:?}");
        assert_eq!(shown, format!("SecretKey({})", key.public()));
        assert_eq!(key.public().to_string().len(), 80);
        let secret = alloy::primitives::hex::encode(key.to_bytes());
        assert!(!shown.contains(&secret));
    }

    #[test]
    fn a_nonce_is_the_keys_and_the_messages_own() {
        let key = Scalar::of([1, 2, 3, 4, 5]);
        let message = Fp5::new([9, 8, 7, 6, 5]);
        let first = nonce(key, message);
        assert!(!first.is_zero());
        // Fresh random bytes go into each.
        assert_ne!(nonce(key, message), first);
        assert_ne!(nonce(key, Fp5::ZERO), first);
        // Forty bytes are drawn as a scalar under the order or not at all.
        assert_eq!(drawn([0; 40]), None);
        assert!(drawn([0x55; 40]).is_some());
        // All ones with the top bit cleared is just over the order.
        assert_eq!(drawn([0xFF; 40]), None);
    }
}
