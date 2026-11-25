//! The two fields Lighter's signatures are computed in: the Goldilocks
//! field ([`Fp`], the integers modulo `2^64 - 2^32 + 1`) and its extension
//! of degree five ([`Fp5`], polynomials in `x` over it modulo `x^5 - 3`),
//! which is what the curve is defined over ([`super::curve`]) and what a
//! message hashes to ([`super::poseidon`]).
//!
//! An element is always kept reduced, so two that are equal compare equal
//! and each has one encoding: its integer below the order for [`Fp`], and
//! for [`Fp5`] its five coefficients, eight bytes each, little endian.
//!
//! Written for clarity over speed (every product is reduced as it is made):
//! a signature takes about a hundred thousand multiplications, a few
//! milliseconds. Nothing here runs in time independent of its operands,
//! which Lighter's own signer does not either.

use std::ops::{Add, Mul, Neg, Sub};

/// The field's order: `2^64 - 2^32 + 1`.
const ORDER: u64 = 0xFFFF_FFFF_0000_0001;
/// `2^64` modulo the order: `2^32 - 1`.
const EPSILON: u64 = 0xFFFF_FFFF;

/// An integer modulo [`ORDER`], reduced.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Fp(u64);

impl Fp {
    pub(super) const ZERO: Self = Self(0);
    pub(super) const ONE: Self = Self(1);

    /// `value` modulo the order.
    pub(super) const fn new(value: u64) -> Self {
        Self(value % ORDER)
    }

    /// The element as the integer below the order it is.
    pub(super) const fn value(self) -> u64 {
        self.0
    }

    pub(super) fn is_zero(self) -> bool {
        self.0 == 0
    }

    pub(super) fn double(self) -> Self {
        self + self
    }

    pub(super) fn square(self) -> Self {
        self * self
    }

    /// The element to the power `exponent`.
    pub(super) fn pow(self, mut exponent: u64) -> Self {
        let (mut power, mut product) = (self, Self::ONE);
        while exponent > 0 {
            if exponent & 1 == 1 {
                product = product * power;
            }
            power = power.square();
            exponent >>= 1;
        }
        product
    }

    /// One over the element; zero for zero.
    pub(super) fn inverse(self) -> Self {
        // Fermat: the order is prime.
        self.pow(ORDER - 2)
    }
}

/// `value` modulo the order, for any 128 bits: `2^64` is [`EPSILON`] there
/// and `2^96` is `-1`, so the high half folds into the low one.
fn reduced(value: u128) -> Fp {
    // The two halves, on purpose.
    #[expect(clippy::cast_possible_truncation)]
    let (low, high) = (value as u64, (value >> 64) as u64);
    let (high_high, high_low) = (high >> 32, high & EPSILON);
    let (mut folded, borrowed) = low.overflowing_sub(high_high);
    if borrowed {
        // Wrapped by 2^64, which is the order and EPSILON.
        folded = folded.wrapping_sub(EPSILON);
    }
    let (mut sum, carried) = folded.overflowing_add(high_low * EPSILON);
    if carried {
        sum = sum.wrapping_add(EPSILON);
    }
    Fp(if sum >= ORDER { sum - ORDER } else { sum })
}

impl Add for Fp {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        let (sum, carried) = self.0.overflowing_add(other.0);
        // Both are below the order, so the sum is below twice it.
        Self(if carried || sum >= ORDER {
            sum.wrapping_sub(ORDER)
        } else {
            sum
        })
    }
}

impl Sub for Fp {
    type Output = Self;

    fn sub(self, other: Self) -> Self {
        let (difference, borrowed) = self.0.overflowing_sub(other.0);
        Self(if borrowed {
            difference.wrapping_add(ORDER)
        } else {
            difference
        })
    }
}

impl Neg for Fp {
    type Output = Self;

    fn neg(self) -> Self {
        Self::ZERO - self
    }
}

impl Mul for Fp {
    type Output = Self;

    fn mul(self, other: Self) -> Self {
        reduced(u128::from(self.0) * u128::from(other.0))
    }
}

/// What `x^5` is in the extension.
const W: Fp = Fp(3);
/// The fifth root of one the Frobenius map multiplies `x` by.
const FROBENIUS: Fp = Fp(1_041_288_259_238_279_555);

/// An element of the extension: `c0 + c1·x + … + c4·x^4`, its coefficients
/// in that order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Fp5(pub(super) [Fp; 5]);

impl Fp5 {
    pub(super) const ZERO: Self = Self([Fp::ZERO; 5]);
    pub(super) const ONE: Self = Self::of(1);

    /// The integer `value` (below the order) as an element.
    pub(super) const fn of(value: u64) -> Self {
        Self([Fp::new(value), Fp::ZERO, Fp::ZERO, Fp::ZERO, Fp::ZERO])
    }

    /// The element whose coefficients are `values`, each modulo the order.
    pub(super) const fn new(values: [u64; 5]) -> Self {
        Self([
            Fp::new(values[0]),
            Fp::new(values[1]),
            Fp::new(values[2]),
            Fp::new(values[3]),
            Fp::new(values[4]),
        ])
    }

    pub(super) fn is_zero(self) -> bool {
        self.0.iter().all(|coefficient| coefficient.is_zero())
    }

    pub(super) fn double(self) -> Self {
        self + self
    }

    pub(super) fn square(self) -> Self {
        self * self
    }

    /// The element with every coefficient multiplied by `factor`.
    pub(super) fn scaled(self, factor: Fp) -> Self {
        Self(self.0.map(|coefficient| coefficient * factor))
    }

    /// The Frobenius map applied `times` over: the element to the power of
    /// the order, that many times.
    fn frobenius(self, times: u32) -> Self {
        let step = FROBENIUS.pow(u64::from(times % 5));
        let mut factor = Fp::ONE;
        Self(self.0.map(|coefficient| {
            let mapped = coefficient * factor;
            factor = factor * step;
            mapped
        }))
    }

    /// One over the element; zero for zero.
    pub(super) fn inverse(self) -> Self {
        if self.is_zero() {
            return Self::ZERO;
        }
        // The product of the element's four conjugates, times the element,
        // is its norm: an element of the base field, which is inverted there.
        let first = self.frobenius(1);
        let two = first * first.frobenius(1);
        let conjugates = two * two.frobenius(2);
        let ([a0, a1, a2, a3, a4], [b0, b1, b2, b3, b4]) = (self.0, conjugates.0);
        let norm = a0 * b0 + W * (a1 * b4 + a2 * b3 + a3 * b2 + a4 * b1);
        conjugates.scaled(norm.inverse())
    }

    /// The element's forty bytes: its coefficients, each little endian.
    pub(super) fn to_bytes(self) -> [u8; 40] {
        let mut bytes = [0; 40];
        for (chunk, coefficient) in bytes.as_chunks_mut::<8>().0.iter_mut().zip(self.0) {
            *chunk = coefficient.value().to_le_bytes();
        }
        bytes
    }
}

impl Add for Fp5 {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        let mut sum = self.0;
        for (coefficient, added) in sum.iter_mut().zip(other.0) {
            *coefficient = *coefficient + added;
        }
        Self(sum)
    }
}

impl Sub for Fp5 {
    type Output = Self;

    fn sub(self, other: Self) -> Self {
        let mut difference = self.0;
        for (coefficient, taken) in difference.iter_mut().zip(other.0) {
            *coefficient = *coefficient - taken;
        }
        Self(difference)
    }
}

impl Neg for Fp5 {
    type Output = Self;

    fn neg(self) -> Self {
        Self(self.0.map(Neg::neg))
    }
}

impl Mul for Fp5 {
    type Output = Self;

    /// The product of the two polynomials, with `x^5` folded back as [`W`].
    fn mul(self, other: Self) -> Self {
        let ([a0, a1, a2, a3, a4], [b0, b1, b2, b3, b4]) = (self.0, other.0);
        Self([
            a0 * b0 + W * (a1 * b4 + a2 * b3 + a3 * b2 + a4 * b1),
            a0 * b1 + a1 * b0 + W * (a2 * b4 + a3 * b3 + a4 * b2),
            a0 * b2 + a1 * b1 + a2 * b0 + W * (a3 * b4 + a4 * b3),
            a0 * b3 + a1 * b2 + a2 * b1 + a3 * b0 + W * (a4 * b4),
            a0 * b4 + a1 * b3 + a2 * b2 + a3 * b1 + a4 * b0,
        ])
    }
}

/// What only checking a signature takes (decoding a public key): square
/// roots, and which of an element's two is the one written.
#[cfg(test)]
mod roots {
    use super::*;

    /// How many times two divides the order less one.
    const TWO_ADICITY: u32 = 32;
    /// A generator of the roots of one of order `2^32`.
    const ROOT_OF_ONE: Fp = Fp(7_277_203_076_849_721_926);

    impl Fp {
        /// Whether the element is a square (zero is).
        fn is_square(self) -> bool {
            self.is_zero() || self.pow((ORDER - 1) / 2) == Self::ONE
        }

        /// A square root of the element (Tonelli and Shanks); `None` for one
        /// that is no square.
        pub(in super::super) fn sqrt(self) -> Option<Self> {
            if self.is_zero() {
                return Some(Self::ZERO);
            }
            if !self.is_square() {
                return None;
            }
            let odd = (ORDER - 1) >> TWO_ADICITY;
            let mut generator = ROOT_OF_ONE;
            let power = self.pow((odd - 1) / 2);
            let mut root = self * power;
            let mut check = root * power;
            let mut order = TWO_ADICITY;
            while check != Self::ONE {
                let mut least = 0;
                let mut squared = check;
                while squared != Self::ONE {
                    squared = squared.square();
                    least += 1;
                }
                let mut factor = generator;
                for _ in 0..order - least - 1 {
                    factor = factor.square();
                }
                generator = factor.square();
                check = check * generator;
                root = root * factor;
                order = least;
            }
            Some(root)
        }
    }

    impl Fp5 {
        /// The element's norm to the power that says whether it is a
        /// square: one for a square, the order less one for none, zero for
        /// zero.
        pub(in super::super) fn legendre(self) -> Fp {
            let first = self.frobenius(1);
            let second = first.frobenius(1);
            let two = first * second;
            let norm = (self * two * two.frobenius(2)).0[0];
            norm.pow((ORDER - 1) / 2)
        }

        /// A square root of the element; `None` for one that is no square.
        fn sqrt(self) -> Option<Self> {
            let mut power = self;
            for _ in 0..31 {
                power = power.square();
            }
            let mut raised = power;
            for _ in 0..32 {
                raised = raised.square();
            }
            let d = self * raised * power.inverse();
            let e = (d * d.frobenius(2)).frobenius(1);
            let f = e.square();
            let ([x0, x1, x2, x3, x4], [f0, f1, f2, f3, f4]) = (self.0, f.0);
            let g = x0 * f0 + W * (x1 * f4 + x2 * f3 + x3 * f2 + x4 * f1);
            Some(Self::of(g.sqrt()?.value()) * e.inverse())
        }

        /// Whether the element is the one of its two square roots that is
        /// not written: its first coefficient that is not zero is even.
        pub(super) fn is_even(self) -> bool {
            self.0
                .iter()
                .find(|coefficient| !coefficient.is_zero())
                .is_none_or(|coefficient| coefficient.value() & 1 == 0)
        }

        /// The square root of the element that is written, of its two.
        pub(in super::super) fn canonical_sqrt(self) -> Option<Self> {
            let root = self.sqrt()?;
            Some(if root.is_even() { -root } else { root })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Operands at the edges, and a few in between.
    const SAMPLES: [u64; 8] = [
        0,
        1,
        2,
        EPSILON,
        EPSILON + 1,
        0x1234_5678_9ABC_DEF0,
        ORDER - 2,
        ORDER - 1,
    ];

    #[test]
    fn the_base_field_is_arithmetic_modulo_its_order() {
        let order = u128::from(ORDER);
        for a in SAMPLES {
            for b in SAMPLES {
                let (wide_a, wide_b) = (u128::from(a), u128::from(b));
                let is = |element: Fp, expected: u128| {
                    assert_eq!(u128::from(element.value()), expected % order, "{a} {b}");
                };
                is(Fp(a) + Fp(b), wide_a + wide_b);
                is(Fp(a) - Fp(b), wide_a + order - wide_b);
                is(Fp(a) * Fp(b), wide_a * wide_b);
            }
            assert_eq!(Fp(a) + -Fp(a), Fp::ZERO);
            if a != 0 {
                assert_eq!(Fp(a) * Fp(a).inverse(), Fp::ONE, "{a}");
            }
        }
        // Any 128 bits fold to their remainder.
        for value in [u128::MAX, u128::MAX - 1, 1 << 127, (1 << 96) - 1, 1 << 64] {
            assert_eq!(u128::from(reduced(value).value()), value % order);
        }
        // An integer at the order or above is the element it is modulo it.
        assert_eq!(Fp::new(ORDER), Fp::ZERO);
        assert_eq!(Fp::new(ORDER + 5), Fp(5));
        assert_eq!(Fp::ZERO.inverse(), Fp::ZERO);
    }

    #[test]
    fn the_extension_folds_the_fifth_power_back_as_three() {
        let x = Fp5::new([0, 1, 0, 0, 0]);
        let fourth = x * x * x * x;
        assert_eq!(fourth, Fp5::new([0, 0, 0, 0, 1]));
        assert_eq!(fourth * x, Fp5::of(3));

        let a = Fp5::new([
            9_148_693_690_730_647_261,
            18_018_159_403_417_097_655,
            11_126_638_999_692_053_441,
            10_314_403_417_097_655,
            15_998_333_998_318_935_536,
        ]);
        let b = Fp5::new([5, ORDER - 1, 0, EPSILON, 0x1234_5678_9ABC_DEF0]);
        // A field: products commute and distribute, and every element but
        // zero has an inverse.
        assert_eq!(a * b, b * a);
        assert_eq!(a * (b + a), a * b + a.square());
        assert_eq!(a * a.inverse(), Fp5::ONE);
        assert_eq!(b * b.inverse(), Fp5::ONE);
        assert_eq!(Fp5::ZERO.inverse(), Fp5::ZERO);
        assert_eq!(a - a, Fp5::ZERO);
        assert_eq!(a + -a, Fp5::ZERO);
        assert_eq!(a.double(), a.scaled(Fp(2)));
        // The Frobenius map five times over is the element itself.
        assert_eq!(a.frobenius(5), a);
        assert_eq!(a.frobenius(2).frobenius(3), a);
        assert_ne!(a.frobenius(1), a);
    }

    #[test]
    fn an_element_is_its_coefficients_little_endian() {
        let element = Fp5::new([1, 2, 3, 4, ORDER - 1]);
        let bytes = element.to_bytes();
        assert_eq!(bytes[..8], [1, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(bytes[8..16], [2, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(bytes[32..], (ORDER - 1).to_le_bytes());
    }

    #[test]
    fn a_square_has_a_root_and_one_of_the_two_is_written() {
        for value in [4, 9, 0x1234_5678_9ABC_DEF0_u64] {
            let square = Fp(value).square();
            let root = square.sqrt().expect("a square has a root");
            assert_eq!(root.square(), square);
        }
        // Seven generates the field's units, so it is no square.
        assert_eq!(Fp(7).sqrt(), None);

        let a = Fp5::new([3, 1, 4, 1, 5]);
        let square = a.square();
        assert_eq!(square.legendre(), Fp::ONE);
        let root = square.canonical_sqrt().expect("a square has a root");
        assert_eq!(root.square(), square);
        assert!(root == a || root == -a);
        assert!(!root.is_even());
        assert_eq!(Fp5::ZERO.legendre(), Fp::ZERO);
    }
}
