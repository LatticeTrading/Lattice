//! Poseidon2 over the Goldilocks field: the hash Lighter's signatures are
//! made with, of a transaction's members into what is signed and of a
//! signature's commitment into its challenge.
//!
//! The permutation is of twelve elements: four full rounds, twenty-two
//! partial ones and four full ones more, with the seventh power as its
//! S-box, the round constants of Lighter's own implementation and the
//! internal diagonal of Plonky3's. A hash is a sponge over it that takes
//! eight elements at a time, writing each block over the state's first
//! eight (no padding: a message's length is part of what it is), and gives
//! the state's first elements once all are in.
//!
//! Checked against `github.com/elliottech/poseidon_crypto` (v0.0.19, its
//! `poseidon2_goldilocks_plonky2`, which Lighter's signer hashes with): its
//! own test vectors for the permutation and the sponge are the tests here.

use super::field::{Fp, Fp5};

/// How many elements the permutation is of.
const WIDTH: usize = 12;
/// How many of them a block of the message is written over.
const RATE: usize = 8;

/// The constants of the eight full rounds, one per element of the state.
#[rustfmt::skip]
const EXTERNAL: [[u64; WIDTH]; 8] = [
    [
        0xD701_93D1_7AB3_B7D6, 0xA2C3_662A_78A9_162B, 0x7A9F_DA82_7556_AD44,
        0xE8D5_5018_18C9_9643, 0x4C7A_8FCE_D4D5_FD38, 0x55AB_3898_5C0C_513D,
        0x28A1_7BD0_1621_0B0B, 0x8F82_7767_9EC3_2FA8, 0x768B_3C3D_68A4_60E9,
        0x872A_022E_B559_D941, 0xD131_6DD4_B3B9_7973, 0xA7B6_08E5_7832_1000,
    ],
    [
        0x3FA0_2C87_B0BE_E026, 0x7A38_F002_2E13_C31E, 0x00C0_54F3_C5E8_D20D,
        0x439F_50F4_BCA7_242F, 0x4D09_38AA_57CD_517F, 0xB2E0_3AC5_FB6B_9A7D,
        0xE29D_1F42_37BE_DCA8, 0x05B7_C844_BC99_B848, 0x91CC_0B73_F34E_17ED,
        0x876E_4427_694B_D755, 0x6700_2AE0_725C_612D, 0x0535_1F20_E0B6_315F,
    ],
    [
        0x2E3B_9EF5_457E_B60B, 0xD9AC_1761_8C37_83DD, 0x0807_528A_D887_4BCF,
        0xC78D_546A_455D_2A0E, 0xF8B9_30C8_1E24_81F0, 0x7127_07D8_DFF3_B041,
        0xDCB8_C0AA_0B9D_34C3, 0x9BAD_DBDF_2EE3_A468, 0x2DD1_6D50_C517_6C78,
        0x89EA_C5CF_BC07_5CD3, 0x2A74_1DEA_1815_87F3, 0x1A4D_6AA8_5A11_3D84,
    ],
    [
        0x4D73_6286_A238_7E34, 0x8BAD_5DFC_4FCB_3EE3, 0x84FB_D03A_DB77_C56A,
        0x8D5C_DD1A_23EC_53A2, 0x036F_08F0_8FFF_28EC, 0xB717_A3F4_DBDF_B443,
        0x58A0_74B5_509D_645C, 0xF92B_F834_E4B8_7718, 0x1541_C3A0_BAA5_AC4B,
        0x2214_9E67_83E6_7692, 0x9BE8_B5D9_E112_476F, 0x41E0_969F_62BA_BB76,
    ],
    [
        0xBC58_5AD3_B944_3DBB, 0xF28D_D320_6975_CBB1, 0xDD88_15E5_3CA0_45E0,
        0xDE82_C416_B9E7_01BA, 0xC5CB_8752_33AF_A025, 0x7212_697C_D897_FFA9,
        0x6784_4790_AA63_CFD7, 0xDC0B_9CFA_97FE_65C3, 0xE8FE_0918_69A8_2070,
        0x6290_2BB2_E413_C6D1, 0x29F9_F500_1FB8_4F57, 0xBE10_1479_6EF5_F8BE,
    ],
    [
        0x71FE_B53E_9BDB_A19C, 0x2510_54F5_92EB_B71C, 0xE1A5_7643_A4BB_284B,
        0xA4BA_6F87_A45B_739B, 0x2C1F_CADE_0B95_8C49, 0xBBB4_24CD_A9A3_E360,
        0x2CA6_4735_4C5F_3F54, 0xC927_7B64_D152_E084, 0xDBC9_AC97_445E_FF17,
        0x6F6C_DF31_9896_9F70, 0x1DE2_9D14_FA76_D8F1, 0x7333_7458_A8CC_1D19,
    ],
    [
        0xB87E_775E_2FB3_AB23, 0xF166_A1C7_A565_C80B, 0xB24B_E06F_426C_747F,
        0xC281_E8C4_9482_CE00, 0x5197_4C3B_3B72_6C2D, 0x8744_4CF8_CAF7_D619,
        0x7C36_2F82_7A58_0CED, 0x9567_AF14_6676_47A0, 0xCBF0_473C_BEC5_4E37,
        0xE320_9DED_EFF4_F620, 0xD43A_D94E_45A4_C4EE, 0x9769_81EE_73F4_1768,
    ],
    [
        0xEF70_7A22_4E20_7258, 0x2FC7_79E1_0E63_62EE, 0x29B5_EE60_AD8C_891F,
        0x96B3_7B39_D8BF_D667, 0x877D_F68A_8B22_E733, 0x5C41_746F_562C_8D9F,
        0x0C9D_7675_1052_B71A, 0xFB34_6534_1BF1_C087, 0xA0D1_4DC6_14D1_5EB1,
        0xDC27_D171_3690_6FA6, 0x482E_163B_05EC_397F, 0x0273_A462_9923_66EF,
    ],
];

/// The constants of the partial rounds, each added to the first element.
#[rustfmt::skip]
const INTERNAL: [u64; 22] = [
    0xA571_418D_9589_7B60, 0x8F32_6765_74FC_F6D3, 0x7311_02D4_E3FB_1BBE,
    0x0330_F083_28A8_2D2B, 0x7F04_49B6_557F_785D, 0x62F0_6210_658D_CBCB,
    0xD5A9_8AF9_F89C_458B, 0x77EC_6908_3A34_6385, 0xEF7C_A48B_BC27_F890,
    0x53E9_652F_61EA_C532, 0xA71C_634A_BFF4_F0CC, 0xB16F_5F0D_7E28_EA29,
    0xC9DD_E31D_0A00_3AB2, 0x2DDA_DF97_7590_2533, 0xE4FA_73FB_1640_8B47,
    0x9024_2EBC_00D2_EE59, 0xBB02_DFFD_9F38_1982, 0xDEA3_2836_4C50_907C,
    0x1395_D3B9_2485_7CF8, 0x7D3E_AD0D_5AEC_04E6, 0xC2F1_2BE3_FED7_4668,
    0x0BA3_C338_F8C3_D285,
];

/// The diagonal of the partial rounds' linear layer.
#[rustfmt::skip]
const DIAGONAL: [u64; WIDTH] = [
    0xC3B6_C08E_23BA_9300, 0xD84B_5DE9_4A32_4FB6, 0x0D0C_371C_5B35_B84F,
    0x7964_F570_E718_8037, 0x5DAF_18BB_D996_604B, 0x6743_BC47_B959_5257,
    0x5528_B936_2C59_BB70, 0xAC45_E25B_7127_B68B, 0xA207_7D7D_FBB6_06B5,
    0xF3FA_AC6F_AEE3_78AE, 0x0C63_88B5_1545_E883, 0xD27D_BB69_4491_7B60,
];

/// The S-box: the seventh power.
fn seventh(element: Fp) -> Fp {
    let square = element.square();
    square.square() * square * element
}

/// The full rounds' linear layer: each four elements mixed among
/// themselves, then every element added the sum of those in its place of
/// the three fours.
fn mix_fully(state: &mut [Fp; WIDTH]) {
    for four in state.as_chunks_mut::<4>().0 {
        let (first, second) = (four[0] + four[1], four[2] + four[3]);
        let all = first + second;
        let (odd, even) = (all + four[1], all + four[3]);
        let (twice_first, twice_third) = (four[0].double(), four[2].double());
        four[0] = odd + first;
        four[1] = twice_third + odd;
        four[2] = second + even;
        four[3] = twice_first + even;
    }
    let mut sums = [Fp::ZERO; 4];
    for (place, element) in state.iter().enumerate() {
        sums[place % 4] = sums[place % 4] + *element;
    }
    for (place, element) in state.iter_mut().enumerate() {
        *element = *element + sums[place % 4];
    }
}

/// The partial rounds' linear layer: each element times its place of the
/// diagonal, added the sum of them all.
fn mix_partly(state: &mut [Fp; WIDTH]) {
    let sum = state.iter().fold(Fp::ZERO, |sum, element| sum + *element);
    for (element, diagonal) in state.iter_mut().zip(DIAGONAL) {
        *element = *element * Fp::new(diagonal) + sum;
    }
}

/// Four full rounds, with the constants `rounds` names.
fn full_rounds(state: &mut [Fp; WIDTH], rounds: &[[u64; WIDTH]]) {
    for constants in rounds {
        for (element, constant) in state.iter_mut().zip(constants) {
            *element = seventh(*element + Fp::new(*constant));
        }
        mix_fully(state);
    }
}

/// The permutation.
fn permute(state: &mut [Fp; WIDTH]) {
    mix_fully(state);
    full_rounds(state, &EXTERNAL[..4]);
    for constant in INTERNAL {
        state[0] = seventh(state[0] + Fp::new(constant));
        mix_partly(state);
    }
    full_rounds(state, &EXTERNAL[4..]);
}

/// The first `N` elements the sponge gives of `message` (at most a block of
/// them).
fn hashed<const N: usize>(message: &[Fp]) -> [Fp; N] {
    let mut state = [Fp::ZERO; WIDTH];
    for block in message.chunks(RATE) {
        state[..block.len()].copy_from_slice(block);
        permute(&mut state);
    }
    let mut output = [Fp::ZERO; N];
    output.copy_from_slice(&state[..N]);
    output
}

/// A hash of four elements: what several of a transaction's members are
/// folded into before they are hashed with the rest.
pub(super) type Digest = [Fp; 4];

/// `message` hashed to four elements.
pub(super) fn digest(message: &[Fp]) -> Digest {
    hashed(message)
}

/// Two digests hashed to one.
pub(super) fn digest_of_two(first: &Digest, second: &Digest) -> Digest {
    let mut both = [Fp::ZERO; 8];
    both[..4].copy_from_slice(first);
    both[4..].copy_from_slice(second);
    digest(&both)
}

/// `message` hashed to an element of the extension: what is signed, and
/// what a signature's challenge is read from.
pub(super) fn hash(message: &[Fp]) -> Fp5 {
    Fp5(hashed(message))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn elements<const N: usize>(values: [u64; N]) -> [Fp; N] {
        values.map(Fp::new)
    }

    /// `TestPermute` of the reference implementation.
    #[test]
    fn the_permutation_is_the_reference_implementations() {
        let mut state = elements([
            5_417_613_058_500_526_590,
            2_481_548_824_842_427_254,
            6_473_243_198_879_784_792,
            1_720_313_757_066_167_274,
            2_806_320_291_675_974_571,
            7_407_976_414_706_455_446,
            1_105_257_841_424_046_885,
            7_613_435_757_403_328_049,
            3_376_066_686_066_811_538,
            5_888_575_799_323_675_710,
            6_689_309_723_188_675_948,
            2_468_250_420_241_012_720,
        ]);
        permute(&mut state);
        assert_eq!(
            state,
            elements([
                5_364_184_781_011_389_007,
                15_309_475_861_242_939_136,
                5_983_386_513_087_443_499,
                886_942_118_604_446_276,
                14_903_657_885_227_062_600,
                7_742_650_891_575_941_298,
                1_962_182_278_500_985_790,
                10_213_480_816_595_178_755,
                3_510_799_061_817_443_836,
                4_610_029_967_627_506_430,
                7_566_382_334_276_534_836,
                2_288_460_879_362_380_348,
            ])
        );
    }

    /// `TestHashNToMNoPad` of the reference implementation: twelve elements
    /// in, which is a block and half of another.
    #[test]
    fn a_message_is_hashed_a_block_at_a_time() {
        let message = elements([
            2_963_773_914_414_780_088,
            8_389_525_300_242_074_234,
            3_700_959_901_615_818_008,
            6_116_199_383_751_757_212,
            3_418_607_418_699_599_889,
            8_793_277_256_263_635_044,
            448_623_437_464_918_480,
            1_857_310_021_116_627_925,
            6_145_634_616_307_237_342,
            1_548_353_948_794_474_539,
            2_318_110_128_254_703_527,
            8_347_759_953_730_634_762,
        ]);
        let expected = elements([
            3_627_923_032_009_111_551,
            1_460_752_551_327_577_353,
            1_084_214_837_491_058_067,
            1_841_622_875_286_057_462,
            3_996_252_440_506_437_984,
            1_276_718_204_392_552_803,
            8_564_515_621_134_952_155,
            9_252_927_025_993_202_701,
        ]);
        assert_eq!(hashed::<8>(&message), expected);
        // A digest and a hash are the first four and the first five.
        assert_eq!(digest(&message), expected[..4]);
        assert_eq!(hash(&message).0, expected[..5]);
        // Two digests are hashed as the eight elements they are.
        let (first, second) = (digest(&message[..4]), digest(&message[4..]));
        let mut both = first.to_vec();
        both.extend(second);
        assert_eq!(digest_of_two(&first, &second), digest(&both));
    }
}
