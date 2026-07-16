use core::arch::global_asm;
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};

use kernel::{
    display::FixedText,
    elf::{ElfImage, FLAG_EXECUTE, FLAG_READ, FLAG_WRITE},
    syscall::{self, SyscallAction},
};

use crate::{arch, memory, paging};

pub const SYSCALL_VECTOR: usize = 0x80;
const PROCESS_COUNT: usize = 3;
const HEALTHY_PROCESS_COUNT: u8 = 2;
const FAULT_EXIT_CODE: u8 = 128 + 14;
const TOKEN_FAULT: u64 = 0xffff_ffff_ffff_fff0;
const TOKEN_A: u64 = 0x1111_aaaa_1111_aaaa;
const TOKEN_B: u64 = 0x2222_bbbb_2222_bbbb;
const TOKEN_DYNAMIC_BASE: u64 = 0x3333_cccc_3333_0000;
const TOKEN_HOLD_BIT: u64 = 1 << 63;
pub const MAX_ASYNC_PROCESSES: usize = 4;

static PROBE_PASSED: AtomicBool = AtomicBool::new(false);
static ELF_READY: AtomicBool = AtomicBool::new(false);
static CONTEXT_PASSED: AtomicBool = AtomicBool::new(false);
static PING_COUNT: AtomicU8 = AtomicU8::new(0);
static ABI_COUNT: AtomicU8 = AtomicU8::new(0);
static REPORT_COUNT: AtomicU8 = AtomicU8::new(0);
static WRITE_COUNT: AtomicU8 = AtomicU8::new(0);
static COMPLETED_PROCESSES: AtomicU8 = AtomicU8::new(0);
static ADDRESS_SPACES: AtomicU8 = AtomicU8::new(0);
static TOTAL_YIELDS: AtomicU8 = AtomicU8::new(0);
static TOTAL_PREEMPTIONS: AtomicU64 = AtomicU64::new(0);
static LOCAL_FAULTS: AtomicU8 = AtomicU8::new(0);
static COMPLETION_SEQUENCE: AtomicU8 = AtomicU8::new(0);
static DYNAMIC_PROCESSES: AtomicU8 = AtomicU8::new(0);
static ACTIVE_PROCESSES: AtomicU8 = AtomicU8::new(0);
static RECLAIMED_SPACES: AtomicU8 = AtomicU8::new(0);
static RECLAIMED_FRAMES: AtomicU64 = AtomicU64::new(0);
static NEXT_DYNAMIC_PID: AtomicU8 = AtomicU8::new(4);
static mut USER_ELF_ADDRESS: u64 = 0;
static mut USER_ELF_LENGTH: usize = 0;
static mut CURRENT_PROCESS: *mut UserProcess = core::ptr::null_mut();

#[derive(Clone, Copy)]
#[repr(C)]
pub(crate) struct UserContext {
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
    const fn initial(token: u64, entry: u64) -> Self {
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
            rip: entry,
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
    Preempt,
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
    preemptions: u64,
    fault_vector: u8,
    fault_error: u64,
    fault_address: u64,
    completion_order: u8,
    preemption_armed: bool,
    elf_segments: u8,
    elf_pages: u8,
    executable_start: u64,
    executable_end: u64,
    output: FixedText,
    output_pending: bool,
    frames_released: bool,
    killed: bool,
    completed: bool,
}

#[derive(Clone, Copy)]
struct LoadedImage {
    entry: u64,
    data_frame: u64,
    segment_count: u8,
    page_count: u8,
    executable_start: u64,
    executable_end: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ProcessBuildError {
    InvalidElf,
    InvalidLayout,
    Paging,
}

#[derive(Clone, Copy)]
pub struct UserProbeResult {
    pub exit_codes: [u8; PROCESS_COUNT],
}

#[derive(Clone, Copy)]
pub struct LaunchResult {
    pub pid: u8,
    pub exit_code: u8,
    pub preemptions: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LaunchError {
    ImageUnavailable,
    ProcessBuildFailed,
    ProcessFaulted,
    InvalidResult,
    ProcessTableFull,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManagedState {
    Ready,
    Exited,
    Faulted,
    Killed,
}

#[derive(Clone, Copy)]
pub struct ProcessUpdate {
    pub task_id: u32,
    pub pid: u8,
    pub state: ManagedState,
    pub exit_code: u8,
    pub preemptions: u64,
    pub output: FixedText,
}

#[derive(Clone, Copy)]
pub struct WaitResult {
    pub pid: u8,
    pub state: ManagedState,
    pub exit_code: u8,
    pub preemptions: u64,
}

struct ManagedProcess {
    task_id: u32,
    process: UserProcess,
}

pub struct ProcessManager {
    slots: [Option<ManagedProcess>; MAX_ASYNC_PROCESSES],
    cursor: usize,
}

impl ProcessManager {
    pub const fn new() -> Self {
        Self {
            slots: [const { None }; MAX_ASYNC_PROCESSES],
            cursor: 0,
        }
    }

    pub fn spawn_init(&mut self, task_id: u32, hold: bool) -> Result<u8, LaunchError> {
        let slot = self
            .slots
            .iter()
            .position(Option::is_none)
            .ok_or(LaunchError::ProcessTableFull)?;
        let elf_bytes = user_elf()?;
        let pid = NEXT_DYNAMIC_PID.fetch_add(1, Ordering::AcqRel);
        let token = TOKEN_DYNAMIC_BASE | pid as u64 | if hold { TOKEN_HOLD_BIT } else { 0 };
        let process =
            build_process(pid, token, elf_bytes).map_err(|_| LaunchError::ProcessBuildFailed)?;
        self.slots[slot] = Some(ManagedProcess { task_id, process });
        DYNAMIC_PROCESSES.fetch_add(1, Ordering::AcqRel);
        crate::serial::print("USER_ASYNC_SPAWN pid=");
        crate::serial::print_u64(pid as u64);
        crate::serial::print(" task=");
        crate::serial::print_u64(task_id as u64);
        crate::serial::print(" hold=");
        crate::serial::print_u64(hold as u64);
        crate::serial::println("");
        Ok(pid)
    }

    pub fn poll(&mut self) -> Option<ProcessUpdate> {
        for offset in 1..=MAX_ASYNC_PROCESSES {
            let index = (self.cursor + offset) % MAX_ASYNC_PROCESSES;
            let Some(managed) = self.slots[index].as_mut() else {
                continue;
            };
            if managed.process.completed {
                continue;
            }
            self.cursor = index;
            run_slice(&mut managed.process);
            let output = if managed.process.output_pending {
                managed.process.output_pending = false;
                managed.process.output
            } else {
                FixedText::empty()
            };
            let state = match managed.process.event {
                ProcessEvent::Yield => {
                    TOTAL_YIELDS.fetch_add(1, Ordering::AcqRel);
                    ManagedState::Ready
                }
                ProcessEvent::Preempt => {
                    TOTAL_PREEMPTIONS.fetch_add(1, Ordering::AcqRel);
                    ManagedState::Ready
                }
                ProcessEvent::Exit => ManagedState::Exited,
                ProcessEvent::Fault => ManagedState::Faulted,
                ProcessEvent::None => return None,
            };
            if managed.process.completed && reclaim_process(&mut managed.process).is_err() {
                fail("USER_RECLAIM_FAILED");
            }
            return Some(ProcessUpdate {
                task_id: managed.task_id,
                pid: managed.process.pid,
                state,
                exit_code: managed.process.exit_code,
                preemptions: managed.process.preemptions,
                output,
            });
        }
        None
    }

    pub fn kill(&mut self, task_id: u32) -> Result<ProcessUpdate, LaunchError> {
        let managed = self
            .slots
            .iter_mut()
            .flatten()
            .find(|managed| managed.task_id == task_id)
            .ok_or(LaunchError::ImageUnavailable)?;
        if managed.process.completed {
            return Err(LaunchError::InvalidResult);
        }
        managed.process.completed = true;
        managed.process.killed = true;
        managed.process.event = ProcessEvent::Exit;
        managed.process.exit_code = 137;
        managed.process.completion_order = COMPLETION_SEQUENCE.fetch_add(1, Ordering::AcqRel) + 1;
        reclaim_process(&mut managed.process).map_err(|_| LaunchError::InvalidResult)?;
        crate::serial::print("USER_KILLED pid=");
        crate::serial::print_u64(managed.process.pid as u64);
        crate::serial::print(" task=");
        crate::serial::print_u64(task_id as u64);
        crate::serial::println("");
        Ok(ProcessUpdate {
            task_id,
            pid: managed.process.pid,
            state: ManagedState::Killed,
            exit_code: managed.process.exit_code,
            preemptions: managed.process.preemptions,
            output: FixedText::empty(),
        })
    }

    pub fn wait(&mut self, task_id: u32) -> Result<WaitResult, LaunchError> {
        let index = self
            .slots
            .iter()
            .position(|slot| {
                slot.as_ref()
                    .is_some_and(|managed| managed.task_id == task_id)
            })
            .ok_or(LaunchError::ImageUnavailable)?;
        let managed = self.slots[index]
            .as_ref()
            .ok_or(LaunchError::ImageUnavailable)?;
        if !managed.process.completed {
            return Err(LaunchError::InvalidResult);
        }
        let result = WaitResult {
            pid: managed.process.pid,
            state: if managed.process.killed {
                ManagedState::Killed
            } else if managed.process.event == ProcessEvent::Fault {
                ManagedState::Faulted
            } else {
                ManagedState::Exited
            },
            exit_code: managed.process.exit_code,
            preemptions: managed.process.preemptions,
        };
        self.slots[index] = None;
        crate::serial::print("USER_WAIT_REAPED pid=");
        crate::serial::print_u64(result.pid as u64);
        crate::serial::println("");
        Ok(result)
    }

    pub fn live_count(&self) -> usize {
        self.slots
            .iter()
            .flatten()
            .filter(|managed| !managed.process.completed)
            .count()
    }
}

impl Default for ProcessManager {
    fn default() -> Self {
        Self::new()
    }
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

    .global genos_leave_userspace
genos_leave_userspace:
    mov rsp, [rip + genos_user_return_rsp]
    jmp [rip + genos_user_return_rip]

    .section .bss
    .balign 8
genos_user_return_rsp:
    .quad 0
genos_user_return_rip:
    .quad 0

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
);

unsafe extern "C" {
    fn genos_enter_user_context(context: *const UserContext);
    fn genos_syscall_stub();
}

pub fn syscall_handler() -> unsafe extern "C" fn() {
    genos_syscall_stub
}

pub fn run_probe(elf_bytes: &'static [u8]) -> UserProbeResult {
    unsafe {
        core::ptr::addr_of_mut!(USER_ELF_ADDRESS).write(elf_bytes.as_ptr() as u64);
        core::ptr::addr_of_mut!(USER_ELF_LENGTH).write(elf_bytes.len());
    }
    let faulting = require_process(build_process(1, TOKEN_FAULT, elf_bytes));
    ELF_READY.store(true, Ordering::Release);
    crate::serial::print("USER_ELF_VALIDATED entry=0x");
    crate::serial::print_hex(faulting.context.rip);
    crate::serial::print(" segments=");
    crate::serial::print_u64(faulting.elf_segments as u64);
    crate::serial::print(" pages=");
    crate::serial::print_u64(faulting.elf_pages as u64);
    crate::serial::print(" bytes=");
    crate::serial::print_u64(elf_bytes.len() as u64);
    crate::serial::println("");
    let first = require_process(build_process(2, TOKEN_A, elf_bytes));
    let second = require_process(build_process(3, TOKEN_B, elf_bytes));
    let mut processes = [faulting, first, second];
    crate::serial::println("ADDRESS_SPACES_READY count=3");

    let mut live = PROCESS_COUNT;
    let mut cursor = 0usize;
    let mut switches = 0u8;
    while live > 0 && switches < 16 {
        if !processes[cursor].completed {
            run_slice(&mut processes[cursor]);
            switches = switches.saturating_add(1);
            match processes[cursor].event {
                ProcessEvent::Yield | ProcessEvent::Preempt => {}
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
    let result = UserProbeResult {
        exit_codes: [
            processes[0].exit_code,
            processes[1].exit_code,
            processes[2].exit_code,
        ],
    };
    COMPLETED_PROCESSES.store(PROCESS_COUNT as u8, Ordering::Release);
    TOTAL_YIELDS.store(
        processes
            .iter()
            .fold(0u8, |total, process| total.saturating_add(process.yields)),
        Ordering::Release,
    );
    TOTAL_PREEMPTIONS.store(
        processes.iter().fold(0u64, |total, process| {
            total.saturating_add(process.preemptions)
        }),
        Ordering::Release,
    );
    for process in &mut processes {
        if reclaim_process(process).is_err() {
            fail("USER_RECLAIM_FAILED");
        }
    }
    crate::serial::println("USER_RECLAIM_OK");
    PROBE_PASSED.store(true, Ordering::Release);
    crate::serial::println("USER_CONTEXT_RESUME_OK");
    crate::serial::println("USER_PREEMPT_OK");
    crate::serial::println("USER_FAULT_ISOLATED");
    crate::serial::println("USER_ISOLATION_OK");
    crate::serial::println("USERMODE_READY");

    result
}

pub fn probe_passed() -> bool {
    PROBE_PASSED.load(Ordering::Acquire)
}

pub fn elf_ready() -> bool {
    ELF_READY.load(Ordering::Acquire)
}

pub fn process_count() -> u8 {
    COMPLETED_PROCESSES
        .load(Ordering::Acquire)
        .saturating_add(DYNAMIC_PROCESSES.load(Ordering::Acquire))
}

pub fn address_space_count() -> u8 {
    ADDRESS_SPACES.load(Ordering::Acquire)
}

pub fn yield_count() -> u8 {
    TOTAL_YIELDS.load(Ordering::Acquire)
}

pub fn preemption_count() -> u64 {
    TOTAL_PREEMPTIONS.load(Ordering::Acquire)
}

pub fn local_fault_count() -> u8 {
    LOCAL_FAULTS.load(Ordering::Acquire)
}

pub fn active_process_count() -> u8 {
    ACTIVE_PROCESSES.load(Ordering::Acquire)
}

pub fn reclaimed_space_count() -> u8 {
    RECLAIMED_SPACES.load(Ordering::Acquire)
}

pub fn reclaimed_frame_count() -> u64 {
    RECLAIMED_FRAMES.load(Ordering::Acquire)
}

pub fn launch_init() -> Result<LaunchResult, LaunchError> {
    let elf_bytes = user_elf()?;
    let pid = NEXT_DYNAMIC_PID.fetch_add(1, Ordering::AcqRel);
    let token = TOKEN_DYNAMIC_BASE | pid as u64;
    let mut process =
        build_process(pid, token, elf_bytes).map_err(|_| LaunchError::ProcessBuildFailed)?;
    crate::serial::print("USER_ELF_LAUNCH pid=");
    crate::serial::print_u64(pid as u64);
    crate::serial::println(" image=INIT.ELF");

    for _ in 0..8 {
        if process.completed {
            break;
        }
        run_slice(&mut process);
        if process.event == ProcessEvent::Fault {
            paging::activate_kernel();
            let _ = reclaim_process(&mut process);
            return Err(LaunchError::ProcessFaulted);
        }
    }
    paging::activate_kernel();
    if !process.completed
        || process.event != ProcessEvent::Exit
        || process.exit_code != 0
        || process.report != token
        || process.preemptions == 0
    {
        let _ = reclaim_process(&mut process);
        return Err(LaunchError::InvalidResult);
    }

    let result = LaunchResult {
        pid,
        exit_code: process.exit_code,
        preemptions: process.preemptions,
    };
    reclaim_process(&mut process).map_err(|_| LaunchError::InvalidResult)?;
    DYNAMIC_PROCESSES.fetch_add(1, Ordering::AcqRel);
    TOTAL_PREEMPTIONS.fetch_add(result.preemptions, Ordering::AcqRel);
    crate::serial::print("USER_ELF_LAUNCH_OK pid=");
    crate::serial::print_u64(pid as u64);
    crate::serial::print(" preemptions=");
    crate::serial::print_u64(result.preemptions);
    crate::serial::println("");
    Ok(result)
}

pub fn run_lifecycle_probe() {
    const NORMAL_TASK: u32 = 0x1000;
    const HOLD_TASK: u32 = 0x1001;

    let reclaimed_before = reclaimed_frame_count();
    let mut manager = ProcessManager::new();

    if manager.spawn_init(NORMAL_TASK, false).is_err() {
        fail("USER_ASYNC_SPAWN_FAILED");
    }
    let first = manager.poll();
    let second = manager.poll();
    if !matches!(first, Some(update) if update.state == ManagedState::Ready)
        || !matches!(second, Some(update) if update.state == ManagedState::Exited && update.exit_code == 0 && !update.output.is_empty())
    {
        fail("USER_ASYNC_EXIT_FAILED");
    }
    if !matches!(manager.wait(NORMAL_TASK), Ok(result) if result.state == ManagedState::Exited && result.exit_code == 0)
    {
        fail("USER_ASYNC_WAIT_FAILED");
    }
    crate::serial::println("USER_ASYNC_EXIT_OK");
    crate::serial::println("USER_OUTPUT_ASYNC_OK");

    if manager.spawn_init(HOLD_TASK, true).is_err() {
        fail("USER_ASYNC_HOLD_FAILED");
    }
    let first = manager.poll();
    let second = manager.poll();
    if !matches!(first, Some(update) if update.state == ManagedState::Ready)
        || !matches!(second, Some(update) if update.state == ManagedState::Ready && !update.output.is_empty())
    {
        fail("USER_ASYNC_HOLD_FAILED");
    }
    if !matches!(manager.kill(HOLD_TASK), Ok(update) if update.state == ManagedState::Killed && update.exit_code == 137)
    {
        fail("USER_KILL_FAILED");
    }
    if !matches!(manager.wait(HOLD_TASK), Ok(result) if result.state == ManagedState::Killed && result.exit_code == 137)
    {
        fail("USER_WAIT_FAILED");
    }
    if manager.live_count() != 0
        || active_process_count() != 0
        || reclaimed_frame_count() < reclaimed_before + 20
    {
        fail("USER_RECLAIM_FAILED");
    }
    crate::serial::println("USER_KILL_OK");
    crate::serial::println("USER_WAIT_OK");
    crate::serial::println("USER_ASYNC_LIFECYCLE_OK");
}

fn user_elf() -> Result<&'static [u8], LaunchError> {
    let address = unsafe { *core::ptr::addr_of!(USER_ELF_ADDRESS) };
    let length = unsafe { *core::ptr::addr_of!(USER_ELF_LENGTH) };
    if address == 0 || length == 0 {
        return Err(LaunchError::ImageUnavailable);
    }
    Ok(unsafe { core::slice::from_raw_parts(address as *const u8, length) })
}

fn load_elf(space: paging::AddressSpace, bytes: &[u8]) -> Result<LoadedImage, ProcessBuildError> {
    let image = ElfImage::parse(bytes).map_err(|_| ProcessBuildError::InvalidElf)?;
    if image.entry() < paging::USER_CODE || image.entry() >= paging::USER_STACK_GUARD {
        return Err(ProcessBuildError::InvalidLayout);
    }

    let mut mapped_pages = 0u64;
    let mut entry_is_executable = false;
    let mut executable_start = u64::MAX;
    let mut executable_end = 0u64;
    let mut data_frame = 0u64;
    let mut segment_count = 0u8;
    let mut page_count = 0u8;
    for segment in image.segments() {
        let segment = segment.map_err(|_| ProcessBuildError::InvalidElf)?;
        let writable = segment.flags & FLAG_WRITE != 0;
        let executable = segment.flags & FLAG_EXECUTE != 0;
        if segment.flags & FLAG_READ == 0
            || segment.flags & !(FLAG_READ | FLAG_WRITE | FLAG_EXECUTE) != 0
            || (writable && executable)
            || segment.align < paging::PAGE_SIZE
            || segment.virtual_address & (segment.align - 1)
                != segment.file_offset & (segment.align - 1)
            || segment.virtual_address & (paging::PAGE_SIZE - 1) != 0
        {
            return Err(ProcessBuildError::InvalidLayout);
        }
        let segment_end = segment
            .virtual_address
            .checked_add(segment.memory_size)
            .ok_or(ProcessBuildError::InvalidLayout)?;
        if segment.virtual_address < paging::USER_CODE || segment_end > paging::USER_STACK_GUARD {
            return Err(ProcessBuildError::InvalidLayout);
        }
        if executable && image.entry() >= segment.virtual_address && image.entry() < segment_end {
            entry_is_executable = true;
        }
        if executable {
            executable_start = executable_start.min(segment.virtual_address);
            executable_end = executable_end.max(segment_end);
        }

        let pages = segment.memory_size.div_ceil(paging::PAGE_SIZE);
        if pages == 0 || pages > 16 {
            return Err(ProcessBuildError::InvalidLayout);
        }
        for page in 0..pages {
            let virtual_address = segment.virtual_address + page * paging::PAGE_SIZE;
            let image_page = (virtual_address - paging::USER_CODE) / paging::PAGE_SIZE;
            if image_page >= 64 || mapped_pages & (1 << image_page) != 0 {
                return Err(ProcessBuildError::InvalidLayout);
            }
            mapped_pages |= 1 << image_page;

            let frame = paging::allocate_zeroed_frame().map_err(|_| ProcessBuildError::Paging)?;
            let file_offset = (page * paging::PAGE_SIZE) as usize;
            if file_offset < segment.file_data.len() {
                let copy_len =
                    (segment.file_data.len() - file_offset).min(paging::PAGE_SIZE as usize);
                unsafe {
                    core::ptr::copy_nonoverlapping(
                        segment.file_data.as_ptr().add(file_offset),
                        frame as *mut u8,
                        copy_len,
                    );
                }
            }
            if paging::map_user_page(space, virtual_address, frame, writable, executable).is_err() {
                let _ = memory::free_frame(frame);
                return Err(ProcessBuildError::Paging);
            }
            if virtual_address == paging::USER_DATA
                && writable
                && segment.memory_size >= 16
                && segment.file_data.len() >= 16
            {
                data_frame = frame;
            }
            page_count = page_count
                .checked_add(1)
                .ok_or(ProcessBuildError::InvalidLayout)?;
        }
        segment_count = segment_count
            .checked_add(1)
            .ok_or(ProcessBuildError::InvalidLayout)?;
    }

    if !entry_is_executable
        || data_frame == 0
        || paging::translate(space, paging::USER_DATA) != Some(data_frame)
    {
        return Err(ProcessBuildError::InvalidLayout);
    }
    Ok(LoadedImage {
        entry: image.entry(),
        data_frame,
        segment_count,
        page_count,
        executable_start,
        executable_end,
    })
}

fn build_process(pid: u8, token: u64, elf_bytes: &[u8]) -> Result<UserProcess, ProcessBuildError> {
    let space = paging::create_user_address_space().map_err(|_| ProcessBuildError::Paging)?;
    let loaded = match load_elf(space, elf_bytes) {
        Ok(loaded) => loaded,
        Err(error) => {
            let _ = paging::destroy_user_address_space(space);
            return Err(error);
        }
    };
    for index in 0..paging::USER_STACK_PAGES {
        let stack_frame = match paging::allocate_zeroed_frame() {
            Ok(frame) => frame,
            Err(_) => {
                let _ = paging::destroy_user_address_space(space);
                return Err(ProcessBuildError::Paging);
            }
        };
        if paging::map_user_page(
            space,
            paging::USER_STACK_BOTTOM + index as u64 * paging::PAGE_SIZE,
            stack_frame,
            true,
            false,
        )
        .is_err()
        {
            let _ = memory::free_frame(stack_frame);
            let _ = paging::destroy_user_address_space(space);
            return Err(ProcessBuildError::Paging);
        }
    }

    crate::serial::print("USER_ELF_LOADED pid=");
    crate::serial::print_u64(pid as u64);
    crate::serial::print(" root=0x");
    crate::serial::print_hex(space.root());
    crate::serial::println("");

    ADDRESS_SPACES.fetch_add(1, Ordering::AcqRel);
    ACTIVE_PROCESSES.fetch_add(1, Ordering::AcqRel);

    Ok(UserProcess {
        pid,
        space,
        context: UserContext::initial(token, loaded.entry),
        data_frame: loaded.data_frame,
        token,
        event: ProcessEvent::None,
        report: 0,
        exit_code: u8::MAX,
        yields: 0,
        preemptions: 0,
        fault_vector: 0,
        fault_error: 0,
        fault_address: 0,
        completion_order: 0,
        preemption_armed: false,
        elf_segments: loaded.segment_count,
        elf_pages: loaded.page_count,
        executable_start: loaded.executable_start,
        executable_end: loaded.executable_end,
        output: FixedText::empty(),
        output_pending: false,
        frames_released: false,
        killed: false,
        completed: false,
    })
}

fn reclaim_process(process: &mut UserProcess) -> Result<u64, paging::PagingError> {
    if process.frames_released {
        return Ok(0);
    }
    paging::activate_kernel();
    let released = paging::destroy_user_address_space(process.space)?;
    process.frames_released = true;
    ACTIVE_PROCESSES.fetch_sub(1, Ordering::AcqRel);
    RECLAIMED_SPACES.fetch_add(1, Ordering::AcqRel);
    RECLAIMED_FRAMES.fetch_add(released, Ordering::AcqRel);
    crate::serial::print("USER_FRAMES_RECLAIMED pid=");
    crate::serial::print_u64(process.pid as u64);
    crate::serial::print(" frames=");
    crate::serial::print_u64(released);
    crate::serial::println("");
    Ok(released)
}

fn run_slice(process: &mut UserProcess) {
    let restore_interrupts = arch::interrupts_enabled();
    arch::disable_interrupts();
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
    if restore_interrupts {
        arch::enable_interrupts();
    }
}

pub(crate) fn timer_preempt(frame: *mut UserContext) -> bool {
    let Some(process) = current_process() else {
        return false;
    };
    let frame = unsafe { &mut *frame };
    if !process.preemption_armed {
        return false;
    }
    if !valid_user_frame(frame, process) {
        terminate_process_fault(process, 13, 0, frame.rip, 0);
        return true;
    }

    process.context = *frame;
    process.event = ProcessEvent::Preempt;
    process.preemptions = process.preemptions.saturating_add(1);
    unsafe {
        core::ptr::write_volatile((process.data_frame + 8) as *mut u64, process.preemptions);
    }
    if process.preemptions == 1 {
        crate::serial::print("USER_PREEMPT pid=");
        crate::serial::print_u64(process.pid as u64);
        crate::serial::println("");
    }
    true
}

pub(crate) fn terminate_current_fault(vector: u8, error: u64, rip: u64, cr2: u64) -> bool {
    let Some(process) = current_process() else {
        return false;
    };
    terminate_process_fault(process, vector, error, rip, cr2);
    true
}

fn terminate_process_fault(process: &mut UserProcess, vector: u8, error: u64, rip: u64, cr2: u64) {
    process.event = ProcessEvent::Fault;
    process.exit_code = 128u8.saturating_add(vector);
    process.fault_vector = vector;
    process.fault_error = error;
    process.fault_address = cr2;
    process.completed = true;
    process.completion_order = COMPLETION_SEQUENCE.fetch_add(1, Ordering::AcqRel) + 1;
    LOCAL_FAULTS.fetch_add(1, Ordering::AcqRel);
    crate::serial::print("USER_FAULT_TERMINATED pid=");
    crate::serial::print_u64(process.pid as u64);
    crate::serial::print(" vector=");
    crate::serial::print_u64(vector as u64);
    crate::serial::print(" error=0x");
    crate::serial::print_hex(error);
    crate::serial::print(" rip=0x");
    crate::serial::print_hex(rip);
    crate::serial::print(" cr2=0x");
    crate::serial::print_hex(cr2);
    crate::serial::println("");
}

#[no_mangle]
extern "C" fn genos_syscall_rust(frame: *mut UserContext) -> u64 {
    let frame = unsafe { &mut *frame };
    let Some(process) = current_process() else {
        crate::serial::println("USER_PROCESS_MISSING");
        return 1;
    };
    if !valid_user_frame(frame, process) {
        terminate_process_fault(process, 13, 0, frame.rip, 0);
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
                process.preemption_armed = true;
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
                if count == HEALTHY_PROCESS_COUNT {
                    crate::serial::println("USER_COPY_OK");
                }
            } else {
                frame.rax = syscall::error_code(syscall::SyscallError::InvalidArgument);
            }
            0
        }
        Ok(SyscallAction::Write { address, length }) => {
            if let Some(text) = copy_user_text(process, address, length) {
                process.output = text;
                process.output_pending = true;
                frame.rax = length;
                let count = WRITE_COUNT.fetch_add(1, Ordering::AcqRel) + 1;
                crate::serial::print("USER_OUTPUT pid=");
                crate::serial::print_u64(process.pid as u64);
                crate::serial::print(" text=");
                crate::serial::println(text.as_str());
                if count == HEALTHY_PROCESS_COUNT {
                    crate::serial::println("USER_OUTPUT_OK");
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
            process.completion_order = COMPLETION_SEQUENCE.fetch_add(1, Ordering::AcqRel) + 1;
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
        && frame.rip >= process.executable_start
        && frame.rip < process.executable_end
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

fn copy_user_text(process: &UserProcess, address: u64, length: u64) -> Option<FixedText> {
    if length == 0
        || length > 80
        || !syscall::validate_user_buffer(
            address,
            length,
            paging::USER_CODE,
            paging::USER_STACK_TOP - paging::USER_CODE,
        )
    {
        return None;
    }
    let length = length as usize;
    let mut bytes = [0u8; 80];
    for (index, slot) in bytes.iter_mut().take(length).enumerate() {
        let virtual_address = address.checked_add(index as u64)?;
        paging::translate(process.space, virtual_address)?;
        let byte = unsafe { core::ptr::read_volatile(virtual_address as *const u8) };
        *slot = if byte.is_ascii() && !byte.is_ascii_control() {
            byte
        } else {
            b'?'
        };
    }
    let text = core::str::from_utf8(&bytes[..length]).ok()?;
    Some(FixedText::from_str(text))
}

fn verify_processes(processes: &[UserProcess; PROCESS_COUNT], switches: u8) -> bool {
    let roots_are_distinct = processes.iter().enumerate().all(|(index, process)| {
        processes
            .iter()
            .skip(index + 1)
            .all(|other| process.space.root() != other.space.root())
    });
    let frames_are_distinct = processes.iter().enumerate().all(|(index, process)| {
        processes
            .iter()
            .skip(index + 1)
            .all(|other| process.data_frame != other.data_frame)
    });
    let mappings_are_private = processes.iter().all(|process| {
        paging::translate(process.space, paging::USER_DATA) == Some(process.data_frame)
            && paging::translate(process.space, paging::USER_STACK_GUARD).is_none()
            && process.elf_segments == 2
            && process.elf_pages == 2
            && unsafe { core::ptr::read_volatile(process.data_frame as *const u64) }
                == process.token
            && unsafe { core::ptr::read_volatile((process.data_frame + 8) as *const u64) }
                == process.preemptions
    });
    let faulting = &processes[0];
    let healthy = &processes[1..];

    (6..=12).contains(&switches)
        && roots_are_distinct
        && frames_are_distinct
        && mappings_are_private
        && faulting.completed
        && faulting.exit_code == FAULT_EXIT_CODE
        && faulting.fault_vector == 14
        && faulting.fault_error == 0x6
        && faulting.fault_address == paging::USER_STACK_GUARD
        && (1..=4).contains(&faulting.preemptions)
        && faulting.preemption_armed
        && faulting.yields == 0
        && faulting.report == 0
        && faulting.completion_order == 1
        && healthy.iter().all(|process| {
            process.completed
                && process.exit_code == 0
                && process.fault_vector == 0
                && (1..=4).contains(&process.preemptions)
                && process.preemption_armed
                && process.yields == 0
                && process.report == process.token
                && process.completion_order > faulting.completion_order
        })
        && PING_COUNT.load(Ordering::Acquire) == PROCESS_COUNT as u8
        && ABI_COUNT.load(Ordering::Acquire) == PROCESS_COUNT as u8
        && REPORT_COUNT.load(Ordering::Acquire) == HEALTHY_PROCESS_COUNT
        && WRITE_COUNT.load(Ordering::Acquire) == HEALTHY_PROCESS_COUNT
        && LOCAL_FAULTS.load(Ordering::Acquire) == 1
}

fn require_process(result: Result<UserProcess, ProcessBuildError>) -> UserProcess {
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
