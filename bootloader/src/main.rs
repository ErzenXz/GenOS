#![no_main]
#![no_std]

extern crate alloc;

mod elf;

use alloc::vec::Vec;
use core::ptr::{addr_of_mut, copy_nonoverlapping};
use core::{mem::size_of, panic::PanicInfo, time::Duration};
use genos_abi::{
    BootInfo, FramebufferInfo, MemoryRegion, MemoryRegionKind, PixelFormat, BOOTLOADER_VERSION,
    MAX_MEMORY_REGIONS,
};
use uefi::boot::{self, AllocateType, MemoryType};
use uefi::fs::FileSystem;
use uefi::mem::memory_map::{MemoryMap, MemoryMapOwned};
use uefi::proto::console::gop::{GraphicsOutput, PixelFormat as UefiPixelFormat};
use uefi::runtime::ResetType;
use uefi::{prelude::*, CStr16};

const CMDLINE: &str = "root=initrd console=fb";

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
    .unwrap_or_else(Vec::new);

    uefi::println!("Initializing framebuffer");
    let framebuffer = init_framebuffer()?;
    uefi::println!("Loading kernel ELF");
    let loaded_kernel = elf::load_kernel(&kernel)?;
    uefi::println!("Loading initrd");
    let initrd_info = load_initrd(&initrd)?;

    let boot_info_ptr = allocate_boot_info()?;
    let mut boot_info = BootInfo::empty();
    boot_info.bootloader_version = BOOTLOADER_VERSION;
    boot_info.framebuffer = framebuffer;
    boot_info.initrd = initrd_info;
    boot_info.set_cmdline(CMDLINE);

    let memory_map = unsafe { boot::exit_boot_services(Some(MemoryType::LOADER_DATA)) };
    fill_memory_map(&mut boot_info, &memory_map);

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

fn init_framebuffer() -> Result<FramebufferInfo, Status> {
    let gop_handle = boot::get_handle_for_protocol::<GraphicsOutput>().map_err(|e| e.status())?;
    let mut gop =
        boot::open_protocol_exclusive::<GraphicsOutput>(gop_handle).map_err(|e| e.status())?;
    let mode = gop.current_mode_info();
    let (width, height) = mode.resolution();
    let stride = mode.stride();
    let pixel_format = match mode.pixel_format() {
        UefiPixelFormat::Rgb => PixelFormat::Rgb,
        UefiPixelFormat::Bgr => PixelFormat::Bgr,
        UefiPixelFormat::Bitmask => PixelFormat::Bitmask,
        _ => PixelFormat::Unknown,
    };
    let mut fb = gop.frame_buffer();
    Ok(FramebufferInfo {
        base: fb.as_mut_ptr() as u64,
        size: fb.size() as u64,
        width: width as u32,
        height: height as u32,
        stride: stride as u32,
        pixel_format,
    })
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
    loop {
        boot::stall(Duration::from_secs(1));
    }
}

fn fill_memory_map(boot_info: &mut BootInfo, map: &MemoryMapOwned) {
    let mut count = 0usize;
    for desc in map.entries() {
        if count >= MAX_MEMORY_REGIONS {
            break;
        }
        boot_info.memory_map.regions[count] = MemoryRegion {
            start: desc.phys_start,
            size: desc.page_count * 4096,
            kind: classify_memory(desc.ty),
        };
        count += 1;
    }
    boot_info.memory_map.region_count = count as u64;
}

fn classify_memory(kind: MemoryType) -> MemoryRegionKind {
    match kind {
        MemoryType::CONVENTIONAL => MemoryRegionKind::Usable,
        MemoryType::LOADER_CODE | MemoryType::LOADER_DATA => MemoryRegionKind::Bootloader,
        // Keep firmware boot-services memory reserved until GenOS can prove that no active
        // page table or firmware-owned structure still references it.
        MemoryType::BOOT_SERVICES_CODE | MemoryType::BOOT_SERVICES_DATA => {
            MemoryRegionKind::Reserved
        }
        MemoryType::ACPI_RECLAIM | MemoryType::ACPI_NON_VOLATILE => MemoryRegionKind::Acpi,
        MemoryType::MMIO | MemoryType::MMIO_PORT_SPACE | MemoryType::PAL_CODE => {
            MemoryRegionKind::Mmio
        }
        _ => MemoryRegionKind::Reserved,
    }
}
