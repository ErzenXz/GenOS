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
pub const USER_STACK_GUARD: u64 = USER_BASE + 0x7000;
pub const USER_STACK_BOTTOM: u64 = USER_BASE + 0x8000;
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
        || index(virtual_address, 39) != USER_PML4_INDEX
    {
        return Err(PagingError::InvalidAddress);
    }

    unsafe {
        let pml4 = table_mut(space.root);
        let pdpt_phys = ensure_user_table(&mut pml4[index(virtual_address, 39)])?;
        let pdpt = table_mut(pdpt_phys);
        let pd_phys = ensure_user_table(&mut pdpt[index(virtual_address, 30)])?;
        let pd = table_mut(pd_phys);
        let pt_phys = ensure_user_table(&mut pd[index(virtual_address, 21)])?;
        let pt = table_mut(pt_phys);
        let pte = &mut pt[index(virtual_address, 12)];
        if *pte & PRESENT != 0 {
            return Err(PagingError::AddressInUse);
        }

        let mut flags = PRESENT | USER;
        if writable {
            flags |= WRITABLE;
        }
        if !executable && nx_enabled() {
            flags |= NO_EXECUTE;
        }
        *pte = physical_address | flags;
        asm!("invlpg [{page}]", page = in(reg) virtual_address, options(nostack, preserves_flags));
    }
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

unsafe fn ensure_user_table(entry: &mut u64) -> Result<u64, PagingError> {
    if *entry & PRESENT == 0 {
        let table_phys = allocate_table()?;
        *entry = table_phys | PRESENT | WRITABLE | USER;
    } else if *entry & HUGE_OR_PAT != 0 || *entry & USER == 0 {
        return Err(PagingError::AddressInUse);
    }
    *entry |= PRESENT | WRITABLE | USER;
    *entry &= !NO_EXECUTE;
    Ok(*entry & TABLE_ADDRESS_MASK)
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
