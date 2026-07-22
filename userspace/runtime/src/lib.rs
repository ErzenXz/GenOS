#![no_std]

#[cfg(target_arch = "x86_64")]
use core::arch::asm;

pub use genos_abi::{
    UserFileStat, UserProcessHeader, UserSystemInfo, USER_ABI_VERSION as ABI_VERSION,
    USER_ERROR_INVALID_ARGUMENT as ERROR_INVALID_ARGUMENT,
    USER_ERROR_UNAVAILABLE as ERROR_UNAVAILABLE, USER_FILE_HANDLE_CAPACITY as FILE_HANDLE_CAPACITY,
    USER_FILE_KIND_REGULAR as FILE_KIND_REGULAR, USER_FILE_READ_MAX as FILE_READ_MAX,
    USER_FILE_RIGHT_READ as FILE_RIGHT_READ, USER_MESSAGE_CAPACITY as MESSAGE_CAPACITY,
    USER_PAGE_SIZE as PAGE_SIZE, USER_PING_REPLY as PING_REPLY, USER_TIMER_HZ as TIMER_HZ,
};
pub const STACK_GUARD: u64 = 0x0000_4000_0000_7000;

use genos_abi::{
    USER_SYSCALL_ABI_VERSION as SYSCALL_ABI_VERSION,
    USER_SYSCALL_CLOSE_HANDLE as SYSCALL_CLOSE_HANDLE, USER_SYSCALL_EXIT as SYSCALL_EXIT,
    USER_SYSCALL_OPEN_FILE as SYSCALL_OPEN_FILE, USER_SYSCALL_PING as SYSCALL_PING,
    USER_SYSCALL_READ_FILE as SYSCALL_READ_FILE, USER_SYSCALL_READ_HANDLE as SYSCALL_READ_HANDLE,
    USER_SYSCALL_RECEIVE as SYSCALL_RECEIVE, USER_SYSCALL_REPORT as SYSCALL_REPORT,
    USER_SYSCALL_SEND as SYSCALL_SEND, USER_SYSCALL_SLEEP as SYSCALL_SLEEP,
    USER_SYSCALL_STAT_HANDLE as SYSCALL_STAT_HANDLE,
    USER_SYSCALL_SYSTEM_INFO as SYSCALL_SYSTEM_INFO, USER_SYSCALL_WAIT_CHILD as SYSCALL_WAIT_CHILD,
    USER_SYSCALL_WRITE as SYSCALL_WRITE, USER_SYSCALL_YIELD as SYSCALL_YIELD,
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

pub fn write(bytes: &[u8]) -> u64 {
    unsafe {
        syscall(
            SYSCALL_WRITE,
            [bytes.as_ptr() as u64, bytes.len() as u64, 0, 0, 0, 0],
        )
    }
}

pub fn sleep(ticks: u64) -> u64 {
    unsafe { syscall(SYSCALL_SLEEP, [ticks, 0, 0, 0, 0, 0]) }
}

pub fn send(pid: u8, value: u64) -> u64 {
    unsafe { syscall(SYSCALL_SEND, [pid as u64, value, 0, 0, 0, 0]) }
}

pub fn receive() -> u64 {
    unsafe { syscall(SYSCALL_RECEIVE, [0; 6]) }
}

pub fn wait_child(pid: u8) -> u64 {
    unsafe { syscall(SYSCALL_WAIT_CHILD, [pid as u64, 0, 0, 0, 0, 0]) }
}

pub fn system_info(info: &mut UserSystemInfo) -> u64 {
    unsafe {
        syscall(
            SYSCALL_SYSTEM_INFO,
            [
                info as *mut UserSystemInfo as u64,
                core::mem::size_of::<UserSystemInfo>() as u64,
                0,
                0,
                0,
                0,
            ],
        )
    }
}

pub fn read_file(path: &[u8], output: &mut [u8]) -> u64 {
    unsafe {
        syscall(
            SYSCALL_READ_FILE,
            [
                path.as_ptr() as u64,
                path.len() as u64,
                output.as_mut_ptr() as u64,
                output.len() as u64,
                0,
                0,
            ],
        )
    }
}

pub fn open_file(path: &[u8]) -> u64 {
    unsafe {
        syscall(
            SYSCALL_OPEN_FILE,
            [path.as_ptr() as u64, path.len() as u64, 0, 0, 0, 0],
        )
    }
}

pub fn read_handle(handle: u64, output: &mut [u8]) -> u64 {
    unsafe {
        syscall(
            SYSCALL_READ_HANDLE,
            [
                handle,
                output.as_mut_ptr() as u64,
                output.len() as u64,
                0,
                0,
                0,
            ],
        )
    }
}

pub fn stat_handle(handle: u64, stat: &mut UserFileStat) -> u64 {
    unsafe {
        syscall(
            SYSCALL_STAT_HANDLE,
            [
                handle,
                stat as *mut UserFileStat as u64,
                core::mem::size_of::<UserFileStat>() as u64,
                0,
                0,
                0,
            ],
        )
    }
}

pub fn close_handle(handle: u64) -> u64 {
    unsafe { syscall(SYSCALL_CLOSE_HANDLE, [handle, 0, 0, 0, 0, 0]) }
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
