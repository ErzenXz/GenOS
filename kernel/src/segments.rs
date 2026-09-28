//! Private visible segment selectors for the admitted 64-bit user profile.
//!
//! The GDT has only flat user descriptors, LDTR is null, FSGSBASE is disabled,
//! and no syscall writes a base MSR. FS/GS bases therefore stay zero. Selectors
//! still need preservation: users can load null or either readable descriptor.

pub const USER_DATA: u16 = 0x2b;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct State {
    pub ds: u16,
    pub es: u16,
    pub fs: u16,
    pub gs: u16,
}

impl State {
    pub const fn initial() -> Self {
        Self {
            ds: USER_DATA,
            es: USER_DATA,
            fs: 0,
            gs: 0,
        }
    }

    #[cfg(feature = "validation-boot")]
    pub const fn probe_pattern(seed: u64) -> Self {
        // Different live peers exercise null, data and readable code selectors.
        let selectors = [0, USER_DATA, 0x33];
        Self {
            ds: selectors[(seed % 3) as usize],
            es: selectors[((seed + 1) % 3) as usize],
            fs: selectors[((seed + 2) % 3) as usize],
            gs: selectors[(seed % 3) as usize],
        }
    }

    #[cfg(any(test, feature = "validation-boot"))]
    pub fn matches_saved(&self, bytes: &[u8]) -> bool {
        bytes.len() == 8
            && [self.ds, self.es, self.fs, self.gs]
                .iter()
                .zip(bytes.chunks_exact(2))
                .all(|(selector, saved)| selector.to_le_bytes() == saved)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_selectors_and_assembly_layout_are_exact() {
        assert_eq!(core::mem::size_of::<State>(), 8);
        assert_eq!(core::mem::offset_of!(State, ds), 0);
        assert_eq!(core::mem::offset_of!(State, es), 2);
        assert_eq!(core::mem::offset_of!(State, fs), 4);
        assert_eq!(core::mem::offset_of!(State, gs), 6);
        assert!(State::initial().matches_saved(&[0x2b, 0, 0x2b, 0, 0, 0, 0, 0]));
    }

    #[test]
    fn selector_oracle_rejects_each_corrupted_byte_and_truncation() {
        let bytes = [0x2b, 0, 0x2b, 0, 0, 0, 0, 0];
        for index in 0..bytes.len() {
            let mut changed = bytes;
            changed[index] ^= 1;
            assert!(!State::initial().matches_saved(&changed));
            assert!(!State::initial().matches_saved(&bytes[..index]));
        }
    }
}
