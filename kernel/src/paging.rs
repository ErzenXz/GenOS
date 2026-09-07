use core::arch::asm;

use crate::memory;

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
    let root = unsafe { allocate_table()? };
    unsafe {
        let source = table(kernel_root);
        let destination = table_mut(root);
        destination.copy_from_slice(source);
        if destination[USER_PML4_INDEX] & PRESENT != 0 {
            let _ = memory::free_frame(root);
            return Err(PagingError::AddressInUse);
        }
    }
    Ok(AddressSpace { root })
}

pub fn map_user_page(
    space: AddressSpace,
    virtual_address: u64,
    physical_address: u64,
    writable: bool,
    executable: bool,
) -> Result<(), PagingError> {
    if virtual_address & (PAGE_SIZE - 1) != 0
        || physical_address & (PAGE_SIZE - 1) != 0
        || physical_address == 0
        || physical_address & !TABLE_ADDRESS_MASK != 0
        || (writable && executable)
        || index(virtual_address, 39) != USER_PML4_INDEX
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
                let child = ensure_user_table(entry)?;
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
                memory::free_frame(child),
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
    unsafe {
        let pml4e = table(space.root)[index(virtual_address, 39)];
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

pub fn allocate_zeroed_frame() -> Result<u64, PagingError> {
    unsafe { allocate_table() }
}

pub fn destroy_user_address_space(space: AddressSpace) -> Result<u64, PagingError> {
    if space.root == 0 || active_root() == space.root {
        return Err(PagingError::ActiveAddressSpace);
    }
    let mut released = 0u64;
    unsafe {
        let pml4 = table_mut(space.root);
        let entry = pml4[USER_PML4_INDEX];
        if entry & PRESENT != 0 {
            if entry & HUGE_OR_PAT != 0 || entry & USER == 0 {
                return Err(PagingError::InvalidAddress);
            }
            let user_root = entry & TABLE_ADDRESS_MASK;
            released += release_user_table(user_root, 3)?;
            if !memory::free_frame(user_root) {
                return Err(PagingError::InvalidAddress);
            }
            released += 1;
            pml4[USER_PML4_INDEX] = 0;
        }
    }
    if !memory::free_frame(space.root) {
        return Err(PagingError::InvalidAddress);
    }
    Ok(released + 1)
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
        table(root)[slot]
    }
    unsafe fn write(&mut self, root: u64, slot: usize, entry: u64) {
        table_mut(root)[slot] = entry;
    }
    fn release(&mut self, frame: u64) {
        assert!(
            memory::free_frame(frame),
            "page-table rollback lost ownership"
        );
    }
}

unsafe fn clone_table(source_phys: u64, level: u8) -> Result<u64, PagingError> {
    kernel::page_table::clone_supervisor(&mut PhysicalTables, source_phys, level)
        .ok_or(PagingError::OutOfMemory)
}

unsafe fn ensure_user_table(entry: &mut u64) -> Result<u64, PagingError> {
    if *entry & PRESENT == 0 {
        let table_phys = allocate_table()?;
        *entry = table_phys | PRESENT | WRITABLE | USER;
    } else if *entry & HUGE_OR_PAT != 0
        || *entry & USER == 0
        || *entry & WRITABLE == 0
        || *entry & NO_EXECUTE != 0
    {
        return Err(PagingError::AddressInUse);
    }
    Ok(*entry & TABLE_ADDRESS_MASK)
}

unsafe fn allocate_table() -> Result<u64, PagingError> {
    let frame = memory::alloc_frame().ok_or(PagingError::OutOfMemory)?;
    core::ptr::write_bytes(frame as *mut u8, 0, PAGE_SIZE as usize);
    Ok(frame)
}

unsafe fn release_user_table(table_phys: u64, level: u8) -> Result<u64, PagingError> {
    let entries = table_mut(table_phys);
    let mut released = 0u64;
    for entry in entries.iter_mut() {
        if *entry & PRESENT == 0 {
            continue;
        }
        let frame = *entry & TABLE_ADDRESS_MASK;
        if level == 1 {
            if !memory::free_frame(frame) {
                return Err(PagingError::InvalidAddress);
            }
            released += 1;
        } else {
            if *entry & HUGE_OR_PAT != 0 || *entry & USER == 0 {
                return Err(PagingError::InvalidAddress);
            }
            released += release_user_table(frame, level - 1)?;
            if !memory::free_frame(frame) {
                return Err(PagingError::InvalidAddress);
            }
            released += 1;
        }
        *entry = 0;
    }
    Ok(released)
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
