//! The curve's scalars: the integers modulo the order of its group
//! ([`super::curve`]), a prime a little under `2^319`. A private key is
//! one, and so are the two halves of a signature.
//!
//! A scalar is five 64-bit limbs, the lowest first, always reduced; its
//! encoding is those forty bytes little endian. Products are Montgomery's,
//! as the reference implementation's are (`ecgfp5`, and Lighter's port of
//! it), with the limbs kept in plain form between them.

use super::field::Fp5;

/// The group's order.
const ORDER: [u64; 5] = [
    0xE80F_D996_948B_FFE1,
    0xE888_5C39_D724_A09C,
    0x7FFF_FFE6_CFB8_0639,
    0x7FFF_FFF1_0000_0016,
    0x7FFF_FFFD_8000_0007,
];
/// Minus one over the order's lowest limb, modulo `2^64`.
const ORDER_INVERSE: u64 = 0xD78B_EF72_057B_7BDF;
/// `2^640` modulo the order: what a Montgomery product is multiplied by to
/// be the plain one.
const R2: Scalar = Scalar([
    0xA010_01DC_E33D_C739,
    0x6C32_28D3_3F62_ACCF,
    0xD1D7_96CC_91CF_8525,
    0xAADF_FF5D_1574_C1D8,
    0x4ACA_13B2_8CA2_51F5,
]);

/// An integer modulo the group's order, reduced.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Scalar([u64; 5]);

/// `a - b` over five limbs, and whether it borrowed (it is then `2^320`
/// more).
fn subtracted(a: &[u64; 5], b: &[u64; 5]) -> ([u64; 5], bool) {
    let mut difference = [0; 5];
    let mut borrow = false;
    for ((limb, a), b) in difference.iter_mut().zip(a).zip(b) {
        let (first, under) = a.overflowing_sub(*b);
        let (second, under_again) = first.overflowing_sub(u64::from(borrow));
        *limb = second;
        borrow = under || under_again;
    }
    (difference, borrow)
}

/// `a + b` over five limbs, wrapping at `2^320`.
fn added(a: &[u64; 5], b: &[u64; 5]) -> [u64; 5] {
    let mut sum = [0; 5];
    let mut carry = false;
    for ((limb, a), b) in sum.iter_mut().zip(a).zip(b) {
        let (first, over) = a.overflowing_add(*b);
        let (second, over_again) = first.overflowing_add(u64::from(carry));
        *limb = second;
        carry = over || over_again;
    }
    sum
}

impl Scalar {
    pub(super) const ZERO: Self = Self([0; 5]);

    /// The integer `limbs` are (the lowest first), modulo the order.
    pub(super) fn reduced(mut limbs: [u64; 5]) -> Self {
        // Five limbs hold less than three times the order.
        loop {
            let (less, borrowed) = subtracted(&limbs, &ORDER);
            if borrowed {
                return Self(limbs);
            }
            limbs = less;
        }
    }

    /// The integer `bytes` are, little endian, modulo the order.
    pub(super) fn from_bytes(bytes: &[u8; 40]) -> Self {
        let mut limbs = [0; 5];
        for (limb, chunk) in limbs.iter_mut().zip(bytes.as_chunks::<8>().0) {
            *limb = u64::from_le_bytes(*chunk);
        }
        Self::reduced(limbs)
    }

    /// The scalar `bytes` encode; `None` when they are not below the order,
    /// which is no scalar's encoding.
    pub(super) fn canonical(bytes: &[u8; 40]) -> Option<Self> {
        let scalar = Self::from_bytes(bytes);
        (scalar.to_bytes() == *bytes).then_some(scalar)
    }

    /// The scalar an element of the extension reads as: its coefficients as
    /// limbs, modulo the order. How a hash becomes a signature's challenge.
    pub(super) fn from_element(element: Fp5) -> Self {
        Self::reduced(element.0.map(|coefficient| coefficient.value()))
    }

    /// The scalar's forty bytes, little endian.
    pub(super) fn to_bytes(self) -> [u8; 40] {
        let mut bytes = [0; 40];
        for (chunk, limb) in bytes.as_chunks_mut::<8>().0.iter_mut().zip(self.0) {
            *chunk = limb.to_le_bytes();
        }
        bytes
    }

    pub(super) fn is_zero(self) -> bool {
        self.0 == [0; 5]
    }

    /// The scalar's eighty four-bit digits, the lowest first.
    pub(super) fn digits(self) -> [u8; 80] {
        let mut digits = [0; 80];
        for (place, digit) in digits.iter_mut().enumerate() {
            *digit = ((self.0[place / 16] >> (4 * (place % 16))) & 0xF) as u8;
        }
        digits
    }

    pub(super) fn sub(self, other: Self) -> Self {
        let (difference, borrowed) = subtracted(&self.0, &other.0);
        Self(if borrowed {
            added(&difference, &ORDER)
        } else {
            difference
        })
    }

    /// Montgomery's product: `self * other / 2^320` modulo the order.
    fn montgomery(self, other: Self) -> Self {
        let mut product = [0_u64; 5];
        for factor in other.0 {
            // Each round adds `self * factor` and the multiple of the order
            // that makes the lowest limb zero, and drops that limb.
            let fold = self.0[0]
                .wrapping_mul(factor)
                .wrapping_add(product[0])
                .wrapping_mul(ORDER_INVERSE);
            let (mut carry, mut folded) = (0_u64, 0_u64);
            for place in 0..5 {
                let wide = u128::from(self.0[place]) * u128::from(factor)
                    + u128::from(product[place])
                    + u128::from(carry);
                // The halves of 128 bits, on purpose.
                #[expect(clippy::cast_possible_truncation)]
                let (low, high) = (wide as u64, (wide >> 64) as u64);
                carry = high;
                let wide = u128::from(fold) * u128::from(ORDER[place])
                    + u128::from(low)
                    + u128::from(folded);
                #[expect(clippy::cast_possible_truncation)]
                let (low, high) = (wide as u64, (wide >> 64) as u64);
                folded = high;
                if place > 0 {
                    product[place - 1] = low;
                }
            }
            // What is left fits the top limb: the sum is below `2^384`.
            product[4] = carry.wrapping_add(folded);
        }
        // Below twice the order: one subtraction at most.
        let (less, borrowed) = subtracted(&product, &ORDER);
        Self(if borrowed { product } else { less })
    }

    pub(super) fn mul(self, other: Self) -> Self {
        self.montgomery(R2).montgomery(other)
    }
}

#[cfg(test)]
impl Scalar {
    /// The scalar whose limbs are `limbs`, modulo the order.
    pub(super) fn of(limbs: [u64; 5]) -> Self {
        Self::reduced(limbs)
    }

    pub(super) fn limbs(self) -> [u64; 5] {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ONE: Scalar = Scalar([1, 0, 0, 0, 0]);

    fn minus_one() -> Scalar {
        Scalar::ZERO.sub(ONE)
    }

    #[test]
    fn scalars_are_integers_modulo_the_groups_order() {
        // The order itself is zero, and one under it is minus one.
        assert_eq!(Scalar::reduced(ORDER), Scalar::ZERO);
        let mut under = ORDER;
        under[0] -= 1;
        assert_eq!(Scalar::reduced(under), minus_one());
        assert_eq!(Scalar::reduced(under).limbs(), under);
        // Five full limbs are a little over twice the order.
        let all = Scalar::reduced([u64::MAX; 5]);
        let (once, _) = subtracted(&[u64::MAX; 5], &ORDER);
        let (twice, borrowed) = subtracted(&once, &ORDER);
        assert!(!borrowed);
        assert_eq!(all.limbs(), twice);

        let a = Scalar::of([
            12_235_002_942_052_073_545,
            1_175_977_464_658_719_998,
            8_536_934_969_147_463_310,
            6_524_687_619_313_720_391,
            2_922_072_024_880_609_112,
        ]);
        let b = Scalar::of([7, 0, u64::MAX, 3, 1]);
        assert_eq!(a.sub(a), Scalar::ZERO);
        assert_eq!(a.sub(b).sub(a.sub(b)), Scalar::ZERO);
        assert_eq!(Scalar::ZERO.sub(Scalar::ZERO.sub(a)), a);
        // Products: by one, by two, by minus one, either way round.
        assert_eq!(a.mul(ONE), a);
        assert_eq!(ONE.mul(a), a);
        let two = Scalar([2, 0, 0, 0, 0]);
        assert_eq!(a.mul(two), a.sub(Scalar::ZERO.sub(a)));
        assert_eq!(a.mul(minus_one()), Scalar::ZERO.sub(a));
        assert_eq!(minus_one().mul(minus_one()), ONE);
        assert_eq!(a.mul(b), b.mul(a));
        assert_eq!(a.mul(b.sub(ONE)), a.mul(b).sub(a));
    }

    #[test]
    fn a_scalar_is_forty_bytes_little_endian() {
        let scalar = Scalar::of([0x0807_0605_0403_0201, 2, 3, 4, 5]);
        let bytes = scalar.to_bytes();
        assert_eq!(bytes[..9], [1, 2, 3, 4, 5, 6, 7, 8, 2]);
        assert_eq!(Scalar::from_bytes(&bytes), scalar);
        assert_eq!(Scalar::canonical(&bytes), Some(scalar));
        // The order and anything above it encode no scalar, and read as
        // what they are modulo it.
        assert_eq!(Scalar::canonical(&[0xFF; 40]), None);
        assert_eq!(
            Scalar::from_bytes(&[0xFF; 40]),
            Scalar::reduced([u64::MAX; 5])
        );
        assert!(Scalar::ZERO.is_zero() && !scalar.is_zero());

        // Four bits at a time, the lowest first.
        let digits = Scalar::of([0x21, 0, 0, 0, 0x3000_0000_0000_0000]).digits();
        assert_eq!(digits[..3], [1, 2, 0]);
        assert_eq!(digits[79], 3);
        assert_eq!(digits[3..79], [0; 76]);
    }
}
