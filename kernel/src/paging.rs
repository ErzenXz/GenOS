use core::arch::asm;

use crate::memory;

const ENTRY_COUNT: usize = 512;
const PRESENT: u64 = 1 << 0;
const WRITABLE: u64 = 1 << 1;
const USER: u64 = 1 << 2;
const WRITE_THROUGH: u64 = 1 << 3;
const CACHE_DISABLE: u64 = 1 << 4;
const ACCESSED: u64 = 1 << 5;
const DIRTY: u64 = 1 << 6;
const HUGE_OR_PAT: u64 = 1 << 7;
const GLOBAL: u64 = 1 << 8;
const LARGE_PAT: u64 = 1 << 12;
const NO_EXECUTE: u64 = 1 << 63;
const TABLE_ADDRESS_MASK: u64 = 0x000f_ffff_ffff_f000;
const PAGE_2M_ADDRESS_MASK: u64 = 0x000f_ffff_ffe0_0000;
const PAGE_1G_ADDRESS_MASK: u64 = 0x000f_ffff_c000_0000;
const PAGE_SIZE: u64 = 4096;

static mut ACTIVE_ROOT: u64 = 0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PagingError {
    OutOfMemory,
    MissingMapping,
    InvalidAddress,
}

pub fn init_protected_address_space() -> Result<(), PagingError> {
    let current = read_cr3() & TABLE_ADDRESS_MASK;
    if current == 0 {
        return Err(PagingError::MissingMapping);
    }
    crate::serial::print("PAGING_CLONE_BEGIN root=0x");
    crate::serial::print_hex(current);
    crate::serial::println("");

    // The bootloader installs an identity map. New page-table frames come from ranges that
    // remain present in that map, so their physical addresses are valid kernel pointers here.
    let allocated_before = memory::allocated_frames();
    let protected = unsafe { clone_table(current, 4)? };
    let table_frames = memory::allocated_frames().saturating_sub(allocated_before);
    unsafe {
        asm!("mov cr3, {root}", root = in(reg) protected, options(nostack, preserves_flags));
        core::ptr::addr_of_mut!(ACTIVE_ROOT).write(protected);
    }
    crate::serial::print("PAGING_READY root=0x");
    crate::serial::print_hex(protected);
    crate::serial::print(" tables=");
    crate::serial::print_u64(table_frames);
    crate::serial::println("");
    Ok(())
}

pub fn expose_user_page(address: u64, writable: bool, executable: bool) -> Result<(), PagingError> {
    if address & (PAGE_SIZE - 1) != 0 {
        return Err(PagingError::InvalidAddress);
    }
    let root = unsafe { *core::ptr::addr_of!(ACTIVE_ROOT) };
    if root == 0 {
        return Err(PagingError::MissingMapping);
    }

    unsafe {
        let pml4 = table_mut(root);
        let pml4e = &mut pml4[index(address, 39)];
        let pdpt_phys = child_table(pml4e, 4)?;

        let pdpt = table_mut(pdpt_phys);
        let pdpte = &mut pdpt[index(address, 30)];
        if *pdpte & HUGE_OR_PAT != 0 {
            split_1g(pdpte)?;
        }
        let pd_phys = child_table(pdpte, 3)?;

        let pd = table_mut(pd_phys);
        let pde = &mut pd[index(address, 21)];
        if *pde & HUGE_OR_PAT != 0 {
            split_2m(pde)?;
        }
        let pt_phys = child_table(pde, 2)?;

        let pt = table_mut(pt_phys);
        let pte = &mut pt[index(address, 12)];
        if *pte & PRESENT == 0 {
            return Err(PagingError::MissingMapping);
        }
        *pte |= USER;
        if writable {
            *pte |= WRITABLE;
        } else {
            *pte &= !WRITABLE;
        }
        if executable || !nx_enabled() {
            *pte &= !NO_EXECUTE;
        } else {
            *pte |= NO_EXECUTE;
        }
        asm!("invlpg [{page}]", page = in(reg) address, options(nostack, preserves_flags));
    }
    Ok(())
}

unsafe fn clone_table(source_phys: u64, level: u8) -> Result<u64, PagingError> {
    let destination_phys = allocate_table()?;
    let source = table(source_phys);
    let destination = table_mut(destination_phys);

    for slot in 0..ENTRY_COUNT {
        let entry = source[slot];
        if entry & PRESENT == 0 {
            continue;
        }
        let is_leaf = level == 1 || (level <= 3 && entry & HUGE_OR_PAT != 0);
        if is_leaf {
            destination[slot] = entry & !USER;
        } else {
            let child = entry & TABLE_ADDRESS_MASK;
            let cloned_child = clone_table(child, level - 1)?;
            destination[slot] = cloned_child | ((entry & !TABLE_ADDRESS_MASK) & !USER);
        }
    }
    Ok(destination_phys)
}

unsafe fn child_table(entry: &mut u64, _level: u8) -> Result<u64, PagingError> {
    if *entry & PRESENT == 0 || *entry & HUGE_OR_PAT != 0 {
        return Err(PagingError::MissingMapping);
    }
    *entry |= PRESENT | WRITABLE | USER;
    *entry &= !NO_EXECUTE;
    Ok(*entry & TABLE_ADDRESS_MASK)
}

unsafe fn split_1g(entry: &mut u64) -> Result<(), PagingError> {
    let original = *entry;
    let base = original & PAGE_1G_ADDRESS_MASK;
    let table_phys = allocate_table()?;
    let table = table_mut(table_phys);
    let leaf_flags = preserved_leaf_flags(original) | HUGE_OR_PAT;
    for (index, slot) in table.iter_mut().enumerate() {
        *slot = (base + index as u64 * (1 << 21)) | leaf_flags;
    }
    *entry = table_phys | parent_flags(original) | PRESENT | WRITABLE | USER;
    Ok(())
}

unsafe fn split_2m(entry: &mut u64) -> Result<(), PagingError> {
    let original = *entry;
    let base = original & PAGE_2M_ADDRESS_MASK;
    let table_phys = allocate_table()?;
    let table = table_mut(table_phys);
    let mut leaf_flags = preserved_leaf_flags(original) & !HUGE_OR_PAT;
    if original & LARGE_PAT != 0 {
        leaf_flags |= HUGE_OR_PAT;
    }
    for (index, slot) in table.iter_mut().enumerate() {
        *slot = (base + index as u64 * PAGE_SIZE) | leaf_flags;
    }
    *entry = table_phys | parent_flags(original) | PRESENT | WRITABLE | USER;
    Ok(())
}

const fn preserved_leaf_flags(entry: u64) -> u64 {
    entry
        & (PRESENT
            | WRITABLE
            | WRITE_THROUGH
            | CACHE_DISABLE
            | ACCESSED
            | DIRTY
            | GLOBAL
            | NO_EXECUTE)
}

const fn parent_flags(entry: u64) -> u64 {
    entry & (WRITE_THROUGH | CACHE_DISABLE | ACCESSED)
}

unsafe fn allocate_table() -> Result<u64, PagingError> {
    let frame = memory::alloc_frame().ok_or(PagingError::OutOfMemory)?;
    core::ptr::write_bytes(frame as *mut u8, 0, PAGE_SIZE as usize);
    Ok(frame)
}

unsafe fn table(physical: u64) -> &'static [u64; ENTRY_COUNT] {
    &*(physical as *const [u64; ENTRY_COUNT])
}

unsafe fn table_mut(physical: u64) -> &'static mut [u64; ENTRY_COUNT] {
    &mut *(physical as *mut [u64; ENTRY_COUNT])
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
