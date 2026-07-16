#![no_std]

pub const BOOT_INFO_MAGIC: u64 = 0x4745_4e4f_535f_4249; // GENOS_BI
pub const BOOT_INFO_VERSION: u32 = 1;
pub const BOOTLOADER_VERSION: u32 = 1;
pub const MAX_MEMORY_REGIONS: usize = 256;
pub const MAX_CMDLINE_LEN: usize = 128;
pub const USER_ABI_VERSION: u64 = 4;
pub const USER_SYSCALL_PING: u64 = 0;
pub const USER_SYSCALL_ABI_VERSION: u64 = 1;
pub const USER_SYSCALL_EXIT: u64 = 2;
pub const USER_SYSCALL_YIELD: u64 = 3;
pub const USER_SYSCALL_REPORT: u64 = 4;
pub const USER_SYSCALL_WRITE: u64 = 5;
pub const USER_SYSCALL_SLEEP: u64 = 6;
pub const USER_SYSCALL_SEND: u64 = 7;
pub const USER_SYSCALL_RECEIVE: u64 = 8;
pub const USER_SYSCALL_WAIT_CHILD: u64 = 9;
pub const USER_PING_REPLY: u64 = 0x4745_4e4f_535f_4f4b;

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BootInfo {
    pub magic: u64,
    pub version: u32,
    pub bootloader_version: u32,
    pub framebuffer: FramebufferInfo,
    pub memory_map: MemoryMapInfo,
    pub initrd: InitrdInfo,
    pub cmdline_len: u32,
    pub cmdline: [u8; MAX_CMDLINE_LEN],
}

impl BootInfo {
    pub const fn empty() -> Self {
        Self {
            magic: BOOT_INFO_MAGIC,
            version: BOOT_INFO_VERSION,
            bootloader_version: BOOTLOADER_VERSION,
            framebuffer: FramebufferInfo::empty(),
            memory_map: MemoryMapInfo::empty(),
            initrd: InitrdInfo { base: 0, size: 0 },
            cmdline_len: 0,
            cmdline: [0; MAX_CMDLINE_LEN],
        }
    }

    pub fn set_cmdline(&mut self, text: &str) {
        let bytes = text.as_bytes();
        let len = bytes.len().min(MAX_CMDLINE_LEN);
        self.cmdline[..len].copy_from_slice(&bytes[..len]);
        self.cmdline_len = len as u32;
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FramebufferInfo {
    pub base: u64,
    pub size: u64,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub pixel_format: PixelFormat,
}

impl FramebufferInfo {
    pub const fn empty() -> Self {
        Self {
            base: 0,
            size: 0,
            width: 0,
            height: 0,
            stride: 0,
            pixel_format: PixelFormat::Bgr,
        }
    }
}

#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PixelFormat {
    Rgb = 0,
    Bgr = 1,
    Bitmask = 2,
    Unknown = 3,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MemoryMapInfo {
    pub region_count: u64,
    pub regions: [MemoryRegion; MAX_MEMORY_REGIONS],
}

impl MemoryMapInfo {
    pub const fn empty() -> Self {
        Self {
            region_count: 0,
            regions: [MemoryRegion::empty(); MAX_MEMORY_REGIONS],
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MemoryRegion {
    pub start: u64,
    pub size: u64,
    pub kind: MemoryRegionKind,
}

impl MemoryRegion {
    pub const fn empty() -> Self {
        Self {
            start: 0,
            size: 0,
            kind: MemoryRegionKind::Reserved,
        }
    }
}

#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoryRegionKind {
    Usable = 0,
    Bootloader = 1,
    Kernel = 2,
    Framebuffer = 3,
    Acpi = 4,
    Mmio = 5,
    Reserved = 6,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InitrdInfo {
    pub base: u64,
    pub size: u64,
}
