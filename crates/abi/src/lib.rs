#![no_std]

pub const BOOT_INFO_MAGIC: u64 = 0x4745_4e4f_535f_4249; // GENOS_BI
pub const BOOT_INFO_VERSION: u32 = 1;
pub const BOOTLOADER_VERSION: u32 = 1;
pub const MAX_MEMORY_REGIONS: usize = 256;
pub const MAX_CMDLINE_LEN: usize = 128;
pub const USER_ABI_VERSION: u64 = 15;
pub const USER_IMAGE_LAYOUT_VERSION: u64 = 2;
pub const USER_EXECUTABLE_PAGE_CAPACITY: u64 = 8;
pub const USER_SYSCALL_PING: u64 = 0;
pub const USER_SYSCALL_ABI_VERSION: u64 = 1;
pub const USER_SYSCALL_EXIT: u64 = 2;
pub const USER_SYSCALL_YIELD: u64 = 3;
pub const USER_SYSCALL_REPORT: u64 = 4;
pub const USER_SYSCALL_WRITE: u64 = 5;
pub const USER_SYSCALL_SLEEP: u64 = 6;
/// Reserved: legacy direct-PID send, removed in ABI 9 and never reassigned.
pub const USER_SYSCALL_SEND: u64 = 7;
/// Reserved: legacy inbox receive, removed in ABI 9 and never reassigned.
pub const USER_SYSCALL_RECEIVE: u64 = 8;
pub const USER_SYSCALL_WAIT_CHILD: u64 = 9;
pub const USER_SYSCALL_SYSTEM_INFO: u64 = 10;
pub const USER_SYSCALL_READ_FILE: u64 = 11;
pub const USER_SYSCALL_OPEN_FILE: u64 = 12;
pub const USER_SYSCALL_READ_HANDLE: u64 = 13;
pub const USER_SYSCALL_STAT_HANDLE: u64 = 14;
pub const USER_SYSCALL_CLOSE_HANDLE: u64 = 15;
pub const USER_SYSCALL_OPEN_FILE_WITH_RIGHTS: u64 = 16;
pub const USER_SYSCALL_WRITE_HANDLE: u64 = 17;
pub const USER_SYSCALL_WAIT_INPUT: u64 = 18;
pub const USER_SYSCALL_CREATE_ENDPOINT: u64 = 19;
pub const USER_SYSCALL_CONNECT_ENDPOINT: u64 = 20;
pub const USER_SYSCALL_SEND_ENDPOINT: u64 = 21;
pub const USER_SYSCALL_RECEIVE_ENDPOINT: u64 = 22;
pub const USER_SYSCALL_CLOSE_ENDPOINT: u64 = 23;
pub const USER_SYSCALL_CONSOLE_WRITE: u64 = 24;
pub const USER_SYSCALL_CONSOLE_SET_INPUT: u64 = 25;
pub const USER_SYSCALL_CONSOLE_CLEAR: u64 = 26;
pub const USER_SYSCALL_READ_DIRECTORY: u64 = 27;
pub const USER_SYSCALL_TRUNCATE_HANDLE: u64 = 28;
pub const USER_SYSCALL_PROCESS_LAUNCH: u64 = 29;
pub const USER_SYSCALL_PROCESS_STATUS: u64 = 30;
pub const USER_SYSCALL_PROCESS_KILL: u64 = 31;
pub const USER_SYSCALL_PROCESS_REAP: u64 = 32;
pub const USER_SYSCALL_CREATE_DIRECTORY: u64 = 33;
pub const USER_SYSCALL_REMOVE_PATH: u64 = 34;
pub const USER_SYSCALL_NETWORK_CONFIG: u64 = 35;
pub const USER_SYSCALL_UDP_EXCHANGE: u64 = 36;
pub const USER_SYSCALL_TCP_EXCHANGE: u64 = 37;
pub const USER_CONSOLE_LINE_OUTPUT: u64 = 0;
pub const USER_CONSOLE_LINE_PROMPT: u64 = 1;
pub const USER_CONSOLE_LINE_ERROR: u64 = 2;
pub const USER_CONSOLE_LINE_STATUS: u64 = 3;
pub const USER_CONSOLE_TEXT_MAX: usize = 80;
pub const USER_MESSAGE_CAPACITY: u64 = 4;
pub const USER_CHANNEL_MESSAGE_SIZE: u64 = core::mem::size_of::<UserChannelMessage>() as u64;
pub const USER_ENDPOINT_HANDLE_CAPACITY: u64 = 4;
pub const USER_ENDPOINT_QUEUE_CAPACITY: usize = 4;
pub const USER_FILE_READ_MAX: usize = 128;
pub const USER_FILE_WRITE_MAX: usize = 128;
pub const USER_FILE_HANDLE_CAPACITY: u64 = 4;
pub const USER_PROCESS_HANDLE_CAPACITY: u64 = 3;
pub const USER_PROCESS_IMAGE_INIT: u64 = 1;
pub const USER_PROCESS_MODE_NORMAL: u64 = 0;
pub const USER_PROCESS_MODE_HOLD: u64 = 1;
pub const USER_PROCESS_STATE_READY: u64 = 1;
pub const USER_PROCESS_STATE_SLEEPING: u64 = 2;
pub const USER_PROCESS_STATE_WAITING: u64 = 3;
pub const USER_PROCESS_STATE_EXITED: u64 = 4;
pub const USER_PROCESS_STATE_FAULTED: u64 = 5;
pub const USER_PROCESS_STATE_KILLED: u64 = 6;
pub const USER_PATH_MAX: usize = 64;
pub const USER_DIRECTORY_NAME_MAX: usize = 64;
pub const USER_FILE_KIND_REGULAR: u64 = 1;
pub const USER_FILE_KIND_DIRECTORY: u64 = 2;
pub const USER_FILE_RIGHT_READ: u64 = 1;
pub const USER_FILE_RIGHT_WRITE: u64 = 2;
pub const USER_FILE_RIGHT_MANAGE: u64 = 4;
pub const USER_FILE_RIGHTS_MASK: u64 =
    USER_FILE_RIGHT_READ | USER_FILE_RIGHT_WRITE | USER_FILE_RIGHT_MANAGE;
pub const USER_WRITABLE_PREFIX: &str = "/USER/";
pub const USER_INPUT_MASK_KEYBOARD: u64 = 1;
pub const USER_INPUT_MASK_POINTER: u64 = 2;
pub const USER_INPUT_MASK_ALL: u64 = USER_INPUT_MASK_KEYBOARD | USER_INPUT_MASK_POINTER;
pub const USER_INPUT_KIND_KEY: u64 = 1;
pub const USER_INPUT_KIND_POINTER_MOVE: u64 = 2;
pub const USER_INPUT_KIND_POINTER_BUTTON: u64 = 3;
pub const USER_KEY_CHAR: u64 = 1;
pub const USER_KEY_ENTER: u64 = 2;
pub const USER_KEY_BACKSPACE: u64 = 3;
pub const USER_KEY_ESCAPE: u64 = 4;
pub const USER_KEY_TAB: u64 = 5;
pub const USER_KEY_ARROW_UP: u64 = 6;
pub const USER_KEY_ARROW_DOWN: u64 = 7;
pub const USER_POINTER_BUTTON_LEFT: u64 = 1;
pub const USER_POINTER_BUTTON_RIGHT: u64 = 2;
pub const USER_POINTER_BUTTON_MIDDLE: u64 = 4;
pub const USER_ERROR_UNKNOWN_SYSCALL: u64 = u64::MAX;
pub const USER_ERROR_INVALID_ARGUMENT: u64 = u64::MAX - 1;
pub const USER_ERROR_UNAVAILABLE: u64 = u64::MAX - 2;
pub const USER_PAGE_SIZE: u64 = 4096;
pub const USER_TIMER_HZ: u64 = 100;
pub const USER_PING_REPLY: u64 = 0x4745_4e4f_535f_4f4b;

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UserNetworkConfig {
    pub address: u32,
    pub subnet: u32,
    pub gateway: u32,
    pub dns: u32,
    pub mac: [u8; 6],
    pub reserved: [u8; 2],
}

impl UserNetworkConfig {
    pub const fn empty() -> Self {
        Self {
            address: 0,
            subnet: 0,
            gateway: 0,
            dns: 0,
            mac: [0; 6],
            reserved: [0; 2],
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UserProcessHeader {
    pub token: u64,
    pub preemptions: u64,
}

impl UserProcessHeader {
    pub const fn empty() -> Self {
        Self {
            token: 0,
            preemptions: 0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UserSystemInfo {
    pub abi_version: u64,
    pub page_size: u64,
    pub timer_hz: u64,
    pub message_capacity: u64,
    pub max_file_read: u64,
    pub file_handle_capacity: u64,
    pub max_file_write: u64,
    pub input_event_size: u64,
    pub input_mask: u64,
    pub endpoint_handle_capacity: u64,
    pub channel_message_size: u64,
    pub directory_entry_size: u64,
    pub max_path_length: u64,
    pub process_status_size: u64,
    pub process_handle_capacity: u64,
    pub image_layout_version: u64,
    pub executable_page_capacity: u64,
}

impl UserSystemInfo {
    pub const fn empty() -> Self {
        Self {
            abi_version: 0,
            page_size: 0,
            timer_hz: 0,
            message_capacity: 0,
            max_file_read: 0,
            file_handle_capacity: 0,
            max_file_write: 0,
            input_event_size: 0,
            input_mask: 0,
            endpoint_handle_capacity: 0,
            channel_message_size: 0,
            directory_entry_size: 0,
            max_path_length: 0,
            process_status_size: 0,
            process_handle_capacity: 0,
            image_layout_version: 0,
            executable_page_capacity: 0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UserProcessStatus {
    pub instance_id: u64,
    pub task_id: u64,
    pub runtime_pid: u64,
    pub state: u64,
    pub exit_code: u64,
    pub fault_vector: u64,
    pub preemptions: u64,
    pub reserved: u64,
}

impl UserProcessStatus {
    pub const fn empty() -> Self {
        Self {
            instance_id: 0,
            task_id: 0,
            runtime_pid: 0,
            state: 0,
            exit_code: 0,
            fault_vector: 0,
            preemptions: 0,
            reserved: 0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UserDirectoryEntry {
    pub kind: u64,
    pub size: u64,
    pub name_length: u64,
    pub reserved: u64,
    pub name: [u8; USER_DIRECTORY_NAME_MAX],
}

impl UserDirectoryEntry {
    pub const fn empty() -> Self {
        Self {
            kind: 0,
            size: 0,
            name_length: 0,
            reserved: 0,
            name: [0; USER_DIRECTORY_NAME_MAX],
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UserChannelMessage {
    pub sender_pid: u64,
    pub value: u64,
}

impl UserChannelMessage {
    pub const fn empty() -> Self {
        Self {
            sender_pid: 0,
            value: 0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UserInputEvent {
    pub kind: u64,
    pub code: u64,
    pub value0: i64,
    pub value1: i64,
}

impl UserInputEvent {
    pub const fn empty() -> Self {
        Self {
            kind: 0,
            code: 0,
            value0: 0,
            value1: 0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UserFileStat {
    pub size: u64,
    pub offset: u64,
    pub kind: u64,
    pub rights: u64,
}

impl UserFileStat {
    pub const fn empty() -> Self {
        Self {
            size: 0,
            offset: 0,
            kind: 0,
            rights: 0,
        }
    }
}

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
