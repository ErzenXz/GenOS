#![no_main]
#![no_std]

extern crate alloc;

mod elf;

use alloc::vec::Vec;
use core::ptr::{addr_of_mut, copy_nonoverlapping};
use core::{mem::size_of, panic::PanicInfo, time::Duration};
use genos_abi::{boot_memory, BootInfo, BOOTLOADER_VERSION};
use uefi::boot::{self, AllocateType, MemoryType};
use uefi::fs::FileSystem;
use uefi::mem::memory_map::{MemoryMap, MemoryMapOwned};
use uefi::runtime::ResetType;
use uefi::{prelude::*, CStr16};

const CMDLINE: &str = "root=initrd console=serial ui=off";

type KernelEntry = extern "sysv64" fn(&'static BootInfo) -> !;

#[entry]
fn main() -> Status {
    if let Err(status) = boot_main() {
        uefi::println!("GenOS boot failed: {:?}", status);
        boot::stall(Duration::from_secs(5));
        uefi::runtime::reset(ResetType::SHUTDOWN, Status::ABORTED, None);
    }
    Status::SUCCESS
}

fn boot_main() -> Result<(), Status> {
    uefi::helpers::init().map_err(|e| e.status())?;
    uefi::println!("GenOS UEFI loader v{}", BOOTLOADER_VERSION);

    let mut fs =
        FileSystem::new(boot::get_image_file_system(boot::image_handle()).map_err(|e| e.status())?);
    let kernel = read_first(
        &mut fs,
        &[
            uefi::cstr16!("\\EFI\\GENOS\\KERNEL.ELF"),
            uefi::cstr16!("EFI\\GENOS\\KERNEL.ELF"),
            uefi::cstr16!("\\EFI\\BOOT\\KERNEL.ELF"),
            uefi::cstr16!("EFI\\BOOT\\KERNEL.ELF"),
        ],
    )
    .ok_or(Status::NOT_FOUND)?;
    let initrd = read_first(
        &mut fs,
        &[
            uefi::cstr16!("\\EFI\\GENOS\\INITRD.GRD"),
            uefi::cstr16!("EFI\\GENOS\\INITRD.GRD"),
            uefi::cstr16!("\\EFI\\BOOT\\INITRD.GRD"),
            uefi::cstr16!("EFI\\BOOT\\INITRD.GRD"),
        ],
    )
    .unwrap_or_default();

    uefi::println!("Loading kernel ELF");
    let loaded_kernel = elf::load_kernel(&kernel)?;
    uefi::println!("Loading initrd");
    let initrd_info = load_initrd(&initrd)?;

    let boot_info_ptr = allocate_boot_info()?;
    let mut boot_info = BootInfo::empty();
    boot_info.bootloader_version = BOOTLOADER_VERSION;
    boot_info.initrd = initrd_info;
    boot_info.set_cmdline(CMDLINE);

    // Release file/protocol owners before ExitBootServices. Loaded segments,
    // initrd and BootInfo have separate retained page allocations.
    drop(fs);
    drop(kernel);
    drop(initrd);
    // SAFETY: no boot-service protocol or pool owner is used after this call.
    // uefi 0.36.1 owns the map buffer and retries a stale exit key once.
    let memory_map = unsafe { boot::exit_boot_services(Some(MemoryType::LOADER_DATA)) };
    if fill_memory_map(&mut boot_info, &memory_map).is_err() {
        halt_after_exit(b"BOOT_MEMORY_MAP_REJECTED\r\n");
    }

    // SAFETY: boot_info_ptr is a retained, aligned page allocation; the loaded
    // ELF supplies the trusted SysV64 kernel entry. Neither operation returns
    // to boot services, and the handoff object remains live indefinitely.
    unsafe {
        addr_of_mut!(*boot_info_ptr).write(boot_info);
        let entry: KernelEntry = core::mem::transmute(loaded_kernel.entry);
        entry(&*boot_info_ptr);
    }
}

fn read_first(fs: &mut FileSystem, paths: &[&CStr16]) -> Option<Vec<u8>> {
    for path in paths {
        match fs.read(*path) {
            Ok(bytes) => {
                uefi::println!("Loaded {}", path);
                return Some(bytes);
            }
            Err(error) => {
                uefi::println!("Could not read {}: {:?}", path, error);
            }
        }
    }
    None
}

fn load_initrd(bytes: &[u8]) -> Result<genos_abi::InitrdInfo, Status> {
    if bytes.is_empty() {
        return Ok(genos_abi::InitrdInfo { base: 0, size: 0 });
    }

    let pages = bytes.len().div_ceil(4096);
    let ptr = boot::allocate_pages(AllocateType::AnyPages, MemoryType::LOADER_DATA, pages)
        .map_err(|e| e.status())?;
    unsafe {
        copy_nonoverlapping(bytes.as_ptr(), ptr.as_ptr(), bytes.len());
    }
    Ok(genos_abi::InitrdInfo {
        base: ptr.as_ptr() as u64,
        size: bytes.len() as u64,
    })
}

fn allocate_boot_info() -> Result<*mut BootInfo, Status> {
    let pages = size_of::<BootInfo>().div_ceil(4096);
    let ptr = boot::allocate_pages(AllocateType::AnyPages, MemoryType::LOADER_DATA, pages)
        .map_err(|e| e.status())?;
    Ok(ptr.as_ptr().cast::<BootInfo>())
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    halt_after_exit(b"BOOTLOADER_PANIC\r\n")
}

fn fill_memory_map(
    boot_info: &mut BootInfo,
    map: &MemoryMapOwned,
) -> Result<(), boot_memory::BootMemoryError> {
    let meta = map.meta();
    let bytes = map
        .buffer()
        .get(..meta.map_size)
        .ok_or(boot_memory::BootMemoryError::DescriptorLength)?;
    boot_info.memory_map = boot_memory::decode_uefi_map(bytes, meta.desc_size, meta.desc_version)?;
    Ok(())
}

/// Fatal handoff errors cannot call boot services (including stall/printing).
fn halt_after_exit(message: &[u8]) -> ! {
    // SAFETY: this x86_64 loader owns the boot CPU. Bounded COM1 port access has
    // no memory aliases; no firmware operation or allocator is called. CLI/HLT
    // deliberately prevents continuation with a rejected map, even if no UART
    // exists. The assembly clobbers only its declared AL/DX registers.
    unsafe {
        core::arch::asm!("cli", options(nomem, nostack));
        for (port, byte) in [
            (0x3f9u16, 0u8),
            (0x3fb, 0x80),
            (0x3f8, 3),
            (0x3f9, 0),
            (0x3fb, 3),
            (0x3fa, 0xc7),
            (0x3fc, 0x0b),
        ] {
            core::arch::asm!("out dx, al", in("dx") port, in("al") byte, options(nomem, nostack));
        }
        for &byte in message {
            for _ in 0..100_000 {
                let status: u8;
                core::arch::asm!("in al, dx", in("dx") 0x3fdu16, out("al") status, options(nomem, nostack));
                if status & 0x20 != 0 {
                    core::arch::asm!("out dx, al", in("dx") 0x3f8u16, in("al") byte, options(nomem, nostack));
                    break;
                }
            }
        }
        loop {
            core::arch::asm!("hlt", options(nomem, nostack));
        }
    }
}
