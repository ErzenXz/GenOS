use core::arch::global_asm;
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};

use genos_abi::{
    UserChannelMessage, UserDirectoryEntry, UserFileStat, UserInputEvent, UserProcessHeader,
    UserSystemInfo, USER_ABI_VERSION, USER_CHANNEL_MESSAGE_SIZE, USER_ENDPOINT_HANDLE_CAPACITY,
    USER_ENDPOINT_QUEUE_CAPACITY, USER_FILE_HANDLE_CAPACITY, USER_FILE_KIND_REGULAR,
    USER_FILE_READ_MAX, USER_FILE_RIGHTS_MASK, USER_FILE_RIGHT_READ, USER_FILE_RIGHT_WRITE,
    USER_FILE_WRITE_MAX, USER_INPUT_MASK_ALL, USER_PATH_MAX, USER_TIMER_HZ, USER_WRITABLE_PREFIX,
};
use kernel::{
    display::{FixedText, LineKind},
    elf::{ElfImage, FLAG_EXECUTE, FLAG_READ, FLAG_WRITE},
    input::{InputEvent, KeyEvent, MouseButtons},
    ipc::ChannelQueue,
    syscall::{self, SyscallAction},
    vfs::{NodeKind, RamVfs},
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
const TOKEN_SLEEP_MODE: u64 = 0x4000_0000_0000_0000;
const TOKEN_CHILD_MODE: u64 = 0x5000_0000_0000_0000;
const TOKEN_PARENT_MODE: u64 = 0x6000_0000_0000_0000;
const TOKEN_FILE_MODE: u64 = 0x7000_0000_0000_0000;
const TOKEN_INPUT_MODE: u64 = 0x9000_0000_0000_0000;
const TOKEN_WRITE_MODE: u64 = 0xa000_0000_0000_0000;
/// Fan-in trio. The receiver token carries both producer pids, one per byte;
/// each producer token carries the receiver pid it must connect to.
const TOKEN_FANIN_RECEIVER_MODE: u64 = 0xc000_0000_0000_0000;
const TOKEN_FANIN_PRODUCER_A_MODE: u64 = 0xd000_0000_0000_0000;
const TOKEN_FANIN_PRODUCER_B_MODE: u64 = 0xe000_0000_0000_0000;
const FILE_HANDLE_CAPACITY: usize = USER_FILE_HANDLE_CAPACITY as usize;
const ENDPOINT_HANDLE_CAPACITY: usize = USER_ENDPOINT_HANDLE_CAPACITY as usize;
const ENDPOINT_QUEUE_CAPACITY: usize = USER_ENDPOINT_QUEUE_CAPACITY;
/// Endpoint handles carry a dedicated tag byte in the position file handles use
/// for their owner pid, so endpoint authority can never be spent on the file
/// tables and file authority can never be spent on an endpoint.
const ENDPOINT_HANDLE_TAG: u64 = 0xe9 << 56;
const ENDPOINT_HANDLE_TAG_MASK: u64 = 0xff << 56;
/// Generations stay inside 32 bits so the owner pid, generation and slot fields
/// of a handle never overlap.
const ENDPOINT_GENERATION_MAX: u64 = u32::MAX as u64;
pub const MAX_ASYNC_PROCESSES: usize = 4;

static PROBE_PASSED: AtomicBool = AtomicBool::new(false);
static ELF_READY: AtomicBool = AtomicBool::new(false);
static CONTEXT_PASSED: AtomicBool = AtomicBool::new(false);
static COPY_OUT_PASSED: AtomicBool = AtomicBool::new(false);
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
static COMPLETED_FILE_READS: AtomicU64 = AtomicU64::new(0);
static COMPLETED_FILE_WRITES: AtomicU64 = AtomicU64::new(0);
static COMPLETED_INPUT_WAITS: AtomicU64 = AtomicU64::new(0);
static OPENED_FILE_HANDLES: AtomicU64 = AtomicU64::new(0);
static CLOSED_FILE_HANDLES: AtomicU64 = AtomicU64::new(0);
static COMPLETED_ENDPOINT_MESSAGES: AtomicU64 = AtomicU64::new(0);
static ENDPOINT_FAIRNESS_DENIALS: AtomicU64 = AtomicU64::new(0);
static ENDPOINT_WAKES: AtomicU64 = AtomicU64::new(0);
static NEXT_DYNAMIC_PID: AtomicU8 = AtomicU8::new(4);
static NEXT_CONSOLE_GENERATION: AtomicU64 = AtomicU64::new(1);
static mut USER_ELF_ADDRESS: u64 = 0;
static mut USER_ELF_LENGTH: usize = 0;
static mut SHELL_ELF_ADDRESS: u64 = 0;
static mut SHELL_ELF_LENGTH: usize = 0;
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
    Sleep(u64),
    WaitChild(u8),
    ReadFile {
        path: FixedText,
        address: u64,
        capacity: u64,
    },
    ReadDirectory {
        handle: u64,
        cursor: u64,
        address: u64,
        length: u64,
    },
    OpenFile {
        path: FixedText,
        rights: u64,
    },
    ReadHandle {
        handle: u64,
        address: u64,
        capacity: u64,
    },
    StatHandle {
        handle: u64,
        address: u64,
        length: u64,
    },
    CloseHandle(u64),
    WriteHandle {
        handle: u64,
        data: FileWriteBuffer,
    },
    WaitInput {
        address: u64,
        length: u64,
        mask: u64,
    },
    CreateEndpoint,
    ConnectEndpoint(u8),
    SendEndpoint {
        handle: u64,
        value: u64,
    },
    ReceiveEndpoint {
        handle: u64,
        address: u64,
        length: u64,
    },
    CloseEndpoint(u64),
    ConsoleWrite {
        text: FixedText,
        kind: LineKind,
    },
    ConsoleSetInput(FixedText),
    ConsoleClear,
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
    console_handle: u64,
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
    Sleeping,
    Waiting,
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
    pub console_process: bool,
    pub console: Option<ConsoleUpdate>,
    pub vfs_request: Option<UserVfsRequest>,
}

#[derive(Clone, Copy)]
pub enum ConsoleUpdate {
    Write { kind: LineKind, text: FixedText },
    SetInput(FixedText),
    Clear,
}

#[derive(Clone, Copy)]
pub enum UserVfsRequest {
    Open(FileOpenRequest),
    Read(FileReadRequest),
    Write(FileWriteRequest),
    ReadDirectory(DirectoryReadRequest),
}

#[derive(Clone, Copy)]
pub struct DirectoryReadRequest {
    pub task_id: u32,
    pub pid: u8,
    pub path: FixedText,
    pub handle: u64,
    pub cursor: u64,
}

#[derive(Clone, Copy)]
pub struct DirectoryEntryInfo {
    pub name: FixedText,
    pub kind: u64,
    pub size: u64,
}

#[derive(Clone, Copy)]
pub enum DirectoryReadResult {
    Entry(DirectoryEntryInfo),
    End,
    Unavailable,
}

#[derive(Clone, Copy)]
pub struct FileOpenRequest {
    pub task_id: u32,
    pub pid: u8,
    pub path: FixedText,
    pub rights: u64,
}

#[derive(Clone, Copy)]
pub struct FileOpenInfo {
    pub size: u64,
    pub kind: u64,
}

#[derive(Clone, Copy)]
pub struct FileReadRequest {
    pub task_id: u32,
    pub pid: u8,
    pub path: FixedText,
    pub handle: u64,
    pub offset: u64,
    pub capacity: u64,
}

#[derive(Clone, Copy)]
pub struct FileWriteRequest {
    pub task_id: u32,
    pub pid: u8,
    pub path: FixedText,
    pub handle: u64,
    pub offset: u64,
    pub data: FileWriteBuffer,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct FileWriteBuffer {
    bytes: [u8; USER_FILE_WRITE_MAX],
    len: usize,
}

impl FileWriteBuffer {
    const fn empty() -> Self {
        Self {
            bytes: [0; USER_FILE_WRITE_MAX],
            len: 0,
        }
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.bytes[..self.len]
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
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
    parent_pid: u8,
    state: ManagedState,
    wake_at: u64,
    blocked_on: BlockReason,
    file_handles: [Option<FileCapability>; FILE_HANDLE_CAPACITY],
    next_file_generation: u64,
    endpoints: EndpointState,
    pending_file_open: Option<PendingFileOpen>,
    pending_file_read: Option<PendingFileRead>,
    pending_file_write: Option<PendingFileWrite>,
    pending_directory_read: Option<PendingDirectoryRead>,
    pending_input: Option<PendingInput>,
    process: UserProcess,
}

#[derive(Clone, Copy)]
struct PendingFileRead {
    handle: u64,
    path: FixedText,
    offset: u64,
    address: u64,
    capacity: u64,
}

#[derive(Clone, Copy, Eq, PartialEq)]
struct PendingFileOpen {
    path: FixedText,
    rights: u64,
}

#[derive(Clone, Copy)]
struct PendingFileWrite {
    handle: u64,
    path: FixedText,
    offset: u64,
    data: FileWriteBuffer,
}

#[derive(Clone, Copy, Eq, PartialEq)]
struct PendingDirectoryRead {
    path: FixedText,
    handle: u64,
    cursor: u64,
    address: u64,
    length: u64,
}

#[derive(Clone, Copy, Eq, PartialEq)]
struct PendingInput {
    address: u64,
    length: u64,
    mask: u64,
}

/// Metadata of a receive that already validated its output buffer and is now
/// parked on the published endpoint.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PendingReceive {
    handle: u64,
    generation: u64,
    address: u64,
    length: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EndpointRole {
    /// Names the generation of the endpoint this process publishes itself.
    Receive { generation: u64 },
    /// Names one remote endpoint: a pid plus the generation it published.
    Send {
        target_pid: u8,
        target_generation: u64,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct EndpointCapability {
    handle: u64,
    owner_pid: u8,
    generation: u64,
    slot: u8,
    role: EndpointRole,
}

struct PublishedEndpoint {
    generation: u64,
    queue: ChannelQueue<ENDPOINT_QUEUE_CAPACITY>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EndpointDelivery {
    /// Copied straight into a parked receiver's validated buffer.
    Woken,
    /// Admitted to the endpoint queue, which now holds this many messages.
    Queued(usize),
    /// The sender already has a message queued on this endpoint.
    DuplicateProducer,
    QueueFull,
    CopyFailed,
    /// No live process publishes the generation this handle names.
    Stale,
}

/// Per-process endpoint authority: a small capability table, the single
/// endpoint this process publishes, and the receive it is parked on.
struct EndpointState {
    owner_pid: u8,
    handles: [Option<EndpointCapability>; ENDPOINT_HANDLE_CAPACITY],
    next_generation: u64,
    published: Option<PublishedEndpoint>,
    pending_receive: Option<PendingReceive>,
}

const fn endpoint_handle(owner_pid: u8, generation: u64, slot: usize) -> u64 {
    ENDPOINT_HANDLE_TAG | ((owner_pid as u64) << 40) | (generation << 8) | (slot as u64 + 1)
}

fn endpoint_handle_slot(handle: u64) -> Option<usize> {
    if handle & ENDPOINT_HANDLE_TAG_MASK != ENDPOINT_HANDLE_TAG {
        return None;
    }
    let slot = (handle & 0xff) as usize;
    (1..=ENDPOINT_HANDLE_CAPACITY)
        .contains(&slot)
        .then_some(slot - 1)
}

impl EndpointState {
    const fn new(owner_pid: u8) -> Self {
        Self {
            owner_pid,
            handles: [None; ENDPOINT_HANDLE_CAPACITY],
            next_generation: 1,
            published: None,
            pending_receive: None,
        }
    }

    fn published_generation(&self) -> Option<u64> {
        self.published.as_ref().map(|endpoint| endpoint.generation)
    }

    fn queue_depth(&self) -> usize {
        self.published
            .as_ref()
            .map_or(0, |endpoint| endpoint.queue.len())
    }

    fn next_generation(&mut self) -> Option<u64> {
        let generation = self.next_generation;
        if generation > ENDPOINT_GENERATION_MAX {
            return None;
        }
        self.next_generation = generation + 1;
        Some(generation)
    }

    fn allocate(&mut self, role: EndpointRole) -> Option<u64> {
        let slot = self.handles.iter().position(Option::is_none)?;
        let generation = self.next_generation()?;
        let handle = endpoint_handle(self.owner_pid, generation, slot);
        self.handles[slot] = Some(EndpointCapability {
            handle,
            owner_pid: self.owner_pid,
            generation,
            slot: slot as u8,
            role,
        });
        Some(handle)
    }

    /// Resolves a handle this process owns. The tag, decoded slot, owner pid and
    /// generation must reproduce the handle exactly, so neither a guessed value
    /// nor another process' handle nor a stale local handle can ever resolve.
    fn capability(&self, handle: u64) -> Option<EndpointCapability> {
        let slot = endpoint_handle_slot(handle)?;
        let capability = self.handles[slot]?;
        (capability.handle == handle
            && capability.owner_pid == self.owner_pid
            && capability.slot as usize == slot
            && endpoint_handle(capability.owner_pid, capability.generation, slot) == handle)
            .then_some(capability)
    }

    fn send_capability(&self, handle: u64) -> Option<(u8, u64)> {
        match self.capability(handle)?.role {
            EndpointRole::Send {
                target_pid,
                target_generation,
            } => Some((target_pid, target_generation)),
            EndpointRole::Receive { .. } => None,
        }
    }

    /// A receive capability is only usable while it still names the exact
    /// endpoint generation this process publishes right now.
    fn receive_generation(&self, handle: u64) -> Option<u64> {
        let EndpointRole::Receive { generation } = self.capability(handle)?.role else {
            return None;
        };
        (self.published_generation() == Some(generation)).then_some(generation)
    }

    /// Publishes an empty endpoint and returns its owned receive handle. Fails
    /// when an endpoint is already published or the handle table is full.
    fn create(&mut self) -> Option<u64> {
        if self.published.is_some() || !self.handles.iter().any(Option::is_none) {
            return None;
        }
        let slot = self.handles.iter().position(Option::is_none)?;
        let generation = self.next_generation()?;
        let handle = endpoint_handle(self.owner_pid, generation, slot);
        self.handles[slot] = Some(EndpointCapability {
            handle,
            owner_pid: self.owner_pid,
            generation,
            slot: slot as u8,
            role: EndpointRole::Receive { generation },
        });
        self.published = Some(PublishedEndpoint {
            generation,
            queue: ChannelQueue::new(),
        });
        Some(handle)
    }

    /// Closing a send capability revokes only that handle; closing the receive
    /// capability also drops the queue and unpublishes the endpoint.
    fn close(&mut self, handle: u64) -> Option<EndpointRole> {
        let capability = self.capability(handle)?;
        if let EndpointRole::Receive { generation } = capability.role {
            if self.published_generation() != Some(generation) {
                return None;
            }
            self.published = None;
            self.pending_receive = None;
        }
        self.handles[capability.slot as usize] = None;
        Some(capability.role)
    }

    /// Drops every send capability naming one remote endpoint generation.
    fn revoke_send_handles(&mut self, target_pid: u8, target_generation: u64) -> usize {
        let mut revoked = 0;
        for entry in self.handles.iter_mut() {
            let names_target = matches!(
                entry.map(|capability| capability.role),
                Some(EndpointRole::Send {
                    target_pid: pid,
                    target_generation: generation,
                }) if pid == target_pid && generation == target_generation
            );
            if names_target {
                *entry = None;
                revoked += 1;
            }
        }
        revoked
    }

    fn clear(&mut self) {
        self.handles = [None; ENDPOINT_HANDLE_CAPACITY];
        self.published = None;
        self.pending_receive = None;
    }
}

#[derive(Clone, Copy)]
struct FileCapability {
    handle: u64,
    path: FixedText,
    offset: u64,
    size: u64,
    kind: u64,
    rights: u64,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum BlockReason {
    None,
    Endpoint,
    Child(u8),
    FileOpen,
    FileRead,
    FileWrite,
    DirectoryRead,
    Input,
}

impl ManagedProcess {
    fn new(task_id: u32, parent_pid: u8, process: UserProcess) -> Self {
        let endpoints = EndpointState::new(process.pid);
        Self {
            task_id,
            parent_pid,
            state: ManagedState::Ready,
            wake_at: 0,
            blocked_on: BlockReason::None,
            file_handles: [None; FILE_HANDLE_CAPACITY],
            next_file_generation: 1,
            endpoints,
            pending_file_open: None,
            pending_file_read: None,
            pending_file_write: None,
            pending_directory_read: None,
            pending_input: None,
            process,
        }
    }

    fn allocate_file_handle(
        &mut self,
        path: FixedText,
        info: FileOpenInfo,
        rights: u64,
    ) -> Option<u64> {
        if rights == 0 || rights & !USER_FILE_RIGHTS_MASK != 0 {
            return None;
        }
        let slot = self.file_handles.iter().position(Option::is_none)?;
        let generation = self.next_file_generation;
        self.next_file_generation = self.next_file_generation.saturating_add(1);
        let handle = ((self.process.pid as u64) << 56) | (generation << 8) | (slot as u64 + 1);
        self.file_handles[slot] = Some(FileCapability {
            handle,
            path,
            offset: 0,
            size: info.size,
            kind: info.kind,
            rights,
        });
        Some(handle)
    }

    fn file_handle(&self, handle: u64) -> Option<&FileCapability> {
        self.file_handles
            .iter()
            .flatten()
            .find(|capability| capability.handle == handle)
    }

    fn file_handle_mut(&mut self, handle: u64) -> Option<&mut FileCapability> {
        self.file_handles
            .iter_mut()
            .flatten()
            .find(|capability| capability.handle == handle)
    }

    fn close_file_handle(&mut self, handle: u64) -> bool {
        let Some(slot) = self
            .file_handles
            .iter()
            .position(|entry| entry.is_some_and(|capability| capability.handle == handle))
        else {
            return false;
        };
        self.file_handles[slot] = None;
        true
    }

    fn revoke_resources(&mut self) {
        self.file_handles = [None; FILE_HANDLE_CAPACITY];
        self.endpoints.clear();
        self.pending_file_open = None;
        self.pending_file_read = None;
        self.pending_file_write = None;
        self.pending_directory_read = None;
        self.pending_input = None;
    }
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

    pub fn spawn_shell(&mut self, task_id: u32) -> Result<u8, LaunchError> {
        let slot = self
            .slots
            .iter()
            .position(Option::is_none)
            .ok_or(LaunchError::ProcessTableFull)?;
        let elf_bytes = shell_elf()?;
        let pid = NEXT_DYNAMIC_PID.fetch_add(1, Ordering::AcqRel);
        let generation = NEXT_CONSOLE_GENERATION.fetch_add(1, Ordering::AcqRel);
        let handle = syscall::console_handle(pid, generation);
        let mut process =
            build_process(pid, handle, elf_bytes).map_err(|_| LaunchError::ProcessBuildFailed)?;
        process.console_handle = handle;
        self.slots[slot] = Some(ManagedProcess::new(task_id, 0, process));
        DYNAMIC_PROCESSES.fetch_add(1, Ordering::AcqRel);
        crate::serial::print("USER_SHELL_SPAWN pid=");
        crate::serial::print_u64(pid as u64);
        crate::serial::print(" task=");
        crate::serial::print_u64(task_id as u64);
        crate::serial::println("");
        Ok(pid)
    }

    pub fn spawn_init(&mut self, task_id: u32, hold: bool) -> Result<u8, LaunchError> {
        let token_mode = if hold { TOKEN_HOLD_BIT } else { 0 };
        self.spawn_single(task_id, token_mode, if hold { "hold" } else { "normal" })
    }

    pub fn spawn_sleep_init(&mut self, task_id: u32) -> Result<u8, LaunchError> {
        self.spawn_single(task_id, TOKEN_SLEEP_MODE, "sleep")
    }

    pub fn spawn_file_init(&mut self, task_id: u32) -> Result<u8, LaunchError> {
        self.spawn_single(task_id, TOKEN_FILE_MODE, "file")
    }

    pub fn spawn_write_init(&mut self, task_id: u32) -> Result<u8, LaunchError> {
        self.spawn_single(task_id, TOKEN_WRITE_MODE, "write")
    }

    pub fn spawn_input_init(&mut self, task_id: u32) -> Result<u8, LaunchError> {
        self.spawn_single(task_id, TOKEN_INPUT_MODE, "input")
    }

    fn spawn_single(
        &mut self,
        task_id: u32,
        token_mode: u64,
        mode: &str,
    ) -> Result<u8, LaunchError> {
        let slot = self
            .slots
            .iter()
            .position(Option::is_none)
            .ok_or(LaunchError::ProcessTableFull)?;
        let elf_bytes = user_elf()?;
        let pid = NEXT_DYNAMIC_PID.fetch_add(1, Ordering::AcqRel);
        let token = if token_mode == 0 || token_mode == TOKEN_HOLD_BIT {
            TOKEN_DYNAMIC_BASE | token_mode | pid as u64
        } else {
            token_mode | pid as u64
        };
        let process =
            build_process(pid, token, elf_bytes).map_err(|_| LaunchError::ProcessBuildFailed)?;
        self.slots[slot] = Some(ManagedProcess::new(task_id, 0, process));
        DYNAMIC_PROCESSES.fetch_add(1, Ordering::AcqRel);
        crate::serial::print("USER_ASYNC_SPAWN pid=");
        crate::serial::print_u64(pid as u64);
        crate::serial::print(" task=");
        crate::serial::print_u64(task_id as u64);
        crate::serial::print(" mode=");
        crate::serial::print(mode);
        crate::serial::println("");
        Ok(pid)
    }

    pub fn spawn_coordination_pair(
        &mut self,
        parent_task_id: u32,
        child_task_id: u32,
    ) -> Result<(u8, u8), LaunchError> {
        let parent_slot = self
            .slots
            .iter()
            .position(Option::is_none)
            .ok_or(LaunchError::ProcessTableFull)?;
        let child_slot = self
            .slots
            .iter()
            .enumerate()
            .find_map(|(index, slot)| (index != parent_slot && slot.is_none()).then_some(index))
            .ok_or(LaunchError::ProcessTableFull)?;
        let elf_bytes = user_elf()?;
        let parent_pid = NEXT_DYNAMIC_PID.fetch_add(1, Ordering::AcqRel);
        let child_pid = NEXT_DYNAMIC_PID.fetch_add(1, Ordering::AcqRel);
        let mut parent = build_process(parent_pid, TOKEN_PARENT_MODE | child_pid as u64, elf_bytes)
            .map_err(|_| LaunchError::ProcessBuildFailed)?;
        let child = match build_process(child_pid, TOKEN_CHILD_MODE | parent_pid as u64, elf_bytes)
        {
            Ok(process) => process,
            Err(_) => {
                let _ = reclaim_process(&mut parent);
                return Err(LaunchError::ProcessBuildFailed);
            }
        };
        self.slots[parent_slot] = Some(ManagedProcess::new(parent_task_id, 0, parent));
        self.slots[child_slot] = Some(ManagedProcess::new(child_task_id, parent_pid, child));
        DYNAMIC_PROCESSES.fetch_add(2, Ordering::AcqRel);
        crate::serial::print("USER_PAIR_SPAWN parent=");
        crate::serial::print_u64(parent_pid as u64);
        crate::serial::print(" child=");
        crate::serial::print_u64(child_pid as u64);
        crate::serial::println("");
        Ok((parent_pid, child_pid))
    }

    /// Launches a receiver and the two producers that fan into it. Both
    /// producers are children of the receiver, so the receiver owns their
    /// terminal states and can reap them itself.
    pub fn spawn_endpoint_fan_in(
        &mut self,
        receiver_task_id: u32,
        a_task_id: u32,
        b_task_id: u32,
    ) -> Result<(u8, u8, u8), LaunchError> {
        let mut free = self
            .slots
            .iter()
            .enumerate()
            .filter_map(|(index, slot)| slot.is_none().then_some(index));
        let receiver_slot = free.next().ok_or(LaunchError::ProcessTableFull)?;
        let a_slot = free.next().ok_or(LaunchError::ProcessTableFull)?;
        let b_slot = free.next().ok_or(LaunchError::ProcessTableFull)?;
        let elf_bytes = user_elf()?;
        let receiver_pid = NEXT_DYNAMIC_PID.fetch_add(1, Ordering::AcqRel);
        let a_pid = NEXT_DYNAMIC_PID.fetch_add(1, Ordering::AcqRel);
        let b_pid = NEXT_DYNAMIC_PID.fetch_add(1, Ordering::AcqRel);
        let receiver_token = TOKEN_FANIN_RECEIVER_MODE | ((a_pid as u64) << 8) | b_pid as u64;
        let mut receiver = build_process(receiver_pid, receiver_token, elf_bytes)
            .map_err(|_| LaunchError::ProcessBuildFailed)?;
        let mut producer_a = match build_process(
            a_pid,
            TOKEN_FANIN_PRODUCER_A_MODE | receiver_pid as u64,
            elf_bytes,
        ) {
            Ok(process) => process,
            Err(_) => {
                let _ = reclaim_process(&mut receiver);
                return Err(LaunchError::ProcessBuildFailed);
            }
        };
        let producer_b = match build_process(
            b_pid,
            TOKEN_FANIN_PRODUCER_B_MODE | receiver_pid as u64,
            elf_bytes,
        ) {
            Ok(process) => process,
            Err(_) => {
                let _ = reclaim_process(&mut producer_a);
                let _ = reclaim_process(&mut receiver);
                return Err(LaunchError::ProcessBuildFailed);
            }
        };
        self.slots[receiver_slot] = Some(ManagedProcess::new(receiver_task_id, 0, receiver));
        self.slots[a_slot] = Some(ManagedProcess::new(a_task_id, receiver_pid, producer_a));
        self.slots[b_slot] = Some(ManagedProcess::new(b_task_id, receiver_pid, producer_b));
        DYNAMIC_PROCESSES.fetch_add(3, Ordering::AcqRel);
        crate::serial::print("USER_FANIN_SPAWN receiver=");
        crate::serial::print_u64(receiver_pid as u64);
        crate::serial::print(" a=");
        crate::serial::print_u64(a_pid as u64);
        crate::serial::print(" b=");
        crate::serial::print_u64(b_pid as u64);
        crate::serial::println("");
        Ok((receiver_pid, a_pid, b_pid))
    }

    pub fn poll(&mut self, tick: u64) -> Option<ProcessUpdate> {
        self.wake_sleepers(tick);
        for offset in 1..=MAX_ASYNC_PROCESSES {
            let index = (self.cursor + offset) % MAX_ASYNC_PROCESSES;
            let Some(managed) = self.slots[index].as_ref() else {
                continue;
            };
            if managed.state != ManagedState::Ready || managed.process.completed {
                continue;
            }
            self.cursor = index;
            let event = {
                let managed = self.slots[index].as_mut().expect("selected process exists");
                run_slice(&mut managed.process);
                managed.process.event
            };
            let mut vfs_request = None;
            let mut console = None;
            match event {
                ProcessEvent::Yield => {
                    TOTAL_YIELDS.fetch_add(1, Ordering::AcqRel);
                }
                ProcessEvent::Preempt => {
                    TOTAL_PREEMPTIONS.fetch_add(1, Ordering::AcqRel);
                }
                ProcessEvent::Sleep(ticks) => self.block_sleep(index, tick, ticks),
                ProcessEvent::WaitChild(pid) => self.complete_child_wait(index, pid),
                ProcessEvent::ReadFile {
                    path,
                    address,
                    capacity,
                } => {
                    vfs_request = Some(UserVfsRequest::Read(
                        self.block_file_read(index, path, address, capacity),
                    ));
                }
                ProcessEvent::ReadDirectory {
                    handle,
                    cursor,
                    address,
                    length,
                } => {
                    vfs_request = self
                        .block_directory_read(index, handle, cursor, address, length)
                        .map(UserVfsRequest::ReadDirectory);
                }
                ProcessEvent::OpenFile { path, rights } => {
                    vfs_request = self
                        .block_file_open(index, path, rights)
                        .map(UserVfsRequest::Open);
                }
                ProcessEvent::ReadHandle {
                    handle,
                    address,
                    capacity,
                } => {
                    vfs_request = self
                        .block_file_handle_read(index, handle, address, capacity)
                        .map(UserVfsRequest::Read);
                }
                ProcessEvent::StatHandle {
                    handle,
                    address,
                    length,
                } => self.complete_file_stat(index, handle, address, length),
                ProcessEvent::CloseHandle(handle) => self.complete_file_close(index, handle),
                ProcessEvent::WriteHandle { handle, data } => {
                    vfs_request = self
                        .block_file_handle_write(index, handle, data)
                        .map(UserVfsRequest::Write);
                }
                ProcessEvent::WaitInput {
                    address,
                    length,
                    mask,
                } => self.block_input(index, address, length, mask),
                ProcessEvent::CreateEndpoint => self.complete_endpoint_create(index),
                ProcessEvent::ConnectEndpoint(pid) => self.complete_endpoint_connect(index, pid),
                ProcessEvent::SendEndpoint { handle, value } => {
                    self.complete_endpoint_send(index, handle, value)
                }
                ProcessEvent::ReceiveEndpoint {
                    handle,
                    address,
                    length,
                } => self.complete_endpoint_receive(index, handle, address, length),
                ProcessEvent::CloseEndpoint(handle) => self.complete_endpoint_close(index, handle),
                ProcessEvent::ConsoleWrite { text, kind } => {
                    let managed = self.slots[index].as_mut().expect("selected process exists");
                    managed.process.context.rax = text.len() as u64;
                    managed.state = ManagedState::Ready;
                    console = Some(ConsoleUpdate::Write { kind, text });
                    crate::serial::print("USER_CONSOLE_WRITE pid=");
                    crate::serial::print_u64(managed.process.pid as u64);
                    crate::serial::print(" text=");
                    crate::serial::print(text.as_str());
                    crate::serial::println("");
                    if text.as_str().starts_with("SHELL.ELF ready") {
                        crate::serial::println("USER_SHELL_READY");
                    }
                }
                ProcessEvent::ConsoleSetInput(text) => {
                    let managed = self.slots[index].as_mut().expect("selected process exists");
                    managed.process.context.rax = text.len() as u64;
                    managed.state = ManagedState::Ready;
                    console = Some(ConsoleUpdate::SetInput(text));
                }
                ProcessEvent::ConsoleClear => {
                    let managed = self.slots[index].as_mut().expect("selected process exists");
                    managed.process.context.rax = 0;
                    managed.state = ManagedState::Ready;
                    console = Some(ConsoleUpdate::Clear);
                    crate::serial::print("USER_CONSOLE_CLEAR pid=");
                    crate::serial::print_u64(managed.process.pid as u64);
                    crate::serial::println("");
                }
                ProcessEvent::Exit => self.complete_terminal(index, ManagedState::Exited),
                ProcessEvent::Fault => self.complete_terminal(index, ManagedState::Faulted),
                ProcessEvent::None => return None,
            }
            let managed = self.slots[index].as_mut().expect("selected process exists");
            let output = if managed.process.output_pending {
                managed.process.output_pending = false;
                managed.process.output
            } else {
                FixedText::empty()
            };
            return Some(ProcessUpdate {
                task_id: managed.task_id,
                pid: managed.process.pid,
                state: managed.state,
                exit_code: managed.process.exit_code,
                preemptions: managed.process.preemptions,
                output,
                console_process: managed.process.console_handle != 0,
                console,
                vfs_request,
            });
        }
        None
    }

    fn wake_sleepers(&mut self, tick: u64) {
        for managed in self.slots.iter_mut().flatten() {
            if managed.state == ManagedState::Sleeping && tick >= managed.wake_at {
                managed.state = ManagedState::Ready;
                managed.wake_at = 0;
                managed.process.context.rax = 0;
                crate::serial::print("USER_SLEEP_WAKE pid=");
                crate::serial::print_u64(managed.process.pid as u64);
                crate::serial::println("");
            }
        }
    }

    fn block_directory_read(
        &mut self,
        index: usize,
        handle: u64,
        cursor: u64,
        address: u64,
        length: u64,
    ) -> Option<DirectoryReadRequest> {
        let managed = self.slots[index].as_mut().expect("selected process exists");
        let Some(capability) = managed.file_handle(handle).copied().filter(|capability| {
            capability.kind == genos_abi::USER_FILE_KIND_DIRECTORY
                && capability.rights & USER_FILE_RIGHT_READ != 0
        }) else {
            managed.process.context.rax =
                syscall::error_code(syscall::SyscallError::InvalidArgument);
            managed.state = ManagedState::Ready;
            return None;
        };
        managed.state = ManagedState::Waiting;
        managed.blocked_on = BlockReason::DirectoryRead;
        managed.pending_directory_read = Some(PendingDirectoryRead {
            path: capability.path,
            handle,
            cursor,
            address,
            length,
        });
        crate::serial::print("USER_DIRECTORY_READ_BLOCK pid=");
        crate::serial::print_u64(managed.process.pid as u64);
        crate::serial::print(" path=");
        crate::serial::print(capability.path.as_str());
        crate::serial::print(" cursor=");
        crate::serial::print_u64(cursor);
        crate::serial::println("");
        Some(DirectoryReadRequest {
            task_id: managed.task_id,
            pid: managed.process.pid,
            path: capability.path,
            handle,
            cursor,
        })
    }

    pub fn complete_directory_read(
        &mut self,
        request: DirectoryReadRequest,
        result: DirectoryReadResult,
    ) -> Result<ProcessUpdate, LaunchError> {
        let managed = self
            .slots
            .iter_mut()
            .flatten()
            .find(|managed| {
                managed.task_id == request.task_id && managed.process.pid == request.pid
            })
            .ok_or(LaunchError::ImageUnavailable)?;
        if managed.state != ManagedState::Waiting
            || managed.blocked_on != BlockReason::DirectoryRead
            || managed.process.completed
        {
            return Err(LaunchError::InvalidResult);
        }
        let pending = managed
            .pending_directory_read
            .as_ref()
            .ok_or(LaunchError::InvalidResult)?;
        if pending.path != request.path
            || pending.handle != request.handle
            || pending.cursor != request.cursor
        {
            return Err(LaunchError::InvalidResult);
        }
        if !managed
            .file_handle(request.handle)
            .is_some_and(|capability| {
                capability.path == request.path
                    && capability.kind == genos_abi::USER_FILE_KIND_DIRECTORY
                    && capability.rights & USER_FILE_RIGHT_READ != 0
            })
        {
            return Err(LaunchError::InvalidResult);
        }
        let pending = managed
            .pending_directory_read
            .take()
            .ok_or(LaunchError::InvalidResult)?;
        let return_value = match result {
            DirectoryReadResult::Entry(info) => {
                let name = info.name.as_str().as_bytes();
                if name.is_empty()
                    || name.len() > genos_abi::USER_DIRECTORY_NAME_MAX
                    || !matches!(
                        info.kind,
                        genos_abi::USER_FILE_KIND_REGULAR | genos_abi::USER_FILE_KIND_DIRECTORY
                    )
                {
                    syscall::error_code(syscall::SyscallError::Unavailable)
                } else {
                    let mut entry = UserDirectoryEntry::empty();
                    entry.kind = info.kind;
                    entry.size = info.size;
                    entry.name_length = name.len() as u64;
                    entry.name[..name.len()].copy_from_slice(name);
                    let bytes = unsafe {
                        core::slice::from_raw_parts(
                            core::ptr::addr_of!(entry).cast::<u8>(),
                            core::mem::size_of::<UserDirectoryEntry>(),
                        )
                    };
                    if pending.length as usize == bytes.len()
                        && copy_to_user_data(&managed.process, pending.address, bytes)
                    {
                        crate::serial::println("USER_DIRECTORY_READ_OK");
                        pending.length
                    } else {
                        syscall::error_code(syscall::SyscallError::InvalidArgument)
                    }
                }
            }
            DirectoryReadResult::End => 0,
            DirectoryReadResult::Unavailable => {
                syscall::error_code(syscall::SyscallError::Unavailable)
            }
        };
        managed.process.context.rax = return_value;
        managed.state = ManagedState::Ready;
        managed.blocked_on = BlockReason::None;
        crate::serial::print("USER_DIRECTORY_READ_WAKE pid=");
        crate::serial::print_u64(managed.process.pid as u64);
        crate::serial::println("");
        Ok(process_update(managed))
    }

    fn block_sleep(&mut self, index: usize, tick: u64, ticks: u64) {
        let managed = self.slots[index].as_mut().expect("selected process exists");
        managed.state = ManagedState::Sleeping;
        managed.wake_at = tick.saturating_add(ticks);
        crate::serial::print("USER_SLEEP_BLOCK pid=");
        crate::serial::print_u64(managed.process.pid as u64);
        crate::serial::print(" until=");
        crate::serial::print_u64(managed.wake_at);
        crate::serial::println("");
    }

    fn complete_endpoint_create(&mut self, index: usize) {
        let managed = self.slots[index].as_mut().expect("selected process exists");
        let handle = managed.endpoints.create();
        managed.process.context.rax =
            handle.unwrap_or_else(|| syscall::error_code(syscall::SyscallError::Unavailable));
        managed.state = ManagedState::Ready;
        match handle {
            Some(handle) => {
                crate::serial::print("USER_ENDPOINT_CREATED pid=");
                crate::serial::print_u64(managed.process.pid as u64);
                crate::serial::print(" handle=0x");
                crate::serial::print_hex(handle);
                crate::serial::print(" generation=");
                crate::serial::print_u64(managed.endpoints.published_generation().unwrap_or(0));
                crate::serial::println("");
            }
            None => {
                crate::serial::print("USER_ENDPOINT_CREATE_DENIED pid=");
                crate::serial::print_u64(managed.process.pid as u64);
                crate::serial::println("");
            }
        }
    }

    fn complete_endpoint_connect(&mut self, index: usize, target_pid: u8) {
        let target_generation = self.slots.iter().flatten().find_map(|managed| {
            (managed.process.pid == target_pid && !managed.process.completed)
                .then(|| managed.endpoints.published_generation())
                .flatten()
        });
        let managed = self.slots[index].as_mut().expect("selected process exists");
        let handle = target_generation.and_then(|target_generation| {
            managed.endpoints.allocate(EndpointRole::Send {
                target_pid,
                target_generation,
            })
        });
        managed.process.context.rax =
            handle.unwrap_or_else(|| syscall::error_code(syscall::SyscallError::Unavailable));
        managed.state = ManagedState::Ready;
        match handle {
            Some(handle) => {
                crate::serial::print("USER_ENDPOINT_CONNECTED from=");
                crate::serial::print_u64(managed.process.pid as u64);
                crate::serial::print(" to=");
                crate::serial::print_u64(target_pid as u64);
                crate::serial::print(" handle=0x");
                crate::serial::print_hex(handle);
                crate::serial::print(" generation=");
                crate::serial::print_u64(target_generation.unwrap_or(0));
                crate::serial::println("");
            }
            None => {
                crate::serial::print("USER_ENDPOINT_CONNECT_DENIED from=");
                crate::serial::print_u64(managed.process.pid as u64);
                crate::serial::print(" to=");
                crate::serial::print_u64(target_pid as u64);
                crate::serial::println("");
            }
        }
    }

    fn complete_endpoint_send(&mut self, index: usize, handle: u64, value: u64) {
        let (sender_pid, target) = {
            let managed = self.slots[index].as_ref().expect("selected process exists");
            (
                managed.process.pid,
                managed.endpoints.send_capability(handle),
            )
        };
        let Some((target_pid, target_generation)) = target else {
            let managed = self.slots[index].as_mut().expect("selected process exists");
            managed.process.context.rax =
                syscall::error_code(syscall::SyscallError::InvalidArgument);
            managed.state = ManagedState::Ready;
            crate::serial::print("USER_ENDPOINT_SEND_DENIED pid=");
            crate::serial::print_u64(sender_pid as u64);
            crate::serial::print(" handle=0x");
            crate::serial::print_hex(handle);
            crate::serial::println("");
            return;
        };
        let delivery = self.deliver_endpoint_message(
            target_pid,
            target_generation,
            UserChannelMessage {
                sender_pid: sender_pid as u64,
                value,
            },
        );
        match delivery {
            EndpointDelivery::Woken => {
                ENDPOINT_WAKES.fetch_add(1, Ordering::AcqRel);
                COMPLETED_ENDPOINT_MESSAGES.fetch_add(1, Ordering::AcqRel);
            }
            EndpointDelivery::DuplicateProducer => {
                ENDPOINT_FAIRNESS_DENIALS.fetch_add(1, Ordering::AcqRel);
            }
            _ => {}
        }
        let managed = self.slots[index].as_mut().expect("selected process exists");
        managed.process.context.rax = match delivery {
            EndpointDelivery::Woken | EndpointDelivery::Queued(_) => 0,
            EndpointDelivery::DuplicateProducer
            | EndpointDelivery::QueueFull
            | EndpointDelivery::CopyFailed => {
                syscall::error_code(syscall::SyscallError::Unavailable)
            }
            EndpointDelivery::Stale => syscall::error_code(syscall::SyscallError::InvalidArgument),
        };
        managed.state = ManagedState::Ready;
        crate::serial::print(match delivery {
            EndpointDelivery::Woken => "USER_ENDPOINT_DELIVERED from=",
            EndpointDelivery::Queued(_) => "USER_ENDPOINT_QUEUED from=",
            EndpointDelivery::DuplicateProducer => "USER_CHANNEL_FAIRNESS_DENIED from=",
            EndpointDelivery::QueueFull => "USER_ENDPOINT_QUEUE_FULL from=",
            EndpointDelivery::CopyFailed => "USER_ENDPOINT_COPY_FAILED from=",
            EndpointDelivery::Stale => "USER_ENDPOINT_SEND_STALE from=",
        });
        crate::serial::print_u64(sender_pid as u64);
        crate::serial::print(" to=");
        crate::serial::print_u64(target_pid as u64);
        crate::serial::print(" generation=");
        crate::serial::print_u64(target_generation);
        if let EndpointDelivery::Queued(depth) = delivery {
            crate::serial::print(" depth=");
            crate::serial::print_u64(depth as u64);
        }
        crate::serial::println("");
    }

    /// Hands one message to a published endpoint: straight into a validated
    /// waiter when the target is parked on that exact endpoint, otherwise onto
    /// its fair queue. Nothing is ever overwritten.
    fn deliver_endpoint_message(
        &mut self,
        target_pid: u8,
        target_generation: u64,
        message: UserChannelMessage,
    ) -> EndpointDelivery {
        let target_index = self.slots.iter().position(|slot| {
            slot.as_ref().is_some_and(|managed| {
                managed.process.pid == target_pid
                    && !managed.process.completed
                    && managed.endpoints.published_generation() == Some(target_generation)
            })
        });
        let Some(target_index) = target_index else {
            return EndpointDelivery::Stale;
        };
        let target = self.slots[target_index]
            .as_mut()
            .expect("endpoint target exists");
        let pending = target.endpoints.pending_receive.filter(|pending| {
            target.state == ManagedState::Waiting
                && target.blocked_on == BlockReason::Endpoint
                && pending.generation == target_generation
        });
        if let Some(pending) = pending {
            if pending.length != USER_CHANNEL_MESSAGE_SIZE
                || target.endpoints.receive_generation(pending.handle) != Some(target_generation)
                || !copy_to_user_data(
                    &target.process,
                    pending.address,
                    channel_message_bytes(&message),
                )
            {
                return EndpointDelivery::CopyFailed;
            }
            target.endpoints.pending_receive = None;
            target.process.context.rax = USER_CHANNEL_MESSAGE_SIZE;
            target.state = ManagedState::Ready;
            target.blocked_on = BlockReason::None;
            return EndpointDelivery::Woken;
        }
        let Some(endpoint) = target.endpoints.published.as_mut() else {
            return EndpointDelivery::Stale;
        };
        if endpoint.queue.contains_sender(message.sender_pid) {
            return EndpointDelivery::DuplicateProducer;
        }
        if !endpoint.queue.push(message) {
            return EndpointDelivery::QueueFull;
        }
        EndpointDelivery::Queued(endpoint.queue.len())
    }

    fn complete_endpoint_receive(&mut self, index: usize, handle: u64, address: u64, length: u64) {
        let managed = self.slots[index].as_mut().expect("selected process exists");
        let generation = managed.endpoints.receive_generation(handle).filter(|_| {
            length == USER_CHANNEL_MESSAGE_SIZE
                && valid_user_data_buffer(&managed.process, address, length)
        });
        let Some(generation) = generation else {
            managed.process.context.rax =
                syscall::error_code(syscall::SyscallError::InvalidArgument);
            managed.state = ManagedState::Ready;
            crate::serial::print("USER_ENDPOINT_RECEIVE_DENIED pid=");
            crate::serial::print_u64(managed.process.pid as u64);
            crate::serial::print(" handle=0x");
            crate::serial::print_hex(handle);
            crate::serial::println("");
            return;
        };
        let queued = managed
            .endpoints
            .published
            .as_mut()
            .and_then(|endpoint| endpoint.queue.pop());
        let Some(message) = queued else {
            managed.state = ManagedState::Waiting;
            managed.blocked_on = BlockReason::Endpoint;
            managed.endpoints.pending_receive = Some(PendingReceive {
                handle,
                generation,
                address,
                length,
            });
            crate::serial::print("USER_ENDPOINT_BLOCK pid=");
            crate::serial::print_u64(managed.process.pid as u64);
            crate::serial::print(" handle=0x");
            crate::serial::print_hex(handle);
            crate::serial::print(" generation=");
            crate::serial::print_u64(generation);
            crate::serial::println("");
            return;
        };
        let copied = copy_to_user_data(&managed.process, address, channel_message_bytes(&message));
        if copied {
            COMPLETED_ENDPOINT_MESSAGES.fetch_add(1, Ordering::AcqRel);
        }
        managed.endpoints.pending_receive = None;
        managed.process.context.rax = if copied {
            USER_CHANNEL_MESSAGE_SIZE
        } else {
            syscall::error_code(syscall::SyscallError::InvalidArgument)
        };
        managed.state = ManagedState::Ready;
        managed.blocked_on = BlockReason::None;
        crate::serial::print(if copied {
            "USER_ENDPOINT_RECEIVED pid="
        } else {
            "USER_ENDPOINT_COPY_FAILED pid="
        });
        crate::serial::print_u64(managed.process.pid as u64);
        crate::serial::print(" from=");
        crate::serial::print_u64(message.sender_pid);
        crate::serial::print(" generation=");
        crate::serial::print_u64(generation);
        crate::serial::print(" depth=");
        crate::serial::print_u64(managed.endpoints.queue_depth() as u64);
        crate::serial::println("");
    }

    fn complete_endpoint_close(&mut self, index: usize, handle: u64) {
        let (owner_pid, role) = {
            let managed = self.slots[index].as_mut().expect("selected process exists");
            (managed.process.pid, managed.endpoints.close(handle))
        };
        let revoked = match role {
            Some(EndpointRole::Receive { generation }) => {
                self.revoke_endpoint_send_handles(owner_pid, generation)
            }
            _ => 0,
        };
        let managed = self.slots[index].as_mut().expect("selected process exists");
        managed.process.context.rax = if role.is_some() {
            0
        } else {
            syscall::error_code(syscall::SyscallError::InvalidArgument)
        };
        managed.state = ManagedState::Ready;
        crate::serial::print("USER_ENDPOINT_CLOSED pid=");
        crate::serial::print_u64(owner_pid as u64);
        crate::serial::print(" handle=0x");
        crate::serial::print_hex(handle);
        crate::serial::print(" kind=");
        crate::serial::print(match role {
            Some(EndpointRole::Receive { .. }) => "receive",
            Some(EndpointRole::Send { .. }) => "send",
            None => "rejected",
        });
        crate::serial::print(" revoked=");
        crate::serial::print_u64(revoked as u64);
        crate::serial::println("");
    }

    /// Revokes every send capability held anywhere in the manager that names
    /// one endpoint generation, so no handle outlives the endpoint it targets.
    fn revoke_endpoint_send_handles(&mut self, target_pid: u8, target_generation: u64) -> usize {
        let mut revoked = 0;
        for managed in self.slots.iter_mut().flatten() {
            revoked += managed
                .endpoints
                .revoke_send_handles(target_pid, target_generation);
        }
        if revoked > 0 {
            crate::serial::print("USER_ENDPOINT_REVOKED pid=");
            crate::serial::print_u64(target_pid as u64);
            crate::serial::print(" generation=");
            crate::serial::print_u64(target_generation);
            crate::serial::print(" handles=");
            crate::serial::print_u64(revoked as u64);
            crate::serial::println("");
        }
        revoked
    }

    /// Clears one process' endpoint authority and revokes the remote send
    /// handles that depended on it. Always runs before address-space reclaim.
    fn release_endpoints(&mut self, index: usize) {
        let (owner_pid, generation) = {
            let managed = self.slots[index].as_mut().expect("selected process exists");
            let generation = managed.endpoints.published_generation();
            managed.endpoints.clear();
            (managed.process.pid, generation)
        };
        if let Some(generation) = generation {
            self.revoke_endpoint_send_handles(owner_pid, generation);
        }
    }

    fn block_input(&mut self, index: usize, address: u64, length: u64, mask: u64) {
        let waiter_exists = self.slots.iter().enumerate().any(|(slot, managed)| {
            slot != index
                && managed.as_ref().is_some_and(|managed| {
                    managed.state == ManagedState::Waiting
                        && managed.blocked_on == BlockReason::Input
                })
        });
        let managed = self.slots[index].as_mut().expect("selected process exists");
        if waiter_exists || mask == 0 || mask & !USER_INPUT_MASK_ALL != 0 {
            managed.process.context.rax = syscall::error_code(syscall::SyscallError::Unavailable);
            managed.state = ManagedState::Ready;
            crate::serial::print("USER_INPUT_WAIT_DENIED pid=");
            crate::serial::print_u64(managed.process.pid as u64);
            crate::serial::println("");
            return;
        }
        managed.state = ManagedState::Waiting;
        managed.blocked_on = BlockReason::Input;
        managed.pending_input = Some(PendingInput {
            address,
            length,
            mask,
        });
        crate::serial::print("USER_INPUT_BLOCK pid=");
        crate::serial::print_u64(managed.process.pid as u64);
        crate::serial::print(" mask=");
        crate::serial::print_u64(mask);
        crate::serial::println("");
    }

    pub fn deliver_input(
        &mut self,
        input: InputEvent,
    ) -> Result<Option<ProcessUpdate>, LaunchError> {
        let required_mask = input.user_mask();
        let index = self.slots.iter().position(|managed| {
            managed.as_ref().is_some_and(|managed| {
                managed.state == ManagedState::Waiting
                    && managed.blocked_on == BlockReason::Input
                    && managed
                        .pending_input
                        .is_some_and(|pending| pending.mask & required_mask != 0)
            })
        });
        let Some(index) = index else {
            return Ok(None);
        };
        let managed = self.slots[index].as_mut().expect("input waiter exists");
        let pending = managed.pending_input.ok_or(LaunchError::InvalidResult)?;
        if pending.length as usize != core::mem::size_of::<UserInputEvent>() {
            return Err(LaunchError::InvalidResult);
        }
        let event = input.to_user_event();
        let bytes = unsafe {
            core::slice::from_raw_parts(
                core::ptr::addr_of!(event).cast::<u8>(),
                core::mem::size_of::<UserInputEvent>(),
            )
        };
        if !copy_to_user_data(&managed.process, pending.address, bytes) {
            return Err(LaunchError::InvalidResult);
        }
        managed.pending_input = None;
        managed.process.context.rax = pending.length;
        managed.state = ManagedState::Ready;
        managed.blocked_on = BlockReason::None;
        COMPLETED_INPUT_WAITS.fetch_add(1, Ordering::AcqRel);
        crate::serial::print("USER_INPUT_WAKE pid=");
        crate::serial::print_u64(managed.process.pid as u64);
        crate::serial::print(" kind=");
        crate::serial::print_u64(event.kind);
        crate::serial::print(" code=");
        crate::serial::print_u64(event.code);
        crate::serial::print(" value0=");
        crate::serial::print_u64(event.value0 as u64);
        crate::serial::println("");
        Ok(Some(process_update(managed)))
    }

    fn complete_child_wait(&mut self, parent_index: usize, child_pid: u8) {
        let parent_pid = self.slots[parent_index]
            .as_ref()
            .expect("selected process exists")
            .process
            .pid;
        let child =
            self.slots.iter().flatten().find(|managed| {
                managed.process.pid == child_pid && managed.parent_pid == parent_pid
            });
        let result = child.map(|managed| {
            if managed.process.completed {
                Some(managed.process.exit_code as u64)
            } else {
                None
            }
        });
        let parent = self.slots[parent_index]
            .as_mut()
            .expect("selected process exists");
        match result {
            Some(Some(status)) => {
                parent.process.context.rax = status;
                parent.state = ManagedState::Ready;
            }
            Some(None) => {
                parent.state = ManagedState::Waiting;
                parent.blocked_on = BlockReason::Child(child_pid);
                crate::serial::print("USER_CHILD_WAIT parent=");
                crate::serial::print_u64(parent_pid as u64);
                crate::serial::print(" child=");
                crate::serial::print_u64(child_pid as u64);
                crate::serial::println("");
            }
            None => {
                parent.process.context.rax =
                    syscall::error_code(syscall::SyscallError::InvalidArgument);
                parent.state = ManagedState::Ready;
            }
        }
    }

    fn block_file_open(
        &mut self,
        index: usize,
        path: FixedText,
        rights: u64,
    ) -> Option<FileOpenRequest> {
        let managed = self.slots[index].as_mut().expect("selected process exists");
        if rights == 0
            || rights & !USER_FILE_RIGHTS_MASK != 0
            || (rights & USER_FILE_RIGHT_WRITE != 0 && !is_user_writable_path(path.as_str()))
        {
            managed.process.context.rax =
                syscall::error_code(syscall::SyscallError::InvalidArgument);
            managed.state = ManagedState::Ready;
            crate::serial::print("USER_FILE_OPEN_DENIED pid=");
            crate::serial::print_u64(managed.process.pid as u64);
            crate::serial::print(" path=");
            crate::serial::println(path.as_str());
            return None;
        }
        managed.state = ManagedState::Waiting;
        managed.blocked_on = BlockReason::FileOpen;
        managed.pending_file_open = Some(PendingFileOpen { path, rights });
        crate::serial::print("USER_FILE_OPEN_BLOCK pid=");
        crate::serial::print_u64(managed.process.pid as u64);
        crate::serial::print(" path=");
        crate::serial::print(path.as_str());
        crate::serial::print(" rights=");
        crate::serial::print_u64(rights);
        crate::serial::println("");
        Some(FileOpenRequest {
            task_id: managed.task_id,
            pid: managed.process.pid,
            path,
            rights,
        })
    }

    pub fn complete_file_open(
        &mut self,
        request: FileOpenRequest,
        info: Option<FileOpenInfo>,
    ) -> Result<ProcessUpdate, LaunchError> {
        let managed = self
            .slots
            .iter_mut()
            .flatten()
            .find(|managed| {
                managed.task_id == request.task_id && managed.process.pid == request.pid
            })
            .ok_or(LaunchError::ImageUnavailable)?;
        if managed.state != ManagedState::Waiting
            || managed.blocked_on != BlockReason::FileOpen
            || managed.process.completed
            || managed.pending_file_open
                != Some(PendingFileOpen {
                    path: request.path,
                    rights: request.rights,
                })
        {
            return Err(LaunchError::InvalidResult);
        }
        managed.pending_file_open = None;
        let handle = info
            .filter(|metadata| {
                matches!(
                    metadata.kind,
                    USER_FILE_KIND_REGULAR | genos_abi::USER_FILE_KIND_DIRECTORY
                ) && (metadata.kind == USER_FILE_KIND_REGULAR
                    || request.rights == USER_FILE_RIGHT_READ)
            })
            .and_then(|metadata| {
                managed.allocate_file_handle(request.path, metadata, request.rights)
            });
        if handle.is_some() {
            OPENED_FILE_HANDLES.fetch_add(1, Ordering::AcqRel);
        }
        managed.process.context.rax =
            handle.unwrap_or_else(|| syscall::error_code(syscall::SyscallError::Unavailable));
        managed.state = ManagedState::Ready;
        managed.blocked_on = BlockReason::None;
        crate::serial::print("USER_FILE_OPEN_WAKE pid=");
        crate::serial::print_u64(managed.process.pid as u64);
        crate::serial::print(" handle=0x");
        crate::serial::print_hex(handle.unwrap_or(0));
        crate::serial::println("");
        Ok(process_update(managed))
    }

    fn block_file_handle_read(
        &mut self,
        index: usize,
        handle: u64,
        address: u64,
        capacity: u64,
    ) -> Option<FileReadRequest> {
        let managed = self.slots[index].as_mut().expect("selected process exists");
        let Some(capability) = managed.file_handle(handle).copied().filter(|capability| {
            capability.kind == USER_FILE_KIND_REGULAR
                && capability.rights & USER_FILE_RIGHT_READ != 0
        }) else {
            managed.process.context.rax =
                syscall::error_code(syscall::SyscallError::InvalidArgument);
            managed.state = ManagedState::Ready;
            return None;
        };
        managed.state = ManagedState::Waiting;
        managed.blocked_on = BlockReason::FileRead;
        managed.pending_file_read = Some(PendingFileRead {
            handle,
            path: capability.path,
            offset: capability.offset,
            address,
            capacity,
        });
        crate::serial::print("USER_HANDLE_READ_BLOCK pid=");
        crate::serial::print_u64(managed.process.pid as u64);
        crate::serial::print(" handle=0x");
        crate::serial::print_hex(handle);
        crate::serial::print(" offset=");
        crate::serial::print_u64(capability.offset);
        crate::serial::println("");
        Some(FileReadRequest {
            task_id: managed.task_id,
            pid: managed.process.pid,
            path: capability.path,
            handle,
            offset: capability.offset,
            capacity,
        })
    }

    fn complete_file_stat(&mut self, index: usize, handle: u64, address: u64, length: u64) {
        let managed = self.slots[index].as_mut().expect("selected process exists");
        let stat = managed.file_handle(handle).map(|capability| UserFileStat {
            size: capability.size,
            offset: capability.offset,
            kind: capability.kind,
            rights: capability.rights,
        });
        let copied = stat.is_some_and(|stat| {
            let bytes = unsafe {
                core::slice::from_raw_parts(
                    core::ptr::addr_of!(stat).cast::<u8>(),
                    core::mem::size_of::<UserFileStat>(),
                )
            };
            length as usize == bytes.len() && copy_to_user_data(&managed.process, address, bytes)
        });
        managed.process.context.rax = if copied {
            length
        } else {
            syscall::error_code(syscall::SyscallError::InvalidArgument)
        };
        managed.state = ManagedState::Ready;
    }

    fn complete_file_close(&mut self, index: usize, handle: u64) {
        let managed = self.slots[index].as_mut().expect("selected process exists");
        let closed = managed.close_file_handle(handle);
        if closed {
            CLOSED_FILE_HANDLES.fetch_add(1, Ordering::AcqRel);
        }
        managed.process.context.rax = if closed {
            0
        } else {
            syscall::error_code(syscall::SyscallError::InvalidArgument)
        };
        managed.state = ManagedState::Ready;
        crate::serial::print("USER_FILE_CLOSE pid=");
        crate::serial::print_u64(managed.process.pid as u64);
        crate::serial::print(" handle=0x");
        crate::serial::print_hex(handle);
        crate::serial::print(" result=");
        crate::serial::println(if closed { "closed" } else { "rejected" });
    }

    fn block_file_handle_write(
        &mut self,
        index: usize,
        handle: u64,
        data: FileWriteBuffer,
    ) -> Option<FileWriteRequest> {
        let managed = self.slots[index].as_mut().expect("selected process exists");
        let Some(capability) = managed.file_handle(handle).copied().filter(|capability| {
            capability.kind == USER_FILE_KIND_REGULAR
                && capability.rights & USER_FILE_RIGHT_WRITE != 0
                && is_user_writable_path(capability.path.as_str())
                && !data.is_empty()
        }) else {
            managed.process.context.rax =
                syscall::error_code(syscall::SyscallError::InvalidArgument);
            managed.state = ManagedState::Ready;
            crate::serial::print("USER_HANDLE_WRITE_DENIED pid=");
            crate::serial::print_u64(managed.process.pid as u64);
            crate::serial::println("");
            return None;
        };
        let pending = PendingFileWrite {
            handle,
            path: capability.path,
            offset: capability.offset,
            data,
        };
        managed.state = ManagedState::Waiting;
        managed.blocked_on = BlockReason::FileWrite;
        managed.pending_file_write = Some(pending);
        crate::serial::print("USER_HANDLE_WRITE_BLOCK pid=");
        crate::serial::print_u64(managed.process.pid as u64);
        crate::serial::print(" handle=0x");
        crate::serial::print_hex(handle);
        crate::serial::print(" offset=");
        crate::serial::print_u64(capability.offset);
        crate::serial::print(" bytes=");
        crate::serial::print_u64(data.len() as u64);
        crate::serial::println("");
        Some(FileWriteRequest {
            task_id: managed.task_id,
            pid: managed.process.pid,
            path: capability.path,
            handle,
            offset: capability.offset,
            data,
        })
    }

    pub fn complete_file_write(
        &mut self,
        request: FileWriteRequest,
        written: Option<u64>,
    ) -> Result<ProcessUpdate, LaunchError> {
        let managed = self
            .slots
            .iter_mut()
            .flatten()
            .find(|managed| {
                managed.task_id == request.task_id && managed.process.pid == request.pid
            })
            .ok_or(LaunchError::ImageUnavailable)?;
        if managed.state != ManagedState::Waiting
            || managed.blocked_on != BlockReason::FileWrite
            || managed.process.completed
        {
            return Err(LaunchError::InvalidResult);
        }
        let pending = managed
            .pending_file_write
            .as_ref()
            .ok_or(LaunchError::InvalidResult)?;
        if pending.handle != request.handle
            || pending.path != request.path
            || pending.offset != request.offset
            || pending.data != request.data
        {
            return Err(LaunchError::InvalidResult);
        }
        let capability = managed
            .file_handle(request.handle)
            .copied()
            .filter(|capability| {
                capability.path == request.path
                    && capability.offset == request.offset
                    && capability.rights & USER_FILE_RIGHT_WRITE != 0
            })
            .ok_or(LaunchError::InvalidResult)?;
        let written = written.filter(|count| *count <= request.data.len() as u64);
        managed.pending_file_write = None;
        if let Some(count) = written {
            let capability = managed
                .file_handle_mut(request.handle)
                .ok_or(LaunchError::InvalidResult)?;
            capability.offset = capability.offset.saturating_add(count);
            capability.size = capability.size.max(capability.offset);
            COMPLETED_FILE_WRITES.fetch_add(1, Ordering::AcqRel);
        }
        managed.process.context.rax =
            written.unwrap_or_else(|| syscall::error_code(syscall::SyscallError::Unavailable));
        managed.state = ManagedState::Ready;
        managed.blocked_on = BlockReason::None;
        crate::serial::print("USER_HANDLE_WRITE_WAKE pid=");
        crate::serial::print_u64(managed.process.pid as u64);
        crate::serial::print(" bytes=");
        crate::serial::print_u64(written.unwrap_or(0));
        crate::serial::print(" size=");
        crate::serial::print_u64(
            capability
                .size
                .max(request.offset.saturating_add(written.unwrap_or(0))),
        );
        crate::serial::println("");
        Ok(process_update(managed))
    }

    fn block_file_read(
        &mut self,
        index: usize,
        path: FixedText,
        address: u64,
        capacity: u64,
    ) -> FileReadRequest {
        let managed = self.slots[index].as_mut().expect("selected process exists");
        managed.state = ManagedState::Waiting;
        managed.blocked_on = BlockReason::FileRead;
        managed.pending_file_read = Some(PendingFileRead {
            handle: 0,
            path,
            offset: 0,
            address,
            capacity,
        });
        crate::serial::print("USER_FILE_READ_BLOCK pid=");
        crate::serial::print_u64(managed.process.pid as u64);
        crate::serial::print(" path=");
        crate::serial::print(path.as_str());
        crate::serial::print(" cap=");
        crate::serial::print_u64(capacity);
        crate::serial::println("");
        FileReadRequest {
            task_id: managed.task_id,
            pid: managed.process.pid,
            path,
            handle: 0,
            offset: 0,
            capacity,
        }
    }

    pub fn complete_file_read(
        &mut self,
        request: FileReadRequest,
        data: Option<&[u8]>,
    ) -> Result<ProcessUpdate, LaunchError> {
        let managed = self
            .slots
            .iter_mut()
            .flatten()
            .find(|managed| {
                managed.task_id == request.task_id && managed.process.pid == request.pid
            })
            .ok_or(LaunchError::ImageUnavailable)?;
        if managed.state != ManagedState::Waiting
            || managed.blocked_on != BlockReason::FileRead
            || managed.process.completed
        {
            return Err(LaunchError::InvalidResult);
        }
        let pending = managed
            .pending_file_read
            .as_ref()
            .ok_or(LaunchError::InvalidResult)?;
        if pending.path != request.path
            || pending.handle != request.handle
            || pending.offset != request.offset
            || pending.capacity != request.capacity
        {
            return Err(LaunchError::InvalidResult);
        }
        let pending = managed
            .pending_file_read
            .take()
            .ok_or(LaunchError::InvalidResult)?;
        let copied = data.and_then(|bytes| {
            let length = bytes.len().min(pending.capacity as usize);
            (length == 0 || copy_to_user_data(&managed.process, pending.address, &bytes[..length]))
                .then_some(length as u64)
        });
        if copied.is_some() {
            COMPLETED_FILE_READS.fetch_add(1, Ordering::AcqRel);
        }
        if pending.handle != 0 {
            let Some(capability) = managed.file_handle_mut(pending.handle) else {
                return Err(LaunchError::InvalidResult);
            };
            if capability.path != pending.path || capability.offset != pending.offset {
                return Err(LaunchError::InvalidResult);
            }
            capability.offset = capability
                .offset
                .saturating_add(copied.unwrap_or(0))
                .min(capability.size);
        }
        managed.process.context.rax =
            copied.unwrap_or_else(|| syscall::error_code(syscall::SyscallError::Unavailable));
        managed.state = ManagedState::Ready;
        managed.blocked_on = BlockReason::None;
        crate::serial::print("USER_FILE_READ_WAKE pid=");
        crate::serial::print_u64(managed.process.pid as u64);
        crate::serial::print(" bytes=");
        crate::serial::print_u64(copied.unwrap_or(0));
        crate::serial::println("");
        Ok(process_update(managed))
    }

    fn complete_terminal(&mut self, index: usize, state: ManagedState) {
        self.release_endpoints(index);
        let (pid, exit_code) = {
            let managed = self.slots[index].as_mut().expect("selected process exists");
            managed.state = state;
            managed.revoke_resources();
            if reclaim_process(&mut managed.process).is_err() {
                fail("USER_RECLAIM_FAILED");
            }
            (managed.process.pid, managed.process.exit_code)
        };
        self.wake_waiting_parent(pid, exit_code);
    }

    fn wake_waiting_parent(&mut self, child_pid: u8, exit_code: u8) {
        for managed in self.slots.iter_mut().flatten() {
            if managed.state == ManagedState::Waiting
                && managed.blocked_on == BlockReason::Child(child_pid)
            {
                managed.process.context.rax = exit_code as u64;
                managed.state = ManagedState::Ready;
                managed.blocked_on = BlockReason::None;
                crate::serial::print("USER_CHILD_WAKE parent=");
                crate::serial::print_u64(managed.process.pid as u64);
                crate::serial::print(" child=");
                crate::serial::print_u64(child_pid as u64);
                crate::serial::println("");
            }
        }
    }

    pub fn kill(&mut self, task_id: u32) -> Result<ProcessUpdate, LaunchError> {
        let index = self
            .slots
            .iter()
            .position(|slot| {
                slot.as_ref()
                    .is_some_and(|managed| managed.task_id == task_id)
            })
            .ok_or(LaunchError::ImageUnavailable)?;
        if self.slots[index]
            .as_ref()
            .is_some_and(|managed| managed.process.completed)
        {
            return Err(LaunchError::InvalidResult);
        }
        self.release_endpoints(index);
        let managed = self.slots[index].as_mut().expect("killed process exists");
        managed.process.completed = true;
        managed.process.killed = true;
        managed.process.event = ProcessEvent::Exit;
        managed.process.exit_code = 137;
        managed.process.completion_order = COMPLETION_SEQUENCE.fetch_add(1, Ordering::AcqRel) + 1;
        managed.state = ManagedState::Killed;
        managed.revoke_resources();
        let pid = managed.process.pid;
        reclaim_process(&mut managed.process).map_err(|_| LaunchError::InvalidResult)?;
        crate::serial::print("USER_KILLED pid=");
        crate::serial::print_u64(pid as u64);
        crate::serial::print(" task=");
        crate::serial::print_u64(task_id as u64);
        crate::serial::println("");
        let update = ProcessUpdate {
            task_id,
            pid,
            state: ManagedState::Killed,
            exit_code: managed.process.exit_code,
            preemptions: managed.process.preemptions,
            output: FixedText::empty(),
            console_process: managed.process.console_handle != 0,
            console: None,
            vfs_request: None,
        };
        self.wake_waiting_parent(pid, 137);
        Ok(update)
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
        if !self.slots[index]
            .as_ref()
            .ok_or(LaunchError::ImageUnavailable)?
            .process
            .completed
        {
            return Err(LaunchError::InvalidResult);
        }
        self.release_endpoints(index);
        let managed = self.slots[index]
            .as_ref()
            .ok_or(LaunchError::ImageUnavailable)?;
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

    pub fn console_process_active(&self) -> bool {
        self.slots
            .iter()
            .flatten()
            .any(|managed| managed.process.console_handle != 0 && !managed.process.completed)
    }

    pub fn console_input_ready(&self) -> bool {
        self.slots.iter().flatten().any(|managed| {
            managed.process.console_handle != 0
                && !managed.process.completed
                && managed.state == ManagedState::Waiting
                && managed.blocked_on == BlockReason::Input
        })
    }
}

impl Default for ProcessManager {
    fn default() -> Self {
        Self::new()
    }
}

fn process_update(managed: &ManagedProcess) -> ProcessUpdate {
    ProcessUpdate {
        task_id: managed.task_id,
        pid: managed.process.pid,
        state: managed.state,
        exit_code: managed.process.exit_code,
        preemptions: managed.process.preemptions,
        output: FixedText::empty(),
        console_process: managed.process.console_handle != 0,
        console: None,
        vfs_request: None,
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
    while live > 0 && switches < 64 {
        if !processes[cursor].completed {
            run_slice(&mut processes[cursor]);
            switches = switches.saturating_add(1);
            match processes[cursor].event {
                ProcessEvent::Yield | ProcessEvent::Preempt => {}
                ProcessEvent::Exit | ProcessEvent::Fault => live -= 1,
                ProcessEvent::Sleep(_)
                | ProcessEvent::WaitChild(_)
                | ProcessEvent::ReadFile { .. }
                | ProcessEvent::ReadDirectory { .. }
                | ProcessEvent::OpenFile { .. }
                | ProcessEvent::ReadHandle { .. }
                | ProcessEvent::StatHandle { .. }
                | ProcessEvent::CloseHandle(_)
                | ProcessEvent::WriteHandle { .. }
                | ProcessEvent::WaitInput { .. }
                | ProcessEvent::CreateEndpoint
                | ProcessEvent::ConnectEndpoint(_)
                | ProcessEvent::SendEndpoint { .. }
                | ProcessEvent::ReceiveEndpoint { .. }
                | ProcessEvent::CloseEndpoint(_)
                | ProcessEvent::ConsoleWrite { .. }
                | ProcessEvent::ConsoleSetInput(_)
                | ProcessEvent::ConsoleClear => fail("USER_PROBE_BLOCKED"),
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

pub fn register_shell_elf(elf_bytes: &'static [u8]) {
    unsafe {
        core::ptr::addr_of_mut!(SHELL_ELF_ADDRESS).write(elf_bytes.as_ptr() as u64);
        core::ptr::addr_of_mut!(SHELL_ELF_LENGTH).write(elf_bytes.len());
    }
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

pub fn copy_out_passed() -> bool {
    COPY_OUT_PASSED.load(Ordering::Acquire)
}

pub fn completed_file_read_count() -> u64 {
    COMPLETED_FILE_READS.load(Ordering::Acquire)
}

pub fn completed_file_write_count() -> u64 {
    COMPLETED_FILE_WRITES.load(Ordering::Acquire)
}

pub fn completed_input_wait_count() -> u64 {
    COMPLETED_INPUT_WAITS.load(Ordering::Acquire)
}

pub fn is_user_writable_path(path: &str) -> bool {
    path.starts_with(USER_WRITABLE_PREFIX) && path.len() > USER_WRITABLE_PREFIX.len()
}

pub fn opened_file_handle_count() -> u64 {
    OPENED_FILE_HANDLES.load(Ordering::Acquire)
}

pub fn completed_endpoint_message_count() -> u64 {
    COMPLETED_ENDPOINT_MESSAGES.load(Ordering::Acquire)
}

pub fn channel_fairness_denial_count() -> u64 {
    ENDPOINT_FAIRNESS_DENIALS.load(Ordering::Acquire)
}

pub fn endpoint_wake_count() -> u64 {
    ENDPOINT_WAKES.load(Ordering::Acquire)
}

pub fn closed_file_handle_count() -> u64 {
    CLOSED_FILE_HANDLES.load(Ordering::Acquire)
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

/// Latches the last line each pid wrote. A write syscall only marks its line
/// pending; the line is handed to whichever poll observes the process next, and
/// a timer preemption can land between the write and the syscall the probe is
/// waiting for. Stages therefore ask the latch what a pid printed instead of
/// demanding the line on one specific update.
struct OutputLatch {
    lines: [(u8, FixedText); MAX_ASYNC_PROCESSES],
    used: usize,
}

impl OutputLatch {
    const fn new() -> Self {
        Self {
            lines: [(0, FixedText::empty()); MAX_ASYNC_PROCESSES],
            used: 0,
        }
    }

    fn observe(&mut self, update: &ProcessUpdate) {
        if update.output.is_empty() {
            return;
        }
        if let Some(entry) = self.lines[..self.used]
            .iter_mut()
            .find(|(pid, _)| *pid == update.pid)
        {
            entry.1 = update.output;
            return;
        }
        if self.used < self.lines.len() {
            self.lines[self.used] = (update.pid, update.output);
            self.used += 1;
        }
    }

    fn text(&self, pid: u8) -> &str {
        self.lines[..self.used]
            .iter()
            .find(|(entry, _)| *entry == pid)
            .map_or("", |(_, text)| text.as_str())
    }
}

pub fn run_lifecycle_probe(vfs: &mut RamVfs) {
    /// Upper bound on the polls a single-process stage spends waiting for its
    /// next observation, so an extra preemption cannot strand the stage.
    const PROBE_POLL_BUDGET: usize = 8;
    const NORMAL_TASK: u32 = 0x1000;
    const HOLD_TASK: u32 = 0x1001;
    const PARENT_TASK: u32 = 0x1002;
    const CHILD_TASK: u32 = 0x1003;
    const FILE_TASK: u32 = 0x1004;
    const WRITE_TASK: u32 = 0x1005;
    const INPUT_TASK: u32 = 0x1006;
    const INPUT_CONTENDER_TASK: u32 = 0x1007;
    const FANIN_RECEIVER_TASK: u32 = 0x1008;
    const FANIN_A_TASK: u32 = 0x1009;
    const FANIN_B_TASK: u32 = 0x100a;

    let reclaimed_before = reclaimed_frame_count();
    let mut manager = ProcessManager::new();

    if manager.spawn_init(NORMAL_TASK, false).is_err() {
        fail("USER_ASYNC_SPAWN_FAILED");
    }
    let mut outputs = OutputLatch::new();
    let mut saw_ready = false;
    let mut async_exit = None;
    for _ in 0..PROBE_POLL_BUDGET {
        let Some(update) = manager.poll(0) else {
            continue;
        };
        outputs.observe(&update);
        match update.state {
            ManagedState::Ready => saw_ready = true,
            ManagedState::Exited => {
                async_exit = Some(update);
                break;
            }
            _ => {}
        }
    }
    if !saw_ready
        || !matches!(async_exit, Some(update) if update.exit_code == 0 && !outputs.text(update.pid).is_empty())
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
    let mut outputs = OutputLatch::new();
    let mut held_pid = None;
    for _ in 0..PROBE_POLL_BUDGET {
        let Some(update) = manager.poll(0) else {
            continue;
        };
        outputs.observe(&update);
        if update.state != ManagedState::Ready {
            fail("USER_ASYNC_HOLD_FAILED");
        }
        held_pid = Some(update.pid);
        if !outputs.text(update.pid).is_empty() {
            break;
        }
    }
    if !matches!(held_pid, Some(pid) if !outputs.text(pid).is_empty()) {
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

    let (parent_pid, child_pid) = manager
        .spawn_coordination_pair(PARENT_TASK, CHILD_TASK)
        .unwrap_or_else(|_| fail("USER_PAIR_SPAWN_FAILED"));
    let mut outputs = OutputLatch::new();
    let mut saw_parent_wait = false;
    let mut saw_child_sleep = false;
    let mut saw_child_exit = false;
    let mut saw_parent_exit = false;
    for tick in 1..=24 {
        if let Some(update) = manager.poll(tick) {
            outputs.observe(&update);
            saw_parent_wait |= update.pid == parent_pid && update.state == ManagedState::Waiting;
            saw_child_sleep |= update.pid == child_pid && update.state == ManagedState::Sleeping;
            saw_child_exit |= update.pid == child_pid
                && update.state == ManagedState::Exited
                && update.exit_code == 7;
            saw_parent_exit |= update.pid == parent_pid
                && update.state == ManagedState::Exited
                && update.exit_code == 0;
        }
    }
    if !saw_parent_wait
        || !saw_child_sleep
        || !saw_child_exit
        || !saw_parent_exit
        || outputs.text(parent_pid).is_empty()
    {
        fail("USER_COORDINATION_FAILED");
    }
    if !matches!(manager.wait(CHILD_TASK), Ok(result) if result.pid == child_pid && result.exit_code == 7)
        || !matches!(manager.wait(PARENT_TASK), Ok(result) if result.pid == parent_pid && result.exit_code == 0)
        || manager.live_count() != 0
        || active_process_count() != 0
        || reclaimed_frame_count() < reclaimed_before + 40
    {
        fail("USER_COORDINATION_REAP_FAILED");
    }
    crate::serial::println("USER_SLEEP_OK");
    crate::serial::println("USER_CHILD_WAIT_OK");
    crate::serial::println("USER_MESSAGE_OK");
    crate::serial::println("USER_COORDINATION_OK");

    // The fan-in image sleeps 5 / 10 / 800 ticks before its first send, and
    // producer A waits another 2,000 after its refused send, so the stage cannot
    // finish before tick FANIN_FIRST_TICK + 2_005. That retry gap is deliberately
    // wide: the receiver wakes at 800 and every receive yields, so it needs two
    // more slices to drain A1 and B1 and park on the empty queue before A2 lands
    // and wakes it directly. On top of those deadlines each producer may spend
    // bounded connect retries (ten ticks apart) waiting for the receiver's
    // endpoint to appear, and producer B sleeps a further 200 ticks after its
    // connect so A1 is queued first no matter how the retries interleaved. Here
    // the receiver is spawned first and publishes before either producer wakes,
    // so neither cost is actually paid, but the window carries slack for both
    // plus the extra slices each process needs on either side of a sleep. It is
    // a per-stage tick label, and every fan-in process is reaped here, so later
    // stages restart their own numbering safely.
    const FANIN_FIRST_TICK: u64 = 25;
    const FANIN_LAST_TICK: u64 = FANIN_FIRST_TICK + 2_200;

    let messages_before = completed_endpoint_message_count();
    let denials_before = channel_fairness_denial_count();
    let wakes_before = endpoint_wake_count();
    let (receiver_pid, a_pid, b_pid) = manager
        .spawn_endpoint_fan_in(FANIN_RECEIVER_TASK, FANIN_A_TASK, FANIN_B_TASK)
        .unwrap_or_else(|_| fail("USER_FANIN_SPAWN_FAILED"));
    let mut outputs = OutputLatch::new();
    let mut saw_receiver_sleep = false;
    let mut saw_a_sleep = false;
    let mut saw_b_sleep = false;
    let mut saw_receiver_block = false;
    let mut saw_receiver_exit = false;
    let mut saw_a_exit = false;
    let mut saw_b_exit = false;
    for tick in FANIN_FIRST_TICK..=FANIN_LAST_TICK {
        // Draining every ready process before the tick advances is what makes
        // the fan-in order deterministic: both producers reach their sends
        // while the receiver is still asleep, and producer A only retries its
        // refused send after the queue has been drained.
        while let Some(update) = manager.poll(tick) {
            outputs.observe(&update);
            saw_receiver_sleep |=
                update.pid == receiver_pid && update.state == ManagedState::Sleeping;
            saw_a_sleep |= update.pid == a_pid && update.state == ManagedState::Sleeping;
            saw_b_sleep |= update.pid == b_pid && update.state == ManagedState::Sleeping;
            // Exactly two messages have been consumed when the third receive
            // parks, which separates the endpoint block from the later
            // child-wait blocks.
            saw_receiver_block |= update.pid == receiver_pid
                && update.state == ManagedState::Waiting
                && completed_endpoint_message_count() == messages_before + 2;
            saw_receiver_exit |= update.pid == receiver_pid
                && update.state == ManagedState::Exited
                && update.exit_code == 0;
            saw_a_exit |= update.pid == a_pid
                && update.state == ManagedState::Exited
                && update.exit_code == 0;
            saw_b_exit |= update.pid == b_pid
                && update.state == ManagedState::Exited
                && update.exit_code == 0;
        }
    }
    if !saw_receiver_sleep
        || !saw_a_sleep
        || !saw_b_sleep
        || !saw_receiver_block
        || !saw_receiver_exit
        || !saw_a_exit
        || !saw_b_exit
        || outputs.text(receiver_pid) != "INIT.ELF fan-in A1 B1 A2"
        || completed_endpoint_message_count() != messages_before + 3
        || channel_fairness_denial_count() != denials_before + 1
        || endpoint_wake_count() != wakes_before + 1
    {
        fail("USER_FANIN_PROBE_FAILED");
    }
    if !matches!(manager.wait(FANIN_A_TASK), Ok(result) if result.pid == a_pid && result.exit_code == 0)
        || !matches!(manager.wait(FANIN_B_TASK), Ok(result) if result.pid == b_pid && result.exit_code == 0)
        || !matches!(manager.wait(FANIN_RECEIVER_TASK), Ok(result) if result.pid == receiver_pid && result.exit_code == 0)
        || manager.live_count() != 0
        || active_process_count() != 0
        || reclaimed_frame_count() < reclaimed_before + 70
    {
        fail("USER_FANIN_REAP_FAILED");
    }
    crate::serial::println("USER_ENDPOINT_CAPABILITY_OK");
    crate::serial::println("USER_CHANNEL_FAIRNESS_OK");
    crate::serial::println("USER_ENDPOINT_WAKE_OK");
    crate::serial::println("USER_FANIN_OK");

    if manager.spawn_file_init(FILE_TASK).is_err() {
        fail("USER_FILE_SPAWN_FAILED");
    }
    let opened_before = opened_file_handle_count();
    let closed_before = closed_file_handle_count();
    let reads_before = completed_file_read_count();
    let mut outputs = OutputLatch::new();
    let mut saw_file_wait = false;
    let mut saw_file_exit = false;
    let mut file_pid = 0u8;
    let mut read_offsets = [u64::MAX; 2];
    let mut read_count = 0usize;
    for tick in 70..=112 {
        let Some(update) = manager.poll(tick) else {
            continue;
        };
        outputs.observe(&update);
        if let Some(request) = update.vfs_request {
            saw_file_wait |= update.state == ManagedState::Waiting;
            if manager.poll(tick).is_some() {
                fail("USER_FILE_NOT_BLOCKED");
            }
            match request {
                UserVfsRequest::Open(request) => {
                    let info = vfs.find(request.path.as_str()).and_then(|node| {
                        (node.kind() == NodeKind::File).then_some(FileOpenInfo {
                            size: node.len() as u64,
                            kind: USER_FILE_KIND_REGULAR,
                        })
                    });
                    let mut invalid = request;
                    invalid.pid = invalid.pid.wrapping_add(1);
                    if manager.complete_file_open(invalid, info).is_ok() {
                        fail("USER_FILE_OPEN_IDENTITY_FAILED");
                    }
                    if !matches!(manager.complete_file_open(request, info), Ok(update) if update.state == ManagedState::Ready)
                    {
                        fail("USER_FILE_OPEN_COMPLETION_FAILED");
                    }
                }
                UserVfsRequest::Read(request) => {
                    if request.handle == 0 || read_count >= read_offsets.len() {
                        fail("USER_HANDLE_READ_INVALID");
                    }
                    read_offsets[read_count] = request.offset;
                    read_count += 1;
                    let data = vfs
                        .read(request.path.as_str())
                        .unwrap_or_else(|_| fail("USER_FILE_LOOKUP_FAILED"));
                    let start = (request.offset as usize).min(data.len());
                    let mut invalid = request;
                    invalid.offset = invalid.offset.saturating_add(1);
                    if manager
                        .complete_file_read(invalid, Some(&data[start..]))
                        .is_ok()
                    {
                        fail("USER_HANDLE_READ_IDENTITY_FAILED");
                    }
                    if !matches!(manager.complete_file_read(request, Some(&data[start..])), Ok(update) if update.state == ManagedState::Ready)
                    {
                        fail("USER_FILE_COMPLETION_FAILED");
                    }
                }
                UserVfsRequest::Write(_) => fail("USER_UNEXPECTED_FILE_WRITE"),
                UserVfsRequest::ReadDirectory(_) => fail("USER_UNEXPECTED_DIRECTORY_READ"),
            }
        }
        if update.state == ManagedState::Exited && update.exit_code == 0 {
            saw_file_exit = true;
            file_pid = update.pid;
        }
    }
    if !saw_file_exit
        || outputs.text(file_pid) != "INIT.ELF used open/read/stat/close"
        || !saw_file_wait
        || read_count != 2
        || read_offsets != [0, 17]
        || opened_file_handle_count() != opened_before + 1
        || closed_file_handle_count() != closed_before + 1
        || completed_file_read_count() != reads_before + 2
        || !COPY_OUT_PASSED.load(Ordering::Acquire)
        || !matches!(manager.wait(FILE_TASK), Ok(result) if result.exit_code == 0)
        || manager.live_count() != 0
        || active_process_count() != 0
        || reclaimed_frame_count() < reclaimed_before + 80
    {
        fail("USER_FILE_PROBE_FAILED");
    }
    crate::serial::println("USER_STRUCT_COPY_OK");
    crate::serial::println("USER_VFS_BLOCKING_OK");
    crate::serial::println("USER_FILE_CAPABILITY_OK");
    crate::serial::println("USER_FILE_OFFSET_OK");
    crate::serial::println("USER_FILE_CLOSE_OK");

    if manager.spawn_write_init(WRITE_TASK).is_err() {
        fail("USER_FILE_WRITE_SPAWN_FAILED");
    }
    let opened_before = opened_file_handle_count();
    let closed_before = closed_file_handle_count();
    let reads_before = completed_file_read_count();
    let writes_before = completed_file_write_count();
    let mut write_offsets = [u64::MAX; 2];
    let mut write_count = 0usize;
    let mut outputs = OutputLatch::new();
    let mut saw_write_wait = false;
    let mut saw_write_exit = false;
    let mut write_pid = 0u8;
    for tick in 120..=190 {
        let Some(update) = manager.poll(tick) else {
            continue;
        };
        outputs.observe(&update);
        if let Some(request) = update.vfs_request {
            saw_write_wait |= update.state == ManagedState::Waiting;
            if manager.poll(tick).is_some() {
                fail("USER_FILE_WRITE_NOT_BLOCKED");
            }
            match request {
                UserVfsRequest::Open(request) => {
                    let writable = request.rights & USER_FILE_RIGHT_WRITE != 0;
                    if writable && !is_user_writable_path(request.path.as_str()) {
                        fail("USER_FILE_WRITE_POLICY_BYPASSED");
                    }
                    if writable && vfs.find(request.path.as_str()).is_none() {
                        vfs.touch(request.path.as_str())
                            .unwrap_or_else(|_| fail("USER_FILE_CREATE_FAILED"));
                    }
                    let info = vfs.find(request.path.as_str()).and_then(|node| {
                        (node.kind() == NodeKind::File).then_some(FileOpenInfo {
                            size: node.len() as u64,
                            kind: USER_FILE_KIND_REGULAR,
                        })
                    });
                    if !matches!(manager.complete_file_open(request, info), Ok(update) if update.state == ManagedState::Ready)
                    {
                        fail("USER_FILE_WRITE_OPEN_FAILED");
                    }
                }
                UserVfsRequest::Write(request) => {
                    if write_count >= write_offsets.len() || request.data.is_empty() {
                        fail("USER_FILE_WRITE_REQUEST_INVALID");
                    }
                    write_offsets[write_count] = request.offset;
                    write_count += 1;
                    let mut invalid = request;
                    invalid.offset = invalid.offset.saturating_add(1);
                    if manager.complete_file_write(invalid, Some(0)).is_ok() {
                        fail("USER_FILE_WRITE_IDENTITY_FAILED");
                    }
                    let written = vfs
                        .write_at(
                            request.path.as_str(),
                            request.offset as usize,
                            request.data.as_slice(),
                        )
                        .unwrap_or_else(|_| fail("USER_FILE_WRITE_VFS_FAILED"));
                    if !matches!(manager.complete_file_write(request, Some(written as u64)), Ok(update) if update.state == ManagedState::Ready)
                    {
                        fail("USER_FILE_WRITE_COMPLETION_FAILED");
                    }
                }
                UserVfsRequest::Read(request) => {
                    let data = vfs
                        .read(request.path.as_str())
                        .unwrap_or_else(|_| fail("USER_FILE_WRITE_READBACK_MISSING"));
                    let start = (request.offset as usize).min(data.len());
                    if !matches!(manager.complete_file_read(request, Some(&data[start..])), Ok(update) if update.state == ManagedState::Ready)
                    {
                        fail("USER_FILE_WRITE_READBACK_FAILED");
                    }
                }
                UserVfsRequest::ReadDirectory(_) => fail("USER_UNEXPECTED_DIRECTORY_READ"),
            }
        }
        if update.state == ManagedState::Exited && update.exit_code == 0 {
            saw_write_exit = true;
            write_pid = update.pid;
        }
    }
    if !saw_write_exit
        || outputs.text(write_pid) != "INIT.ELF wrote and verified /USER/APP.TXT"
        || !saw_write_wait
        || write_count != 2
        || write_offsets != [0, 13]
        || opened_file_handle_count() != opened_before + 2
        || closed_file_handle_count() != closed_before + 2
        || completed_file_write_count() != writes_before + 2
        || completed_file_read_count() != reads_before + 1
        || vfs.read("/USER/APP.TXT") != Ok(&b"GenOS Ring 3 writes safely."[..])
        || !matches!(manager.wait(WRITE_TASK), Ok(result) if result.exit_code == 0)
        || manager.live_count() != 0
        || active_process_count() != 0
        || reclaimed_frame_count() < reclaimed_before + 90
    {
        fail("USER_FILE_WRITE_PROBE_FAILED");
    }
    crate::serial::println("USER_FILE_WRITE_OK");
    crate::serial::println("USER_FILE_WRITE_POLICY_OK");
    crate::serial::println("USER_FILE_WRITE_READBACK_OK");

    if manager.spawn_input_init(INPUT_TASK).is_err() {
        fail("USER_INPUT_SPAWN_FAILED");
    }
    let input_before = completed_input_wait_count();
    let mut outputs = OutputLatch::new();
    let mut saw_input_wait = false;
    let mut input_contender_spawned = false;
    let mut saw_input_busy = false;
    let mut saw_input_wake = false;
    let mut saw_input_exit = false;
    for tick in 191..=260 {
        if saw_input_wait && !input_contender_spawned {
            if manager.spawn_input_init(INPUT_CONTENDER_TASK).is_err() {
                fail("USER_INPUT_CONTENDER_SPAWN_FAILED");
            }
            input_contender_spawned = true;
        }
        if !saw_input_wake && saw_input_busy {
            if manager
                .deliver_input(InputEvent::MouseMove {
                    dx: 4,
                    dy: -2,
                    buttons: MouseButtons::empty(),
                })
                .unwrap_or_else(|_| fail("USER_INPUT_FILTER_FAILED"))
                .is_some()
            {
                fail("USER_INPUT_FILTER_BYPASSED");
            }
            let update = manager
                .deliver_input(InputEvent::Key(KeyEvent::Char(b'G')))
                .unwrap_or_else(|_| fail("USER_INPUT_DELIVERY_FAILED"))
                .unwrap_or_else(|| fail("USER_INPUT_WAITER_MISSING"));
            if update.state != ManagedState::Ready {
                fail("USER_INPUT_NOT_READY");
            }
            saw_input_wake = true;
        }
        let Some(update) = manager.poll(tick) else {
            continue;
        };
        outputs.observe(&update);
        if update.state == ManagedState::Waiting {
            saw_input_wait = true;
            if !input_contender_spawned && manager.poll(tick).is_some() {
                fail("USER_INPUT_NOT_BLOCKED");
            }
        }
        saw_input_busy |= update.task_id == INPUT_CONTENDER_TASK
            && update.state == ManagedState::Exited
            && update.exit_code == 0
            && outputs.text(update.pid) == "INIT.ELF input channel is busy";
        saw_input_exit |= update.task_id == INPUT_TASK
            && update.state == ManagedState::Exited
            && update.exit_code == 0
            && outputs.text(update.pid) == "INIT.ELF received key: G";
    }
    if !saw_input_wait
        || !saw_input_wake
        || !saw_input_busy
        || !saw_input_exit
        || completed_input_wait_count() != input_before + 1
        || !matches!(manager.wait(INPUT_TASK), Ok(result) if result.exit_code == 0)
        || !matches!(manager.wait(INPUT_CONTENDER_TASK), Ok(result) if result.exit_code == 0)
        || manager.live_count() != 0
        || active_process_count() != 0
        || reclaimed_frame_count() < reclaimed_before + 110
    {
        fail("USER_INPUT_PROBE_FAILED");
    }
    crate::serial::println("USER_INPUT_BLOCK_OK");
    crate::serial::println("USER_INPUT_FILTER_OK");
    crate::serial::println("USER_INPUT_OWNERSHIP_OK");
    crate::serial::println("USER_INPUT_WAKE_OK");
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

fn shell_elf() -> Result<&'static [u8], LaunchError> {
    let address = unsafe { *core::ptr::addr_of!(SHELL_ELF_ADDRESS) };
    let length = unsafe { *core::ptr::addr_of!(SHELL_ELF_LENGTH) };
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
        console_handle: 0,
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
        core::ptr::write_volatile(
            (process.data_frame + core::mem::offset_of!(UserProcessHeader, preemptions) as u64)
                as *mut u64,
            process.preemptions,
        );
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
        Ok(SyscallAction::Sleep { ticks }) => {
            frame.rax = 0;
            process.context = *frame;
            process.event = ProcessEvent::Sleep(ticks);
            1
        }
        Ok(SyscallAction::WaitChild { pid }) => {
            frame.rax = 0;
            process.context = *frame;
            process.event = ProcessEvent::WaitChild(pid);
            1
        }
        Ok(SyscallAction::SystemInfo { address, length }) => {
            let info = UserSystemInfo {
                abi_version: USER_ABI_VERSION,
                page_size: paging::PAGE_SIZE,
                timer_hz: USER_TIMER_HZ,
                // `message_capacity` keeps its ABI 8 value; it now reports the
                // depth of one endpoint queue.
                message_capacity: ENDPOINT_QUEUE_CAPACITY as u64,
                max_file_read: USER_FILE_READ_MAX as u64,
                file_handle_capacity: USER_FILE_HANDLE_CAPACITY,
                max_file_write: USER_FILE_WRITE_MAX as u64,
                input_event_size: core::mem::size_of::<UserInputEvent>() as u64,
                input_mask: USER_INPUT_MASK_ALL,
                endpoint_handle_capacity: USER_ENDPOINT_HANDLE_CAPACITY,
                channel_message_size: USER_CHANNEL_MESSAGE_SIZE,
                directory_entry_size: core::mem::size_of::<UserDirectoryEntry>() as u64,
                max_path_length: USER_PATH_MAX as u64,
            };
            let bytes = unsafe {
                core::slice::from_raw_parts(
                    core::ptr::addr_of!(info).cast::<u8>(),
                    core::mem::size_of::<UserSystemInfo>(),
                )
            };
            if length as usize == bytes.len() && copy_to_user_data(process, address, bytes) {
                frame.rax = length;
                if !COPY_OUT_PASSED.swap(true, Ordering::AcqRel) {
                    crate::serial::println("USER_COPY_OUT_OK");
                }
            } else {
                frame.rax = syscall::error_code(syscall::SyscallError::InvalidArgument);
            }
            0
        }
        Ok(SyscallAction::ReadFile {
            path_address,
            path_length,
            output_address,
            output_capacity,
        }) => {
            let path = copy_user_path(process, path_address, path_length);
            if let Some(path) =
                path.filter(|_| valid_user_data_buffer(process, output_address, output_capacity))
            {
                frame.rax = 0;
                process.context = *frame;
                process.event = ProcessEvent::ReadFile {
                    path,
                    address: output_address,
                    capacity: output_capacity,
                };
                1
            } else {
                frame.rax = syscall::error_code(syscall::SyscallError::InvalidArgument);
                0
            }
        }
        Ok(SyscallAction::OpenFile {
            path_address,
            path_length,
        }) => {
            if let Some(path) = copy_user_path(process, path_address, path_length) {
                frame.rax = 0;
                process.context = *frame;
                process.event = ProcessEvent::OpenFile {
                    path,
                    rights: USER_FILE_RIGHT_READ,
                };
                1
            } else {
                frame.rax = syscall::error_code(syscall::SyscallError::InvalidArgument);
                0
            }
        }
        Ok(SyscallAction::ReadHandle {
            handle,
            output_address,
            output_capacity,
        }) => {
            if valid_user_data_buffer(process, output_address, output_capacity) {
                frame.rax = 0;
                process.context = *frame;
                process.event = ProcessEvent::ReadHandle {
                    handle,
                    address: output_address,
                    capacity: output_capacity,
                };
                1
            } else {
                frame.rax = syscall::error_code(syscall::SyscallError::InvalidArgument);
                0
            }
        }
        Ok(SyscallAction::StatHandle {
            handle,
            output_address,
            output_length,
        }) => {
            if valid_user_data_buffer(process, output_address, output_length) {
                frame.rax = 0;
                process.context = *frame;
                process.event = ProcessEvent::StatHandle {
                    handle,
                    address: output_address,
                    length: output_length,
                };
                1
            } else {
                frame.rax = syscall::error_code(syscall::SyscallError::InvalidArgument);
                0
            }
        }
        Ok(SyscallAction::CloseHandle { handle }) => {
            frame.rax = 0;
            process.context = *frame;
            process.event = ProcessEvent::CloseHandle(handle);
            1
        }
        Ok(SyscallAction::OpenFileWithRights {
            path_address,
            path_length,
            rights,
        }) => {
            if let Some(path) = copy_user_path(process, path_address, path_length) {
                frame.rax = 0;
                process.context = *frame;
                process.event = ProcessEvent::OpenFile { path, rights };
                1
            } else {
                frame.rax = syscall::error_code(syscall::SyscallError::InvalidArgument);
                0
            }
        }
        Ok(SyscallAction::WriteHandle {
            handle,
            input_address,
            input_length,
        }) => {
            if let Some(data) = copy_user_bytes(process, input_address, input_length) {
                frame.rax = 0;
                process.context = *frame;
                process.event = ProcessEvent::WriteHandle { handle, data };
                1
            } else {
                frame.rax = syscall::error_code(syscall::SyscallError::InvalidArgument);
                0
            }
        }
        Ok(SyscallAction::WaitInput {
            output_address,
            output_length,
            mask,
        }) => {
            if output_length as usize == core::mem::size_of::<UserInputEvent>()
                && valid_user_data_buffer(process, output_address, output_length)
            {
                frame.rax = 0;
                process.context = *frame;
                process.event = ProcessEvent::WaitInput {
                    address: output_address,
                    length: output_length,
                    mask,
                };
                1
            } else {
                frame.rax = syscall::error_code(syscall::SyscallError::InvalidArgument);
                0
            }
        }
        Ok(SyscallAction::CreateEndpoint) => {
            frame.rax = 0;
            process.context = *frame;
            process.event = ProcessEvent::CreateEndpoint;
            1
        }
        Ok(SyscallAction::ConnectEndpoint { pid }) => {
            frame.rax = 0;
            process.context = *frame;
            process.event = ProcessEvent::ConnectEndpoint(pid);
            1
        }
        Ok(SyscallAction::SendEndpoint { handle, value }) => {
            frame.rax = 0;
            process.context = *frame;
            process.event = ProcessEvent::SendEndpoint { handle, value };
            1
        }
        Ok(SyscallAction::ReceiveEndpoint {
            handle,
            output_address,
            output_length,
        }) => {
            if output_length == USER_CHANNEL_MESSAGE_SIZE
                && valid_user_data_buffer(process, output_address, output_length)
            {
                frame.rax = 0;
                process.context = *frame;
                process.event = ProcessEvent::ReceiveEndpoint {
                    handle,
                    address: output_address,
                    length: output_length,
                };
                1
            } else {
                frame.rax = syscall::error_code(syscall::SyscallError::InvalidArgument);
                0
            }
        }
        Ok(SyscallAction::CloseEndpoint { handle }) => {
            frame.rax = 0;
            process.context = *frame;
            process.event = ProcessEvent::CloseEndpoint(handle);
            1
        }
        Ok(SyscallAction::ConsoleWrite {
            handle,
            address,
            length,
            kind,
        }) => {
            if syscall::console_capability_valid(process.console_handle, handle) {
                if let (Some(text), Some(kind)) = (
                    copy_user_text(process, address, length),
                    console_line_kind(kind),
                ) {
                    frame.rax = 0;
                    process.context = *frame;
                    process.event = ProcessEvent::ConsoleWrite { text, kind };
                    return 1;
                }
            }
            frame.rax = syscall::error_code(syscall::SyscallError::InvalidArgument);
            0
        }
        Ok(SyscallAction::ConsoleSetInput {
            handle,
            address,
            length,
        }) => {
            let text = if length == 0 {
                Some(FixedText::empty())
            } else {
                copy_user_text(process, address, length)
            };
            if syscall::console_capability_valid(process.console_handle, handle) {
                if let Some(text) = text {
                    frame.rax = 0;
                    process.context = *frame;
                    process.event = ProcessEvent::ConsoleSetInput(text);
                    return 1;
                }
            }
            frame.rax = syscall::error_code(syscall::SyscallError::InvalidArgument);
            0
        }
        Ok(SyscallAction::ConsoleClear { handle }) => {
            if syscall::console_capability_valid(process.console_handle, handle) {
                frame.rax = 0;
                process.context = *frame;
                process.event = ProcessEvent::ConsoleClear;
                1
            } else {
                frame.rax = syscall::error_code(syscall::SyscallError::InvalidArgument);
                0
            }
        }
        Ok(SyscallAction::ReadDirectory {
            handle,
            cursor,
            output_address,
            output_length,
        }) => {
            if valid_user_data_buffer(process, output_address, output_length)
                && output_length as usize == core::mem::size_of::<UserDirectoryEntry>()
            {
                frame.rax = 0;
                process.context = *frame;
                process.event = ProcessEvent::ReadDirectory {
                    handle,
                    cursor,
                    address: output_address,
                    length: output_length,
                };
                1
            } else {
                frame.rax = syscall::error_code(syscall::SyscallError::InvalidArgument);
                0
            }
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
        // The legacy direct-PID actions stay reserved in the shared ABI but are
        // no longer reachable: `dispatch` rejects their syscall numbers.
        Ok(SyscallAction::Send { .. }) | Ok(SyscallAction::Receive) => {
            frame.rax = syscall::error_code(syscall::SyscallError::UnknownNumber);
            0
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

fn console_line_kind(kind: u64) -> Option<LineKind> {
    match kind {
        genos_abi::USER_CONSOLE_LINE_OUTPUT => Some(LineKind::Output),
        genos_abi::USER_CONSOLE_LINE_PROMPT => Some(LineKind::Prompt),
        genos_abi::USER_CONSOLE_LINE_ERROR => Some(LineKind::Error),
        genos_abi::USER_CONSOLE_LINE_STATUS => Some(LineKind::Status),
        _ => None,
    }
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

fn copy_user_bytes(process: &UserProcess, address: u64, length: u64) -> Option<FileWriteBuffer> {
    if length == 0
        || length > USER_FILE_WRITE_MAX as u64
        || !syscall::validate_user_buffer(
            address,
            length,
            paging::USER_CODE,
            paging::USER_STACK_TOP - paging::USER_CODE,
        )
    {
        return None;
    }
    let mut data = FileWriteBuffer::empty();
    data.len = length as usize;
    for (index, slot) in data.bytes.iter_mut().take(data.len).enumerate() {
        let virtual_address = address.checked_add(index as u64)?;
        paging::translate(process.space, virtual_address)?;
        *slot = unsafe { core::ptr::read_volatile(virtual_address as *const u8) };
    }
    Some(data)
}

fn copy_user_path(process: &UserProcess, address: u64, length: u64) -> Option<FixedText> {
    let path = copy_user_text(process, address, length)?;
    if !path.as_str().starts_with('/')
        || !path
            .as_str()
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'.' | b'_' | b'-'))
    {
        return None;
    }
    Some(path)
}

fn valid_user_data_buffer(process: &UserProcess, address: u64, length: u64) -> bool {
    if !syscall::validate_user_buffer(address, length, paging::USER_DATA, paging::PAGE_SIZE) {
        return false;
    }
    for offset in 0..length {
        let virtual_address = address + offset;
        let Some(physical) = paging::translate(process.space, virtual_address) else {
            return false;
        };
        if physical != process.data_frame + (virtual_address - paging::USER_DATA) {
            return false;
        }
    }
    true
}

fn channel_message_bytes(message: &UserChannelMessage) -> &[u8] {
    unsafe {
        core::slice::from_raw_parts(
            core::ptr::from_ref(message).cast::<u8>(),
            core::mem::size_of::<UserChannelMessage>(),
        )
    }
}

fn copy_to_user_data(process: &UserProcess, address: u64, bytes: &[u8]) -> bool {
    if bytes.is_empty() {
        return true;
    }
    if !valid_user_data_buffer(process, address, bytes.len() as u64) {
        return false;
    }
    for (offset, byte) in bytes.iter().enumerate() {
        let physical = process.data_frame + (address - paging::USER_DATA) + offset as u64;
        unsafe {
            core::ptr::write_volatile(physical as *mut u8, *byte);
        }
    }
    true
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
            && unsafe {
                core::ptr::read_volatile(
                    (process.data_frame + core::mem::offset_of!(UserProcessHeader, token) as u64)
                        as *const u64,
                )
            } == process.token
            && unsafe {
                core::ptr::read_volatile(
                    (process.data_frame
                        + core::mem::offset_of!(UserProcessHeader, preemptions) as u64)
                        as *const u64,
                )
            } == process.preemptions
    });
    let faulting = &processes[0];
    let healthy = &processes[1..];

    (6..=48).contains(&switches)
        && roots_are_distinct
        && frames_are_distinct
        && mappings_are_private
        && faulting.completed
        && faulting.exit_code == FAULT_EXIT_CODE
        && faulting.fault_vector == 14
        && faulting.fault_error == 0x6
        && faulting.fault_address == paging::USER_STACK_GUARD
        && (1..=16).contains(&faulting.preemptions)
        && faulting.preemption_armed
        && faulting.yields == 0
        && faulting.report == 0
        && faulting.completion_order == 1
        && healthy.iter().all(|process| {
            process.completed
                && process.exit_code == 0
                && process.fault_vector == 0
                && (1..=16).contains(&process.preemptions)
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

/// Host tests for the endpoint capability layer. `EndpointState` deliberately
/// owns no paging or context state, so every rule that decides whether a handle
/// is honoured can be exercised without a real address space.
#[cfg(test)]
mod tests {
    use super::*;

    fn message(sender_pid: u64, value: u64) -> UserChannelMessage {
        UserChannelMessage { sender_pid, value }
    }

    fn published(owner_pid: u8) -> (EndpointState, u64) {
        let mut state = EndpointState::new(owner_pid);
        let handle = state.create().expect("first endpoint publishes");
        (state, handle)
    }

    fn push(state: &mut EndpointState, message: UserChannelMessage) -> bool {
        let endpoint = state.published.as_mut().expect("endpoint is published");
        !endpoint.queue.contains_sender(message.sender_pid) && endpoint.queue.push(message)
    }

    #[test]
    fn receive_handles_are_owned_tagged_and_slot_exact() {
        let (state, handle) = published(7);
        let capability = state.capability(handle).expect("owner resolves its handle");

        assert_eq!(handle & ENDPOINT_HANDLE_TAG_MASK, ENDPOINT_HANDLE_TAG);
        assert_eq!(capability.owner_pid, 7);
        assert_eq!(capability.slot, 0);
        assert_eq!(capability.generation, state.published_generation().unwrap());
        assert_eq!(
            capability.role,
            EndpointRole::Receive {
                generation: state.published_generation().expect("endpoint is published"),
            }
        );
        assert_eq!(
            state.receive_generation(handle),
            state.published_generation()
        );
    }

    #[test]
    fn guessed_and_foreign_handles_never_resolve() {
        let (state, handle) = published(7);
        let neighbour = EndpointState::new(8);

        // Same table position, different owner: the owner field is part of the
        // handle, so pid 8's handle cannot name pid 7's capability.
        assert_eq!(state.capability(handle ^ (1 << 40)), None);
        // Neighbouring slots and generations are not authority either.
        assert_eq!(state.capability(handle + 1), None);
        assert_eq!(state.capability(handle + (1 << 8)), None);
        assert_eq!(state.capability(handle & !ENDPOINT_HANDLE_TAG_MASK), None);
        assert_eq!(state.capability(0), None);
        // A file handle is shaped `pid << 56 | generation << 8 | slot`, which
        // never carries the endpoint tag.
        assert_eq!(state.capability((7u64 << 56) | (1 << 8) | 1), None);
        // Another process cannot spend a handle it does not hold.
        assert_eq!(neighbour.capability(handle), None);
    }

    #[test]
    fn only_one_endpoint_can_be_published_per_process() {
        let (mut state, handle) = published(7);

        assert_eq!(state.create(), None);
        assert!(state.capability(handle).is_some());
        assert_eq!(state.published_generation(), Some(1));
    }

    #[test]
    fn the_handle_table_is_bounded() {
        let mut state = EndpointState::new(7);
        state.create().expect("first endpoint publishes");
        for slot in 1..ENDPOINT_HANDLE_CAPACITY {
            assert!(
                state
                    .allocate(EndpointRole::Send {
                        target_pid: 20 + slot as u8,
                        target_generation: 1,
                    })
                    .is_some(),
                "slot {slot} should be free"
            );
        }
        assert_eq!(
            state.allocate(EndpointRole::Send {
                target_pid: 99,
                target_generation: 1,
            }),
            None
        );
        // A full table also blocks publishing after the endpoint is closed.
        let mut full = EndpointState::new(8);
        for _ in 0..ENDPOINT_HANDLE_CAPACITY {
            full.allocate(EndpointRole::Send {
                target_pid: 9,
                target_generation: 1,
            })
            .expect("slot is free");
        }
        assert_eq!(full.create(), None);
        assert_eq!(full.published_generation(), None);
    }

    #[test]
    fn send_and_receive_capabilities_do_not_substitute_for_each_other() {
        let (mut state, receive) = published(7);
        let send = state
            .allocate(EndpointRole::Send {
                target_pid: 9,
                target_generation: 4,
            })
            .expect("slot is free");

        assert_eq!(state.send_capability(send), Some((9, 4)));
        assert_eq!(state.receive_generation(send), None);
        assert_eq!(state.send_capability(receive), None);
        assert_eq!(state.receive_generation(receive), Some(1));
    }

    #[test]
    fn a_second_message_from_one_producer_is_denied_without_overwrite() {
        let (mut state, _) = published(7);

        assert!(push(&mut state, message(2, 100)));
        assert!(!push(&mut state, message(2, 200)));
        assert!(push(&mut state, message(3, 300)));
        assert_eq!(state.queue_depth(), 2);

        let endpoint = state.published.as_mut().expect("endpoint is published");
        // The first admission survives the denied one untouched.
        assert_eq!(endpoint.queue.pop(), Some(message(2, 100)));
        assert_eq!(endpoint.queue.pop(), Some(message(3, 300)));
        assert_eq!(endpoint.queue.pop(), None);
    }

    #[test]
    fn a_full_queue_denies_further_producers() {
        let (mut state, _) = published(7);
        for sender in 1..=ENDPOINT_QUEUE_CAPACITY as u64 {
            assert!(push(&mut state, message(sender, sender)));
        }
        assert_eq!(state.queue_depth(), ENDPOINT_QUEUE_CAPACITY);
        assert!(!push(&mut state, message(99, 99)));
        assert_eq!(state.queue_depth(), ENDPOINT_QUEUE_CAPACITY);
    }

    #[test]
    fn a_parked_receive_keeps_metadata_that_still_names_the_endpoint() {
        let (mut state, handle) = published(7);
        let generation = state.published_generation().expect("endpoint is published");
        state.pending_receive = Some(PendingReceive {
            handle,
            generation,
            address: 0x4010,
            length: USER_CHANNEL_MESSAGE_SIZE,
        });

        // What a delivering sender re-checks before copying 16 bytes out.
        let pending = state.pending_receive.expect("receive is parked");
        assert_eq!(pending.length, USER_CHANNEL_MESSAGE_SIZE);
        assert_eq!(pending.generation, generation);
        assert_eq!(state.receive_generation(pending.handle), Some(generation));

        // Closing the endpoint retires the parked metadata with it.
        state.close(handle).expect("receive handle closes");
        assert_eq!(state.pending_receive, None);
        assert_eq!(state.receive_generation(handle), None);
    }

    #[test]
    fn closing_a_send_handle_revokes_only_that_handle() {
        let (mut state, receive) = published(7);
        let first = state
            .allocate(EndpointRole::Send {
                target_pid: 9,
                target_generation: 3,
            })
            .expect("slot is free");
        let second = state
            .allocate(EndpointRole::Send {
                target_pid: 10,
                target_generation: 5,
            })
            .expect("slot is free");

        assert_eq!(
            state.close(first),
            Some(EndpointRole::Send {
                target_pid: 9,
                target_generation: 3,
            })
        );
        assert_eq!(state.capability(first), None);
        assert_eq!(state.send_capability(second), Some((10, 5)));
        assert_eq!(state.receive_generation(receive), Some(1));
        assert!(state.published.is_some());
        // Closing the same handle twice is a stale use.
        assert_eq!(state.close(first), None);
    }

    #[test]
    fn closing_the_receive_handle_drops_the_queue_and_the_endpoint() {
        let (mut state, receive) = published(7);
        let send = state
            .allocate(EndpointRole::Send {
                target_pid: 9,
                target_generation: 3,
            })
            .expect("slot is free");
        assert!(push(&mut state, message(2, 100)));

        assert_eq!(
            state.close(receive),
            Some(EndpointRole::Receive { generation: 1 })
        );
        assert_eq!(state.published_generation(), None);
        assert_eq!(state.queue_depth(), 0);
        assert_eq!(state.capability(receive), None);
        // Only the local receive authority goes; the process keeps its own send
        // handles to other endpoints.
        assert_eq!(state.send_capability(send), Some((9, 3)));
    }

    #[test]
    fn revocation_drops_exactly_the_handles_naming_one_endpoint() {
        let mut state = EndpointState::new(7);
        let matching = state
            .allocate(EndpointRole::Send {
                target_pid: 9,
                target_generation: 3,
            })
            .expect("slot is free");
        let other_generation = state
            .allocate(EndpointRole::Send {
                target_pid: 9,
                target_generation: 4,
            })
            .expect("slot is free");
        let other_pid = state
            .allocate(EndpointRole::Send {
                target_pid: 10,
                target_generation: 3,
            })
            .expect("slot is free");

        assert_eq!(state.revoke_send_handles(9, 3), 1);
        assert_eq!(state.capability(matching), None);
        assert_eq!(state.send_capability(other_generation), Some((9, 4)));
        assert_eq!(state.send_capability(other_pid), Some((10, 3)));
        assert_eq!(state.revoke_send_handles(9, 3), 0);
    }

    #[test]
    fn exit_cleanup_clears_every_local_endpoint_resource() {
        let (mut state, receive) = published(7);
        let send = state
            .allocate(EndpointRole::Send {
                target_pid: 9,
                target_generation: 3,
            })
            .expect("slot is free");
        assert!(push(&mut state, message(2, 100)));
        state.pending_receive = Some(PendingReceive {
            handle: receive,
            generation: 1,
            address: 0x4010,
            length: USER_CHANNEL_MESSAGE_SIZE,
        });

        state.clear();

        assert_eq!(state.capability(receive), None);
        assert_eq!(state.capability(send), None);
        assert_eq!(state.published_generation(), None);
        assert_eq!(state.queue_depth(), 0);
        assert_eq!(state.pending_receive, None);
    }

    #[test]
    fn a_reused_slot_never_honours_the_previous_generation() {
        let (mut state, first) = published(7);
        state.close(first).expect("receive handle closes");
        let second = state.create().expect("a new endpoint publishes");

        assert_ne!(first, second);
        assert_eq!(state.capability(first), None);
        assert_eq!(state.receive_generation(first), None);
        assert_eq!(
            state.receive_generation(second),
            state.published_generation()
        );
        // The reissued endpoint carries a fresh generation, so send handles held
        // against the closed one stay unusable even in the same slot.
        assert_eq!(state.published_generation(), Some(2));
    }
}
