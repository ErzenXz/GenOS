//! Single-bootstrap ownership policy, independent of privileged CPU access.
//!
//! This is an admission gate, not an SMP synchronization implementation. Only
//! the hardware-designated BSP may claim boot or initialize shared tables.
use core::sync::atomic::{AtomicU8, Ordering};

pub const APIC_BASE_BSP: u64 = 1 << 8;
const CPUID_MSR: u32 = 1 << 5;
const CPUID_APIC: u32 = 1 << 9;
const CPUID_HTT: u32 = 1 << 28;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Features {
    pub msr: bool,
    pub apic: bool,
    pub initial_apic_id: u8,
    /// CPUID's package capacity, not a count of discovered or active CPUs.
    /// Zero means the HTT-gated field did not supply a usable value.
    pub max_logical_per_package: u8,
}

impl Features {
    pub const fn from_cpuid(ebx: u32, edx: u32) -> Self {
        Self {
            msr: edx & CPUID_MSR != 0,
            apic: edx & CPUID_APIC != 0,
            initial_apic_id: (ebx >> 24) as u8,
            max_logical_per_package: if edx & CPUID_HTT != 0 {
                (ebx >> 16) as u8
            } else {
                0
            },
        }
    }

    pub const fn can_read_apic_base(self) -> bool {
        self.msr && self.apic
    }

    pub const fn is_bsp(self, apic_base: Option<u64>) -> bool {
        self.can_read_apic_base()
            && match apic_base {
                Some(base) => base & APIC_BASE_BSP != 0,
                None => false,
            }
    }
}

/// Irreversible boot phases. Refused entrants never consume the BSP's claim.
/// Acquire/release order the gate itself; they do not protect later globals.
pub struct BootClaim {
    phase: AtomicU8,
}

impl Default for BootClaim {
    fn default() -> Self {
        Self::new()
    }
}

impl BootClaim {
    pub const fn new() -> Self {
        Self {
            phase: AtomicU8::new(0),
        }
    }

    pub fn claim(&self, features: Features, apic_base: Option<u64>) -> bool {
        self.advance(features, apic_base, 0, 1)
    }

    pub fn begin_table_init(&self, features: Features, apic_base: Option<u64>) -> bool {
        self.advance(features, apic_base, 1, 2)
    }

    fn advance(&self, features: Features, apic_base: Option<u64>, from: u8, to: u8) -> bool {
        features.is_bsp(apic_base)
            && self
                .phase
                .compare_exchange(from, to, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::{Arc, Barrier},
        thread,
        vec::Vec,
    };

    fn supported() -> Features {
        Features::from_cpuid(0, CPUID_MSR | CPUID_APIC)
    }

    #[test]
    fn both_feature_bits_are_required_before_reading_apic_base() {
        for msr in [false, true] {
            for apic in [false, true] {
                let features = Features::from_cpuid(
                    0,
                    (u32::from(msr) * CPUID_MSR) | (u32::from(apic) * CPUID_APIC),
                );
                assert_eq!(features.can_read_apic_base(), msr && apic);
                assert_eq!(features.is_bsp(Some(APIC_BASE_BSP)), msr && apic);
                assert!(!features.is_bsp(None));
            }
        }
    }

    #[test]
    fn hardware_bsp_role_is_required_even_if_apic_id_is_zero() {
        for base in [0, 0xfee0_0800, !APIC_BASE_BSP] {
            assert!(!supported().is_bsp(Some(base)));
        }
        // BSP identity comes from the MSR, never from assuming APIC ID zero.
        let nonzero_id = Features::from_cpuid(37 << 24, CPUID_MSR | CPUID_APIC);
        assert!(nonzero_id.is_bsp(Some(0xfee0_0900)));
    }

    #[test]
    fn rejected_aps_and_unsupported_cpus_do_not_consume_boot_ownership() {
        let gate = BootClaim::new();
        assert!(!gate.claim(supported(), Some(0xfee0_0800)));
        assert!(!gate.claim(Features::from_cpuid(0, 0), Some(APIC_BASE_BSP)));
        assert!(!gate.claim(supported(), None));
        assert!(gate.claim(supported(), Some(APIC_BASE_BSP)));
        assert!(!gate.claim(supported(), Some(APIC_BASE_BSP)));
    }

    #[test]
    fn tables_require_the_claim_and_only_initialize_once() {
        let gate = BootClaim::new();
        assert!(!gate.begin_table_init(supported(), Some(APIC_BASE_BSP)));
        assert!(gate.claim(supported(), Some(APIC_BASE_BSP)));
        assert!(!gate.begin_table_init(supported(), Some(0)));
        assert!(gate.begin_table_init(supported(), Some(APIC_BASE_BSP)));
        assert!(!gate.begin_table_init(supported(), Some(APIC_BASE_BSP)));
        assert!(!gate.claim(supported(), Some(APIC_BASE_BSP)));
    }

    #[test]
    fn simultaneous_claims_grant_exactly_one_owner() {
        let gate = Arc::new(BootClaim::new());
        let barrier = Arc::new(Barrier::new(32));
        let entrants: Vec<_> = (0..32)
            .map(|_| {
                let gate = Arc::clone(&gate);
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    gate.claim(supported(), Some(APIC_BASE_BSP))
                })
            })
            .collect();
        assert_eq!(
            entrants
                .into_iter()
                .map(|entrant| usize::from(entrant.join().unwrap()))
                .sum::<usize>(),
            1
        );
    }

    #[test]
    fn package_capacity_is_only_reported_when_htt_advertises_it() {
        let ebx = (42 << 24) | (16 << 16);
        let features = Features::from_cpuid(ebx, CPUID_HTT);
        assert_eq!(features.initial_apic_id, 42);
        assert_eq!(features.max_logical_per_package, 16);
        assert_eq!(Features::from_cpuid(ebx, 0).max_logical_per_package, 0);
        assert_eq!(
            Features::from_cpuid(0, CPUID_HTT).max_logical_per_package,
            0
        );
    }
}
