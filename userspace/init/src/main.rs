#![no_std]
#![no_main]

use core::panic::PanicInfo;
use core::ptr::{addr_of, addr_of_mut, read_volatile, write_volatile};

use genos_user_runtime as runtime;

const FAULT_TOKEN: u64 = 0xffff_ffff_ffff_fff0;
const TOKEN_MODE_MASK: u64 = 0xf000_0000_0000_0000;
const HOLD_TOKEN_MODE: u64 = 0xb000_0000_0000_0000;
const SLEEP_TOKEN_MODE: u64 = 0x4000_0000_0000_0000;
const CHILD_TOKEN_MODE: u64 = 0x5000_0000_0000_0000;
const PARENT_TOKEN_MODE: u64 = 0x6000_0000_0000_0000;
const FILE_TOKEN_MODE: u64 = 0x7000_0000_0000_0000;
const INPUT_TOKEN_MODE: u64 = 0x9000_0000_0000_0000;
const WRITE_TOKEN_MODE: u64 = 0xa000_0000_0000_0000;
const COORDINATION_MESSAGE: u64 = 0x4745_4e4f_535f_4950;
const GREETING: &[u8] = b"hello from INIT.ELF in ring 3";
const AWAKENED: &[u8] = b"INIT.ELF woke after deadline";
const COORDINATED: &[u8] = b"parent received child exit + message";
const README_PATH: &[u8] = b"/README.TXT";
const README_CONTENT: &[u8] = b"Welcome to GenOS.\nThis file lives in the V1 RAM disk.\n";
const FILE_COMPLETE: &[u8] = b"INIT.ELF used open/read/stat/close";
const FIRST_READ_BYTES: usize = 17;
const USER_NOTE_PATH: &[u8] = b"/USER/APP.TXT";
const WRITE_FIRST: &[u8] = b"GenOS Ring 3 ";
const WRITE_SECOND: &[u8] = b"writes safely.";
const WRITE_CONTENT: &[u8] = b"GenOS Ring 3 writes safely.";
const WRITE_COMPLETE: &[u8] = b"INIT.ELF wrote and verified /USER/APP.TXT";
const INPUT_PROMPT: &[u8] = b"INIT.ELF waiting for one keyboard event";
const INPUT_BUSY: &[u8] = b"INIT.ELF input channel is busy";

#[repr(C)]
struct ProcessData {
    header: runtime::UserProcessHeader,
    system_info: runtime::UserSystemInfo,
    file_stat: runtime::UserFileStat,
    input_event: runtime::UserInputEvent,
    file_buffer: [u8; 128],
}

#[used]
#[link_section = ".data.process"]
static mut PROCESS_DATA: ProcessData = ProcessData {
    header: runtime::UserProcessHeader::empty(),
    system_info: runtime::UserSystemInfo::empty(),
    file_stat: runtime::UserFileStat::empty(),
    input_event: runtime::UserInputEvent::empty(),
    file_buffer: [0; 128],
};

#[no_mangle]
pub extern "C" fn _start(token: u64) -> ! {
    unsafe {
        write_volatile(addr_of_mut!(PROCESS_DATA.header.token), token);
    }

    if runtime::ping() != runtime::PING_REPLY || runtime::abi_version() != runtime::ABI_VERSION {
        runtime::exit(255);
    }

    while unsafe { read_volatile(addr_of!(PROCESS_DATA.header.preemptions)) } == 0 {
        core::hint::spin_loop();
    }

    if token == FAULT_TOKEN {
        unsafe {
            write_volatile(runtime::STACK_GUARD as *mut u64, token);
        }
        runtime::exit(254);
    }

    if runtime::write(GREETING) != GREETING.len() as u64 {
        runtime::exit(251);
    }

    if token & TOKEN_MODE_MASK == SLEEP_TOKEN_MODE
        && (runtime::sleep(3) != 0 || runtime::write(AWAKENED) != AWAKENED.len() as u64)
    {
        runtime::exit(250);
    }

    if token & TOKEN_MODE_MASK == CHILD_TOKEN_MODE {
        let parent = token as u8;
        if runtime::sleep(3) != 0 || runtime::send(parent, COORDINATION_MESSAGE) != 0 {
            runtime::exit(249);
        }
        runtime::exit(7);
    }

    if token & TOKEN_MODE_MASK == PARENT_TOKEN_MODE {
        let child = token as u8;
        if runtime::wait_child(child) != 7 || runtime::receive() != COORDINATION_MESSAGE {
            runtime::exit(248);
        }
        if runtime::write(COORDINATED) != COORDINATED.len() as u64 {
            runtime::exit(247);
        }
    }

    if token & TOKEN_MODE_MASK == FILE_TOKEN_MODE {
        let info = unsafe { &mut *addr_of_mut!(PROCESS_DATA.system_info) };
        if runtime::system_info(info) != core::mem::size_of::<runtime::UserSystemInfo>() as u64
            || info.abi_version != runtime::ABI_VERSION
            || info.page_size != runtime::PAGE_SIZE
            || info.timer_hz != runtime::TIMER_HZ
            || info.message_capacity != runtime::MESSAGE_CAPACITY
            || info.max_file_read != runtime::FILE_READ_MAX as u64
            || info.file_handle_capacity != runtime::FILE_HANDLE_CAPACITY
            || info.max_file_write != runtime::FILE_WRITE_MAX as u64
            || info.input_event_size != core::mem::size_of::<runtime::UserInputEvent>() as u64
            || info.input_mask != runtime::INPUT_MASK_ALL
        {
            runtime::exit(246);
        }
        let handle = runtime::open_file(README_PATH);
        if handle == 0 || handle >= runtime::ERROR_UNAVAILABLE {
            runtime::exit(245);
        }
        let stat = unsafe { &mut *addr_of_mut!(PROCESS_DATA.file_stat) };
        if runtime::stat_handle(handle, stat)
            != core::mem::size_of::<runtime::UserFileStat>() as u64
            || stat.size != README_CONTENT.len() as u64
            || stat.offset != 0
            || stat.kind != runtime::FILE_KIND_REGULAR
            || stat.rights != runtime::FILE_RIGHT_READ
        {
            runtime::exit(244);
        }
        let buffer = unsafe { &mut *addr_of_mut!(PROCESS_DATA.file_buffer) };
        if runtime::read_handle(handle, &mut buffer[..FIRST_READ_BYTES]) != FIRST_READ_BYTES as u64
            || runtime::stat_handle(handle, stat)
                != core::mem::size_of::<runtime::UserFileStat>() as u64
            || stat.offset != FIRST_READ_BYTES as u64
        {
            runtime::exit(243);
        }
        let remaining = runtime::read_handle(handle, &mut buffer[FIRST_READ_BYTES..]);
        if remaining != (README_CONTENT.len() - FIRST_READ_BYTES) as u64
            || &buffer[..README_CONTENT.len()] != README_CONTENT
        {
            runtime::exit(242);
        }
        if runtime::close_handle(handle) != 0
            || runtime::read_handle(handle, &mut buffer[..1]) != runtime::ERROR_INVALID_ARGUMENT
        {
            runtime::exit(241);
        }
        if runtime::write(FILE_COMPLETE) != FILE_COMPLETE.len() as u64 {
            runtime::exit(240);
        }
    }

    if token & TOKEN_MODE_MASK == WRITE_TOKEN_MODE {
        let denied = runtime::open_file_with_rights(
            README_PATH,
            runtime::FILE_RIGHT_READ | runtime::FILE_RIGHT_WRITE,
        );
        if denied != runtime::ERROR_INVALID_ARGUMENT {
            runtime::exit(239);
        }
        let handle = runtime::open_file_with_rights(
            USER_NOTE_PATH,
            runtime::FILE_RIGHT_READ | runtime::FILE_RIGHT_WRITE,
        );
        if handle == 0 || handle >= runtime::ERROR_UNAVAILABLE {
            runtime::exit(238);
        }
        let stat = unsafe { &mut *addr_of_mut!(PROCESS_DATA.file_stat) };
        if runtime::stat_handle(handle, stat)
            != core::mem::size_of::<runtime::UserFileStat>() as u64
            || (stat.size != 0 && stat.size != WRITE_CONTENT.len() as u64)
            || stat.offset != 0
            || stat.rights != (runtime::FILE_RIGHT_READ | runtime::FILE_RIGHT_WRITE)
        {
            runtime::exit(237);
        }
        if runtime::write_handle(handle, WRITE_FIRST) != WRITE_FIRST.len() as u64
            || runtime::write_handle(handle, WRITE_SECOND) != WRITE_SECOND.len() as u64
            || runtime::stat_handle(handle, stat)
                != core::mem::size_of::<runtime::UserFileStat>() as u64
            || stat.size != WRITE_CONTENT.len() as u64
            || stat.offset != WRITE_CONTENT.len() as u64
        {
            runtime::exit(236);
        }
        if runtime::close_handle(handle) != 0 {
            runtime::exit(235);
        }
        let read_handle = runtime::open_file(USER_NOTE_PATH);
        let buffer = unsafe { &mut *addr_of_mut!(PROCESS_DATA.file_buffer) };
        if read_handle == 0
            || read_handle >= runtime::ERROR_UNAVAILABLE
            || runtime::write_handle(read_handle, b"!") != runtime::ERROR_INVALID_ARGUMENT
            || runtime::read_handle(read_handle, buffer) != WRITE_CONTENT.len() as u64
            || &buffer[..WRITE_CONTENT.len()] != WRITE_CONTENT
            || runtime::close_handle(read_handle) != 0
        {
            runtime::exit(234);
        }
        if runtime::write(WRITE_COMPLETE) != WRITE_COMPLETE.len() as u64 {
            runtime::exit(233);
        }
    }

    if token & TOKEN_MODE_MASK == INPUT_TOKEN_MODE {
        if runtime::write(INPUT_PROMPT) != INPUT_PROMPT.len() as u64 {
            runtime::exit(232);
        }
        let event = unsafe { &mut *addr_of_mut!(PROCESS_DATA.input_event) };
        let result = runtime::wait_input(event, runtime::INPUT_MASK_KEYBOARD);
        if result == runtime::ERROR_UNAVAILABLE {
            if runtime::write(INPUT_BUSY) != INPUT_BUSY.len() as u64 {
                runtime::exit(231);
            }
            runtime::exit(0);
        }
        if result != core::mem::size_of::<runtime::UserInputEvent>() as u64
            || event.kind != runtime::INPUT_KIND_KEY
            || event.code != runtime::KEY_CHAR
            || !(0x20..=0x7e).contains(&event.value0)
            || event.value1 != 0
        {
            runtime::exit(230);
        }
        let mut message = *b"INIT.ELF received key: ?";
        message[23] = event.value0 as u8;
        if runtime::write(&message) != message.len() as u64 {
            runtime::exit(229);
        }
    }

    if token & TOKEN_MODE_MASK == HOLD_TOKEN_MODE {
        loop {
            core::hint::spin_loop();
        }
    }

    let reported = runtime::report_u64(unsafe { addr_of!(PROCESS_DATA.header.token) });
    runtime::exit(if reported == token { 0 } else { 253 });
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    runtime::exit(252)
}
