//! CPU page-protection policy, separate from privileged register access.
pub const EFER_NXE: u64 = 1 << 11;
pub const CR0_WP: u64 = 1 << 16;
pub const CR4_SMEP: u64 = 1 << 20;
pub const CR4_SMAP: u64 = 1 << 21;
pub const RFLAGS_AC: u64 = 1 << 18;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Features {
    pub nx: bool,
    pub smep: bool,
    pub smap: bool,
}

impl Features {
    pub const fn from_cpuid(extended_edx: u32, structured_ebx: u32) -> Self {
        Self {
            nx: extended_edx & (1 << 20) != 0,
            smep: structured_ebx & (1 << 7) != 0,
            smap: structured_ebx & (1 << 20) != 0,
        }
    }

    pub const fn cr4_bits(self) -> u64 {
        (if self.smep { CR4_SMEP } else { 0 }) | (if self.smap { CR4_SMAP } else { 0 })
    }

    pub const fn verified(self, efer: u64, cr0: u64, cr4: u64) -> bool {
        self.nx
            && efer & EFER_NXE != 0
            && cr0 & CR0_WP != 0
            && cr4 & self.cr4_bits() == self.cr4_bits()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn required_and_optional_cpu_features_have_independent_bits() {
        for nx in [false, true] {
            for smep in [false, true] {
                for smap in [false, true] {
                    let features = Features::from_cpuid(
                        u32::from(nx) << 20,
                        (u32::from(smep) << 7) | (u32::from(smap) << 20),
                    );
                    assert_eq!(features, Features { nx, smep, smap });
                    assert_eq!(features.verified(EFER_NXE, CR0_WP, features.cr4_bits()), nx);
                    assert!(!features.verified(0, CR0_WP, features.cr4_bits()));
                    assert!(!features.verified(EFER_NXE, 0, features.cr4_bits()));
                    if smep {
                        assert!(!features.verified(EFER_NXE, CR0_WP, CR4_SMAP));
                    }
                    if smap {
                        assert!(!features.verified(EFER_NXE, CR0_WP, CR4_SMEP));
                    }
                }
            }
        }
    }
    #[test]
    fn ac_mask_preserves_interrupt_direction_and_arithmetic_flags() {
        assert_eq!(0x60246 & !RFLAGS_AC, 0x20246);
    }
}
