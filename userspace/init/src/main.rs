#![no_std]
#![no_main]

use core::panic::PanicInfo;
use core::ptr::{addr_of, addr_of_mut, read_volatile, write_volatile};

use genos_user_runtime as runtime;

const FAULT_TOKEN: u64 = 0xffff_ffff_ffff_fff0;

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

    let reported = runtime::report_u64(unsafe { addr_of!(PROCESS_DATA.token) });
    runtime::exit(if reported == token { 0 } else { 253 });
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    runtime::exit(252)
}
