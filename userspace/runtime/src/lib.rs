#![no_std]

#[cfg(target_arch = "x86_64")]
use core::arch::asm;

pub use genos_abi::{USER_ABI_VERSION as ABI_VERSION, USER_PING_REPLY as PING_REPLY};
pub const STACK_GUARD: u64 = 0x0000_4000_0000_7000;

use genos_abi::{
    USER_SYSCALL_ABI_VERSION as SYSCALL_ABI_VERSION, USER_SYSCALL_EXIT as SYSCALL_EXIT,
    USER_SYSCALL_PING as SYSCALL_PING, USER_SYSCALL_REPORT as SYSCALL_REPORT,
    USER_SYSCALL_YIELD as SYSCALL_YIELD,
};

pub fn ping() -> u64 {
    unsafe { syscall(SYSCALL_PING, [0; 6]) }
}

pub fn abi_version() -> u64 {
    unsafe { syscall(SYSCALL_ABI_VERSION, [0; 6]) }
}

pub fn yield_now() -> u64 {
    unsafe { syscall(SYSCALL_YIELD, [0; 6]) }
}

pub fn report_u64(value: *const u64) -> u64 {
    unsafe { syscall(SYSCALL_REPORT, [value as u64, 8, 0, 0, 0, 0]) }
}

pub fn exit(status: u8) -> ! {
    unsafe {
        let _ = syscall(SYSCALL_EXIT, [status as u64, 0, 0, 0, 0, 0]);
    }
    loop {
        core::hint::spin_loop();
    }
}

#[cfg(target_arch = "x86_64")]
unsafe fn syscall(number: u64, args: [u64; 6]) -> u64 {
    let mut result = number;
    asm!(
        "int 0x80",
        inlateout("rax") result,
        in("rdi") args[0],
        in("rsi") args[1],
        in("rdx") args[2],
        in("r10") args[3],
        in("r8") args[4],
        in("r9") args[5],
    );
    result
}

#[cfg(not(target_arch = "x86_64"))]
unsafe fn syscall(_number: u64, _args: [u64; 6]) -> u64 {
    u64::MAX
}
