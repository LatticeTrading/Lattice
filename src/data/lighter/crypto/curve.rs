//! ECgFp5: the group Lighter's keys live in. It is the points of the curve
//! `y² = x·(x² + 2·x + 263·θ)` over the extension field ([`Fp5`], `θ` what
//! generates it), halved so that what is left has a prime order
//! ([`Scalar`]) and no cofactor: every element has exactly one encoding, an
//! element of the field, and whatever decodes is in the group.
//!
//! A point is kept as the fractions `(X/Z, U/T)` of its `x` and of `x/y`
//! ([`Point`]), which is what makes the addition formulas complete: the
//! same ones for any two points, the neutral one and a point with itself
//! included. A point times a scalar is a fixed window of four bits over
//! them: the same doublings and additions whatever the scalar, and its
//! table read whole each time.
//!
//! The formulas are those of Thomas Pornin's `ecgfp5` (the curve's
//! reference implementation), as `github.com/elliottech/poseidon_crypto`
//! ports them for Lighter's signer.

use super::{
    field::{Fp, Fp5},
    scalar::Scalar,
};

/// The curve's `b`.
const B: Fp5 = Fp5::new([0, 263, 0, 0, 0]);
const TWICE_B: Fp5 = Fp5::new([0, 2 * 263, 0, 0, 0]);
const FOUR_B: Fp5 = Fp5::new([0, 4 * 263, 0, 0, 0]);

/// An element of the group.
#[derive(Clone, Copy, Debug)]
pub(super) struct Point {
    x: Fp5,
    z: Fp5,
    u: Fp5,
    t: Fp5,
}

impl Point {
    /// The neutral element, which adding changes nothing by.
    pub(super) const NEUTRAL: Self = Self {
        x: Fp5::ZERO,
        z: Fp5::ONE,
        u: Fp5::ZERO,
        t: Fp5::ONE,
    };

    /// The group's generator: what a private key is multiplied to make its
    /// public key.
    pub(super) const GENERATOR: Self = Self {
        x: Fp5::new([
            12_883_135_586_176_881_569,
            4_356_519_642_755_055_268,
            5_248_930_565_894_896_907,
            2_165_973_894_480_315_022,
            2_448_410_071_095_648_785,
        ]),
        z: Fp5::ONE,
        u: Fp5::ONE,
        t: Fp5::of(4),
    };

    /// The point's encoding: `y/x`, zero for the neutral one.
    pub(super) fn encode(&self) -> Fp5 {
        self.t * self.u.inverse()
    }

    pub(super) fn add(&self, other: &Self) -> Self {
        let (x1, z1, u1, t1) = (self.x, self.z, self.u, self.t);
        let (x2, z2, u2, t2) = (other.x, other.z, other.u, other.t);
        let xs = x1 * x2;
        let zs = z1 * z2;
        let us = u1 * u2;
        let ts = t1 * t2;
        let cross_xz = (x1 + z1) * (x2 + z2) - xs - zs;
        let cross_ut = (u1 + t1) * (u2 + t2) - us - ts;
        let seventh = xs + zs * B;
        let eighth = ts * seventh;
        let ninth = us * (cross_xz * TWICE_B + seventh.double());
        let tenth = (ts + us.double()) * (cross_xz + seventh);
        Self {
            x: (tenth - eighth) * B,
            z: eighth - ninth,
            u: cross_ut * (zs * B - xs),
            t: eighth + ninth,
        }
    }

    pub(super) fn double(&self) -> Self {
        let (x, z, u, t) = (self.x, self.z, self.u, self.t);
        let zt = z * t;
        let ztt = zt * t;
        let x1 = ztt.square();
        let z1 = zt * u;
        let w1 = ztt - (x + z).double() * u.square();
        let z1_squared = z1.square();
        let w1_squared = w1.square();
        Self {
            x: z1_squared * FOUR_B,
            z: w1_squared,
            u: (w1 + z1).square() - z1_squared - w1_squared,
            t: x1.double() - z1_squared.scaled(Fp::new(4)) - w1_squared,
        }
    }

    /// The point `scalar` times over.
    pub(super) fn mul(&self, scalar: &Scalar) -> Self {
        // The point none to fifteen times over.
        let mut table = [Self::NEUTRAL; 16];
        for times in 1..16 {
            table[times] = table[times - 1].add(self);
        }
        let mut product = Self::NEUTRAL;
        for digit in scalar.digits().into_iter().rev() {
            for _ in 0..4 {
                product = product.double();
            }
            product = product.add(&chosen(&table, digit));
        }
        product
    }
}

/// `table`'s entry at `place`, read without a branch or an index that
/// depends on it: every entry is read, and all but the one are masked out.
fn chosen(table: &[Point; 16], place: u8) -> Point {
    let mut picked = [[0_u64; 5]; 4];
    for (at, point) in (0_u8..).zip(table) {
        // All ones for the entry wanted, none for any other.
        let mask = (u64::from(at ^ place).wrapping_sub(1) >> 63).wrapping_neg();
        for (picked, part) in picked.iter_mut().zip([point.x, point.z, point.u, point.t]) {
            for (limb, coefficient) in picked.iter_mut().zip(part.0) {
                *limb |= coefficient.value() & mask;
            }
        }
    }
    let [x, z, u, t] = picked.map(Fp5::new);
    Point { x, z, u, t }
}

/// What only checking a signature takes: a point from its encoding, and
/// whether two are the same.
#[cfg(test)]
impl Point {
    /// The point `encoded` is of; `None` for what encodes none.
    pub(super) fn decode(encoded: Fp5) -> Option<Self> {
        if encoded.is_zero() {
            return Some(Self::NEUTRAL);
        }
        // With `w = y/x`, the curve's equation is `x² - (w² - a)·x + b = 0`:
        // of its two roots, the point's `x` is the one that is no square.
        let e = encoded.square() - Fp5::of(2);
        let root = (e.square() - FOUR_B).canonical_sqrt()?;
        let half = Fp5::of(2).inverse();
        let (first, second) = ((e + root) * half, (e - root) * half);
        let x = if first.legendre() == Fp::ONE {
            second
        } else {
            first
        };
        Some(Self {
            x,
            z: Fp5::ONE,
            u: Fp5::ONE,
            t: encoded,
        })
    }

    pub(super) fn is_neutral(&self) -> bool {
        self.u.is_zero()
    }

    pub(super) fn same(&self, other: &Self) -> bool {
        self.u * other.t == other.u * self.t
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn times(count: u64) -> Scalar {
        Scalar::of([count, 0, 0, 0, 0])
    }

    #[test]
    fn the_group_adds_and_doubles_by_the_same_formulas() {
        let g = Point::GENERATOR;
        let two = g.double();
        // A point with itself, with the neutral one, and either way round.
        assert!(g.add(&g).same(&two));
        assert!(g.add(&Point::NEUTRAL).same(&g));
        assert!(Point::NEUTRAL.add(&g).same(&g));
        assert!(Point::NEUTRAL.add(&Point::NEUTRAL).is_neutral());
        assert!(Point::NEUTRAL.double().is_neutral());
        assert!(!g.is_neutral() && !g.same(&two));
        let three = two.add(&g);
        assert!(g.add(&two).same(&three));
        assert!(three.add(&g).same(&two.double()));

        // A scalar times the generator is the generator added that often.
        assert!(g.mul(&times(0)).is_neutral());
        assert!(g.mul(&times(1)).same(&g));
        assert!(g.mul(&times(3)).same(&three));
        let seventeen = (0..17).fold(Point::NEUTRAL, |sum, _| sum.add(&g));
        assert!(g.mul(&times(17)).same(&seventeen));
        // The group's order times any point is the neutral one: minus one
        // times it, and once more.
        let minus_one = Scalar::ZERO.sub(times(1));
        assert!(g.mul(&minus_one).add(&g).is_neutral());
        // Scalars multiply as the points they make do.
        let (a, b) = (times(0x1234_5678_9ABC), Scalar::of([5, 6, 7, 8, 9]));
        assert!(g.mul(&a).mul(&b).same(&g.mul(&a.mul(b))));
    }

    #[test]
    fn a_point_has_one_encoding_and_decodes_from_it() {
        let g = Point::GENERATOR;
        // The generator's is four: its `y/x`.
        assert_eq!(g.encode(), Fp5::of(4));
        assert_eq!(Point::NEUTRAL.encode(), Fp5::ZERO);
        for scalar in [times(1), times(2), times(977), Scalar::of([1, 2, 3, 4, 5])] {
            let point = g.mul(&scalar);
            let decoded = Point::decode(point.encode()).expect("a point's encoding decodes");
            assert!(decoded.same(&point));
            assert_eq!(decoded.encode(), point.encode());
        }
        assert!(Point::decode(Fp5::ZERO).unwrap().is_neutral());
        // Most elements encode no point: about half do.
        let none = (1..40).filter(|value| Point::decode(Fp5::of(*value)).is_none());
        assert!(none.count() > 5);
    }

    #[test]
    fn a_table_is_read_by_place() {
        let g = Point::GENERATOR;
        let mut table = [Point::NEUTRAL; 16];
        for place in 1..16 {
            table[place] = table[place - 1].add(&g);
        }
        for place in 0..16_u8 {
            let picked = chosen(&table, place);
            assert_eq!(picked.x, table[usize::from(place)].x);
            assert_eq!(picked.t, table[usize::from(place)].t);
        }
    }
}
