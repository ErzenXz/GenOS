//! Bounded legacy floating-point state policy for the single-CPU reference.
//!
//! AVX and later XSAVE components are deliberately unavailable. FXSAVE64
//! preserves x87/MMX, all sixteen XMM registers and MXCSR in a fixed image.

pub const STATE_BYTES: usize = 512;
pub const CR0_REQUIRED: u64 = (1 << 1) | (1 << 5); // MP, NE
pub const CR0_FORBIDDEN: u64 = (1 << 2) | (1 << 3); // EM, TS
pub const CR4_REQUIRED: u64 = (1 << 9) | (1 << 10); // OSFXSR, OSXMMEXCPT
pub const CR4_OSXSAVE: u64 = 1 << 18;
pub const CR4_PKE: u64 = 1 << 22;
pub const CR4_FSGSBASE: u64 = 1 << 16;
pub const EFER_FFXSR: u64 = 1 << 14;
const REQUIRED_CPUID_EDX: u32 = (1 << 0) | (1 << 23) | (1 << 24) | (1 << 25) | (1 << 26);

pub const fn supported(cpuid_edx: u32) -> bool {
    cpuid_edx & REQUIRED_CPUID_EDX == REQUIRED_CPUID_EDX
}

pub const fn configured(cr0: u64, cr4: u64, efer: u64) -> bool {
    cr0 & CR0_REQUIRED == CR0_REQUIRED
        && cr0 & CR0_FORBIDDEN == 0
        && cr4 & CR4_REQUIRED == CR4_REQUIRED
        && cr4 & (CR4_OSXSAVE | CR4_PKE | CR4_FSGSBASE) == 0
        && efer & EFER_FFXSR == 0
}

/// Only the CPU entry assembly mutates a live process's image. It is never
/// exposed as user memory. Moving an inactive process preserves alignment.
#[repr(C, align(16))]
pub struct State([u8; STATE_BYTES]);

impl State {
    /// Deterministic initial state, including the payload of empty x87 slots.
    /// Every exception starts masked; all XMM values and x87 pointers are zero.
    pub const fn initial() -> Self {
        let mut bytes = [0; STATE_BYTES];
        bytes[0] = 0x7f; // FCW = 0x037f
        bytes[1] = 0x03;
        bytes[24] = 0x80; // MXCSR = 0x1f80
        bytes[25] = 0x1f;
        Self(bytes)
    }

    #[cfg(any(test, feature = "validation-boot"))]
    pub fn bytes(&self) -> &[u8; STATE_BYTES] {
        &self.0
    }

    #[cfg(feature = "validation-boot")]
    pub fn probe_pattern(seed: u64) -> Self {
        let mut image = Self::initial();
        image.0[1] |= ((seed % 3 + 1) as u8) << 2; // distinct x87 rounding
        image.0[4] = 0xff; // all eight x87 registers non-empty
        image.0[25] |= ((seed % 3 + 1) as u8) << 5; // distinct SSE rounding
        for slot in 0..8 {
            let start = 32 + slot * 16;
            let mantissa = 0x8000_0000_0000_0000 | (seed << 8) | slot as u64;
            image.0[start..start + 8].copy_from_slice(&mantissa.to_le_bytes());
            image.0[start + 8..start + 10].copy_from_slice(&0x3fffu16.to_le_bytes());
        }
        for slot in 0..16 {
            let start = 160 + slot * 16;
            image.0[start..start + 8].copy_from_slice(&(seed + slot as u64).to_le_bytes());
            image.0[start + 8..start + 16].copy_from_slice(&(!(seed + slot as u64)).to_le_bytes());
        }
        image
    }

    #[cfg(feature = "validation-boot")]
    pub fn probe_mmx_pattern(seed: u64) -> Self {
        let mut image = Self::probe_pattern(seed);
        // Distinct integer values prove the MMX phase was captured, instead
        // of accidentally restoring the preceding x87 mantissas unchanged.
        for slot in 0..8 {
            let start = 32 + slot * 16;
            let value = 0xa55a_0000_0000_0000 | (seed << 8) | slot as u64;
            image.0[start..start + 8].copy_from_slice(&value.to_le_bytes());
        }
        image
    }

    /// FXSAVE's reserved bytes and CPU-reported MXCSR mask are not process
    /// state. Compare every defined restorable field, including empty payloads.
    #[cfg(any(test, feature = "validation-boot"))]
    pub fn matches_saved(&self, saved: &[u8]) -> bool {
        self.matches_saved_slots(saved, 10)
    }

    /// Intel SDM's FXSAVE format reserves the high sixteen bits of a slot
    /// when it contains MMX data; x87 comparisons above still cover all 80.
    #[cfg(any(test, feature = "validation-boot"))]
    pub fn matches_saved_mmx(&self, saved: &[u8]) -> bool {
        self.matches_saved_slots(saved, 8)
    }

    #[cfg(any(test, feature = "validation-boot"))]
    fn matches_saved_slots(&self, saved: &[u8], payload_bytes: usize) -> bool {
        saved.len() == STATE_BYTES
            && self.0[..5] == saved[..5]
            && self.0[6..28] == saved[6..28]
            && (0..8).all(|slot| {
                let start = 32 + slot * 16;
                self.0[start..start + payload_bytes] == saved[start..start + payload_bytes]
            })
            && self.0[160..416] == saved[160..416]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_each_missing_required_feature() {
        assert!(supported(REQUIRED_CPUID_EDX));
        for bit in [0, 23, 24, 25, 26] {
            assert!(!supported(REQUIRED_CPUID_EDX & !(1 << bit)));
        }
    }

    #[test]
    fn rejects_lazy_and_extended_state_modes() {
        assert!(configured(CR0_REQUIRED, CR4_REQUIRED, 0));
        for bit in [1 << 2, 1 << 3] {
            assert!(!configured(CR0_REQUIRED | bit, CR4_REQUIRED, 0));
        }
        assert!(!configured(CR0_REQUIRED, CR4_REQUIRED | CR4_OSXSAVE, 0));
        assert!(!configured(CR0_REQUIRED, CR4_REQUIRED | CR4_PKE, 0));
        assert!(!configured(CR0_REQUIRED, CR4_REQUIRED | CR4_FSGSBASE, 0));
        assert!(!configured(CR0_REQUIRED, CR4_REQUIRED, EFER_FFXSR));
    }

    #[test]
    fn initial_state_clears_payload_and_masks_exceptions() {
        let state = State::initial();
        assert_eq!(core::mem::size_of::<State>(), STATE_BYTES);
        assert_eq!(core::mem::align_of::<State>(), 16);
        assert_eq!(&state.0[..2], &[0x7f, 3]);
        assert_eq!(&state.0[24..28], &[0x80, 0x1f, 0, 0]);
        assert!(state.0[32..].iter().all(|byte| *byte == 0));
        assert!(state.matches_saved(state.bytes()));
    }

    #[test]
    fn comparison_covers_every_restorable_register_and_ignores_reserved_bytes() {
        let state = State::initial();
        for index in 0..STATE_BYTES {
            let mut saved = *state.bytes();
            saved[index] ^= 1;
            let is_state = index < 5
                || (6..28).contains(&index)
                || (32..160).contains(&index) && (index - 32) % 16 < 10
                || (160..416).contains(&index);
            assert_eq!(state.matches_saved(&saved), !is_state, "byte {index}");
        }
    }

    #[test]
    fn mmx_comparison_ignores_only_the_reserved_high_sixteen_bits() {
        let state = State::initial();
        for slot in 0..8 {
            let start = 32 + slot * 16;
            let mut saved = *state.bytes();
            saved[start + 8] = 0xff;
            saved[start + 9] = 0x3f;
            assert!(state.matches_saved_mmx(&saved));
            assert!(!state.matches_saved(&saved));
            saved[start + 7] ^= 1;
            assert!(!state.matches_saved_mmx(&saved));
        }
    }
}
