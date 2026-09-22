//! Bounded authority ledger for managed physical frames. Addresses are never
//! release authority: a grant identifies its owner, allocation and ledger slot.
use crate::physmem::PAGE_SIZE;

pub const GRANT_CAPACITY: usize = 8192;
/// Tables and leaves both consume the owner's budget. General heap growth must
/// explicitly revise this versioned resource contract, not bypass admission.
pub const USER_FRAME_LIMIT: usize = 64;
pub const KERNEL_FRAME_RESERVE: usize = 256;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Owner(u64);
impl Owner {
    pub const KERNEL: Self = Self(0);
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Kind {
    Table,
    User,
}

/// Copies retain the same authority, not a second allocation. Every operation
/// revalidates the identity; old copies cannot authorize reuse of a slot/frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Grant {
    address: u64,
    generation: u64,
    owner: Owner,
    slot: usize,
    kind: Kind,
}
impl Grant {
    pub const fn address(self) -> u64 {
        self.address
    }
}

#[derive(Clone, Copy)]
struct Record {
    address: u64,
    generation: u64,
    owner: Owner,
    mapped_at: u64,
    kind: Kind,
}
impl Record {
    const EMPTY: Self = Self {
        address: 0,
        generation: 0,
        owner: Owner::KERNEL,
        mapped_at: 0,
        kind: Kind::Table,
    };
    fn grant(self, slot: usize) -> Grant {
        Grant {
            address: self.address,
            generation: self.generation,
            owner: self.owner,
            slot,
            kind: self.kind,
        }
    }
}

pub struct Ledger<const N: usize> {
    records: [Record; N],
    next_generation: u64,
    next_owner: u64,
    live: usize,
    pub denied: u64,
    pub exhausted: u64,
    pub quota_denials: u64,
}
impl<const N: usize> Default for Ledger<N> {
    fn default() -> Self {
        Self::new()
    }
}
impl<const N: usize> Ledger<N> {
    pub const fn new() -> Self {
        Self {
            records: [Record::EMPTY; N],
            next_generation: 1,
            next_owner: 1,
            live: 0,
            denied: 0,
            exhausted: 0,
            quota_denials: 0,
        }
    }

    /// Owner identities never wrap or recycle, including after a failed build.
    pub fn new_owner(&mut self) -> Option<Owner> {
        let next = self.next_owner.checked_add(1)?;
        let owner = Owner(self.next_owner);
        self.next_owner = next;
        Some(owner)
    }

    /// Capacity/generation admission happens before the physical allocator runs.
    pub fn allocate(
        &mut self,
        owner: Owner,
        kind: Kind,
        allocate: impl FnOnce() -> Option<u64>,
    ) -> Option<Grant> {
        if owner != Owner::KERNEL
            && (self.owner_frames(owner) >= USER_FRAME_LIMIT
                || self.live >= N - KERNEL_FRAME_RESERVE.min(N / 4))
        {
            self.quota_denials = self.quota_denials.saturating_add(1);
            return None;
        }
        let slot = self.records.iter().position(|record| record.address == 0);
        let next = self.next_generation.checked_add(1);
        let (Some(slot), Some(next)) = (slot, next) else {
            self.exhausted = self.exhausted.saturating_add(1);
            return None;
        };
        let address = allocate()?;
        // The adapter must supply an exclusive aligned live bitmap frame.
        assert!(address != 0 && address.is_multiple_of(PAGE_SIZE));
        assert!(!self.records.iter().any(|record| record.address == address));
        let record = Record {
            address,
            generation: self.next_generation,
            owner,
            mapped_at: 0,
            kind,
        };
        self.records[slot] = record;
        self.next_generation = next;
        self.live += 1;
        Some(record.grant(slot))
    }

    pub fn is_live(&self, grant: Grant) -> bool {
        self.records
            .get(grant.slot)
            .is_some_and(|record| record.address != 0 && record.grant(grant.slot) == grant)
    }

    /// Only an owning address-space identity may recover authority while walking
    /// its private page tables. A stale root grant must be rejected before walks.
    pub fn find(&self, owner: Owner, address: u64, kind: Kind) -> Option<Grant> {
        self.records
            .iter()
            .enumerate()
            .find(|(_, record)| {
                record.address == address
                    && address != 0
                    && record.owner == owner
                    && record.kind == kind
            })
            .map(|(slot, record)| record.grant(slot))
    }

    /// Publishing the first user PTE pins its frame. Sharing/aliasing is denied.
    pub fn bind(&mut self, grant: Grant, owner: Owner, virtual_address: u64) -> bool {
        if !self.is_live(grant)
            || grant.owner != owner
            || grant.kind != Kind::User
            || virtual_address == 0
            || !virtual_address.is_multiple_of(PAGE_SIZE)
            || self.records[grant.slot].mapped_at != 0
        {
            self.denied = self.denied.saturating_add(1);
            return false;
        }
        self.records[grant.slot].mapped_at = virtual_address;
        true
    }

    pub fn can_bind(&self, grant: Grant, owner: Owner) -> bool {
        self.is_live(grant)
            && grant.owner == owner
            && grant.kind == Kind::User
            && self.records[grant.slot].mapped_at == 0
    }

    pub fn mapping_matches(&self, owner: Owner, address: u64, virtual_address: u64) -> bool {
        self.find(owner, address, Kind::User).is_some_and(|grant| {
            virtual_address != 0 && self.records[grant.slot].mapped_at == virtual_address
        })
    }

    /// Hardware adapter must first unlink the PTE and retire its translation.
    /// Exact owner + address + virtual binding are required; no bare unpin exists.
    pub fn retire(&mut self, owner: Owner, address: u64, virtual_address: u64) -> Option<Grant> {
        let grant = self.find(owner, address, Kind::User)?;
        if virtual_address == 0 || self.records[grant.slot].mapped_at != virtual_address {
            return None;
        }
        self.records[grant.slot].mapped_at = 0;
        Some(grant)
    }

    /// Denial never reaches memory preparation. Failed preparation retains all
    /// authority. Pinned grants cannot be scrubbed or returned to the bitmap.
    pub fn release(&mut self, grant: Grant, prepare_and_free: impl FnOnce(u64) -> bool) -> bool {
        if !self.is_live(grant) || self.records[grant.slot].mapped_at != 0 {
            self.denied = self.denied.saturating_add(1);
            return false;
        }
        if !prepare_and_free(grant.address) {
            return false;
        }
        self.records[grant.slot] = Record::EMPTY;
        self.live -= 1;
        true
    }

    pub const fn live(&self) -> usize {
        self.live
    }
    pub fn owner_frames(&self, owner: Owner) -> usize {
        self.records
            .iter()
            .filter(|record| record.address != 0 && record.owner == owner)
            .count()
    }
    pub fn is_consistent(&self, bitmap_live: u64, contains: impl Fn(u64) -> bool) -> bool {
        self.live as u64 == bitmap_live
            && self
                .records
                .iter()
                .filter(|record| record.address != 0)
                .count()
                == self.live
            && self
                .records
                .iter()
                .filter(|record| record.address != 0)
                .all(|record| contains(record.address))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn denied(_: u64) -> bool {
        panic!("denied authority touched physical memory")
    }

    #[test]
    fn reuse_rejects_stale_grant_before_memory_access() {
        let mut ledger = Ledger::<2>::new();
        let owner = ledger.new_owner().unwrap();
        let old = ledger.allocate(owner, Kind::User, || Some(0x1000)).unwrap();
        assert!(ledger.release(old, |_| true));
        let fresh = ledger.allocate(owner, Kind::User, || Some(0x1000)).unwrap();
        assert_ne!(old, fresh);
        assert!(!ledger.release(old, denied));
        assert!(!ledger.bind(old, owner, 0x4000));
        assert!(ledger.is_live(fresh));
        assert!(ledger.release(fresh, |_| true));
        assert!(!ledger.release(fresh, denied));
    }

    #[test]
    fn owner_kind_slot_and_address_forgery_cannot_release() {
        let mut ledger = Ledger::<2>::new();
        let owner = ledger.new_owner().unwrap();
        let other = ledger.new_owner().unwrap();
        let grant = ledger.allocate(owner, Kind::User, || Some(0x1000)).unwrap();
        for forged in [
            Grant {
                owner: other,
                ..grant
            },
            Grant {
                address: 0,
                ..grant
            },
            Grant {
                address: 0x1001,
                ..grant
            },
            Grant {
                kind: Kind::Table,
                ..grant
            },
            Grant {
                slot: usize::MAX,
                ..grant
            },
        ] {
            assert!(!ledger.release(forged, denied));
        }
        assert_eq!(ledger.find(other, 0x1000, Kind::User), None);
        assert!(!ledger.bind(grant, other, 0x4000));
        assert_eq!(ledger.owner_frames(owner), 1);
        assert_eq!(ledger.owner_frames(other), 0);
        assert!(ledger.is_live(grant));
    }

    #[test]
    fn mapping_pins_deny_release_and_aliases_until_exact_retirement() {
        let mut ledger = Ledger::<2>::new();
        let owner = ledger.new_owner().unwrap();
        let other = ledger.new_owner().unwrap();
        let grant = ledger.allocate(owner, Kind::User, || Some(0x1000)).unwrap();
        assert!(ledger.bind(grant, owner, 0x4000));
        assert!(!ledger.release(grant, denied));
        assert!(!ledger.bind(grant, owner, 0x5000));
        assert!(!ledger.bind(grant, other, 0x4000));
        assert_eq!(ledger.retire(other, 0x1000, 0x4000), None);
        assert_eq!(ledger.retire(owner, 0x1000, 0x5000), None);
        assert!(!ledger.release(grant, denied));
        assert_eq!(ledger.retire(owner, 0x1000, 0x4000), Some(grant));
        assert!(ledger.release(grant, |_| true));
        assert_eq!(ledger.owner_frames(owner), 0);
    }

    #[test]
    fn exhaustion_precedes_physical_allocation_and_recovers() {
        let mut ledger = Ledger::<1>::new();
        let a = ledger
            .allocate(Owner::KERNEL, Kind::Table, || Some(0x1000))
            .unwrap();
        assert!(ledger
            .allocate(Owner::KERNEL, Kind::Table, || panic!(
                "capacity exhaustion allocated"
            ))
            .is_none());
        assert_eq!(ledger.exhausted, 1);
        assert!(ledger.release(a, |_| true));
        assert!(ledger
            .allocate(Owner::KERNEL, Kind::Table, || None)
            .is_none());
        assert_eq!(ledger.live(), 0);
        assert!(ledger
            .allocate(Owner::KERNEL, Kind::Table, || Some(0x1000))
            .is_some());
    }

    #[test]
    fn failed_preparation_preserves_identity_and_accounting() {
        let mut ledger = Ledger::<1>::new();
        let grant = ledger
            .allocate(Owner::KERNEL, Kind::Table, || Some(0x1000))
            .unwrap();
        assert!(!ledger.release(grant, |_| false));
        assert!(ledger.is_live(grant));
        assert!(ledger.is_consistent(1, |address| address == 0x1000));
        assert!(!ledger.is_consistent(0, |_| true));
        assert!(!ledger.is_consistent(1, |_| false));
    }

    #[test]
    fn identity_counters_never_wrap() {
        let mut ledger = Ledger::<1>::new();
        ledger.next_owner = u64::MAX;
        assert_eq!(ledger.new_owner(), None);
        ledger.next_generation = u64::MAX;
        assert!(ledger
            .allocate(Owner::KERNEL, Kind::Table, || panic!(
                "generation exhaustion allocated"
            ))
            .is_none());
        assert_eq!(ledger.live(), 0);
    }

    #[test]
    fn kernel_metadata_budget_is_explicit() {
        assert!(core::mem::size_of::<Ledger<GRANT_CAPACITY>>() <= 328_000);
    }

    #[test]
    fn owner_quota_counts_tables_and_leaves_and_denial_never_allocates() {
        let mut ledger = Ledger::<128>::new();
        let owner = ledger.new_owner().unwrap();
        let other = ledger.new_owner().unwrap();
        let mut grants = std::vec::Vec::new();
        for index in 0..USER_FRAME_LIMIT {
            grants.push(
                ledger
                    .allocate(
                        owner,
                        if index % 2 == 0 {
                            Kind::Table
                        } else {
                            Kind::User
                        },
                        || Some((index as u64 + 1) * PAGE_SIZE),
                    )
                    .unwrap(),
            );
        }
        assert!(ledger
            .allocate(owner, Kind::User, || panic!("quota allocated"))
            .is_none());
        assert_eq!(ledger.quota_denials, 1);
        assert_eq!(ledger.owner_frames(owner), USER_FRAME_LIMIT);
        let witness = ledger
            .allocate(other, Kind::User, || Some(0x100000))
            .unwrap();
        let returned = grants.pop().unwrap();
        assert!(ledger.release(returned, |_| true));
        let replacement = ledger
            .allocate(owner, Kind::User, || Some(returned.address()))
            .unwrap();
        assert_ne!(returned, replacement);
        assert!(!ledger.release(returned, denied));
        assert!(ledger.release(replacement, |_| true));
        assert!(ledger.release(witness, |_| true));
        for grant in grants {
            assert!(ledger.release(grant, |_| true));
        }
        assert_eq!(ledger.live(), 0);
    }

    #[test]
    fn kernel_reserve_remains_available_when_user_admission_is_full() {
        let mut ledger = Ledger::<16>::new();
        let owner = ledger.new_owner().unwrap();
        for index in 0..12 {
            assert!(ledger
                .allocate(owner, Kind::User, || Some((index + 1) * PAGE_SIZE))
                .is_some());
        }
        assert!(ledger
            .allocate(owner, Kind::Table, || panic!("reserve consumed"))
            .is_none());
        for index in 12..16 {
            assert!(ledger
                .allocate(Owner::KERNEL, Kind::Table, || Some((index + 1) * PAGE_SIZE))
                .is_some());
        }
        assert!(ledger
            .allocate(Owner::KERNEL, Kind::Table, || panic!("ledger overflow"))
            .is_none());
        assert_eq!((ledger.quota_denials, ledger.exhausted), (1, 1));
    }
}
