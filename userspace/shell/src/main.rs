#![no_std]
#![no_main]

use core::panic::PanicInfo;
use core::ptr::{addr_of, addr_of_mut, read_volatile, write_volatile};

use genos_user_runtime as runtime;

const LINE_CAPACITY: usize = runtime::CONSOLE_TEXT_MAX;
const READY: &[u8] = b"SHELL.ELF ready - commands now run in Ring 3";
const HELP: &[u8] = b"help clear echo uname ls cat - userspace shell v0.18";
const UNAME: &[u8] = b"GenOS v0.18 ring3-shell x86_64 ABI 11";
const UNKNOWN: &[u8] = b"unknown userspace command";
const DIRECTORY_ERROR: &[u8] = b"directory unavailable";
const FILE_ERROR: &[u8] = b"file unavailable";

#[repr(C)]
struct ShellData {
    header: runtime::UserProcessHeader,
    event: runtime::UserInputEvent,
    directory_entry: runtime::UserDirectoryEntry,
    file_buffer: [u8; runtime::FILE_READ_MAX],
    line: [u8; LINE_CAPACITY],
    len: usize,
}

#[used]
#[link_section = ".data.process"]
static mut DATA: ShellData = ShellData {
    header: runtime::UserProcessHeader::empty(),
    event: runtime::UserInputEvent::empty(),
    directory_entry: runtime::UserDirectoryEntry::empty(),
    file_buffer: [0; runtime::FILE_READ_MAX],
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
    let root = runtime::open_file(b"/");
    if handle_error(root) {
        runtime::exit(249);
    }
    let entry = unsafe { &mut *addr_of_mut!(DATA.directory_entry) };
    if runtime::read_directory(root, 0, entry)
        != core::mem::size_of::<runtime::UserDirectoryEntry>() as u64
        || runtime::close_handle(root) != 0
    {
        runtime::exit(249);
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
    } else if matches(line, b"ls") {
        list_directory(console, b"/");
    } else if line.len() > 3 && matches(&line[..3], b"ls ") {
        list_directory(console, &line[3..]);
    } else if line.len() > 4 && matches(&line[..4], b"cat ") {
        print_file(console, &line[4..]);
    } else if !line.is_empty() {
        let _ = runtime::console_write(console, UNKNOWN, runtime::CONSOLE_LINE_ERROR);
    }
    unsafe {
        DATA.len = 0;
    }
}

fn list_directory(console: u64, path: &[u8]) {
    let mut absolute = [0u8; runtime::PATH_MAX];
    let path = absolute_path(path, &mut absolute);
    if path.is_empty() {
        let _ = runtime::console_write(console, DIRECTORY_ERROR, runtime::CONSOLE_LINE_ERROR);
        return;
    }
    let handle = runtime::open_file(path);
    if handle_error(handle) {
        let _ = runtime::console_write(console, DIRECTORY_ERROR, runtime::CONSOLE_LINE_ERROR);
        return;
    }
    let mut cursor = 0u64;
    loop {
        let entry = unsafe { &mut *addr_of_mut!(DATA.directory_entry) };
        let result = runtime::read_directory(handle, cursor, entry);
        if result == 0 {
            break;
        }
        if result != core::mem::size_of::<runtime::UserDirectoryEntry>() as u64
            || entry.name_length == 0
            || entry.name_length as usize > runtime::DIRECTORY_NAME_MAX
        {
            let _ = runtime::console_write(console, DIRECTORY_ERROR, runtime::CONSOLE_LINE_ERROR);
            break;
        }
        let mut output = [0u8; LINE_CAPACITY];
        let name_len = entry.name_length as usize;
        output[..name_len].copy_from_slice(&entry.name[..name_len]);
        let mut len = name_len;
        if entry.kind == runtime::FILE_KIND_DIRECTORY && len < output.len() {
            output[len] = b'/';
            len += 1;
        } else if entry.kind == runtime::FILE_KIND_REGULAR {
            append(&mut output, &mut len, b"  ");
            append_u64(&mut output, &mut len, entry.size);
            append(&mut output, &mut len, b" B");
        }
        let _ = runtime::console_write(console, &output[..len], runtime::CONSOLE_LINE_OUTPUT);
        cursor = cursor.saturating_add(1);
    }
    let _ = runtime::close_handle(handle);
}

fn print_file(console: u64, path: &[u8]) {
    let mut absolute = [0u8; runtime::PATH_MAX];
    let path = absolute_path(path, &mut absolute);
    if path.is_empty() {
        let _ = runtime::console_write(console, FILE_ERROR, runtime::CONSOLE_LINE_ERROR);
        return;
    }
    let handle = runtime::open_file(path);
    if handle_error(handle) {
        let _ = runtime::console_write(console, FILE_ERROR, runtime::CONSOLE_LINE_ERROR);
        return;
    }
    let mut output = [0u8; LINE_CAPACITY];
    let mut output_len = 0usize;
    loop {
        let buffer = unsafe { &mut *addr_of_mut!(DATA.file_buffer) };
        let count = runtime::read_handle(handle, buffer);
        if count == runtime::ERROR_INVALID_ARGUMENT || count == runtime::ERROR_UNAVAILABLE {
            let _ = runtime::console_write(console, FILE_ERROR, runtime::CONSOLE_LINE_ERROR);
            break;
        }
        if count == 0 {
            if output_len != 0 {
                let _ = runtime::console_write(
                    console,
                    &output[..output_len],
                    runtime::CONSOLE_LINE_OUTPUT,
                );
            }
            break;
        }
        for byte in buffer.iter().take(count as usize) {
            if *byte == b'\n' || output_len == output.len() {
                if output_len != 0 {
                    let _ = runtime::console_write(
                        console,
                        &output[..output_len],
                        runtime::CONSOLE_LINE_OUTPUT,
                    );
                    output_len = 0;
                }
                if *byte == b'\n' {
                    continue;
                }
            }
            output[output_len] = *byte;
            output_len += 1;
        }
    }
    let _ = runtime::close_handle(handle);
}

fn handle_error(value: u64) -> bool {
    value == runtime::ERROR_INVALID_ARGUMENT || value == runtime::ERROR_UNAVAILABLE
}

fn absolute_path<'a>(path: &'a [u8], output: &'a mut [u8; runtime::PATH_MAX]) -> &'a [u8] {
    if path.first() == Some(&b'/') {
        return path;
    }
    if path.is_empty() || path.len() >= output.len() {
        return &[];
    }
    output[0] = b'/';
    output[1..1 + path.len()].copy_from_slice(path);
    &output[..1 + path.len()]
}

fn append(output: &mut [u8; LINE_CAPACITY], len: &mut usize, bytes: &[u8]) {
    for byte in bytes {
        if *len == output.len() {
            return;
        }
        output[*len] = *byte;
        *len += 1;
    }
}

fn append_u64(output: &mut [u8; LINE_CAPACITY], len: &mut usize, mut value: u64) {
    let mut digits = [0u8; 20];
    let mut count = 0usize;
    loop {
        digits[count] = b'0' + (value % 10) as u8;
        count += 1;
        value /= 10;
        if value == 0 {
            break;
        }
    }
    while count != 0 {
        count -= 1;
        append(output, len, &digits[count..count + 1]);
    }
}

fn matches(actual: &[u8], expected: &[u8]) -> bool {
    actual.len() == expected.len() && actual.iter().zip(expected).all(|(a, b)| a == b)
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    runtime::exit(250)
}
