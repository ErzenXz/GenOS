//! Validation-only oracle for the native integer-register fixture.
//! No masked comparisons: every GPR, RSP, RFLAGS and both selectors must match.

use crate::{cpu_context::UserContext, native_image, segments};

pub const STRIDE: usize = 160;
pub const FIRST: u64 = 64;
pub const INPUT: u64 = 1536;
pub const PROGRESS: u64 = 3072;

pub fn pattern(seed: u64) -> [u64; 17] {
    let mut words = [0; 17];
    for (index, word) in words[..15].iter_mut().enumerate() {
        *word = 0xa5c3_796e_0000_0000 ^ (seed << 32) ^ (0x0101_0101 * (index as u64 + 1));
    }
    words[11] = native_image::USER_DATA + PROGRESS + seed * 8; // RBX: private progress
    words[15] = native_image::USER_STACK_TOP;
    words[16] = match seed % 3 {
        0 => 0x202 | 0x40000 | 0x55, // AC and selected arithmetic flags
        1 => 0x202 | 0x400 | 0x880,  // DF, SF, OF
        _ => 0x202 | 0x40000 | 0x400 | 0x8d5 | 0x200000, // AC, DF, arithmetic, ID
    };
    words
}

pub fn initial(seed: u64) -> [u64; 17] {
    let mut words = [0; 17];
    words[8] = seed;
    words[15] = native_image::USER_STACK_TOP;
    words[16] = 0x202;
    words
}

pub fn yielding(seed: u64, result: bool) -> [u64; 17] {
    let mut words = pattern(seed);
    // The admitted YIELD ABI requires all six arguments to be zero.
    for index in [5, 6, 7, 8, 9, 12] {
        words[index] = 0;
    }
    words[14] = if result {
        0
    } else {
        genos_abi::USER_SYSCALL_YIELD
    };
    words
}

pub fn frame_words(frame: &UserContext) -> [u64; 17] {
    [
        frame.r15,
        frame.r14,
        frame.r13,
        frame.r12,
        frame.r11,
        frame.r10,
        frame.r9,
        frame.r8,
        frame.rdi,
        frame.rsi,
        frame.rbp,
        frame.rbx,
        frame.rdx,
        frame.rcx,
        frame.rax,
        frame.rsp,
        frame.rflags,
    ]
}

pub fn matches(words: &[u64; 17], bytes: &[u8]) -> bool {
    bytes.len() == STRIDE
        && words
            .iter()
            .enumerate()
            .all(|(index, word)| bytes[index * 8..index * 8 + 8] == word.to_le_bytes())
        && bytes[136..138] == crate::cpu_context::USER_CODE_SELECTOR.to_le_bytes()
        && bytes[138..140] == segments::USER_DATA.to_le_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_register_flag_and_selector_bit_is_part_of_the_oracle() {
        let words = pattern(2);
        let mut bytes = [0; STRIDE];
        for (index, word) in words.iter().enumerate() {
            bytes[index * 8..index * 8 + 8].copy_from_slice(&word.to_le_bytes());
        }
        bytes[136..138].copy_from_slice(&crate::cpu_context::USER_CODE_SELECTOR.to_le_bytes());
        bytes[138..140].copy_from_slice(&segments::USER_DATA.to_le_bytes());
        assert!(matches(&words, &bytes));
        for index in 0..140 {
            for bit in 0..8 {
                bytes[index] ^= 1 << bit;
                assert!(!matches(&words, &bytes), "ignored byte={index} bit={bit}");
                bytes[index] ^= 1 << bit;
            }
        }
        assert!(!matches(&words, &bytes[..STRIDE - 1]));
        assert!(!matches(&words, &[0; STRIDE + 1]));
    }

    #[test]
    fn peers_reuse_and_abi_constraints_have_distinct_admitted_expectations() {
        let seeds = [2, 4, 7, 10, 12, 15];
        for seed in seeds {
            let words = pattern(seed);
            assert!(crate::exception::user_flags_valid(words[16]));
            assert_eq!(
                initial(seed),
                frame_words(&UserContext::initial(
                    seed,
                    native_image::USER_CODE,
                    native_image::USER_STACK_TOP,
                ))
            );
            for peer in seeds.into_iter().filter(|peer| *peer != seed) {
                for index in 0..15 {
                    assert_ne!(words[index], pattern(peer)[index]);
                }
            }
            let yielded = yielding(seed, true);
            assert_eq!(yielded[14], 0);
            for index in 0..14 {
                assert_eq!(
                    yielded[index],
                    if [5, 6, 7, 8, 9, 12].contains(&index) {
                        0
                    } else {
                        words[index]
                    }
                );
            }
        }
    }
}
