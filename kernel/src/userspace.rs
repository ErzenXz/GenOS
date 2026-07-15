use core::arch::global_asm;
use core::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use kernel::syscall::{self, SyscallAction};

use crate::{arch, paging};

pub const SYSCALL_VECTOR: usize = 0x80;
const PROCESS_COUNT: usize = 2;
const TOKEN_A: u64 = 0x1111_aaaa_1111_aaaa;
const TOKEN_B: u64 = 0x2222_bbbb_2222_bbbb;

static PROBE_PASSED: AtomicBool = AtomicBool::new(false);
static CONTEXT_PASSED: AtomicBool = AtomicBool::new(false);
static PING_COUNT: AtomicU8 = AtomicU8::new(0);
static ABI_COUNT: AtomicU8 = AtomicU8::new(0);
static REPORT_COUNT: AtomicU8 = AtomicU8::new(0);
static COMPLETED_PROCESSES: AtomicU8 = AtomicU8::new(0);
static ADDRESS_SPACES: AtomicU8 = AtomicU8::new(0);
static TOTAL_YIELDS: AtomicU8 = AtomicU8::new(0);
static mut CURRENT_PROCESS: *mut UserProcess = core::ptr::null_mut();

#[derive(Clone, Copy)]
#[repr(C)]
struct UserContext {
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

impl UserContext {
    const fn initial(token: u64) -> Self {
        Self {
            r15: 0,
            r14: 0,
            r13: 0,
            r12: 0,
            r11: 0,
            r10: 0,
            r9: 0,
            r8: 0,
            rdi: token,
            rsi: 0,
            rbp: 0,
            rbx: 0,
            rdx: 0,
            rcx: 0,
            rax: 0,
            rip: paging::USER_CODE,
            cs: arch::USER_CODE_SELECTOR as u64,
            rflags: 0x202,
            rsp: paging::USER_STACK_TOP,
            ss: arch::USER_DATA_SELECTOR as u64,
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum ProcessEvent {
    None,
    Yield,
    Exit,
    Fault,
}

struct UserProcess {
    pid: u8,
    space: paging::AddressSpace,
    context: UserContext,
    data_frame: u64,
    token: u64,
    event: ProcessEvent,
    report: u64,
    exit_code: u8,
    yields: u8,
    completed: bool,
}

#[derive(Clone, Copy)]
pub struct UserProbeResult {
    pub exit_codes: [u8; PROCESS_COUNT],
}

global_asm!(
    r#"
    .global genos_enter_user_context
genos_enter_user_context:
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

    mov rbx, rdi
    push qword ptr [rbx + {ctx_ss}]
    push qword ptr [rbx + {ctx_rsp}]
    push qword ptr [rbx + {ctx_rflags}]
    push qword ptr [rbx + {ctx_cs}]
    push qword ptr [rbx + {ctx_rip}]
    mov r15, [rbx + {ctx_r15}]
    mov r14, [rbx + {ctx_r14}]
    mov r13, [rbx + {ctx_r13}]
    mov r12, [rbx + {ctx_r12}]
    mov r11, [rbx + {ctx_r11}]
    mov r10, [rbx + {ctx_r10}]
    mov r9, [rbx + {ctx_r9}]
    mov r8, [rbx + {ctx_r8}]
    mov rsi, [rbx + {ctx_rsi}]
    mov rbp, [rbx + {ctx_rbp}]
    mov rdx, [rbx + {ctx_rdx}]
    mov rcx, [rbx + {ctx_rcx}]
    mov rdi, [rbx + {ctx_rdi}]
    mov rax, [rbx + {ctx_rax}]
    mov rbx, [rbx + {ctx_rbx}]
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
    mov r12, rdi
    mov rbx, {user_data_address}
    mov [rbx], r12
    xor rdi, rdi
    xor rsi, rsi
    xor rdx, rdx
    xor r10, r10
    xor r8, r8
    xor r9, r9
    mov rax, {sys_ping}
    int {syscall_vector}
    mov r13, {ping_reply}
    cmp rax, r13
    jne genos_user_probe_failed
    mov rax, {sys_abi}
    int {syscall_vector}
    cmp rax, {abi_version}
    jne genos_user_probe_failed
    mov rax, {sys_yield}
    int {syscall_vector}
    mov rdi, rbx
    mov rsi, 8
    xor rdx, rdx
    xor r10, r10
    xor r8, r8
    xor r9, r9
    mov rax, {sys_report}
    int {syscall_vector}
    cmp rax, r12
    jne genos_user_probe_failed
    xor rdi, rdi
    xor rsi, rsi
    xor rdx, rdx
    xor r10, r10
    xor r8, r8
    xor r9, r9
    mov rax, {sys_exit}
    int {syscall_vector}

genos_user_probe_failed:
    mov rdi, 255
    xor rsi, rsi
    xor rdx, rdx
    xor r10, r10
    xor r8, r8
    xor r9, r9
    mov rax, {sys_exit}
    int {syscall_vector}
1:
    pause
    jmp 1b
    .balign 4096
    .global genos_user_text_end
genos_user_text_end:

    .section .text
"#,
    ctx_r15 = const core::mem::offset_of!(UserContext, r15),
    ctx_r14 = const core::mem::offset_of!(UserContext, r14),
    ctx_r13 = const core::mem::offset_of!(UserContext, r13),
    ctx_r12 = const core::mem::offset_of!(UserContext, r12),
    ctx_r11 = const core::mem::offset_of!(UserContext, r11),
    ctx_r10 = const core::mem::offset_of!(UserContext, r10),
    ctx_r9 = const core::mem::offset_of!(UserContext, r9),
    ctx_r8 = const core::mem::offset_of!(UserContext, r8),
    ctx_rdi = const core::mem::offset_of!(UserContext, rdi),
    ctx_rsi = const core::mem::offset_of!(UserContext, rsi),
    ctx_rbp = const core::mem::offset_of!(UserContext, rbp),
    ctx_rbx = const core::mem::offset_of!(UserContext, rbx),
    ctx_rdx = const core::mem::offset_of!(UserContext, rdx),
    ctx_rcx = const core::mem::offset_of!(UserContext, rcx),
    ctx_rax = const core::mem::offset_of!(UserContext, rax),
    ctx_rip = const core::mem::offset_of!(UserContext, rip),
    ctx_cs = const core::mem::offset_of!(UserContext, cs),
    ctx_rflags = const core::mem::offset_of!(UserContext, rflags),
    ctx_rsp = const core::mem::offset_of!(UserContext, rsp),
    ctx_ss = const core::mem::offset_of!(UserContext, ss),
    syscall_vector = const SYSCALL_VECTOR,
    user_data_address = const paging::USER_DATA,
    sys_ping = const syscall::SYSCALL_PING,
    sys_abi = const syscall::SYSCALL_ABI_VERSION,
    sys_exit = const syscall::SYSCALL_EXIT,
    sys_yield = const syscall::SYSCALL_YIELD,
    sys_report = const syscall::SYSCALL_REPORT,
    ping_reply = const syscall::PING_REPLY,
    abi_version = const syscall::USER_ABI_VERSION,
);

unsafe extern "C" {
    fn genos_enter_user_context(context: *const UserContext);
    fn genos_syscall_stub();
    static genos_user_text_start: u8;
    static genos_user_text_end: u8;
}

pub fn syscall_handler() -> unsafe extern "C" fn() {
    genos_syscall_stub
}

pub fn run_probe() -> UserProbeResult {
    let first = require_process(build_process(1, TOKEN_A));
    let second = require_process(build_process(2, TOKEN_B));
    let mut processes = [first, second];
    crate::serial::println("ADDRESS_SPACES_READY count=2");

    let mut live = PROCESS_COUNT;
    let mut cursor = 0usize;
    let mut switches = 0u8;
    while live > 0 && switches < 8 {
        if !processes[cursor].completed {
            run_slice(&mut processes[cursor]);
            switches = switches.saturating_add(1);
            match processes[cursor].event {
                ProcessEvent::Yield => {}
                ProcessEvent::Exit | ProcessEvent::Fault => live -= 1,
                ProcessEvent::None => fail("USER_EVENT_MISSING"),
            }
        }
        cursor = (cursor + 1) % PROCESS_COUNT;
    }
    paging::activate_kernel();

    if !verify_processes(&processes, switches) {
        fail("USER_ISOLATION_FAILED");
    }
    COMPLETED_PROCESSES.store(PROCESS_COUNT as u8, Ordering::Release);
    ADDRESS_SPACES.store(PROCESS_COUNT as u8, Ordering::Release);
    TOTAL_YIELDS.store(
        processes[0].yields.saturating_add(processes[1].yields),
        Ordering::Release,
    );
    PROBE_PASSED.store(true, Ordering::Release);
    crate::serial::println("USER_CONTEXT_RESUME_OK");
    crate::serial::println("USER_ISOLATION_OK");
    crate::serial::println("USERMODE_READY");

    UserProbeResult {
        exit_codes: [processes[0].exit_code, processes[1].exit_code],
    }
}

pub fn probe_passed() -> bool {
    PROBE_PASSED.load(Ordering::Acquire)
}

pub fn process_count() -> u8 {
    COMPLETED_PROCESSES.load(Ordering::Acquire)
}

pub fn address_space_count() -> u8 {
    ADDRESS_SPACES.load(Ordering::Acquire)
}

pub fn yield_count() -> u8 {
    TOTAL_YIELDS.load(Ordering::Acquire)
}

fn build_process(pid: u8, token: u64) -> Result<UserProcess, paging::PagingError> {
    let space = paging::create_user_address_space()?;
    let code_frame = paging::allocate_zeroed_frame()?;
    let data_frame = paging::allocate_zeroed_frame()?;
    let text_start = core::ptr::addr_of!(genos_user_text_start) as u64;
    let text_end = core::ptr::addr_of!(genos_user_text_end) as u64;
    if text_end.saturating_sub(text_start) != paging::PAGE_SIZE {
        return Err(paging::PagingError::InvalidAddress);
    }
    unsafe {
        core::ptr::copy_nonoverlapping(
            text_start as *const u8,
            code_frame as *mut u8,
            paging::PAGE_SIZE as usize,
        );
    }
    paging::map_user_page(space, paging::USER_CODE, code_frame, false, true)?;
    paging::map_user_page(space, paging::USER_DATA, data_frame, true, false)?;
    for index in 0..paging::USER_STACK_PAGES {
        let stack_frame = paging::allocate_zeroed_frame()?;
        paging::map_user_page(
            space,
            paging::USER_STACK_BOTTOM + index as u64 * paging::PAGE_SIZE,
            stack_frame,
            true,
            false,
        )?;
    }

    Ok(UserProcess {
        pid,
        space,
        context: UserContext::initial(token),
        data_frame,
        token,
        event: ProcessEvent::None,
        report: 0,
        exit_code: u8::MAX,
        yields: 0,
        completed: false,
    })
}

fn run_slice(process: &mut UserProcess) {
    process.event = ProcessEvent::None;
    let context = process.context;
    unsafe {
        core::ptr::addr_of_mut!(CURRENT_PROCESS).write(process as *mut UserProcess);
    }
    paging::activate(process.space);
    unsafe { genos_enter_user_context(core::ptr::addr_of!(context)) };
    paging::activate_kernel();
    unsafe {
        core::ptr::addr_of_mut!(CURRENT_PROCESS).write(core::ptr::null_mut());
    }
}

#[no_mangle]
extern "C" fn genos_syscall_rust(frame: *mut UserContext) -> u64 {
    let frame = unsafe { &mut *frame };
    let Some(process) = current_process() else {
        crate::serial::println("USER_PROCESS_MISSING");
        return 1;
    };
    if !valid_user_frame(frame, process) {
        process.event = ProcessEvent::Fault;
        process.exit_code = 254;
        process.completed = true;
        crate::serial::println("USER_CONTEXT_INVALID");
        return 1;
    }
    if !CONTEXT_PASSED.swap(true, Ordering::AcqRel) {
        crate::serial::println("USER_CONTEXT_OK");
    }

    let number = frame.rax;
    let args = [
        frame.rdi, frame.rsi, frame.rdx, frame.r10, frame.r8, frame.r9,
    ];
    match syscall::dispatch(number, args) {
        Ok(SyscallAction::Return(value)) => {
            if number == syscall::SYSCALL_PING && value == syscall::PING_REPLY {
                let count = PING_COUNT.fetch_add(1, Ordering::AcqRel) + 1;
                if count == PROCESS_COUNT as u8 {
                    crate::serial::println("USER_SYSCALL_OK");
                }
            }
            if number == syscall::SYSCALL_ABI_VERSION && value == syscall::USER_ABI_VERSION {
                let count = ABI_COUNT.fetch_add(1, Ordering::AcqRel) + 1;
                if count == PROCESS_COUNT as u8 {
                    crate::serial::println("USER_ABI_OK");
                }
            }
            frame.rax = value;
            0
        }
        Ok(SyscallAction::Yield) => {
            frame.rax = 0;
            process.context = *frame;
            process.event = ProcessEvent::Yield;
            process.yields = process.yields.saturating_add(1);
            crate::serial::print("USER_YIELD pid=");
            crate::serial::print_u64(process.pid as u64);
            crate::serial::println("");
            1
        }
        Ok(SyscallAction::Report { address, length }) => {
            if let Some(value) = copy_user_u64(process, address, length) {
                process.report = value;
                frame.rax = value;
                let count = REPORT_COUNT.fetch_add(1, Ordering::AcqRel) + 1;
                if count == PROCESS_COUNT as u8 {
                    crate::serial::println("USER_COPY_OK");
                }
            } else {
                frame.rax = syscall::error_code(syscall::SyscallError::InvalidArgument);
            }
            0
        }
        Ok(SyscallAction::Exit(code)) => {
            process.event = ProcessEvent::Exit;
            process.exit_code = code;
            process.completed = true;
            crate::serial::print("USER_EXIT pid=");
            crate::serial::print_u64(process.pid as u64);
            crate::serial::print(" code=");
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

fn current_process() -> Option<&'static mut UserProcess> {
    let process = unsafe { *core::ptr::addr_of!(CURRENT_PROCESS) };
    if process.is_null() {
        None
    } else {
        Some(unsafe { &mut *process })
    }
}

fn valid_user_frame(frame: &UserContext, process: &UserProcess) -> bool {
    frame.cs == arch::USER_CODE_SELECTOR as u64
        && frame.ss == arch::USER_DATA_SELECTOR as u64
        && frame.rip >= paging::USER_CODE
        && frame.rip < paging::USER_CODE + paging::PAGE_SIZE
        && frame.rsp > paging::USER_STACK_BOTTOM
        && frame.rsp <= paging::USER_STACK_TOP
        && paging::active_root() == process.space.root()
}

fn copy_user_u64(process: &UserProcess, address: u64, length: u64) -> Option<u64> {
    if length != 8
        || !syscall::validate_user_buffer(address, length, paging::USER_DATA, paging::PAGE_SIZE)
    {
        return None;
    }
    let physical = paging::translate(process.space, address)?;
    let expected = process.data_frame + (address - paging::USER_DATA);
    if physical != expected {
        return None;
    }
    Some(unsafe { core::ptr::read_unaligned(address as *const u64) })
}

fn verify_processes(processes: &[UserProcess; PROCESS_COUNT], switches: u8) -> bool {
    let first_data = unsafe { core::ptr::read_volatile(processes[0].data_frame as *const u64) };
    let second_data = unsafe { core::ptr::read_volatile(processes[1].data_frame as *const u64) };
    switches == 4
        && processes[0].space.root() != processes[1].space.root()
        && processes[0].data_frame != processes[1].data_frame
        && paging::translate(processes[0].space, paging::USER_DATA) == Some(processes[0].data_frame)
        && paging::translate(processes[1].space, paging::USER_DATA) == Some(processes[1].data_frame)
        && paging::translate(processes[0].space, paging::USER_STACK_GUARD).is_none()
        && paging::translate(processes[1].space, paging::USER_STACK_GUARD).is_none()
        && processes.iter().all(|process| {
            process.completed
                && process.exit_code == 0
                && process.yields == 1
                && process.report == process.token
        })
        && first_data == TOKEN_A
        && second_data == TOKEN_B
        && PING_COUNT.load(Ordering::Acquire) == PROCESS_COUNT as u8
        && ABI_COUNT.load(Ordering::Acquire) == PROCESS_COUNT as u8
        && REPORT_COUNT.load(Ordering::Acquire) == PROCESS_COUNT as u8
}

fn require_process(result: Result<UserProcess, paging::PagingError>) -> UserProcess {
    match result {
        Ok(process) => process,
        Err(_) => fail("USER_PROCESS_BUILD_FAILED"),
    }
}

fn fail(marker: &str) -> ! {
    paging::activate_kernel();
    crate::serial::println(marker);
    arch::halt_loop();
}
