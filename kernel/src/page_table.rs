//! Transactional supervisor page-table cloning. Leaf frames stay owned by
//! their original subsystem; only newly allocated table pages are rolled back.
pub const PRESENT: u64 = 1;
pub const USER: u64 = 4;
pub const HUGE: u64 = 128;
pub const ADDRESS_MASK: u64 = 0x000f_ffff_ffff_f000;

/// Table IDs accepted by read/write must refer to live, uniquely owned table
/// storage. Allocation supplies a zeroed table; release accepts allocations
/// from this instance only. A clone caller guarantees the source tree is valid.
///
/// # Safety
/// Implementations must preserve these lifetime/ownership obligations, and
/// callers of read/write must supply valid table IDs and indices below 512.
pub unsafe trait TableMemory {
    fn allocate(&mut self) -> Option<u64>;
    /// # Safety
    /// `table` is live and `slot < 512`.
    unsafe fn read(&self, table: u64, slot: usize) -> u64;
    /// # Safety
    /// `table` is live, exclusively owned and `slot < 512`.
    unsafe fn write(&mut self, table: u64, slot: usize, entry: u64);
    fn release(&mut self, table: u64);
}

/// # Safety
/// Source is a valid live page-table tree of the given depth (1..=4). No
/// other writer or hardware walker mutates it during this bounded traversal.
pub unsafe fn clone_supervisor(
    memory: &mut impl TableMemory,
    source: u64,
    level: u8,
) -> Option<u64> {
    if !(1..=4).contains(&level) {
        return None;
    }
    let destination = memory.allocate()?;
    for slot in 0..512 {
        let entry = memory.read(source, slot);
        if entry & PRESENT == 0 {
            continue;
        }
        if level == 1 || (level <= 3 && entry & HUGE != 0) {
            memory.write(destination, slot, entry & !USER);
        } else {
            let Some(child) = clone_supervisor(memory, entry & ADDRESS_MASK, level - 1) else {
                release_clone(memory, destination, level);
                return None;
            };
            memory.write(destination, slot, child | (entry & !ADDRESS_MASK & !USER));
        }
    }
    Some(destination)
}

/// # Safety
/// Root is a private tree returned or partially built by clone_supervisor.
/// Leaf frames are borrowed; every non-leaf child is a new table allocation.
pub unsafe fn release_clone(memory: &mut impl TableMemory, root: u64, level: u8) {
    if level > 1 {
        for slot in 0..512 {
            let entry = memory.read(root, slot);
            if entry & PRESENT != 0 && !(level <= 3 && entry & HUGE != 0) {
                release_clone(memory, entry & ADDRESS_MASK, level - 1);
            }
        }
    }
    memory.release(root);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, BTreeSet};
    struct Memory {
        tables: BTreeMap<u64, [u64; 512]>,
        owned: BTreeSet<u64>,
        next: u64,
        budget: usize,
    }
    // SAFETY: BTreeMap owns each table independently and bounds-checks access.
    unsafe impl TableMemory for Memory {
        fn allocate(&mut self) -> Option<u64> {
            if self.budget == 0 {
                return None;
            }
            self.budget -= 1;
            self.next += 0x1000;
            self.tables.insert(self.next, [0; 512]);
            self.owned.insert(self.next);
            Some(self.next)
        }
        unsafe fn read(&self, table: u64, slot: usize) -> u64 {
            self.tables[&table][slot]
        }
        unsafe fn write(&mut self, table: u64, slot: usize, entry: u64) {
            self.tables.get_mut(&table).unwrap()[slot] = entry;
        }
        fn release(&mut self, table: u64) {
            assert!(self.owned.remove(&table));
            self.tables.remove(&table).unwrap();
        }
    }
    fn source() -> Memory {
        let mut tables = BTreeMap::new();
        for id in 1..=7 {
            tables.insert(id * 0x1000, [0; 512]);
        }
        tables.get_mut(&0x1000).unwrap()[0] = 0x2007;
        tables.get_mut(&0x1000).unwrap()[1] = 0x3007;
        tables.get_mut(&0x2000).unwrap()[0] = 0x4007;
        tables.get_mut(&0x3000).unwrap()[0] = 0x5007;
        tables.get_mut(&0x4000).unwrap()[0] = 0x6007;
        tables.get_mut(&0x5000).unwrap()[0] = 0x7007;
        tables.get_mut(&0x6000).unwrap()[0] = 0x1234007;
        tables.get_mut(&0x7000).unwrap()[0] = 0x5678007;
        tables.get_mut(&0x3000).unwrap()[5] = 0x80000087; // borrowed 1 GiB leaf
        Memory {
            tables,
            owned: BTreeSet::new(),
            next: 0x100000,
            budget: 0,
        }
    }
    #[test]
    fn every_allocation_failure_restores_the_exact_source_and_frame_set() {
        for budget in 0..7 {
            let mut memory = source();
            memory.budget = budget;
            let baseline = memory.tables.clone();
            // SAFETY: fixture is a fully initialized four-level tree.
            assert!(unsafe { clone_supervisor(&mut memory, 0x1000, 4) }.is_none());
            assert!(memory.owned.is_empty());
            assert_eq!(memory.tables, baseline);
        }
    }
    #[test]
    fn success_removes_user_bits_without_owning_or_freeing_leaf_frames() {
        let mut memory = source();
        memory.budget = 7;
        let baseline = memory.tables.clone();
        // SAFETY: valid fixture tree; returned clone remains exclusively owned.
        unsafe {
            let root = clone_supervisor(&mut memory, 0x1000, 4).unwrap();
            assert_eq!(memory.owned.len(), 7);
            for table in &memory.owned {
                assert!(memory.tables[table].iter().all(|entry| entry & USER == 0));
            }
            release_clone(&mut memory, root, 4);
        }
        assert_eq!(memory.tables, baseline);
        assert!(memory.owned.is_empty());
    }
}
