#![no_std]
#![no_main]

use core::panic::PanicInfo;
use core::ptr::{addr_of, addr_of_mut, read_volatile, write_volatile};

use genos_user_runtime as runtime;

const FAULT_TOKEN: u64 = 0xffff_ffff_ffff_fff0;
const HOLD_TOKEN_BIT: u64 = 1 << 63;
const TOKEN_MODE_MASK: u64 = 0xf000_0000_0000_0000;
const SLEEP_TOKEN_MODE: u64 = 0x4000_0000_0000_0000;
const CHILD_TOKEN_MODE: u64 = 0x5000_0000_0000_0000;
const PARENT_TOKEN_MODE: u64 = 0x6000_0000_0000_0000;
const COORDINATION_MESSAGE: u64 = 0x4745_4e4f_535f_4950;
const GREETING: &[u8] = b"hello from INIT.ELF in ring 3";
const AWAKENED: &[u8] = b"INIT.ELF woke after deadline";
const COORDINATED: &[u8] = b"parent received child exit + message";

#[repr(C)]
struct ProcessData {
    token: u64,
    preemptions: u64,
}

#[used]
#[link_section = ".data.process"]
static mut PROCESS_DATA: ProcessData = ProcessData {
    token: 0,
    preemptions: 0,
};

#[no_mangle]
pub extern "C" fn _start(token: u64) -> ! {
    unsafe {
        write_volatile(addr_of_mut!(PROCESS_DATA.token), token);
    }

    if runtime::ping() != runtime::PING_REPLY || runtime::abi_version() != runtime::ABI_VERSION {
        runtime::exit(255);
    }

    while unsafe { read_volatile(addr_of!(PROCESS_DATA.preemptions)) } == 0 {
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

    if token & HOLD_TOKEN_BIT != 0 {
        loop {
            core::hint::spin_loop();
        }
    }

    let reported = runtime::report_u64(unsafe { addr_of!(PROCESS_DATA.token) });
    runtime::exit(if reported == token { 0 } else { 253 });
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    runtime::exit(252)
}
