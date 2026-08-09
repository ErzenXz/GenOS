#![no_std]
#![no_main]

use core::panic::PanicInfo;
use core::ptr::{addr_of, addr_of_mut, read_volatile, write_volatile};

use genos_user_runtime as runtime;

const LINE_CAPACITY: usize = runtime::CONSOLE_TEXT_MAX;
const READY: &[u8] = b"SHELL.ELF ready - commands now run in Ring 3";
const HELP: &[u8] = b"help clear echo uname - userspace shell v0.17";
const UNAME: &[u8] = b"GenOS v0.17 ring3-shell x86_64 ABI 10";
const UNKNOWN: &[u8] = b"unknown userspace command";

#[repr(C)]
struct ShellData {
    header: runtime::UserProcessHeader,
    event: runtime::UserInputEvent,
    line: [u8; LINE_CAPACITY],
    len: usize,
}

#[used]
#[link_section = ".data.process"]
static mut DATA: ShellData = ShellData {
    header: runtime::UserProcessHeader::empty(),
    event: runtime::UserInputEvent::empty(),
    line: [0; LINE_CAPACITY],
    len: 0,
};

#[no_mangle]
pub extern "C" fn _start(console: u64) -> ! {
    unsafe {
        write_volatile(addr_of_mut!(DATA.header.token), console);
    }
    if runtime::ping() != runtime::PING_REPLY || runtime::abi_version() != runtime::ABI_VERSION {
        runtime::exit(255);
    }
    while unsafe { read_volatile(addr_of!(DATA.header.preemptions)) } == 0 {
        core::hint::spin_loop();
    }
    if runtime::console_write(console, READY, runtime::CONSOLE_LINE_STATUS) != READY.len() as u64 {
        runtime::exit(254);
    }

    loop {
        let event = unsafe { &mut *addr_of_mut!(DATA.event) };
        if runtime::wait_input(event, runtime::INPUT_MASK_KEYBOARD)
            != core::mem::size_of::<runtime::UserInputEvent>() as u64
        {
            runtime::exit(253);
        }
        if event.kind != runtime::INPUT_KIND_KEY {
            continue;
        }
        match event.code {
            runtime::KEY_CHAR if (0x20..=0x7e).contains(&event.value0) => push(event.value0 as u8),
            runtime::KEY_BACKSPACE => backspace(),
            runtime::KEY_ENTER => execute(console),
            _ => continue,
        }
        let line = unsafe { &DATA.line[..DATA.len] };
        if runtime::console_set_input(console, line) != line.len() as u64 {
            runtime::exit(252);
        }
    }
}

fn push(byte: u8) {
    unsafe {
        if DATA.len < LINE_CAPACITY {
            DATA.line[DATA.len] = byte;
            DATA.len += 1;
        }
    }
}

fn backspace() {
    unsafe {
        DATA.len = DATA.len.saturating_sub(1);
    }
}

fn execute(console: u64) {
    let (line, len) = unsafe { (&DATA.line[..DATA.len], DATA.len) };
    if len != 0 {
        let mut prompt = [0u8; LINE_CAPACITY];
        prompt[0] = b'/';
        prompt[1] = b'>';
        prompt[2] = b' ';
        let copy = len.min(LINE_CAPACITY - 3);
        prompt[3..3 + copy].copy_from_slice(&line[..copy]);
        let _ = runtime::console_write(console, &prompt[..3 + copy], runtime::CONSOLE_LINE_PROMPT);
    }
    if matches(line, b"help") {
        let _ = runtime::console_write(console, HELP, runtime::CONSOLE_LINE_OUTPUT);
    } else if matches(line, b"uname") {
        let _ = runtime::console_write(console, UNAME, runtime::CONSOLE_LINE_OUTPUT);
    } else if matches(line, b"clear") {
        let _ = runtime::console_clear(console);
    } else if line.len() >= 5 && matches(&line[..5], b"echo ") {
        let _ = runtime::console_write(console, &line[5..], runtime::CONSOLE_LINE_OUTPUT);
    } else if !line.is_empty() {
        let _ = runtime::console_write(console, UNKNOWN, runtime::CONSOLE_LINE_ERROR);
    }
    unsafe {
        DATA.len = 0;
    }
}

fn matches(actual: &[u8], expected: &[u8]) -> bool {
    actual.len() == expected.len() && actual.iter().zip(expected).all(|(a, b)| a == b)
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    runtime::exit(250)
}
