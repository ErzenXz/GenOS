use core::arch::global_asm;
use core::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use kernel::syscall::{self, SyscallAction};

use crate::{arch, paging};

pub const SYSCALL_VECTOR: usize = 0x80;
const USER_STACK_PAGES: usize = 4;

static PING_PASSED: AtomicBool = AtomicBool::new(false);
static ABI_PASSED: AtomicBool = AtomicBool::new(false);
static CONTEXT_PASSED: AtomicBool = AtomicBool::new(false);
static PROBE_PASSED: AtomicBool = AtomicBool::new(false);
static EXIT_CODE: AtomicU8 = AtomicU8::new(u8::MAX);

#[repr(C)]
struct SyscallFrame {
    r15: u64,
    r14: u64,
    r13: u64,
    r12: u64,
    r11: u64,
    r10: u64,
    r9: u64,
    r8: u64,
    rdi: u64,
    rsi: u64,
    rbp: u64,
    rbx: u64,
    rdx: u64,
    rcx: u64,
    rax: u64,
    rip: u64,
    cs: u64,
    rflags: u64,
    rsp: u64,
    ss: u64,
}

global_asm!(
    r#"
    .global genos_enter_userspace
genos_enter_userspace:
    push rbp
    push rbx
    push r12
    push r13
    push r14
    push r15
    pushfq
    mov [rip + genos_user_return_rsp], rsp
    lea rax, [rip + genos_user_return]
    mov [rip + genos_user_return_rip], rax
    push {user_data}
    push rsi
    pushfq
    push {user_code}
    push rdi
    iretq

genos_user_return:
    popfq
    pop r15
    pop r14
    pop r13
    pop r12
    pop rbx
    pop rbp
    ret

    .global genos_syscall_stub
genos_syscall_stub:
    cld
    push rax
    push rcx
    push rdx
    push rbx
    push rbp
    push rsi
    push rdi
    push r8
    push r9
    push r10
    push r11
    push r12
    push r13
    push r14
    push r15
    mov rdi, rsp
    and rsp, -16
    sub rsp, 16
    mov [rsp], rdi
    call genos_syscall_rust
    mov rsp, [rsp]
    test rax, rax
    jnz genos_leave_userspace
    pop r15
    pop r14
    pop r13
    pop r12
    pop r11
    pop r10
    pop r9
    pop r8
    pop rdi
    pop rsi
    pop rbp
    pop rbx
    pop rdx
    pop rcx
    pop rax
    iretq

genos_leave_userspace:
    mov rsp, [rip + genos_user_return_rsp]
    jmp [rip + genos_user_return_rip]

    .section .bss
    .balign 8
genos_user_return_rsp:
    .quad 0
genos_user_return_rip:
    .quad 0

    .section .usertext,"ax"
    .balign 4096
    .global genos_user_text_start
genos_user_text_start:
    xor rdi, rdi
    xor rsi, rsi
    xor rdx, rdx
    xor r10, r10
    xor r8, r8
    xor r9, r9
    mov rax, {sys_ping}
    int {syscall_vector}
    mov rbx, {ping_reply}
    cmp rax, rbx
    jne genos_user_probe_failed
    mov rax, {sys_abi}
    int {syscall_vector}
    cmp rax, {abi_version}
    jne genos_user_probe_failed
    xor rdi, rdi
    mov rax, {sys_exit}
    int {syscall_vector}

genos_user_probe_failed:
    mov rdi, 255
    mov rax, {sys_exit}
    int {syscall_vector}
1:
    pause
    jmp 1b
    .balign 4096
    .global genos_user_text_end
genos_user_text_end:

    .section .userstack,"aw",@nobits
    .balign 4096
    .global genos_user_stack_guard
genos_user_stack_guard:
    .skip 4096
    .global genos_user_stack_bottom
genos_user_stack_bottom:
    .skip 16384
    .global genos_user_stack_top
genos_user_stack_top:

    .section .text
"#,
    user_data = const arch::USER_DATA_SELECTOR,
    user_code = const arch::USER_CODE_SELECTOR,
    syscall_vector = const SYSCALL_VECTOR,
    sys_ping = const syscall::SYSCALL_PING,
    sys_abi = const syscall::SYSCALL_ABI_VERSION,
    sys_exit = const syscall::SYSCALL_EXIT,
    ping_reply = const syscall::PING_REPLY,
    abi_version = const syscall::USER_ABI_VERSION,
);

unsafe extern "C" {
    fn genos_enter_userspace(entry: u64, stack_top: u64);
    fn genos_syscall_stub();
    static genos_user_text_start: u8;
    static genos_user_text_end: u8;
    static genos_user_stack_bottom: u8;
    static genos_user_stack_top: u8;
}

pub fn syscall_handler() -> unsafe extern "C" fn() {
    genos_syscall_stub
}

pub fn run_probe() {
    let text_start = core::ptr::addr_of!(genos_user_text_start) as u64;
    let text_end = core::ptr::addr_of!(genos_user_text_end) as u64;
    let stack_bottom = core::ptr::addr_of!(genos_user_stack_bottom) as u64;
    let stack_top = core::ptr::addr_of!(genos_user_stack_top) as u64;

    let mut page = text_start;
    while page < text_end {
        require_page(paging::expose_user_page(page, false, true));
        page += 4096;
    }
    page = stack_bottom;
    while page < stack_top {
        require_page(paging::expose_user_page(page, true, false));
        page += 4096;
    }
    crate::serial::print("USER_PAGES_READY text=0x");
    crate::serial::print_hex(text_start);
    crate::serial::print(" stack_pages=");
    crate::serial::print_u64(USER_STACK_PAGES as u64);
    crate::serial::println("");

    unsafe { genos_enter_userspace(text_start, stack_top) };

    let passed = PING_PASSED.load(Ordering::Acquire)
        && ABI_PASSED.load(Ordering::Acquire)
        && CONTEXT_PASSED.load(Ordering::Acquire)
        && EXIT_CODE.load(Ordering::Acquire) == 0;
    if passed {
        PROBE_PASSED.store(true, Ordering::Release);
        crate::serial::println("USERMODE_READY");
    } else {
        crate::serial::println("USERMODE_FAILED");
        arch::halt_loop();
    }
}

pub fn probe_passed() -> bool {
    PROBE_PASSED.load(Ordering::Acquire)
}

#[no_mangle]
extern "C" fn genos_syscall_rust(frame: *mut SyscallFrame) -> u64 {
    let frame = unsafe { &mut *frame };
    if !valid_user_frame(frame) {
        crate::serial::println("USER_CONTEXT_INVALID");
        return 1;
    }
    if !CONTEXT_PASSED.swap(true, Ordering::AcqRel) {
        crate::serial::println("USER_CONTEXT_OK");
    }
    let args = [
        frame.rdi, frame.rsi, frame.rdx, frame.r10, frame.r8, frame.r9,
    ];
    match syscall::dispatch(frame.rax, args) {
        Ok(SyscallAction::Return(value)) => {
            if frame.rax == syscall::SYSCALL_PING && value == syscall::PING_REPLY {
                PING_PASSED.store(true, Ordering::Release);
                crate::serial::println("USER_SYSCALL_OK");
            }
            if frame.rax == syscall::SYSCALL_ABI_VERSION && value == syscall::USER_ABI_VERSION {
                ABI_PASSED.store(true, Ordering::Release);
                crate::serial::println("USER_ABI_OK");
            }
            frame.rax = value;
            0
        }
        Ok(SyscallAction::Exit(code)) => {
            EXIT_CODE.store(code, Ordering::Release);
            crate::serial::print("USER_EXIT code=");
            crate::serial::print_u64(code as u64);
            crate::serial::println("");
            1
        }
        Err(error) => {
            frame.rax = syscall::error_code(error);
            0
        }
    }
}

fn valid_user_frame(frame: &SyscallFrame) -> bool {
    let text_start = core::ptr::addr_of!(genos_user_text_start) as u64;
    let text_end = core::ptr::addr_of!(genos_user_text_end) as u64;
    let stack_bottom = core::ptr::addr_of!(genos_user_stack_bottom) as u64;
    let stack_top = core::ptr::addr_of!(genos_user_stack_top) as u64;
    frame.cs == arch::USER_CODE_SELECTOR as u64
        && frame.ss == arch::USER_DATA_SELECTOR as u64
        && frame.rip >= text_start
        && frame.rip < text_end
        && frame.rsp > stack_bottom
        && frame.rsp <= stack_top
}

fn require_page(result: Result<(), paging::PagingError>) {
    if result.is_err() {
        crate::serial::println("USER_PAGE_MAP_FAILED");
        arch::halt_loop();
    }
}
