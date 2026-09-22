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
static mut KERNEL_MAPPINGS_SEALED: bool = false;

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
    // SAFETY: loader/firmware provide the live identity-mapped table tree used
    // by this CPU. Reject additional aliases before allocating a private clone.
    if !unsafe { kernel::page_table::identity_mappings(&PhysicalTables, current) } {
        return Err(PagingError::InvalidAddress);
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
    if kernel_root == 0 || !unsafe { *core::ptr::addr_of!(KERNEL_MAPPINGS_SEALED) } {
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
            // Seal the only kernel alias before publishing user authority.
            // After this point all admission checks have passed under one
            // critical section; pin publication cannot recoverably fail.
            set_user_alias_writable(physical_address, writable)?;
            assert!(memory::pin_mapping(grant, space.owner, virtual_address));
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
        || unsafe { *core::ptr::addr_of!(KERNEL_MAPPINGS_SEALED) }
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

/// Finish the bootstrap-only supervisor map. Thereafter no public API may
/// change reserved/firmware/kernel permissions or create another physical alias.
/// All allocator RAM has a 4-KiB NX identity leaf; every other retained leaf is
/// NX except the read-only linked kernel text. User RO/RX publication can then
/// tighten its one supervisor alias without allocating or changing neighbors.
pub fn seal_kernel_mappings(boot_info: &genos_abi::BootInfo) -> Result<(), PagingError> {
    if crate::arch::interrupts_enabled() || unsafe { *core::ptr::addr_of!(KERNEL_MAPPINGS_SEALED) }
    {
        return Err(PagingError::InvalidAddress);
    }
    let regions = genos_abi::boot_memory::validate_map(&boot_info.memory_map)
        .map_err(|_| PagingError::InvalidAddress)?;
    for region in regions {
        if region.kind != genos_abi::MemoryRegionKind::Usable {
            continue;
        }
        let mut address = (region.start.max(PAGE_SIZE) + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
        let end = (region.start + region.size) & !(PAGE_SIZE - 1);
        while address < end {
            protect_kernel_page(address, true, false)?;
            address = (address | ((1 << 21) - 1)) + 1;
        }
    }
    unsafe extern "C" {
        static __kernel_text_start: u8;
        static __kernel_text_end: u8;
    }
    let text_start = core::ptr::addr_of!(__kernel_text_start) as u64;
    let text_end = core::ptr::addr_of!(__kernel_text_end) as u64;
    // SAFETY: BSP owns the fully cloned identity-only supervisor tree before
    // process publication. Restriction is global across its future shared roots.
    unsafe fn seal_table(physical: u64, level: u8, text_start: u64, text_end: u64) -> bool {
        for slot in 0..ENTRY_COUNT {
            let entry = table(physical)[slot];
            if entry & PRESENT == 0 {
                continue;
            }
            if level == 1 || entry & HUGE_OR_PAT != 0 {
                let shift = 12 + 9 * (level - 1);
                let frame = entry & TABLE_ADDRESS_MASK & !((1u64 << shift) - 1);
                let end = frame + (1u64 << shift);
                if frame < text_end && text_start < end {
                    if level != 1 || frame < text_start || end > text_end || entry & WRITABLE != 0 {
                        return false;
                    }
                } else {
                    table_mut(physical)[slot] |= NO_EXECUTE;
                }
            } else if !seal_table(entry & TABLE_ADDRESS_MASK, level - 1, text_start, text_end) {
                return false;
            }
        }
        true
    }
    // SAFETY: IF clear, kernel root active and no user root exists; the CR3
    // reload retires all old non-global permissions before runtime admission.
    unsafe {
        let root = *core::ptr::addr_of!(KERNEL_ROOT);
        if root == 0 || root != active_root() || !seal_table(root, 4, text_start, text_end) {
            return Err(PagingError::InvalidAddress);
        }
        write_cr3(root);
        core::ptr::addr_of_mut!(KERNEL_MAPPINGS_SEALED).write(true);
    }
    crate::serial::println("PHYSICAL_ALIAS_POLICY_READY identity=only user_alias=sealed direct=nx");
    Ok(())
}

/// Caller holds the BSP critical section and either has not yet published the
/// user leaf or has unlinked it from an inactive, retired address space.
fn set_user_alias_writable(frame: u64, writable: bool) -> Result<(), PagingError> {
    // SAFETY: all managed RAM was split and sealed before user construction;
    // shared supervisor tables remain live for the entire kernel lifetime.
    unsafe {
        if !*core::ptr::addr_of!(KERNEL_MAPPINGS_SEALED) || crate::arch::interrupts_enabled() {
            return Err(PagingError::InvalidAddress);
        }
        let mut current = *core::ptr::addr_of!(KERNEL_ROOT);
        for shift in [39, 30, 21] {
            let entry = table(current)[index(frame, shift)];
            if entry & PRESENT == 0 || entry & (USER | HUGE_OR_PAT) != 0 {
                return Err(PagingError::InvalidAddress);
            }
            current = entry & TABLE_ADDRESS_MASK;
        }
        let entry = &mut table_mut(current)[index(frame, 12)];
        if *entry & PRESENT == 0
            || *entry & USER != 0
            || *entry & NO_EXECUTE == 0
            || *entry & TABLE_ADDRESS_MASK != frame
        {
            return Err(PagingError::InvalidAddress);
        }
        if writable {
            *entry |= WRITABLE;
        } else {
            *entry &= !WRITABLE;
        }
        asm!("invlpg [{}]", in(reg) frame, options(nostack, preserves_flags));
    }
    Ok(())
}

pub fn activate(space: AddressSpace) {
    crate::arch::without_interrupts(|| {
        assert!(
            memory::is_live(space.authority),
            "stale address-space activation"
        );
        // SAFETY: the admitted BSP holds the live root authority; CR3 and its
        // bookkeeping change atomically with respect to local interrupts.
        unsafe {
            write_cr3(space.root);
            core::ptr::addr_of_mut!(ACTIVE_ROOT).write(space.root);
        }
    });
}

pub fn activate_kernel() {
    crate::arch::without_interrupts(|| {
        let root = unsafe { *core::ptr::addr_of!(KERNEL_ROOT) };
        if root != 0 {
            // SAFETY: kernel root outlives the machine. This CR3 reload retires
            // every previous private translation before teardown is admitted.
            unsafe {
                write_cr3(root);
                core::ptr::addr_of_mut!(ACTIVE_ROOT).write(root);
            }
        }
    });
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

/// Resolve only the exact owner's pinned 4-KiB user leaf, including effective
/// ancestor permissions. Never return a supervisor/huge/foreign mapping as user
/// memory. Called only with local IRQs masked for the complete copy lifetime.
fn user_page(space: AddressSpace, address: u64, write: bool) -> Option<u64> {
    if !memory::is_live(space.authority)
        || !address.is_multiple_of(PAGE_SIZE)
        || !(USER_BASE..USER_BASE + (1 << 39)).contains(&address)
    {
        return None;
    }
    // SAFETY: the live root grant and owner check protect every table lifetime.
    // The BSP excludes mapping/teardown/scheduling until the caller finishes;
    // each child grant is checked before dereference. No reference escapes.
    unsafe {
        let mut current = space.root;
        for shift in [39, 30, 21, 12] {
            memory::find_grant(space.owner, current, Kind::Table)?;
            let entry = table(current)[index(address, shift)];
            if entry & (PRESENT | USER) != PRESENT | USER
                || (write && entry & WRITABLE == 0)
                || (shift != 12 && entry & HUGE_OR_PAT != 0)
            {
                return None;
            }
            current = entry & TABLE_ADDRESS_MASK;
        }
        memory::mapping_matches(space.owner, current, address).then_some(current)
    }
}

pub fn valid_user_range(space: AddressSpace, address: u64, length: usize, write: bool) -> bool {
    crate::arch::without_interrupts(|| {
        memory::is_live(space.authority)
            && kernel::user_copy::Plan::new(address, length)
                .and_then(|plan| plan.resolve(|page| user_page(space, page, write)))
                .is_some()
    })
}

pub fn copy_from_user(space: AddressSpace, address: u64, output: &mut [u8]) -> bool {
    crate::arch::without_interrupts(|| {
        let Some(plan) = kernel::user_copy::Plan::new(address, output.len()) else {
            return false;
        };
        if !memory::is_live(space.authority) {
            return false;
        }
        let Some(physical) = plan.resolve(|page| user_page(space, page, false)) else {
            return false;
        };
        let mut offset = 0;
        for (chunk, physical) in plan.chunks().iter().zip(physical) {
            for (index, byte) in output[offset..offset + chunk.length].iter_mut().enumerate() {
                // SAFETY: the complete plan was validated under the same BSP
                // critical section. This supervisor alias belongs to a live,
                // pinned user grant; no teardown, remap or user runs until exit.
                *byte = unsafe { core::ptr::read_volatile((physical + index as u64) as *const u8) };
            }
            offset += chunk.length;
        }
        true
    })
}

pub fn copy_to_user(space: AddressSpace, address: u64, input: &[u8]) -> bool {
    crate::arch::without_interrupts(|| {
        let Some(plan) = kernel::user_copy::Plan::new(address, input.len()) else {
            return false;
        };
        if !memory::is_live(space.authority) {
            return false;
        }
        let Some(physical) = plan.resolve(|page| user_page(space, page, true)) else {
            return false;
        };
        let mut offset = 0;
        for (chunk, physical) in plan.chunks().iter().zip(physical) {
            for (index, &byte) in input[offset..offset + chunk.length].iter().enumerate() {
                // SAFETY: every destination page and its effective writable
                // authority was checked before the first write. The entire
                // copy holds the BSP critical section; no raw user borrow lives.
                unsafe { core::ptr::write_volatile((physical + index as u64) as *mut u8, byte) };
            }
            offset += chunk.length;
        }
        true
    })
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
            // The owning root is inactive and its prior CR3 was retired. No
            // executable/read-only user translation survives this unlink.
            // Restore write access only now, so scrub/reuse cannot defeat it.
            set_user_alias_writable(frame, true).expect("validated supervisor alias");
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

#[cfg(feature = "memory-test-faults")]
pub fn run_pressure_probe() -> bool {
    crate::arch::without_interrupts(|| {
        let baseline = memory::allocated_frames();
        for _ in 0..2 {
            let Ok(space) = create_user_address_space() else {
                return false;
            };
            let mut grants = [None; kernel::frame_grant::USER_FRAME_LIMIT - 1];
            for grant in &mut grants {
                let Ok(frame) = allocate_zeroed_frame(space) else {
                    return false;
                };
                *grant = Some(frame);
            }
            let full = memory::allocated_frames();
            if allocate_zeroed_frame(space) != Err(PagingError::OutOfMemory)
                || memory::allocated_frames() != full
                || memory::owner_frames(space.owner) != kernel::frame_grant::USER_FRAME_LIMIT
            {
                return false;
            }
            let Ok(witness) = create_user_address_space() else {
                return false;
            };
            if destroy_user_address_space(witness) != Ok(1) {
                return false;
            }
            for grant in grants.into_iter().flatten() {
                // SAFETY: this probe retains every unpublished grant, with no
                // references, mappings or device users; IRQs remain masked.
                if !unsafe { memory::free_frame(grant) } {
                    return false;
                }
            }
            if destroy_user_address_space(space) != Ok(1) || memory::allocated_frames() != baseline
            {
                return false;
            }
        }
        true
    })
}

#[cfg(feature = "memory-test-faults")]
pub fn run_user_copy_probe() -> bool {
    crate::arch::without_interrupts(|| {
        let baseline = memory::allocated_frames();
        let Ok(space) = create_user_address_space() else {
            return false;
        };
        let Ok(other) = create_user_address_space() else {
            return false;
        };
        let Ok(data) = allocate_zeroed_frame(space) else {
            return false;
        };
        let Ok(readonly) = allocate_zeroed_frame(space) else {
            return false;
        };
        if map_user_page(space, USER_DATA, data, true, false).is_err() {
            return false;
        }
        let edge = USER_DATA + PAGE_SIZE - 1;
        if !copy_to_user(space, edge, &[0x31]) || copy_to_user(space, edge, &[0x41, 0x42]) {
            return false;
        }
        let mut byte = [0];
        if !copy_from_user(space, edge, &mut byte) || byte != [0x31] {
            return false;
        }
        if map_user_page(space, USER_DATA + PAGE_SIZE, readonly, false, false).is_err()
            || copy_to_user(space, edge, &[0x41, 0x42])
        {
            return false;
        }
        let mut pair = [0xff; 2];
        if !copy_from_user(space, edge, &mut pair)
            || pair != [0x31, 0]
            || copy_to_user(other, USER_DATA, &[7])
            || copy_to_user(space, readonly.address(), &[7])
            || copy_to_user(space, USER_DATA | (1 << 63), &[7])
        {
            return false;
        }
        pair.fill(0xff);
        if copy_from_user(other, edge, &mut pair) || pair != [0xff; 2] {
            return false;
        }
        if destroy_user_address_space(space).is_err()
            || destroy_user_address_space(other).is_err()
            || copy_from_user(space, USER_DATA, &mut pair)
            || copy_to_user(space, USER_DATA, &[])
            || memory::allocated_frames() != baseline
        {
            return false;
        }
        true
    })
}

#[cfg(feature = "memory-test-faults")]
pub fn run_translation_probe() -> bool {
    // SAFETY: fixtures below activate a live root with a known user byte. The
    // BSP masks IRQs; this bounded read temporarily sets AC for the SMAP-enabled
    // profile and restores all flags before returning. No user reference escapes.
    unsafe fn read_user_byte(address: u64) -> u8 {
        let value: u32;
        asm!("pushfq", "pop {saved}", "mov {access}, {saved}", "or {access}, 0x40000",
            "push {access}", "popfq", "movzx {value:e}, byte ptr [{address}]",
            "push {saved}", "popfq", saved = out(reg) _, access = out(reg) _,
            value = lateout(reg) value, address = in(reg) address, options(preserves_flags));
        value as u8
    }
    crate::arch::without_interrupts(|| {
        let baseline = memory::allocated_frames();
        let Ok(first) = create_user_address_space() else {
            return false;
        };
        let Ok(other) = create_user_address_space() else {
            return false;
        };
        for (space, value) in [(first, 0xa1), (other, 0xb2)] {
            let Ok(grant) = allocate_zeroed_frame(space) else {
                return false;
            };
            if map_user_page(space, USER_DATA, grant, true, false).is_err()
                || !copy_to_user(space, USER_DATA, &[value])
            {
                return false;
            }
        }
        for (space, expected) in [(first, 0xa1), (other, 0xb2), (first, 0xa1)] {
            activate(space);
            // SAFETY: exact fixture mapping and AC/IRQ contract above.
            if unsafe { read_user_byte(USER_DATA) } != expected {
                return false;
            }
        }
        activate_kernel();
        if destroy_user_address_space(first).is_err() {
            return false;
        }
        let Ok(reused) = create_user_address_space() else {
            return false;
        };
        let Ok(grant) = allocate_zeroed_frame(reused) else {
            return false;
        };
        if map_user_page(reused, USER_DATA, grant, true, false).is_err()
            || !copy_to_user(reused, USER_DATA, &[0xc3])
        {
            return false;
        }
        activate(reused);
        // SAFETY: exact live replacement mapping, never a stale grant.
        let replaced = unsafe { read_user_byte(USER_DATA) } == 0xc3;
        activate_kernel();
        replaced
            && destroy_user_address_space(reused).is_ok()
            && destroy_user_address_space(other).is_ok()
            && memory::allocated_frames() == baseline
    })
}
