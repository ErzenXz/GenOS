use core::arch::asm;

use crate::memory::{self, Grant, Kind, Owner};

const ENTRY_COUNT: usize = 512;
const PRESENT: u64 = 1 << 0;
const WRITABLE: u64 = 1 << 1;
const USER: u64 = 1 << 2;
const HUGE_OR_PAT: u64 = 1 << 7;
const NO_EXECUTE: u64 = 1 << 63;
const TABLE_ADDRESS_MASK: u64 = 0x000f_ffff_ffff_f000;
const PAGE_2M_ADDRESS_MASK: u64 = 0x000f_ffff_ffe0_0000;
const PAGE_1G_ADDRESS_MASK: u64 = 0x000f_ffff_c000_0000;
pub const PAGE_SIZE: u64 = 4096;

pub const USER_BASE: u64 = 0x0000_4000_0000_0000;
pub const USER_CODE: u64 = USER_BASE + 0x1000;
pub const USER_DATA: u64 = USER_BASE + 0x2000;
pub const USER_STACK_GUARD: u64 = USER_BASE + 0xb000;
pub const USER_STACK_BOTTOM: u64 = USER_BASE + 0xc000;
pub const USER_STACK_PAGES: usize = 4;
pub const USER_STACK_TOP: u64 = USER_STACK_BOTTOM + USER_STACK_PAGES as u64 * PAGE_SIZE;
const USER_PML4_INDEX: usize = 128;

static mut KERNEL_ROOT: u64 = 0;
static mut ACTIVE_ROOT: u64 = 0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AddressSpace {
    root: u64,
    authority: Grant,
    owner: Owner,
}

impl AddressSpace {
    pub const fn root(self) -> u64 {
        self.root
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PagingError {
    OutOfMemory,
    MissingMapping,
    InvalidAddress,
    AddressInUse,
    ActiveAddressSpace,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AddressSpaceSwitchBenchmark {
    pub samples: u32,
    pub min_pair_cycles: u64,
    pub average_pair_cycles: u64,
}

pub fn init_protected_address_space() -> Result<(), PagingError> {
    // This bounded BSP implementation retires translations by CR3 reload. Do
    // not inherit firmware PCIDs/global retention into the ownership contract.
    let mut cr4: u64;
    // SAFETY: CPL0 bootstrap owns the MMU with IF clear. If PCID was enabled,
    // select PCID0 before clearing PCIDE; clearing PGE/PCIDE invalidates retained
    // translations. The subsequent root switch flushes ordinary translations.
    unsafe {
        asm!("mov {}, cr4", out(reg) cr4, options(nostack, preserves_flags));
        if cr4 & (1 << 17) != 0 {
            write_cr3(read_cr3() & TABLE_ADDRESS_MASK);
        }
        cr4 &= !((1 << 7) | (1 << 17));
        asm!("mov cr4, {}", in(reg) cr4, options(nostack, preserves_flags));
        asm!("mov {}, cr4", out(reg) cr4, options(nostack, preserves_flags));
    }
    if cr4 & ((1 << 7) | (1 << 17)) != 0 {
        return Err(PagingError::InvalidAddress);
    }
    crate::serial::println("CPU_TLB_POLICY_READY pcid=off global=off");
    let current = read_cr3() & TABLE_ADDRESS_MASK;
    if current == 0 {
        return Err(PagingError::MissingMapping);
    }
    crate::serial::print("PAGING_CLONE_BEGIN root=0x");
    crate::serial::print_hex(current);
    crate::serial::println("");

    // The firmware map is identity mapped. The clone removes every inherited user bit,
    // producing the supervisor-only template shared by later process roots.
    let allocated_before = memory::allocated_frames();
    let protected = unsafe { clone_table(current, 4)? };
    let table_frames = memory::allocated_frames().saturating_sub(allocated_before);
    unsafe {
        write_cr3(protected);
        core::ptr::addr_of_mut!(KERNEL_ROOT).write(protected);
        core::ptr::addr_of_mut!(ACTIVE_ROOT).write(protected);
    }
    crate::serial::print("PAGING_READY root=0x");
    crate::serial::print_hex(protected);
    crate::serial::print(" tables=");
    crate::serial::print_u64(table_frames);
    crate::serial::println("");
    Ok(())
}

pub fn create_user_address_space() -> Result<AddressSpace, PagingError> {
    let kernel_root = unsafe { *core::ptr::addr_of!(KERNEL_ROOT) };
    if kernel_root == 0 {
        return Err(PagingError::MissingMapping);
    }
    let owner = memory::new_owner().ok_or(PagingError::OutOfMemory)?;
    let authority = memory::alloc_frame(owner, Kind::Table).ok_or(PagingError::OutOfMemory)?;
    let root = authority.address();
    unsafe {
        let source = table(kernel_root);
        let destination = table_mut(root);
        destination.copy_from_slice(source);
        if destination[USER_PML4_INDEX] & PRESENT != 0 {
            let _ = memory::free_frame(authority);
            return Err(PagingError::AddressInUse);
        }
    }
    Ok(AddressSpace {
        root,
        authority,
        owner,
    })
}

pub fn map_user_page(
    space: AddressSpace,
    virtual_address: u64,
    grant: Grant,
    writable: bool,
    executable: bool,
) -> Result<(), PagingError> {
    crate::arch::without_interrupts(|| {
        map_user_page_inner(space, virtual_address, grant, writable, executable)
    })
}

fn map_user_page_inner(
    space: AddressSpace,
    virtual_address: u64,
    grant: Grant,
    writable: bool,
    executable: bool,
) -> Result<(), PagingError> {
    if active_root() == space.root {
        return Err(PagingError::ActiveAddressSpace);
    }
    let physical_address = grant.address();
    if !memory::is_live(space.authority) || !memory::can_map(grant, space.owner) {
        return Err(PagingError::InvalidAddress);
    }
    if virtual_address & (PAGE_SIZE - 1) != 0
        || physical_address & (PAGE_SIZE - 1) != 0
        || physical_address == 0
        || physical_address & !TABLE_ADDRESS_MASK != 0
        || (writable && executable)
        || !(USER_BASE..USER_BASE + (1 << 39)).contains(&virtual_address)
    {
        return Err(PagingError::InvalidAddress);
    }

    let mut flags = PRESENT | USER;
    if writable {
        flags |= WRITABLE;
    }
    if !executable {
        if !nx_enabled() {
            return Err(PagingError::InvalidAddress);
        }
        flags |= NO_EXECUTE;
    }
    let mut created = [(core::ptr::null_mut::<u64>(), 0u64); 3];
    let mut count = 0;
    // SAFETY: this private process root is owned by the caller. Allocation
    // records every new parent edge so failure can revoke it before freeing.
    let result = unsafe {
        (|| {
            let mut current = space.root;
            for shift in [39, 30, 21] {
                let entry = &mut table_mut(current)[index(virtual_address, shift)];
                let empty = *entry & PRESENT == 0;
                let child = ensure_user_table(entry, space.owner)?;
                if empty {
                    created[count] = (entry as *mut u64, child);
                    count += 1;
                }
                current = child;
            }
            let pte = &mut table_mut(current)[index(virtual_address, 12)];
            if *pte & PRESENT != 0 {
                return Err(PagingError::AddressInUse);
            }
            if !memory::pin_mapping(grant, space.owner, virtual_address) {
                return Err(PagingError::InvalidAddress);
            }
            *pte = physical_address | flags;
            Ok(())
        })()
    };
    if result.is_err() {
        for &(parent, child) in created[..count].iter().rev() {
            // SAFETY: children are unpublished in reverse order, while each
            // parent still exists. No failed transaction published a leaf.
            unsafe {
                parent.write(0);
            }
            assert!(
                // SAFETY: rollback removed this new parent edge; the unpublished
                // child is exclusively owned by this mapping transaction.
                unsafe { release_owned_table(space.owner, child) },
                "user-table rollback lost ownership"
            );
        }
    } else {
        // SAFETY: invalidate only the mapping just published by this caller.
        unsafe {
            asm!("invlpg [{page}]", page = in(reg) virtual_address, options(nostack, preserves_flags));
        }
    }
    result
}

/// Seal a supervisor mapping, splitting firmware huge pages without changing
/// the neighboring mappings. Bootstrap-only: no process root may be active.
pub fn protect_kernel_page(
    address: u64,
    writable: bool,
    executable: bool,
) -> Result<(), PagingError> {
    let root = unsafe { *core::ptr::addr_of!(KERNEL_ROOT) };
    if root == 0
        || active_root() != root
        || address & (PAGE_SIZE - 1) != 0
        || !(PAGE_SIZE..USER_BASE).contains(&address)
        || (writable && executable)
        || !nx_enabled()
    {
        return Err(PagingError::InvalidAddress);
    }
    // SAFETY: BSP initialization owns these supervisor tables with IF clear;
    // every child table is populated before its parent entry is published.
    unsafe {
        let mut current = root;
        for shift in [39, 30, 21] {
            let entry = &mut table_mut(current)[index(address, shift)];
            if *entry & PRESENT == 0 || *entry & USER != 0 {
                return Err(PagingError::MissingMapping);
            }
            // This API only tightens effective permissions. Relaxing a leaf
            // cannot override an ancestor restriction (including one copied
            // from a huge mapping), so reject such requests explicitly.
            if (writable && *entry & WRITABLE == 0) || (executable && *entry & NO_EXECUTE != 0) {
                return Err(PagingError::InvalidAddress);
            }
            if *entry & HUGE_OR_PAT != 0 {
                if shift == 39 {
                    return Err(PagingError::InvalidAddress);
                }
                let child = allocate_table()?;
                let mask = if shift == 30 {
                    PAGE_1G_ADDRESS_MASK
                } else {
                    PAGE_2M_ADDRESS_MASK
                };
                let base = *entry & mask;
                let mut flags = *entry & !mask;
                let step = if shift == 30 { 1 << 21 } else { PAGE_SIZE };
                if shift == 21 {
                    let pat = flags & (1 << 12) != 0;
                    flags &= !(HUGE_OR_PAT | (1 << 12));
                    if pat {
                        flags |= 1 << 7;
                    }
                }
                for (slot, leaf) in table_mut(child).iter_mut().enumerate() {
                    *leaf = (base + slot as u64 * step) | flags;
                }
                // Leaf-only PAT/dirty/global attributes must not become
                // table-address bits or reserved bits in the parent.
                *entry = child | (*entry & (0x3f | NO_EXECUTE));
            }
            current = *entry & TABLE_ADDRESS_MASK;
        }
        let entry = &mut table_mut(current)[index(address, 12)];
        if *entry & PRESENT == 0 || *entry & USER != 0 {
            return Err(PagingError::MissingMapping);
        }
        if (writable && *entry & WRITABLE == 0) || (executable && *entry & NO_EXECUTE != 0) {
            return Err(PagingError::InvalidAddress);
        }
        *entry &= !(WRITABLE | NO_EXECUTE);
        if writable {
            *entry |= WRITABLE;
        }
        if !executable {
            *entry |= NO_EXECUTE;
        }
        asm!("invlpg [{}]", in(reg) address, options(nostack, preserves_flags));
    }
    Ok(())
}

/// Remove an already split, supervisor-only identity leaf without releasing
/// its reserved backing storage. All process roots inherit these guard holes.
///
/// # Safety
/// BSP bootstrap, with IF clear and the protected kernel root active, must own
/// this dedicated guard page in the reserved kernel data image. It must contain
/// no live object, current stack bytes, page table or device storage. No process
/// root may have been published. Failure is boot-fatal; no guard is remapped.
pub unsafe fn guard_kernel_page(address: u64) -> Result<(), PagingError> {
    unsafe extern "C" {
        static __kernel_data_start: u8;
        static __kernel_data_end: u8;
    }
    let root = *core::ptr::addr_of!(KERNEL_ROOT);
    let data_start = core::ptr::addr_of!(__kernel_data_start) as u64;
    let data_end = core::ptr::addr_of!(__kernel_data_end) as u64;
    if root == 0
        || active_root() != root
        || crate::arch::interrupts_enabled()
        || !address.is_multiple_of(PAGE_SIZE)
        || !(data_start..data_end).contains(&address)
    {
        return Err(PagingError::InvalidAddress);
    }
    let mut current = root;
    for shift in [39, 30, 21] {
        let entry = table(current)[index(address, shift)];
        // protect_kernel_image has already split every data page. This helper
        // neither allocates tables nor changes neighboring mappings.
        if entry & PRESENT == 0 || entry & (USER | HUGE_OR_PAT) != 0 {
            return Err(PagingError::MissingMapping);
        }
        current = entry & TABLE_ADDRESS_MASK;
    }
    let entry = &mut table_mut(current)[index(address, 12)];
    if *entry & PRESENT == 0
        || *entry & USER != 0
        || *entry & TABLE_ADDRESS_MASK != address
        || *entry & NO_EXECUTE == 0
    {
        return Err(PagingError::InvalidAddress);
    }
    *entry = 0;
    // SAFETY: the caller's bootstrap ownership has retired this dedicated
    // guard mapping; invalidate the BSP's old translation before verification.
    asm!("invlpg [{}]", in(reg) address, options(nostack, preserves_flags));
    if translate_root(root, address).is_some() {
        return Err(PagingError::InvalidAddress);
    }
    Ok(())
}

pub fn protect_kernel_image() -> Result<(), PagingError> {
    unsafe extern "C" {
        static __kernel_text_start: u8;
        static __kernel_text_end: u8;
        static __kernel_rodata_start: u8;
        static __kernel_rodata_end: u8;
        static __kernel_data_start: u8;
        static __kernel_data_end: u8;
    }
    for (start, end, writable, executable) in [
        (
            core::ptr::addr_of!(__kernel_text_start) as u64,
            core::ptr::addr_of!(__kernel_text_end) as u64,
            false,
            true,
        ),
        (
            core::ptr::addr_of!(__kernel_rodata_start) as u64,
            core::ptr::addr_of!(__kernel_rodata_end) as u64,
            false,
            false,
        ),
        (
            core::ptr::addr_of!(__kernel_data_start) as u64,
            core::ptr::addr_of!(__kernel_data_end) as u64,
            true,
            false,
        ),
    ] {
        if start >= end || end & (PAGE_SIZE - 1) != 0 {
            return Err(PagingError::InvalidAddress);
        }
        for address in (start..end).step_by(PAGE_SIZE as usize) {
            protect_kernel_page(address, writable, executable)?;
        }
    }
    crate::serial::println("KERNEL_IMAGE_PROTECTED text=rx rodata=r data=rw-nx");
    Ok(())
}

pub fn activate(space: AddressSpace) {
    assert!(
        memory::is_live(space.authority),
        "stale address-space activation"
    );
    unsafe {
        write_cr3(space.root);
        core::ptr::addr_of_mut!(ACTIVE_ROOT).write(space.root);
    }
}

pub fn activate_kernel() {
    let root = unsafe { *core::ptr::addr_of!(KERNEL_ROOT) };
    if root != 0 {
        unsafe {
            write_cr3(root);
            core::ptr::addr_of_mut!(ACTIVE_ROOT).write(root);
        }
    }
}

pub fn active_root() -> u64 {
    unsafe { *core::ptr::addr_of!(ACTIVE_ROOT) }
}

pub fn benchmark_address_space_switch(
    space: AddressSpace,
    samples: u32,
) -> Option<AddressSpaceSwitchBenchmark> {
    if samples == 0 || unsafe { *core::ptr::addr_of!(KERNEL_ROOT) } == 0 {
        return None;
    }

    let interrupts_were_enabled = crate::arch::interrupts_enabled();
    crate::arch::disable_interrupts();
    activate_kernel();

    let mut total_cycles = 0u64;
    let mut min_pair_cycles = u64::MAX;
    for _ in 0..samples {
        let started = crate::arch::timestamp_cycles();
        activate(space);
        activate_kernel();
        let elapsed = crate::arch::timestamp_cycles().saturating_sub(started);
        total_cycles = total_cycles.saturating_add(elapsed);
        min_pair_cycles = min_pair_cycles.min(elapsed);
    }

    if interrupts_were_enabled {
        crate::arch::enable_interrupts();
    }

    Some(AddressSpaceSwitchBenchmark {
        samples,
        min_pair_cycles,
        average_pair_cycles: total_cycles / u64::from(samples),
    })
}

pub fn translate(space: AddressSpace, virtual_address: u64) -> Option<u64> {
    if !memory::is_live(space.authority) {
        return None;
    }
    translate_root(space.root, virtual_address)
}

fn translate_root(root: u64, virtual_address: u64) -> Option<u64> {
    unsafe {
        let pml4e = table(root)[index(virtual_address, 39)];
        if pml4e & PRESENT == 0 {
            return None;
        }
        let pdpte = table(pml4e & TABLE_ADDRESS_MASK)[index(virtual_address, 30)];
        if pdpte & PRESENT == 0 {
            return None;
        }
        if pdpte & HUGE_OR_PAT != 0 {
            return Some((pdpte & PAGE_1G_ADDRESS_MASK) + (virtual_address & ((1 << 30) - 1)));
        }
        let pde = table(pdpte & TABLE_ADDRESS_MASK)[index(virtual_address, 21)];
        if pde & PRESENT == 0 {
            return None;
        }
        if pde & HUGE_OR_PAT != 0 {
            return Some((pde & PAGE_2M_ADDRESS_MASK) + (virtual_address & ((1 << 21) - 1)));
        }
        let pte = table(pde & TABLE_ADDRESS_MASK)[index(virtual_address, 12)];
        if pte & PRESENT == 0 {
            return None;
        }
        Some((pte & TABLE_ADDRESS_MASK) + (virtual_address & (PAGE_SIZE - 1)))
    }
}

pub fn allocate_zeroed_frame(space: AddressSpace) -> Result<Grant, PagingError> {
    if !memory::is_live(space.authority) {
        return Err(PagingError::InvalidAddress);
    }
    memory::alloc_frame(space.owner, Kind::User).ok_or(PagingError::OutOfMemory)
}

pub fn destroy_user_address_space(space: AddressSpace) -> Result<u64, PagingError> {
    crate::arch::without_interrupts(|| {
        if !memory::is_live(space.authority) {
            return Err(PagingError::InvalidAddress);
        }
        if active_root() == space.root {
            return Err(PagingError::ActiveAddressSpace);
        }
        // SAFETY: the live root token identifies this private owner. This BSP
        // has switched away (CR3 flushes nonglobal user translations; PCID is
        // not enabled). Validate the complete tree before any mutation, then
        // unlink each edge before scrubbing/releasing its backing grant.
        unsafe {
            let entry = table(space.root)[USER_PML4_INDEX];
            let mut released = 0;
            if entry & PRESENT != 0 {
                if entry & HUGE_OR_PAT != 0 || entry & USER == 0 {
                    return Err(PagingError::InvalidAddress);
                }
                let user_root = entry & TABLE_ADDRESS_MASK;
                let tree_frames = validate_user_table(space.owner, user_root, 3, USER_BASE)?;
                if memory::owner_frames(space.owner) != tree_frames + 1 {
                    return Err(PagingError::InvalidAddress);
                }
                table_mut(space.root)[USER_PML4_INDEX] = 0;
                released += release_user_table(space.owner, user_root, 3, USER_BASE);
                assert!(
                    release_owned_table(space.owner, user_root),
                    "validated user root ownership"
                );
                released += 1;
            } else if memory::owner_frames(space.owner) != 1 {
                return Err(PagingError::InvalidAddress);
            }
            assert!(
                memory::free_frame(space.authority),
                "validated address-space ownership"
            );
            assert_eq!(
                memory::owner_frames(space.owner),
                0,
                "unpublished user grant leaked"
            );
            Ok(released + 1)
        }
    })
}

struct PhysicalTables;
// SAFETY: the BSP owns cloned tables exclusively; callers validate source
// roots. Frames are identity-mapped, zeroed before publication, and allocated
// by the ownership bitmap. No leaf frame is released by this adapter.
unsafe impl kernel::page_table::TableMemory for PhysicalTables {
    fn allocate(&mut self) -> Option<u64> {
        unsafe { allocate_table().ok() }
    }
    unsafe fn read(&self, root: u64, slot: usize) -> u64 {
        // Hardware may update accessed/dirty flags in the active source tree.
        // Do not create an immutable Rust reference spanning those updates.
        core::ptr::read_volatile((root as *const u64).add(slot))
    }
    unsafe fn write(&mut self, root: u64, slot: usize, entry: u64) {
        core::ptr::write_volatile((root as *mut u64).add(slot), entry);
    }
    fn release(&mut self, frame: u64) {
        assert!(
            // SAFETY: clone rollback passes only new, unpublished table grants.
            unsafe { release_owned_table(Owner::KERNEL, frame) },
            "page-table rollback lost ownership"
        );
    }
}

unsafe fn clone_table(source_phys: u64, level: u8) -> Result<u64, PagingError> {
    kernel::page_table::clone_supervisor(&mut PhysicalTables, source_phys, level)
        .ok_or(PagingError::OutOfMemory)
}

unsafe fn ensure_user_table(entry: &mut u64, owner: Owner) -> Result<u64, PagingError> {
    if *entry & PRESENT == 0 {
        let table_phys = memory::alloc_frame(owner, Kind::Table)
            .ok_or(PagingError::OutOfMemory)?
            .address();
        *entry = table_phys | PRESENT | WRITABLE | USER;
    } else if *entry & HUGE_OR_PAT != 0
        || *entry & USER == 0
        || *entry & WRITABLE == 0
        || *entry & NO_EXECUTE != 0
    {
        return Err(PagingError::AddressInUse);
    }
    let child = *entry & TABLE_ADDRESS_MASK;
    if memory::find_grant(owner, child, Kind::Table).is_none() {
        return Err(PagingError::InvalidAddress);
    }
    Ok(child)
}

unsafe fn allocate_table() -> Result<u64, PagingError> {
    memory::alloc_frame(Owner::KERNEL, Kind::Table)
        .map(Grant::address)
        .ok_or(PagingError::OutOfMemory)
}

/// Caller owns the table identity and has removed every published parent edge.
unsafe fn release_owned_table(owner: Owner, frame: u64) -> bool {
    let Some(grant) = memory::find_grant(owner, frame, Kind::Table) else {
        return false;
    };
    memory::free_frame(grant)
}

unsafe fn validate_user_table(
    owner: Owner,
    physical: u64,
    level: u8,
    base: u64,
) -> Result<usize, PagingError> {
    if memory::find_grant(owner, physical, Kind::Table).is_none() {
        return Err(PagingError::InvalidAddress);
    }
    let mut frames = 1;
    for (slot, &entry) in table(physical).iter().enumerate() {
        if entry & PRESENT == 0 {
            continue;
        }
        if entry & USER == 0 || (level > 1 && entry & HUGE_OR_PAT != 0) {
            return Err(PagingError::InvalidAddress);
        }
        let frame = entry & TABLE_ADDRESS_MASK;
        let address = base + ((slot as u64) << (12 + 9 * (level - 1)));
        if level == 1 {
            if !memory::mapping_matches(owner, frame, address) {
                return Err(PagingError::InvalidAddress);
            }
            frames += 1;
        } else {
            frames += validate_user_table(owner, frame, level - 1, address)?;
        }
    }
    Ok(frames)
}

unsafe fn release_user_table(owner: Owner, physical: u64, level: u8, base: u64) -> u64 {
    let mut released = 0;
    for slot in 0..ENTRY_COUNT {
        let entry = table(physical)[slot];
        if entry & PRESENT == 0 {
            continue;
        }
        let frame = entry & TABLE_ADDRESS_MASK;
        let address = base + ((slot as u64) << (12 + 9 * (level - 1)));
        table_mut(physical)[slot] = 0;
        if level == 1 {
            let grant =
                memory::retire_mapping(owner, frame, address).expect("validated mapping pin");
            assert!(memory::free_frame(grant), "validated leaf ownership");
        } else {
            released += release_user_table(owner, frame, level - 1, address);
            assert!(
                release_owned_table(owner, frame),
                "validated table ownership"
            );
        }
        released += 1;
    }
    released
}

unsafe fn table(physical: u64) -> &'static [u64; ENTRY_COUNT] {
    &*(physical as *const [u64; ENTRY_COUNT])
}

unsafe fn table_mut(physical: u64) -> &'static mut [u64; ENTRY_COUNT] {
    &mut *(physical as *mut [u64; ENTRY_COUNT])
}

unsafe fn write_cr3(root: u64) {
    asm!("mov cr3, {root}", root = in(reg) root, options(nostack, preserves_flags));
}

fn read_cr3() -> u64 {
    let value: u64;
    unsafe { asm!("mov {}, cr3", out(reg) value, options(nomem, nostack, preserves_flags)) };
    value
}

fn nx_enabled() -> bool {
    let low: u32;
    let high: u32;
    unsafe {
        asm!(
            "rdmsr",
            in("ecx") 0xc000_0080u32,
            out("eax") low,
            out("edx") high,
            options(nomem, nostack, preserves_flags),
        )
    };
    ((u64::from(high) << 32) | u64::from(low)) & (1 << 11) != 0
}

const fn index(address: u64, shift: u8) -> usize {
    ((address >> shift) & 0x1ff) as usize
}

/// Exercise the production allocator and mapper, including denial before any
/// scrubbing, stale root reuse, active-root refusal and exact owner cleanup.
#[cfg(feature = "memory-test-faults")]
pub fn run_ownership_probe() -> bool {
    crate::arch::without_interrupts(|| {
        let baseline = memory::allocated_frames();
        let Ok(first) = create_user_address_space() else {
            return false;
        };
        let Ok(other) = create_user_address_space() else {
            return false;
        };
        let Ok(grant) = allocate_zeroed_frame(first) else {
            return false;
        };
        let frame = grant.address();
        // SAFETY: this unpublished grant is owned solely by the probe; its
        // identity map spans exactly one page, with local IRQs masked.
        unsafe {
            core::ptr::write_bytes(frame as *mut u8, 0xa5, PAGE_SIZE as usize);
        }
        let before_denial = memory::allocated_frames();
        let pending_denied =
            destroy_user_address_space(first).is_err() && memory::is_live(first.authority);
        let foreign = map_user_page(other, USER_DATA, grant, true, false).is_err();
        let noncanonical = map_user_page(first, USER_DATA | (1 << 63), grant, true, false).is_err();
        if !pending_denied
            || !foreign
            || !noncanonical
            || memory::allocated_frames() != before_denial
        {
            return false;
        }
        if map_user_page(first, USER_DATA, grant, true, false).is_err() {
            return false;
        }
        let before_alias = memory::allocated_frames();
        let alias = map_user_page(first, USER_DATA + PAGE_SIZE, grant, true, false).is_err();
        // SAFETY: deliberately retrying a pinned token must fail before touching
        // memory. The page remains mapped; no successful release is permitted.
        let pinned = unsafe { !memory::free_frame(grant) };
        // SAFETY: the validated identity grant is still live and held by this
        // probe. Volatile reads verify a denied release did not erase its bytes.
        let intact = unsafe {
            (0..PAGE_SIZE)
                .all(|offset| core::ptr::read_volatile((frame + offset) as *const u8) == 0xa5)
        };
        let mappings =
            translate(first, USER_DATA) == Some(frame) && translate(other, USER_DATA).is_none();
        activate(first);
        let active_map_denied = map_user_page(first, USER_DATA + PAGE_SIZE, grant, true, false)
            == Err(PagingError::ActiveAddressSpace);
        let active_denied =
            destroy_user_address_space(first) == Err(PagingError::ActiveAddressSpace);
        activate_kernel();
        if !alias
            || !pinned
            || !intact
            || !mappings
            || !active_denied
            || !active_map_denied
            || memory::allocated_frames() != before_alias
        {
            return false;
        }
        if destroy_user_address_space(first).is_err() || destroy_user_address_space(other).is_err()
        {
            return false;
        }
        let Ok(reused) = create_user_address_space() else {
            return false;
        };
        let stale = allocate_zeroed_frame(first).is_err()
            && translate(first, USER_DATA).is_none()
            && destroy_user_address_space(first).is_err()
            // SAFETY: a retired generation cannot authorize access even when
            // the physical allocator has reused the root/leaf address.
            && unsafe { !memory::free_frame(grant) };
        if destroy_user_address_space(reused).is_err() {
            return false;
        }
        stale && memory::allocated_frames() == baseline && memory::snapshot().1
    })
}
