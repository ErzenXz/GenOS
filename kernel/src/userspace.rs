use core::arch::global_asm;
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};

use genos_abi::{
    UserChannelMessage, UserDirectoryEntry, UserFileStat, UserInputEvent, UserNetworkConfig,
    UserProcessHeader, UserProcessStatus, UserSocketStatus, UserSystemInfo, USER_ABI_VERSION,
    USER_CHANNEL_MESSAGE_SIZE, USER_ENDPOINT_HANDLE_CAPACITY, USER_ENDPOINT_QUEUE_CAPACITY,
    USER_EXECUTABLE_PAGE_CAPACITY, USER_FILE_HANDLE_CAPACITY, USER_FILE_KIND_DIRECTORY,
    USER_FILE_KIND_REGULAR, USER_FILE_READ_MAX, USER_FILE_RIGHTS_MASK, USER_FILE_RIGHT_MANAGE,
    USER_FILE_RIGHT_READ, USER_FILE_RIGHT_WRITE, USER_FILE_WRITE_MAX, USER_IMAGE_LAYOUT_VERSION,
    USER_INPUT_MASK_ALL, USER_PATH_MAX, USER_PROCESS_HANDLE_CAPACITY, USER_PROCESS_IMAGE_INIT,
    USER_PROCESS_MODE_HOLD, USER_PROCESS_MODE_NORMAL, USER_PROCESS_STATE_EXITED,
    USER_PROCESS_STATE_FAULTED, USER_PROCESS_STATE_KILLED, USER_PROCESS_STATE_READY,
    USER_PROCESS_STATE_SLEEPING, USER_PROCESS_STATE_WAITING, USER_SOCKET_BUFFER_CAPACITY,
    USER_SOCKET_HANDLE_CAPACITY, USER_SOCKET_SHUTDOWN_READ, USER_SOCKET_SHUTDOWN_WRITE,
    USER_TIMER_HZ, USER_WRITABLE_PREFIX,
};
use kernel::{
    capability::{HandleKind, HandleTable},
    display::{FixedText, LineKind},
    elf::{ElfImage, FLAG_EXECUTE, FLAG_READ, FLAG_WRITE},
    input::{InputEvent, KeyEvent, MouseButtons},
    ipc::ChannelQueue,
    request::RequestSequence,
    socket::{
        local_port_is_available, SocketError, SocketOwner, SocketProtocol, SocketSet, TcpServerPeer,
    },
    syscall::{self, SyscallAction},
    tasks::{TaskClass, TaskSnapshot, TaskSnapshotSet, TaskState},
    vfs::{NodeKind, RamVfs},
};

use crate::{arch, memory, network, paging};

pub const SYSCALL_VECTOR: usize = 0x80;
const PROCESS_COUNT: usize = 3;
const HEALTHY_PROCESS_COUNT: u8 = 2;
const FAULT_EXIT_CODE: u8 = 128 + 14;
const TOKEN_FAULT: u64 = 0xffff_ffff_ffff_fff0;
const TOKEN_A: u64 = 0x1111_aaaa_1111_aaaa;
const TOKEN_B: u64 = 0x2222_bbbb_2222_bbbb;
const TOKEN_DYNAMIC_BASE: u64 = 0x3333_cccc_3333_0000;
const TOKEN_HOLD_BIT: u64 = 1 << 63;
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
const HANDLE_TABLE_CAPACITY: usize = 20;
const HANDLE_RIGHT_USE: u64 = 1;
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
static COMPLETION_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static DYNAMIC_PROCESSES: AtomicU64 = AtomicU64::new(0);
static ACTIVE_PROCESSES: AtomicU8 = AtomicU8::new(0);
static RECLAIMED_SPACES: AtomicU64 = AtomicU64::new(0);
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
static NEXT_LIFECYCLE_GENERATION: AtomicU64 = AtomicU64::new(1);
static NEXT_PROCESS_HANDLE_GENERATION: AtomicU64 = AtomicU64::new(1);
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
    TruncateHandle {
        handle: u64,
    },
    CreateDirectory {
        parent: u64,
        name: FixedText,
    },
    RemovePath {
        parent: u64,
        name: FixedText,
    },
    ProcessLaunch {
        supervisor: u64,
        image: u64,
        mode: u64,
    },
    ProcessStatus {
        handle: u64,
        address: u64,
        length: u64,
    },
    ProcessKill {
        handle: u64,
    },
    ProcessReap {
        handle: u64,
        address: u64,
        length: u64,
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
        handle: u64,
        text: FixedText,
        kind: LineKind,
    },
    ConsoleSetInput {
        handle: u64,
        text: FixedText,
    },
    ConsoleClear(u64),
    SocketOpen {
        protocol: u64,
    },
    SocketConnect {
        handle: u64,
        target: u32,
        port: u16,
    },
    SocketBind {
        handle: u64,
        port: u16,
    },
    SocketListen {
        handle: u64,
        backlog: u64,
    },
    SocketAccept {
        handle: u64,
    },
    SocketSend {
        handle: u64,
        data: FileWriteBuffer,
    },
    SocketReceive {
        handle: u64,
        address: u64,
        capacity: u64,
    },
    SocketStatus {
        handle: u64,
        address: u64,
        length: u64,
    },
    SocketShutdown {
        handle: u64,
        direction: u64,
    },
    SocketClose(u64),
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
    completion_order: u64,
    preemption_armed: bool,
    elf_segments: u8,
    elf_pages: u8,
    executable_start: u64,
    executable_end: u64,
    output: FixedText,
    output_pending: bool,
    console_handle: u64,
    lifecycle_handle: u64,
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
pub struct LaunchResult {
    pub pid: u8,
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
    pub lifecycle_request: Option<UserLifecycleRequest>,
    pub socket_request: Option<UserSocketRequest>,
}

#[derive(Clone, Copy)]
pub enum UserLifecycleRequest {
    Launch(ProcessLaunchRequest),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AsyncOperation {
    ProcessLaunch,
    FileOpen,
    FileRead,
    FileWrite,
    FileTruncate,
    DirectoryRead,
    DirectoryCreate,
    PathRemove,
    UdpSocketExchange,
    TcpSocketExchange,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct UserSocketRequest {
    pub request_id: u64,
    pub owner_slot: u8,
    pub owner_instance: u64,
    pub owner_task_id: u32,
    pub owner_pid: u8,
    pub handle: u64,
    pub protocol: SocketProtocol,
    pub target: u32,
    pub port: u16,
    pub data: FileWriteBuffer,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UserTcpListener {
    pub owner_slot: u8,
    pub owner_instance: u64,
    pub owner_pid: u8,
    pub handle: u64,
    pub port: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UserTcpStream {
    pub owner_slot: u8,
    pub owner_instance: u64,
    pub owner_task_id: u32,
    pub owner_pid: u8,
    pub handle: u64,
    pub peer: TcpServerPeer,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct UserTcpStreamSend {
    pub stream: UserTcpStream,
    pub request_id: u64,
    pub data: FileWriteBuffer,
}

impl UserSocketRequest {
    pub const fn identity(self) -> AsyncRequestIdentity {
        AsyncRequestIdentity {
            request_id: self.request_id,
            owner_slot: self.owner_slot,
            owner_instance: self.owner_instance,
            operation: match self.protocol {
                SocketProtocol::Udp => AsyncOperation::UdpSocketExchange,
                SocketProtocol::TcpStream => AsyncOperation::TcpSocketExchange,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AsyncRequestIdentity {
    pub request_id: u64,
    pub owner_slot: u8,
    pub owner_instance: u64,
    pub operation: AsyncOperation,
}

impl UserLifecycleRequest {
    pub const fn identity(self) -> AsyncRequestIdentity {
        match self {
            Self::Launch(request) => AsyncRequestIdentity {
                request_id: request.request_id,
                owner_slot: request.owner_slot,
                owner_instance: request.owner_instance,
                operation: AsyncOperation::ProcessLaunch,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProcessLaunchRequest {
    pub request_id: u64,
    pub owner_slot: u8,
    pub owner_instance: u64,
    pub owner_task_id: u32,
    pub owner_pid: u8,
    pub image: u64,
    pub mode: u64,
}

pub struct ProcessLaunchCompletion {
    pub owner: ProcessUpdate,
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
    Truncate(FileTruncateRequest),
    ReadDirectory(DirectoryReadRequest),
    CreateDirectory(NamespaceMutationRequest),
    RemovePath(NamespaceMutationRequest),
}

impl UserVfsRequest {
    pub const fn identity(self) -> AsyncRequestIdentity {
        match self {
            Self::Open(request) => AsyncRequestIdentity {
                request_id: request.request_id,
                owner_slot: request.owner_slot,
                owner_instance: request.owner_instance,
                operation: AsyncOperation::FileOpen,
            },
            Self::Read(request) => AsyncRequestIdentity {
                request_id: request.request_id,
                owner_slot: request.owner_slot,
                owner_instance: request.owner_instance,
                operation: AsyncOperation::FileRead,
            },
            Self::Write(request) => AsyncRequestIdentity {
                request_id: request.request_id,
                owner_slot: request.owner_slot,
                owner_instance: request.owner_instance,
                operation: AsyncOperation::FileWrite,
            },
            Self::Truncate(request) => AsyncRequestIdentity {
                request_id: request.request_id,
                owner_slot: request.owner_slot,
                owner_instance: request.owner_instance,
                operation: AsyncOperation::FileTruncate,
            },
            Self::ReadDirectory(request) => AsyncRequestIdentity {
                request_id: request.request_id,
                owner_slot: request.owner_slot,
                owner_instance: request.owner_instance,
                operation: AsyncOperation::DirectoryRead,
            },
            Self::CreateDirectory(request) => AsyncRequestIdentity {
                request_id: request.request_id,
                owner_slot: request.owner_slot,
                owner_instance: request.owner_instance,
                operation: AsyncOperation::DirectoryCreate,
            },
            Self::RemovePath(request) => AsyncRequestIdentity {
                request_id: request.request_id,
                owner_slot: request.owner_slot,
                owner_instance: request.owner_instance,
                operation: AsyncOperation::PathRemove,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NamespaceMutationRequest {
    pub request_id: u64,
    pub owner_slot: u8,
    pub owner_instance: u64,
    pub task_id: u32,
    pub pid: u8,
    pub parent: FixedText,
    pub target: FixedText,
    pub handle: u64,
}

#[derive(Clone, Copy)]
pub struct DirectoryReadRequest {
    pub request_id: u64,
    pub owner_slot: u8,
    pub owner_instance: u64,
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
    pub request_id: u64,
    pub owner_slot: u8,
    pub owner_instance: u64,
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
    pub request_id: u64,
    pub owner_slot: u8,
    pub owner_instance: u64,
    pub task_id: u32,
    pub pid: u8,
    pub path: FixedText,
    pub handle: u64,
    pub offset: u64,
    pub capacity: u64,
}

#[derive(Clone, Copy)]
pub struct FileWriteRequest {
    pub request_id: u64,
    pub owner_slot: u8,
    pub owner_instance: u64,
    pub task_id: u32,
    pub pid: u8,
    pub path: FixedText,
    pub handle: u64,
    pub offset: u64,
    pub data: FileWriteBuffer,
}

#[derive(Clone, Copy)]
pub struct FileTruncateRequest {
    pub request_id: u64,
    pub owner_slot: u8,
    pub owner_instance: u64,
    pub task_id: u32,
    pub pid: u8,
    pub path: FixedText,
    pub handle: u64,
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
}

#[derive(Clone, Copy)]
struct TerminalRecord {
    key: ProcessKey,
    task_id: u32,
    pid: u8,
    exit_code: u8,
    preemptions: u64,
    console_process: bool,
}

struct ManagedProcess {
    key: ProcessKey,
    task_id: u32,
    parent_key: Option<ProcessKey>,
    console_process: bool,
    supervisor: bool,
    state: ManagedState,
    wake_at: u64,
    blocked_on: BlockReason,
    request_ids: RequestSequence,
    handles: HandleTable<HANDLE_TABLE_CAPACITY>,
    file_handles: [Option<FileCapability>; FILE_HANDLE_CAPACITY],
    next_file_generation: u64,
    endpoints: EndpointState,
    sockets: SocketSet,
    pending_file_open: Option<PendingFileOpen>,
    pending_file_read: Option<PendingFileRead>,
    pending_file_write: Option<PendingFileWrite>,
    pending_file_truncate: Option<PendingFileTruncate>,
    pending_directory_read: Option<PendingDirectoryRead>,
    pending_namespace_mutation: Option<PendingNamespaceMutation>,
    pending_process_launch: Option<PendingProcessLaunch>,
    process_handles: [Option<ProcessCapability>; PROCESS_HANDLE_CAPACITY],
    pending_input: Option<PendingInput>,
    process: UserProcess,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ProcessKey {
    slot: u8,
    incarnation: u64,
}

fn socket_owner(managed: &ManagedProcess) -> SocketOwner {
    SocketOwner {
        slot: managed.key.slot,
        incarnation: managed.key.incarnation,
    }
}

fn socket_error_code(error: SocketError) -> u64 {
    match error {
        SocketError::InvalidHandle | SocketError::InvalidState => {
            syscall::error_code(syscall::SyscallError::InvalidArgument)
        }
        SocketError::WouldBlock => genos_abi::USER_ERROR_WOULD_BLOCK,
        SocketError::Capacity | SocketError::Unavailable => {
            syscall::error_code(syscall::SyscallError::Unavailable)
        }
    }
}

const PROCESS_HANDLE_CAPACITY: usize = USER_PROCESS_HANDLE_CAPACITY as usize;
const PROCESS_HANDLE_TAG: u64 = 0xd1 << 56;
const PROCESS_HANDLE_GENERATION_MAX: u64 = (1u64 << 48) - 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ProcessCapability {
    handle: u64,
    target: ProcessKey,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PendingProcessLaunch {
    request_id: u64,
    image: u64,
    mode: u64,
}

#[derive(Clone, Copy)]
struct PendingFileRead {
    request_id: u64,
    handle: u64,
    path: FixedText,
    offset: u64,
    address: u64,
    capacity: u64,
}

#[derive(Clone, Copy, Eq, PartialEq)]
struct PendingFileOpen {
    request_id: u64,
    path: FixedText,
    rights: u64,
}

#[derive(Clone, Copy)]
struct PendingFileWrite {
    request_id: u64,
    handle: u64,
    path: FixedText,
    offset: u64,
    data: FileWriteBuffer,
}

#[derive(Clone, Copy, Eq, PartialEq)]
struct PendingFileTruncate {
    request_id: u64,
    handle: u64,
    path: FixedText,
}

#[derive(Clone, Copy, Eq, PartialEq)]
struct PendingDirectoryRead {
    request_id: u64,
    path: FixedText,
    handle: u64,
    cursor: u64,
    address: u64,
    length: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PendingNamespaceMutation {
    request_id: u64,
    parent: FixedText,
    target: FixedText,
    handle: u64,
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

    fn allocate(
        &mut self,
        handles: &mut HandleTable<HANDLE_TABLE_CAPACITY>,
        role: EndpointRole,
    ) -> Option<u64> {
        let slot = self.handles.iter().position(Option::is_none)?;
        let generation = self.next_generation()?;
        let handle = endpoint_handle(self.owner_pid, generation, slot);
        let kind = match role {
            EndpointRole::Receive { .. } => HandleKind::EndpointReceive,
            EndpointRole::Send { .. } => HandleKind::EndpointSend,
        };
        if !handles.register(handle, kind, HANDLE_RIGHT_USE) {
            return None;
        }
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
    fn capability(
        &self,
        handles: &HandleTable<HANDLE_TABLE_CAPACITY>,
        handle: u64,
    ) -> Option<EndpointCapability> {
        let slot = endpoint_handle_slot(handle)?;
        let capability = self.handles[slot]?;
        let kind = match capability.role {
            EndpointRole::Receive { .. } => HandleKind::EndpointReceive,
            EndpointRole::Send { .. } => HandleKind::EndpointSend,
        };
        (capability.handle == handle
            && capability.owner_pid == self.owner_pid
            && capability.slot as usize == slot
            && endpoint_handle(capability.owner_pid, capability.generation, slot) == handle
            && handles.allows(handle, kind, HANDLE_RIGHT_USE))
        .then_some(capability)
    }

    fn send_capability(
        &self,
        handles: &HandleTable<HANDLE_TABLE_CAPACITY>,
        handle: u64,
    ) -> Option<(u8, u64)> {
        match self.capability(handles, handle)?.role {
            EndpointRole::Send {
                target_pid,
                target_generation,
            } => Some((target_pid, target_generation)),
            EndpointRole::Receive { .. } => None,
        }
    }

    /// A receive capability is only usable while it still names the exact
    /// endpoint generation this process publishes right now.
    fn receive_generation(
        &self,
        handles: &HandleTable<HANDLE_TABLE_CAPACITY>,
        handle: u64,
    ) -> Option<u64> {
        let EndpointRole::Receive { generation } = self.capability(handles, handle)?.role else {
            return None;
        };
        (self.published_generation() == Some(generation)).then_some(generation)
    }

    /// Publishes an empty endpoint and returns its owned receive handle. Fails
    /// when an endpoint is already published or the handle table is full.
    fn create(&mut self, handles: &mut HandleTable<HANDLE_TABLE_CAPACITY>) -> Option<u64> {
        if self.published.is_some() || !self.handles.iter().any(Option::is_none) {
            return None;
        }
        let slot = self.handles.iter().position(Option::is_none)?;
        let generation = self.next_generation()?;
        let handle = endpoint_handle(self.owner_pid, generation, slot);
        if !handles.register(handle, HandleKind::EndpointReceive, HANDLE_RIGHT_USE) {
            return None;
        }
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
    fn close(
        &mut self,
        handles: &mut HandleTable<HANDLE_TABLE_CAPACITY>,
        handle: u64,
    ) -> Option<EndpointRole> {
        let capability = self.capability(handles, handle)?;
        if let EndpointRole::Receive { generation } = capability.role {
            if self.published_generation() != Some(generation) {
                return None;
            }
            self.published = None;
            self.pending_receive = None;
        }
        let kind = match capability.role {
            EndpointRole::Receive { .. } => HandleKind::EndpointReceive,
            EndpointRole::Send { .. } => HandleKind::EndpointSend,
        };
        if !handles.unregister(handle, kind) {
            return None;
        }
        self.handles[capability.slot as usize] = None;
        Some(capability.role)
    }

    /// Drops every send capability naming one remote endpoint generation.
    fn revoke_send_handles(
        &mut self,
        handles: &mut HandleTable<HANDLE_TABLE_CAPACITY>,
        target_pid: u8,
        target_generation: u64,
    ) -> usize {
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
                let handle = entry.expect("matched endpoint capability").handle;
                let removed = handles.unregister(handle, HandleKind::EndpointSend);
                debug_assert!(removed);
                *entry = None;
                revoked += 1;
            }
        }
        revoked
    }

    fn clear(&mut self, handles: &mut HandleTable<HANDLE_TABLE_CAPACITY>) {
        for capability in self.handles.iter().flatten() {
            let kind = match capability.role {
                EndpointRole::Receive { .. } => HandleKind::EndpointReceive,
                EndpointRole::Send { .. } => HandleKind::EndpointSend,
            };
            let removed = handles.unregister(capability.handle, kind);
            debug_assert!(removed);
        }
        self.clear_payload();
    }

    fn clear_payload(&mut self) {
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
    Child(ProcessKey),
    FileOpen,
    FileRead,
    FileWrite,
    FileTruncate,
    ProcessLaunch,
    DirectoryRead,
    DirectoryCreate,
    PathRemove,
    Input,
}

impl ManagedProcess {
    fn new(
        key: ProcessKey,
        task_id: u32,
        parent_key: Option<ProcessKey>,
        process: UserProcess,
    ) -> Self {
        let endpoints = EndpointState::new(process.pid);
        let console_process = process.console_handle != 0;
        let supervisor = process.lifecycle_handle != 0;
        let mut handles = HandleTable::new();
        if process.console_handle != 0 {
            assert!(handles.register(
                process.console_handle,
                HandleKind::Console,
                HANDLE_RIGHT_USE,
            ));
        }
        if process.lifecycle_handle != 0 {
            assert!(handles.register(
                process.lifecycle_handle,
                HandleKind::Lifecycle,
                HANDLE_RIGHT_USE,
            ));
        }
        Self {
            key,
            task_id,
            parent_key,
            console_process,
            supervisor,
            state: ManagedState::Ready,
            wake_at: 0,
            blocked_on: BlockReason::None,
            request_ids: RequestSequence::new(),
            handles,
            file_handles: [None; FILE_HANDLE_CAPACITY],
            next_file_generation: 1,
            endpoints,
            sockets: SocketSet::new(),
            pending_file_open: None,
            pending_file_read: None,
            pending_file_write: None,
            pending_file_truncate: None,
            pending_directory_read: None,
            pending_namespace_mutation: None,
            pending_process_launch: None,
            process_handles: [None; PROCESS_HANDLE_CAPACITY],
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
        if !self.handles.register(handle, HandleKind::File, rights) {
            return None;
        }
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

    fn allocate_request_id(&mut self) -> Option<u64> {
        self.request_ids.allocate()
    }

    fn file_handle(&self, handle: u64, required_rights: u64) -> Option<&FileCapability> {
        if !self
            .handles
            .allows(handle, HandleKind::File, required_rights)
        {
            return None;
        }
        self.file_handles
            .iter()
            .flatten()
            .find(|capability| capability.handle == handle)
    }

    fn file_handle_mut(
        &mut self,
        handle: u64,
        required_rights: u64,
    ) -> Option<&mut FileCapability> {
        if !self
            .handles
            .allows(handle, HandleKind::File, required_rights)
        {
            return None;
        }
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
        if !self.handles.unregister(handle, HandleKind::File) {
            return false;
        }
        self.file_handles[slot] = None;
        true
    }

    fn allocate_process_handle(&mut self, target: ProcessKey) -> Option<u64> {
        let slot = self.process_handles.iter().position(Option::is_none)?;
        let generation = NEXT_PROCESS_HANDLE_GENERATION
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                (current < PROCESS_HANDLE_GENERATION_MAX).then_some(current + 1)
            })
            .ok()?;
        if generation == 0 || generation > PROCESS_HANDLE_GENERATION_MAX {
            return None;
        }
        let handle = PROCESS_HANDLE_TAG | (generation << 8) | (slot as u64 + 1);
        if !self
            .handles
            .register(handle, HandleKind::Process, HANDLE_RIGHT_USE)
        {
            return None;
        }
        self.process_handles[slot] = Some(ProcessCapability { handle, target });
        Some(handle)
    }

    fn process_capability(&self, handle: u64) -> Option<ProcessCapability> {
        if handle & (0xff << 56) != PROCESS_HANDLE_TAG
            || !self
                .handles
                .allows(handle, HandleKind::Process, HANDLE_RIGHT_USE)
        {
            return None;
        }
        self.process_handles
            .iter()
            .flatten()
            .find(|capability| capability.handle == handle)
            .copied()
    }

    fn close_process_handle(&mut self, handle: u64) -> bool {
        let Some(slot) = self
            .process_handles
            .iter()
            .position(|entry| entry.is_some_and(|capability| capability.handle == handle))
        else {
            return false;
        };
        if !self.handles.unregister(handle, HandleKind::Process) {
            return false;
        }
        self.process_handles[slot] = None;
        true
    }

    fn revoke_resources(&mut self) {
        self.sockets.close_owner(SocketOwner {
            slot: self.key.slot,
            incarnation: self.key.incarnation,
        });
        self.handles.clear();
        self.file_handles = [None; FILE_HANDLE_CAPACITY];
        self.endpoints.clear_payload();
        self.pending_file_open = None;
        self.pending_file_read = None;
        self.pending_file_write = None;
        self.pending_file_truncate = None;
        self.pending_directory_read = None;
        self.pending_namespace_mutation = None;
        self.pending_process_launch = None;
        self.process_handles = [None; PROCESS_HANDLE_CAPACITY];
        self.pending_input = None;
        self.process.console_handle = 0;
        self.process.lifecycle_handle = 0;
    }

    fn handle_table_is_consistent(&self) -> bool {
        let expected = self.file_handles.iter().flatten().count()
            + self.endpoints.handles.iter().flatten().count()
            + self.process_handles.iter().flatten().count()
            + self.sockets.len_owner(SocketOwner {
                slot: self.key.slot,
                incarnation: self.key.incarnation,
            })
            + usize::from(self.process.console_handle != 0)
            + usize::from(self.process.lifecycle_handle != 0);
        if self.handles.len() != expected {
            return false;
        }
        if self.process.console_handle != 0
            && (!self.handles.allows(
                self.process.console_handle,
                HandleKind::Console,
                HANDLE_RIGHT_USE,
            ) || self.handles.allows(
                self.process.console_handle,
                HandleKind::Lifecycle,
                HANDLE_RIGHT_USE,
            ))
        {
            return false;
        }
        if self.process.lifecycle_handle != 0
            && (!self.handles.allows(
                self.process.lifecycle_handle,
                HandleKind::Lifecycle,
                HANDLE_RIGHT_USE,
            ) || self.handles.allows(
                self.process.lifecycle_handle,
                HandleKind::Console,
                HANDLE_RIGHT_USE,
            ))
        {
            return false;
        }
        if self.file_handles.iter().flatten().any(|capability| {
            !self
                .handles
                .allows(capability.handle, HandleKind::File, capability.rights)
                || self.handles.allows(
                    capability.handle,
                    HandleKind::EndpointSend,
                    HANDLE_RIGHT_USE,
                )
        }) {
            return false;
        }
        if self.endpoints.handles.iter().flatten().any(|capability| {
            let kind = match capability.role {
                EndpointRole::Receive { .. } => HandleKind::EndpointReceive,
                EndpointRole::Send { .. } => HandleKind::EndpointSend,
            };
            !self
                .handles
                .allows(capability.handle, kind, HANDLE_RIGHT_USE)
                || self.handles.allows(capability.handle, HandleKind::File, 0)
        }) {
            return false;
        }
        let owner = socket_owner(self);
        if self.sockets.handles(owner).any(|handle| {
            !self
                .handles
                .allows(handle, HandleKind::Socket, HANDLE_RIGHT_USE)
                || self.handles.allows(handle, HandleKind::File, 0)
        }) {
            return false;
        }
        !self.process_handles.iter().flatten().any(|capability| {
            !self
                .handles
                .allows(capability.handle, HandleKind::Process, HANDLE_RIGHT_USE)
                || self
                    .handles
                    .allows(capability.handle, HandleKind::Lifecycle, HANDLE_RIGHT_USE)
        })
    }

    fn resources_are_revoked(&self) -> bool {
        self.handles.is_empty()
            && self.file_handles.iter().all(Option::is_none)
            && self.endpoints.handles.iter().all(Option::is_none)
            && self.endpoints.published.is_none()
            && self.endpoints.pending_receive.is_none()
            && self.sockets.len_owner(SocketOwner {
                slot: self.key.slot,
                incarnation: self.key.incarnation,
            }) == 0
            && self.process_handles.iter().all(Option::is_none)
            && self.pending_file_open.is_none()
            && self.pending_file_read.is_none()
            && self.pending_file_write.is_none()
            && self.pending_file_truncate.is_none()
            && self.pending_directory_read.is_none()
            && self.pending_namespace_mutation.is_none()
            && self.pending_process_launch.is_none()
            && self.pending_input.is_none()
            && self.process.console_handle == 0
            && self.process.lifecycle_handle == 0
    }
}

pub struct ProcessManager {
    slots: [Option<ManagedProcess>; MAX_ASYNC_PROCESSES],
    slot_incarnations: [u64; MAX_ASYNC_PROCESSES],
    cursor: usize,
}

impl ProcessManager {
    pub const fn new() -> Self {
        Self {
            slots: [const { None }; MAX_ASYNC_PROCESSES],
            slot_incarnations: [0; MAX_ASYNC_PROCESSES],
            cursor: 0,
        }
    }

    pub fn unified_handle_table_is_authoritative(&self) -> bool {
        self.slots
            .iter()
            .flatten()
            .all(ManagedProcess::handle_table_is_consistent)
    }

    pub fn tcp_listener(&self, port: u16) -> Option<UserTcpListener> {
        self.slots.iter().flatten().find_map(|managed| {
            let handle = managed.sockets.listener_handle(port)?;
            managed
                .handles
                .allows(handle, HandleKind::Socket, HANDLE_RIGHT_USE)
                .then_some(UserTcpListener {
                    owner_slot: managed.key.slot,
                    owner_instance: managed.key.incarnation,
                    owner_pid: managed.process.pid,
                    handle,
                    port,
                })
        })
    }

    pub fn tcp_listener_active(&self, listener: UserTcpListener) -> bool {
        self.slots
            .get(listener.owner_slot as usize)
            .and_then(Option::as_ref)
            .is_some_and(|managed| {
                managed.key.incarnation == listener.owner_instance
                    && managed.process.pid == listener.owner_pid
                    && managed
                        .handles
                        .allows(listener.handle, HandleKind::Socket, HANDLE_RIGHT_USE)
                    && managed.sockets.listener_handle(listener.port) == Some(listener.handle)
            })
    }

    pub fn queue_tcp_peer(
        &mut self,
        listener: UserTcpListener,
        peer: TcpServerPeer,
    ) -> Result<(), SocketError> {
        let managed = self
            .slots
            .get_mut(listener.owner_slot as usize)
            .and_then(Option::as_mut)
            .filter(|managed| {
                managed.key.incarnation == listener.owner_instance
                    && managed.process.pid == listener.owner_pid
                    && managed
                        .handles
                        .allows(listener.handle, HandleKind::Socket, HANDLE_RIGHT_USE)
                    && managed.sockets.listener_handle(listener.port) == Some(listener.handle)
            })
            .ok_or(SocketError::InvalidHandle)?;
        managed
            .sockets
            .queue_incoming(socket_owner(managed), listener.handle, peer)
    }

    pub fn tcp_stream(&self, peer: TcpServerPeer) -> Option<UserTcpStream> {
        self.slots.iter().flatten().find_map(|managed| {
            let handle = managed.sockets.server_handle(peer)?;
            managed
                .handles
                .allows(handle, HandleKind::Socket, HANDLE_RIGHT_USE)
                .then_some(UserTcpStream {
                    owner_slot: managed.key.slot,
                    owner_instance: managed.key.incarnation,
                    owner_task_id: managed.task_id,
                    owner_pid: managed.process.pid,
                    handle,
                    peer,
                })
        })
    }

    pub fn drop_tcp_peer(
        &mut self,
        listener: UserTcpListener,
        peer: TcpServerPeer,
    ) -> Result<(), SocketError> {
        let managed = self
            .slots
            .get_mut(listener.owner_slot as usize)
            .and_then(Option::as_mut)
            .filter(|managed| {
                managed.key.incarnation == listener.owner_instance
                    && managed.process.pid == listener.owner_pid
                    && managed
                        .handles
                        .allows(listener.handle, HandleKind::Socket, HANDLE_RIGHT_USE)
                    && managed.sockets.listener_handle(listener.port) == Some(listener.handle)
            })
            .ok_or(SocketError::InvalidHandle)?;
        managed
            .sockets
            .drop_incoming(socket_owner(managed), listener.handle, peer)
    }

    pub fn tcp_stream_active(&self, stream: UserTcpStream) -> bool {
        self.slots
            .get(stream.owner_slot as usize)
            .and_then(Option::as_ref)
            .is_some_and(|managed| {
                managed.key.incarnation == stream.owner_instance
                    && managed.task_id == stream.owner_task_id
                    && managed.process.pid == stream.owner_pid
                    && !managed.process.completed
                    && managed
                        .handles
                        .allows(stream.handle, HandleKind::Socket, HANDLE_RIGHT_USE)
                    && managed
                        .sockets
                        .server_peer(socket_owner(managed), stream.handle)
                        == Ok(stream.peer)
            })
    }

    pub fn queue_tcp_stream_receive(
        &mut self,
        stream: UserTcpStream,
        bytes: &[u8],
    ) -> Result<usize, SocketError> {
        let managed = self
            .slots
            .get_mut(stream.owner_slot as usize)
            .and_then(Option::as_mut)
            .filter(|managed| {
                managed.key.incarnation == stream.owner_instance
                    && managed.task_id == stream.owner_task_id
                    && managed.process.pid == stream.owner_pid
                    && !managed.process.completed
                    && managed
                        .handles
                        .allows(stream.handle, HandleKind::Socket, HANDLE_RIGHT_USE)
                    && managed
                        .sockets
                        .server_peer(socket_owner(managed), stream.handle)
                        == Ok(stream.peer)
            })
            .ok_or(SocketError::InvalidHandle)?;
        managed
            .sockets
            .push_receive(socket_owner(managed), stream.handle, bytes)
    }

    pub fn begin_tcp_stream_send(&mut self, stream: UserTcpStream) -> Option<UserTcpStreamSend> {
        let managed = self
            .slots
            .get_mut(stream.owner_slot as usize)
            .and_then(Option::as_mut)?;
        let owner = socket_owner(managed);
        if managed.key.incarnation != stream.owner_instance
            || managed.task_id != stream.owner_task_id
            || managed.process.pid != stream.owner_pid
            || managed.process.completed
            || !managed
                .handles
                .allows(stream.handle, HandleKind::Socket, HANDLE_RIGHT_USE)
            || !managed
                .sockets
                .server_send_pending(owner, stream.handle, stream.peer)
        {
            return None;
        }
        let request_id = managed.allocate_request_id()?;
        let mut data = FileWriteBuffer::empty();
        let len = managed
            .sockets
            .begin_server_send(owner, stream.handle, request_id, &mut data.bytes)
            .ok()?;
        data.len = len;
        Some(UserTcpStreamSend {
            stream,
            request_id,
            data,
        })
    }

    pub fn tcp_stream_send_active(&self, request: UserTcpStreamSend) -> bool {
        self.slots
            .get(request.stream.owner_slot as usize)
            .and_then(Option::as_ref)
            .is_some_and(|managed| {
                managed.key.incarnation == request.stream.owner_instance
                    && managed.task_id == request.stream.owner_task_id
                    && managed.process.pid == request.stream.owner_pid
                    && !managed.process.completed
                    && managed.handles.allows(
                        request.stream.handle,
                        HandleKind::Socket,
                        HANDLE_RIGHT_USE,
                    )
                    && managed.sockets.server_send_active(
                        socket_owner(managed),
                        request.stream.handle,
                        request.stream.peer,
                        request.request_id,
                        request.data.len(),
                    )
            })
    }

    pub fn complete_tcp_stream_send(
        &mut self,
        request: UserTcpStreamSend,
    ) -> Result<usize, SocketError> {
        if !self.tcp_stream_send_active(request) {
            return Err(SocketError::InvalidHandle);
        }
        let managed = self.slots[request.stream.owner_slot as usize]
            .as_mut()
            .ok_or(SocketError::InvalidHandle)?;
        managed.sockets.complete_server_send(
            socket_owner(managed),
            request.stream.handle,
            request.stream.peer,
            request.request_id,
        )
    }

    pub fn tcp_stream_write_closed(&self, stream: UserTcpStream) -> bool {
        self.slots
            .get(stream.owner_slot as usize)
            .and_then(Option::as_ref)
            .is_some_and(|managed| {
                managed.key.incarnation == stream.owner_instance
                    && managed.task_id == stream.owner_task_id
                    && managed.process.pid == stream.owner_pid
                    && managed.sockets.server_write_closed(
                        socket_owner(managed),
                        stream.handle,
                        stream.peer,
                    )
            })
    }

    pub fn mark_tcp_stream_read_closed(
        &mut self,
        stream: UserTcpStream,
    ) -> Result<(), SocketError> {
        let managed = self.slots[stream.owner_slot as usize]
            .as_mut()
            .ok_or(SocketError::InvalidHandle)?;
        managed
            .sockets
            .mark_server_read_closed(socket_owner(managed), stream.handle, stream.peer)
    }

    pub fn mark_tcp_stream_closed(&mut self, stream: UserTcpStream) -> Result<(), SocketError> {
        let managed = self.slots[stream.owner_slot as usize]
            .as_mut()
            .ok_or(SocketError::InvalidHandle)?;
        managed
            .sockets
            .mark_server_closed(socket_owner(managed), stream.handle, stream.peer)
    }

    pub fn fail_tcp_stream(&mut self, stream: UserTcpStream) -> Result<(), SocketError> {
        let managed = self.slots[stream.owner_slot as usize]
            .as_mut()
            .ok_or(SocketError::InvalidHandle)?;
        if managed
            .sockets
            .server_peer(socket_owner(managed), stream.handle)
            != Ok(stream.peer)
        {
            return Err(SocketError::InvalidHandle);
        }
        managed.sockets.fail(socket_owner(managed), stream.handle)
    }

    fn allocate_key(&mut self, slot: usize) -> Result<ProcessKey, LaunchError> {
        let incarnation = self.slot_incarnations[slot]
            .checked_add(1)
            .ok_or(LaunchError::ProcessTableFull)?;
        self.slot_incarnations[slot] = incarnation;
        Ok(ProcessKey {
            slot: slot as u8,
            incarnation,
        })
    }

    fn allocate_pid(&self) -> Result<u8, LaunchError> {
        for _ in 0..u8::MAX {
            let candidate = NEXT_DYNAMIC_PID.fetch_add(1, Ordering::AcqRel);
            if candidate != 0
                && !self
                    .slots
                    .iter()
                    .flatten()
                    .any(|managed| managed.process.pid == candidate)
            {
                return Ok(candidate);
            }
        }
        Err(LaunchError::ProcessTableFull)
    }

    pub fn spawn_shell(&mut self, task_id: u32) -> Result<u8, LaunchError> {
        let slot = self
            .slots
            .iter()
            .position(Option::is_none)
            .ok_or(LaunchError::ProcessTableFull)?;
        let elf_bytes = shell_elf()?;
        let pid = self.allocate_pid()?;
        let key = self.allocate_key(slot)?;
        let generation = NEXT_CONSOLE_GENERATION.fetch_add(1, Ordering::AcqRel);
        let handle = syscall::console_handle(pid, generation);
        let lifecycle_generation = NEXT_LIFECYCLE_GENERATION
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                (current < 0x00ff_ffff_ffff_ffff).then_some(current + 1)
            })
            .map_err(|_| LaunchError::ProcessTableFull)?;
        let lifecycle_handle = syscall::lifecycle_handle(lifecycle_generation);
        let mut process =
            build_process(pid, handle, elf_bytes).map_err(|_| LaunchError::ProcessBuildFailed)?;
        process.console_handle = handle;
        process.lifecycle_handle = lifecycle_handle;
        process.context.rsi = lifecycle_handle;
        self.slots[slot] = Some(ManagedProcess::new(key, task_id, None, process));
        DYNAMIC_PROCESSES.fetch_add(1, Ordering::AcqRel);
        crate::serial::print("USER_SHELL_SPAWN pid=");
        crate::serial::print_u64(pid as u64);
        crate::serial::print(" task=");
        crate::serial::print_u64(task_id as u64);
        crate::serial::print(" supervisor=0x");
        crate::serial::print_hex(lifecycle_handle);
        crate::serial::println("");
        Ok(pid)
    }

    pub fn spawn_init(&mut self, task_id: u32, hold: bool) -> Result<u8, LaunchError> {
        let token_mode = if hold { TOKEN_HOLD_BIT } else { 0 };
        self.spawn_single(task_id, token_mode, if hold { "hold" } else { "normal" })
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
        let pid = self.allocate_pid()?;
        let key = self.allocate_key(slot)?;
        let token = if token_mode == 0 || token_mode == TOKEN_HOLD_BIT {
            TOKEN_DYNAMIC_BASE | token_mode | pid as u64
        } else {
            token_mode | pid as u64
        };
        let process =
            build_process(pid, token, elf_bytes).map_err(|_| LaunchError::ProcessBuildFailed)?;
        self.slots[slot] = Some(ManagedProcess::new(key, task_id, None, process));
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
        let parent_pid = self.allocate_pid()?;
        let parent_key = self.allocate_key(parent_slot)?;
        let child_pid = self.allocate_pid()?;
        let child_key = self.allocate_key(child_slot)?;
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
        self.slots[parent_slot] = Some(ManagedProcess::new(
            parent_key,
            parent_task_id,
            None,
            parent,
        ));
        self.slots[child_slot] = Some(ManagedProcess::new(
            child_key,
            child_task_id,
            Some(parent_key),
            child,
        ));
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
        let receiver_pid = self.allocate_pid()?;
        let receiver_key = self.allocate_key(receiver_slot)?;
        let a_pid = self.allocate_pid()?;
        let a_key = self.allocate_key(a_slot)?;
        let b_pid = self.allocate_pid()?;
        let b_key = self.allocate_key(b_slot)?;
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
        self.slots[receiver_slot] = Some(ManagedProcess::new(
            receiver_key,
            receiver_task_id,
            None,
            receiver,
        ));
        self.slots[a_slot] = Some(ManagedProcess::new(
            a_key,
            a_task_id,
            Some(receiver_key),
            producer_a,
        ));
        self.slots[b_slot] = Some(ManagedProcess::new(
            b_key,
            b_task_id,
            Some(receiver_key),
            producer_b,
        ));
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
            let mut lifecycle_request = None;
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
                    vfs_request = self
                        .block_file_read(index, path, address, capacity)
                        .map(UserVfsRequest::Read);
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
                ProcessEvent::TruncateHandle { handle } => {
                    vfs_request = self
                        .block_file_handle_truncate(index, handle)
                        .map(UserVfsRequest::Truncate);
                }
                ProcessEvent::CreateDirectory { parent, name } => {
                    vfs_request = self
                        .block_namespace_mutation(index, parent, name, BlockReason::DirectoryCreate)
                        .map(UserVfsRequest::CreateDirectory);
                }
                ProcessEvent::RemovePath { parent, name } => {
                    vfs_request = self
                        .block_namespace_mutation(index, parent, name, BlockReason::PathRemove)
                        .map(UserVfsRequest::RemovePath);
                }
                ProcessEvent::ProcessLaunch {
                    supervisor,
                    image,
                    mode,
                } => {
                    lifecycle_request = self
                        .block_process_launch(index, supervisor, image, mode)
                        .map(UserLifecycleRequest::Launch);
                }
                ProcessEvent::ProcessStatus {
                    handle,
                    address,
                    length,
                } => self.complete_process_status(index, handle, address, length),
                ProcessEvent::ProcessKill { handle } => {
                    self.complete_controlled_kill(index, handle)
                }
                ProcessEvent::ProcessReap {
                    handle,
                    address,
                    length,
                } => self.complete_controlled_reap(index, handle, address, length),
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
                ProcessEvent::ConsoleWrite { handle, text, kind } => {
                    let managed = self.slots[index].as_mut().expect("selected process exists");
                    if !managed
                        .handles
                        .allows(handle, HandleKind::Console, HANDLE_RIGHT_USE)
                    {
                        managed.process.context.rax =
                            syscall::error_code(syscall::SyscallError::InvalidArgument);
                        managed.state = ManagedState::Ready;
                        continue;
                    }
                    managed.process.context.rax = text.len() as u64;
                    managed.state = ManagedState::Ready;
                    console = Some(ConsoleUpdate::Write { kind, text });
                    crate::serial::print("USER_CONSOLE_WRITE pid=");
                    crate::serial::print_u64(managed.process.pid as u64);
                    crate::serial::print(" text=");
                    crate::serial::print(text.as_str());
                    crate::serial::println("");
                    if text.as_str() == "storage status visible" {
                        crate::serial::println("USER_STORAGE_STATUS_VISIBLE_OK");
                    }
                    if text.as_str() == "storage failure visible" {
                        crate::serial::println("USER_STORAGE_FAILURE_VISIBLE_OK");
                        crate::serial::println("STORAGE_FAILURE_SURFACE_READY");
                    }
                    if text.as_str() == "storage read-only visible" {
                        crate::serial::println("USER_STORAGE_READ_ONLY_OK");
                    }
                    if text.as_str() == "RAMFS temp visible" {
                        crate::serial::println("USER_RAMFS_TEMP_APP_OK");
                    }
                    if text.as_str() == "network DNS resolved" {
                        crate::serial::println("USER_DNS_RESOLVE_OK");
                    }
                    if text.as_str() == "network HTTP complete" {
                        crate::serial::println("USER_HTTP_REQUEST_OK");
                        crate::serial::println("USER_SOCKET_API_READY");
                    }
                    if text.as_str() == "network timeout handled" {
                        crate::serial::println("USER_NETWORK_TIMEOUT_OK");
                    }
                    if text.as_str() == "network diagnostics ready" {
                        crate::serial::println("USER_NETWORK_DIAGNOSTICS_READY");
                    }
                    if text.as_str() == "nonblocking socket capabilities ready" {
                        crate::serial::println("USER_SOCKET_CAPABILITY_READY abi=17");
                    }
                    if text.as_str() == "asynchronous UDP socket ready" {
                        crate::serial::println("USER_SOCKET_UDP_ASYNC_READY");
                    }
                    if text.as_str() == "asynchronous TCP socket ready" {
                        crate::serial::println("USER_SOCKET_TCP_ASYNC_READY");
                    }
                    if text.as_str() == "listener capability authority ready" {
                        crate::serial::println("USER_SOCKET_LISTENER_CAPABILITY_READY abi=17");
                    }
                    if text.as_str() == "passive TCP accept ready" {
                        crate::serial::println("USER_SOCKET_PASSIVE_ACCEPT_READY");
                    }
                    if text.as_str() == "passive TCP stream ready" {
                        crate::serial::println("USER_SOCKET_PASSIVE_STREAM_READY");
                    }
                    if text.as_str() == "durable file committed" {
                        crate::serial::println("USER_DURABLE_WRITE_OK path=/USER/SHELL.TXT");
                    }
                    if text.as_str() == "durable file restored" {
                        crate::serial::println("USER_DURABLE_RESTORE_OK path=/USER/SHELL.TXT");
                    }
                    if text.as_str() == "durable file restored read-only" {
                        crate::serial::println("USER_DURABLE_RESTORE_OK path=/USER/SHELL.TXT");
                        crate::serial::println("USER_READ_ONLY_MUTATION_DENIED_OK");
                    }
                    if text.as_str() == "session file written" {
                        crate::serial::println("USER_SESSION_WRITE_OK path=/USER/SHELL.TXT");
                    }
                    if text.as_str().starts_with("SHELL.ELF ready") {
                        if text.as_str().contains("process control") {
                            crate::serial::println("USER_SHELL_PROCESS_CONTROL_OK");
                        }
                        if text.as_str().contains("filesystem") {
                            crate::serial::println("USER_SHELL_NAMESPACE_OK");
                            crate::serial::println("USER_SHELL_HISTORY_OK");
                        }
                        crate::serial::println("USER_SHELL_READY");
                    }
                }
                ProcessEvent::ConsoleSetInput { handle, text } => {
                    let managed = self.slots[index].as_mut().expect("selected process exists");
                    if !managed
                        .handles
                        .allows(handle, HandleKind::Console, HANDLE_RIGHT_USE)
                    {
                        managed.process.context.rax =
                            syscall::error_code(syscall::SyscallError::InvalidArgument);
                        managed.state = ManagedState::Ready;
                        continue;
                    }
                    managed.process.context.rax = text.len() as u64;
                    managed.state = ManagedState::Ready;
                    console = Some(ConsoleUpdate::SetInput(text));
                }
                ProcessEvent::ConsoleClear(handle) => {
                    let managed = self.slots[index].as_mut().expect("selected process exists");
                    if !managed
                        .handles
                        .allows(handle, HandleKind::Console, HANDLE_RIGHT_USE)
                    {
                        managed.process.context.rax =
                            syscall::error_code(syscall::SyscallError::InvalidArgument);
                        managed.state = ManagedState::Ready;
                        continue;
                    }
                    managed.process.context.rax = 0;
                    managed.state = ManagedState::Ready;
                    console = Some(ConsoleUpdate::Clear);
                    crate::serial::print("USER_CONSOLE_CLEAR pid=");
                    crate::serial::print_u64(managed.process.pid as u64);
                    crate::serial::println("");
                }
                ProcessEvent::SocketOpen { protocol } => self.complete_socket_open(index, protocol),
                ProcessEvent::SocketConnect {
                    handle,
                    target,
                    port,
                } => self.complete_socket_connect(index, handle, target, port),
                ProcessEvent::SocketBind { handle, port } => {
                    self.complete_socket_bind(index, handle, port)
                }
                ProcessEvent::SocketListen { handle, backlog } => {
                    self.complete_socket_listen(index, handle, backlog)
                }
                ProcessEvent::SocketAccept { handle } => self.complete_socket_accept(index, handle),
                ProcessEvent::SocketSend { handle, data } => {
                    self.complete_socket_send(index, handle, data)
                }
                ProcessEvent::SocketReceive {
                    handle,
                    address,
                    capacity,
                } => self.complete_socket_receive(index, handle, address, capacity),
                ProcessEvent::SocketStatus {
                    handle,
                    address,
                    length,
                } => self.complete_socket_status(index, handle, address, length),
                ProcessEvent::SocketShutdown { handle, direction } => {
                    self.complete_socket_shutdown(index, handle, direction)
                }
                ProcessEvent::SocketClose(handle) => self.complete_socket_close(index, handle),
                ProcessEvent::Exit => self.complete_terminal(index, ManagedState::Exited),
                ProcessEvent::Fault => self.complete_terminal(index, ManagedState::Faulted),
                ProcessEvent::None => return None,
            }
            let socket_request = self.begin_socket_request(index);
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
                console_process: managed.handles.allows(
                    managed.process.console_handle,
                    HandleKind::Console,
                    HANDLE_RIGHT_USE,
                ),
                console,
                vfs_request,
                lifecycle_request,
                socket_request,
            });
        }
        None
    }

    fn begin_socket_request(&mut self, index: usize) -> Option<UserSocketRequest> {
        let managed = self.slots[index].as_mut()?;
        if managed.process.completed {
            return None;
        }
        let owner = socket_owner(managed);
        let (handle, protocol) = managed.sockets.pending_transport(owner)?;
        let request_id = managed.allocate_request_id()?;
        let mut data = FileWriteBuffer::empty();
        let (target, port, len) = managed
            .sockets
            .begin_transport(owner, handle, protocol, request_id, &mut data.bytes)
            .ok()?;
        data.len = len;
        crate::serial::print(match protocol {
            SocketProtocol::Udp => "USER_SOCKET_UDP_QUEUED pid=",
            SocketProtocol::TcpStream => "USER_SOCKET_TCP_QUEUED pid=",
        });
        crate::serial::print_u64(managed.process.pid as u64);
        crate::serial::print(" request=");
        crate::serial::print_u64(request_id);
        crate::serial::println("");
        Some(UserSocketRequest {
            request_id,
            owner_slot: managed.key.slot,
            owner_instance: managed.key.incarnation,
            owner_task_id: managed.task_id,
            owner_pid: managed.process.pid,
            handle,
            protocol,
            target,
            port,
            data,
        })
    }

    fn complete_socket_open(&mut self, index: usize, protocol: u64) {
        let managed = self.slots[index].as_mut().expect("selected process exists");
        let owner = socket_owner(managed);
        let result = SocketProtocol::from_raw(protocol)
            .ok_or(SocketError::InvalidState)
            .and_then(|protocol| managed.sockets.open(owner, protocol));
        managed.process.context.rax = match result {
            Ok(handle)
                if managed
                    .handles
                    .register(handle, HandleKind::Socket, HANDLE_RIGHT_USE) =>
            {
                crate::serial::print("USER_SOCKET_OPEN pid=");
                crate::serial::print_u64(managed.process.pid as u64);
                crate::serial::print(" handle=0x");
                crate::serial::print_hex(handle);
                crate::serial::println("");
                handle
            }
            Ok(handle) => {
                let _ = managed.sockets.close(owner, handle);
                syscall::error_code(syscall::SyscallError::Unavailable)
            }
            Err(error) => socket_error_code(error),
        };
        managed.state = ManagedState::Ready;
    }

    fn complete_socket_connect(&mut self, index: usize, handle: u64, target: u32, port: u16) {
        let managed = self.slots[index].as_mut().expect("selected process exists");
        let allowed = managed
            .handles
            .allows(handle, HandleKind::Socket, HANDLE_RIGHT_USE);
        let result = allowed
            .then(|| {
                managed
                    .sockets
                    .connect(socket_owner(managed), handle, target, port)
            })
            .unwrap_or(Err(SocketError::InvalidHandle));
        managed.process.context.rax = result.map_or_else(socket_error_code, |_| 0);
        managed.state = ManagedState::Ready;
    }

    fn complete_socket_bind(&mut self, index: usize, handle: u64, port: u16) {
        let validation = self.slots[index]
            .as_ref()
            .filter(|managed| {
                managed
                    .handles
                    .allows(handle, HandleKind::Socket, HANDLE_RIGHT_USE)
            })
            .map(|managed| {
                managed
                    .sockets
                    .validate_bind(socket_owner(managed), handle, port)
            })
            .unwrap_or(Err(SocketError::InvalidHandle));
        let port_available = local_port_is_available(
            self.slots.iter().flatten().map(|managed| &managed.sockets),
            port,
        );
        let managed = self.slots[index].as_mut().expect("selected process exists");
        let result = if let Err(error) = validation {
            Err(error)
        } else if !port_available {
            Err(SocketError::Unavailable)
        } else {
            managed.sockets.bind(socket_owner(managed), handle, port)
        };
        managed.process.context.rax = result.map_or_else(socket_error_code, |_| 0);
        managed.state = ManagedState::Ready;
    }

    fn complete_socket_listen(&mut self, index: usize, handle: u64, backlog: u64) {
        let managed = self.slots[index].as_mut().expect("selected process exists");
        let allowed = managed
            .handles
            .allows(handle, HandleKind::Socket, HANDLE_RIGHT_USE);
        let result = allowed
            .then(|| {
                managed
                    .sockets
                    .listen(socket_owner(managed), handle, backlog as usize)
            })
            .unwrap_or(Err(SocketError::InvalidHandle));
        managed.process.context.rax = result.map_or_else(socket_error_code, |_| 0);
        managed.state = ManagedState::Ready;
    }

    fn complete_socket_accept(&mut self, index: usize, handle: u64) {
        let managed = self.slots[index].as_mut().expect("selected process exists");
        let owner = socket_owner(managed);
        let allowed = managed
            .handles
            .allows(handle, HandleKind::Socket, HANDLE_RIGHT_USE);
        let result = allowed
            .then(|| managed.sockets.accept(owner, handle))
            .unwrap_or(Err(SocketError::InvalidHandle));
        managed.process.context.rax = match result {
            Ok(accepted)
                if managed
                    .handles
                    .register(accepted, HandleKind::Socket, HANDLE_RIGHT_USE) =>
            {
                accepted
            }
            Ok(accepted) => {
                let _ = managed.sockets.close(owner, accepted);
                syscall::error_code(syscall::SyscallError::Unavailable)
            }
            Err(error) => socket_error_code(error),
        };
        managed.state = ManagedState::Ready;
    }

    fn complete_socket_send(&mut self, index: usize, handle: u64, data: FileWriteBuffer) {
        let managed = self.slots[index].as_mut().expect("selected process exists");
        let server_stream = managed
            .sockets
            .server_peer(socket_owner(managed), handle)
            .is_ok();
        let allowed = managed
            .handles
            .allows(handle, HandleKind::Socket, HANDLE_RIGHT_USE);
        let result = allowed
            .then(|| {
                managed
                    .sockets
                    .send(socket_owner(managed), handle, data.as_slice())
            })
            .unwrap_or(Err(SocketError::InvalidHandle));
        managed.process.context.rax = result.map_or_else(socket_error_code, |length| {
            if server_stream {
                crate::serial::print("USER_SOCKET_PASSIVE_SEND_QUEUED bytes=");
                crate::serial::print_u64(length as u64);
                crate::serial::println("");
            }
            length as u64
        });
        managed.state = ManagedState::Ready;
    }

    fn complete_socket_receive(&mut self, index: usize, handle: u64, address: u64, capacity: u64) {
        let managed = self.slots[index].as_mut().expect("selected process exists");
        let mut output = [0u8; USER_SOCKET_BUFFER_CAPACITY as usize];
        let server_stream = managed
            .sockets
            .server_peer(socket_owner(managed), handle)
            .is_ok();
        let allowed = managed
            .handles
            .allows(handle, HandleKind::Socket, HANDLE_RIGHT_USE);
        let result = allowed
            .then(|| {
                managed.sockets.receive(
                    socket_owner(managed),
                    handle,
                    &mut output[..capacity as usize],
                )
            })
            .unwrap_or(Err(SocketError::InvalidHandle));
        managed.process.context.rax = match result {
            Ok(length) if copy_to_user_data(&managed.process, address, &output[..length]) => {
                if server_stream {
                    crate::serial::print("USER_SOCKET_PASSIVE_RECEIVE bytes=");
                    crate::serial::print_u64(length as u64);
                    crate::serial::println("");
                }
                length as u64
            }
            Ok(_) => syscall::error_code(syscall::SyscallError::InvalidArgument),
            Err(error) => {
                if server_stream {
                    crate::serial::println("USER_SOCKET_PASSIVE_RECEIVE_BLOCKED");
                }
                socket_error_code(error)
            }
        };
        managed.state = ManagedState::Ready;
    }

    fn complete_socket_status(&mut self, index: usize, handle: u64, address: u64, length: u64) {
        let managed = self.slots[index].as_mut().expect("selected process exists");
        let allowed = managed
            .handles
            .allows(handle, HandleKind::Socket, HANDLE_RIGHT_USE);
        let result = allowed
            .then(|| managed.sockets.status(socket_owner(managed), handle))
            .unwrap_or(Err(SocketError::InvalidHandle));
        managed.process.context.rax = match result {
            Ok(status) => {
                let status = UserSocketStatus {
                    protocol: status.protocol as u64,
                    state: status.state as u64,
                    readiness: status.readiness,
                    queued_send: status.queued_send as u64,
                    queued_receive: status.queued_receive as u64,
                };
                let bytes = unsafe {
                    core::slice::from_raw_parts(
                        core::ptr::from_ref(&status).cast::<u8>(),
                        core::mem::size_of::<UserSocketStatus>(),
                    )
                };
                if length as usize == bytes.len()
                    && copy_to_user_data(&managed.process, address, bytes)
                {
                    length
                } else {
                    syscall::error_code(syscall::SyscallError::InvalidArgument)
                }
            }
            Err(error) => socket_error_code(error),
        };
        managed.state = ManagedState::Ready;
    }

    fn complete_socket_shutdown(&mut self, index: usize, handle: u64, direction: u64) {
        let managed = self.slots[index].as_mut().expect("selected process exists");
        let allowed = managed
            .handles
            .allows(handle, HandleKind::Socket, HANDLE_RIGHT_USE);
        let result = allowed
            .then(|| {
                managed.sockets.shutdown(
                    socket_owner(managed),
                    handle,
                    direction & USER_SOCKET_SHUTDOWN_READ != 0,
                    direction & USER_SOCKET_SHUTDOWN_WRITE != 0,
                )
            })
            .unwrap_or(Err(SocketError::InvalidHandle));
        managed.process.context.rax = result.map_or_else(socket_error_code, |_| 0);
        managed.state = ManagedState::Ready;
    }

    fn complete_socket_close(&mut self, index: usize, handle: u64) {
        let managed = self.slots[index].as_mut().expect("selected process exists");
        let allowed = managed
            .handles
            .allows(handle, HandleKind::Socket, HANDLE_RIGHT_USE);
        let result = allowed
            .then(|| managed.sockets.close(socket_owner(managed), handle))
            .unwrap_or(Err(SocketError::InvalidHandle));
        managed.process.context.rax = match result {
            Ok(()) if managed.handles.unregister(handle, HandleKind::Socket) => 0,
            Ok(()) => syscall::error_code(syscall::SyscallError::InvalidArgument),
            Err(error) => socket_error_code(error),
        };
        managed.state = ManagedState::Ready;
    }

    fn block_process_launch(
        &mut self,
        index: usize,
        supervisor: u64,
        image: u64,
        mode: u64,
    ) -> Option<ProcessLaunchRequest> {
        let managed = self.slots[index].as_mut().expect("selected process exists");
        if !managed
            .handles
            .allows(supervisor, HandleKind::Lifecycle, HANDLE_RIGHT_USE)
            || image != USER_PROCESS_IMAGE_INIT
            || !matches!(mode, USER_PROCESS_MODE_NORMAL | USER_PROCESS_MODE_HOLD)
            || !managed.process_handles.iter().any(Option::is_none)
        {
            managed.process.context.rax =
                syscall::error_code(syscall::SyscallError::InvalidArgument);
            managed.state = ManagedState::Ready;
            crate::serial::print("USER_PROCESS_LAUNCH_DENIED owner=");
            crate::serial::print_u64(managed.process.pid as u64);
            crate::serial::println("");
            return None;
        }
        let Some(request_id) = managed.allocate_request_id() else {
            managed.process.context.rax = syscall::error_code(syscall::SyscallError::Unavailable);
            managed.state = ManagedState::Ready;
            return None;
        };
        managed.state = ManagedState::Waiting;
        managed.blocked_on = BlockReason::ProcessLaunch;
        managed.pending_process_launch = Some(PendingProcessLaunch {
            request_id,
            image,
            mode,
        });
        Some(ProcessLaunchRequest {
            request_id,
            owner_slot: managed.key.slot,
            owner_instance: managed.key.incarnation,
            owner_task_id: managed.task_id,
            owner_pid: managed.process.pid,
            image,
            mode,
        })
    }

    pub fn complete_process_launch(
        &mut self,
        request: ProcessLaunchRequest,
        task_id: Option<u32>,
    ) -> Result<ProcessLaunchCompletion, LaunchError> {
        let owner_index = self
            .slots
            .iter()
            .position(|slot| {
                slot.as_ref().is_some_and(|managed| {
                    managed.key
                        == (ProcessKey {
                            slot: request.owner_slot,
                            incarnation: request.owner_instance,
                        })
                        && managed.task_id == request.owner_task_id
                        && managed.process.pid == request.owner_pid
                })
            })
            .ok_or(LaunchError::ImageUnavailable)?;
        let pending = self.slots[owner_index]
            .as_ref()
            .and_then(|managed| managed.pending_process_launch)
            .ok_or(LaunchError::InvalidResult)?;
        if self.slots[owner_index].as_ref().is_none_or(|managed| {
            managed.state != ManagedState::Waiting
                || managed.blocked_on != BlockReason::ProcessLaunch
                || managed.process.completed
        }) || pending.request_id != request.request_id
            || pending.image != request.image
            || pending.mode != request.mode
        {
            return Err(LaunchError::InvalidResult);
        }

        let child_slot = self
            .slots
            .iter()
            .enumerate()
            .find_map(|(slot, entry)| (slot != owner_index && entry.is_none()).then_some(slot));
        let launch = task_id.zip(child_slot).and_then(|(task_id, child_slot)| {
            let pid = self.allocate_pid().ok()?;
            let key = self.allocate_key(child_slot).ok()?;
            let token = TOKEN_DYNAMIC_BASE
                | if request.mode == USER_PROCESS_MODE_HOLD {
                    TOKEN_HOLD_BIT
                } else {
                    0
                }
                | pid as u64;
            let mut process = build_process(pid, token, user_elf().ok()?).ok()?;
            let handle = {
                let owner = self.slots[owner_index].as_mut()?;
                owner.allocate_process_handle(key)
            };
            let Some(handle) = handle else {
                let _ = reclaim_process(&mut process);
                return None;
            };
            let owner_key = self.slots[owner_index].as_ref()?.key;
            self.slots[child_slot] =
                Some(ManagedProcess::new(key, task_id, Some(owner_key), process));
            DYNAMIC_PROCESSES.fetch_add(1, Ordering::AcqRel);
            Some((pid, handle))
        });

        let owner = self.slots[owner_index]
            .as_mut()
            .ok_or(LaunchError::ImageUnavailable)?;
        owner.pending_process_launch = None;
        owner.blocked_on = BlockReason::None;
        owner.state = ManagedState::Ready;
        owner.process.context.rax = launch
            .map(|(_, handle)| handle)
            .unwrap_or_else(|| syscall::error_code(syscall::SyscallError::Unavailable));
        if let Some((pid, handle)) = launch {
            crate::serial::print("USER_PROCESS_LAUNCHED owner=");
            crate::serial::print_u64(request.owner_pid as u64);
            crate::serial::print(" pid=");
            crate::serial::print_u64(pid as u64);
            crate::serial::print(" handle=0x");
            crate::serial::print_hex(handle);
            crate::serial::print(" mode=");
            crate::serial::println(if request.mode == USER_PROCESS_MODE_HOLD {
                "hold"
            } else {
                "normal"
            });
        }
        Ok(ProcessLaunchCompletion {
            owner: process_update(owner),
        })
    }

    fn process_status_for_key(&self, key: ProcessKey) -> Option<UserProcessStatus> {
        let managed = self.slots.get(key.slot as usize)?.as_ref()?;
        (managed.key == key).then(|| UserProcessStatus {
            instance_id: key.incarnation,
            task_id: managed.task_id as u64,
            runtime_pid: managed.process.pid as u64,
            state: managed_state_abi(managed.state),
            exit_code: if managed.process.completed {
                managed.process.exit_code as u64
            } else {
                0
            },
            fault_vector: managed.process.fault_vector as u64,
            preemptions: managed.process.preemptions,
            reserved: 0,
        })
    }

    fn complete_process_status(&mut self, index: usize, handle: u64, address: u64, length: u64) {
        let capability = self.slots[index]
            .as_ref()
            .and_then(|managed| managed.process_capability(handle));
        let status =
            capability.and_then(|capability| self.process_status_for_key(capability.target));
        let copied = status.is_some_and(|status| {
            let bytes = user_process_status_bytes(&status);
            length as usize == bytes.len()
                && self.slots[index]
                    .as_ref()
                    .is_some_and(|managed| copy_to_user_data(&managed.process, address, bytes))
        });
        let managed = self.slots[index].as_mut().expect("selected process exists");
        managed.process.context.rax = if copied {
            length
        } else {
            syscall::error_code(syscall::SyscallError::InvalidArgument)
        };
        managed.state = ManagedState::Ready;
        if copied {
            crate::serial::print("USER_PROCESS_STATUS owner=");
            crate::serial::print_u64(managed.process.pid as u64);
            crate::serial::print(" target=");
            crate::serial::print_u64(status.map(|status| status.runtime_pid).unwrap_or(0));
            crate::serial::println("");
        }
    }

    fn complete_controlled_kill(&mut self, owner_index: usize, handle: u64) {
        let capability = self.slots[owner_index]
            .as_ref()
            .and_then(|managed| managed.process_capability(handle));
        let target_index = capability.and_then(|capability| {
            self.slots
                .get(capability.target.slot as usize)
                .filter(|slot| {
                    slot.as_ref()
                        .is_some_and(|managed| managed.key == capability.target)
                })
                .map(|_| capability.target.slot as usize)
        });
        let live_target = target_index.filter(|target_index| {
            *target_index != owner_index
                && self.slots[*target_index]
                    .as_ref()
                    .is_some_and(|managed| !managed.process.completed)
        });
        let Some(target_index) = live_target else {
            let managed = self.slots[owner_index]
                .as_mut()
                .expect("selected process exists");
            managed.process.context.rax = if target_index.is_some() {
                syscall::error_code(syscall::SyscallError::Unavailable)
            } else {
                syscall::error_code(syscall::SyscallError::InvalidArgument)
            };
            managed.state = ManagedState::Ready;
            return;
        };
        let target_pid = self
            .terminate_process_at(target_index, ManagedState::Killed, Some(137))
            .unwrap_or_else(|_| fail("USER_RECLAIM_FAILED"))
            .pid;
        let owner = self.slots[owner_index]
            .as_mut()
            .expect("selected process exists");
        owner.process.context.rax = 0;
        owner.state = ManagedState::Ready;
        crate::serial::print("USER_PROCESS_KILLED owner=");
        crate::serial::print_u64(owner.process.pid as u64);
        crate::serial::print(" pid=");
        crate::serial::print_u64(target_pid as u64);
        crate::serial::println(" code=137");
    }

    fn complete_controlled_reap(
        &mut self,
        owner_index: usize,
        handle: u64,
        address: u64,
        length: u64,
    ) {
        let capability = self.slots[owner_index]
            .as_ref()
            .and_then(|managed| managed.process_capability(handle));
        let target_index = capability.and_then(|capability| {
            self.slots
                .get(capability.target.slot as usize)
                .filter(|slot| {
                    slot.as_ref()
                        .is_some_and(|managed| managed.key == capability.target)
                })
                .map(|_| capability.target.slot as usize)
        });
        let status =
            capability.and_then(|capability| self.process_status_for_key(capability.target));
        let terminal = target_index.is_some_and(|target_index| {
            self.slots[target_index]
                .as_ref()
                .is_some_and(|managed| managed.process.completed)
        });
        if !terminal {
            let owner = self.slots[owner_index]
                .as_mut()
                .expect("selected process exists");
            owner.process.context.rax = if target_index.is_some() {
                syscall::error_code(syscall::SyscallError::Unavailable)
            } else {
                syscall::error_code(syscall::SyscallError::InvalidArgument)
            };
            owner.state = ManagedState::Ready;
            return;
        }
        let copied = status.is_some_and(|status| {
            let bytes = user_process_status_bytes(&status);
            length as usize == bytes.len()
                && self.slots[owner_index]
                    .as_ref()
                    .is_some_and(|owner| copy_to_user_data(&owner.process, address, bytes))
        });
        if !copied {
            let owner = self.slots[owner_index]
                .as_mut()
                .expect("selected process exists");
            owner.process.context.rax = syscall::error_code(syscall::SyscallError::InvalidArgument);
            owner.state = ManagedState::Ready;
            return;
        }
        let target_index = target_index.expect("terminal target exists");
        self.release_endpoints(target_index);
        self.slots[target_index] = None;
        let owner = self.slots[owner_index]
            .as_mut()
            .expect("selected process exists");
        if !owner.close_process_handle(handle) {
            fail("USER_PROCESS_HANDLE_RELEASE_FAILED");
        }
        owner.process.context.rax = length;
        owner.state = ManagedState::Ready;
        crate::serial::print("USER_PROCESS_REAPED owner=");
        crate::serial::print_u64(owner.process.pid as u64);
        crate::serial::print(" pid=");
        crate::serial::print_u64(status.map(|status| status.runtime_pid).unwrap_or(0));
        crate::serial::println("");
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
        let Some(capability) = managed
            .file_handle(handle, USER_FILE_RIGHT_READ)
            .copied()
            .filter(|capability| {
                capability.kind == genos_abi::USER_FILE_KIND_DIRECTORY
                    && capability.rights & USER_FILE_RIGHT_READ != 0
            })
        else {
            managed.process.context.rax =
                syscall::error_code(syscall::SyscallError::InvalidArgument);
            managed.state = ManagedState::Ready;
            return None;
        };
        let Some(request_id) = managed.allocate_request_id() else {
            managed.process.context.rax = syscall::error_code(syscall::SyscallError::Unavailable);
            managed.state = ManagedState::Ready;
            return None;
        };
        managed.state = ManagedState::Waiting;
        managed.blocked_on = BlockReason::DirectoryRead;
        managed.pending_directory_read = Some(PendingDirectoryRead {
            request_id,
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
            request_id,
            owner_slot: managed.key.slot,
            owner_instance: managed.key.incarnation,
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
                managed.key.slot == request.owner_slot
                    && managed.key.incarnation == request.owner_instance
                    && managed.task_id == request.task_id
                    && managed.process.pid == request.pid
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
        if pending.request_id != request.request_id
            || pending.path != request.path
            || pending.handle != request.handle
            || pending.cursor != request.cursor
        {
            return Err(LaunchError::InvalidResult);
        }
        if !managed
            .file_handle(request.handle, USER_FILE_RIGHT_READ)
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

    fn block_namespace_mutation(
        &mut self,
        index: usize,
        parent: u64,
        name: FixedText,
        reason: BlockReason,
    ) -> Option<NamespaceMutationRequest> {
        let managed = self.slots[index].as_mut().expect("selected process exists");
        let capability = managed
            .file_handle(parent, USER_FILE_RIGHT_MANAGE)
            .copied()
            .filter(|capability| {
                capability.kind == USER_FILE_KIND_DIRECTORY
                    && capability.rights & USER_FILE_RIGHT_MANAGE != 0
                    && is_user_writable_directory(capability.path.as_str())
            });
        let target = capability.and_then(|capability| join_child_path(capability.path, name));
        let Some((capability, target)) = capability.zip(target) else {
            managed.process.context.rax =
                syscall::error_code(syscall::SyscallError::InvalidArgument);
            managed.state = ManagedState::Ready;
            return None;
        };
        if !matches!(
            reason,
            BlockReason::DirectoryCreate | BlockReason::PathRemove
        ) {
            managed.process.context.rax =
                syscall::error_code(syscall::SyscallError::InvalidArgument);
            managed.state = ManagedState::Ready;
            return None;
        }
        let Some(request_id) = managed.allocate_request_id() else {
            managed.process.context.rax = syscall::error_code(syscall::SyscallError::Unavailable);
            managed.state = ManagedState::Ready;
            return None;
        };
        let pending = PendingNamespaceMutation {
            request_id,
            parent: capability.path,
            target,
            handle: parent,
        };
        managed.state = ManagedState::Waiting;
        managed.blocked_on = reason;
        managed.pending_namespace_mutation = Some(pending);
        crate::serial::print(match reason {
            BlockReason::DirectoryCreate => "USER_DIRECTORY_CREATE_BLOCK pid=",
            BlockReason::PathRemove => "USER_PATH_REMOVE_BLOCK pid=",
            _ => "USER_NAMESPACE_BLOCK pid=",
        });
        crate::serial::print_u64(managed.process.pid as u64);
        crate::serial::print(" target=");
        crate::serial::println(target.as_str());
        Some(NamespaceMutationRequest {
            request_id,
            owner_slot: managed.key.slot,
            owner_instance: managed.key.incarnation,
            task_id: managed.task_id,
            pid: managed.process.pid,
            parent: capability.path,
            target,
            handle: parent,
        })
    }

    pub fn complete_directory_create(
        &mut self,
        request: NamespaceMutationRequest,
        created: bool,
    ) -> Result<ProcessUpdate, LaunchError> {
        self.complete_namespace_mutation(request, BlockReason::DirectoryCreate, created)
    }

    pub fn complete_path_remove(
        &mut self,
        request: NamespaceMutationRequest,
        removed: bool,
    ) -> Result<ProcessUpdate, LaunchError> {
        self.complete_namespace_mutation(request, BlockReason::PathRemove, removed)
    }

    fn complete_namespace_mutation(
        &mut self,
        request: NamespaceMutationRequest,
        reason: BlockReason,
        succeeded: bool,
    ) -> Result<ProcessUpdate, LaunchError> {
        let index = self
            .slots
            .iter()
            .position(|slot| {
                slot.as_ref().is_some_and(|managed| {
                    managed.key.slot == request.owner_slot
                        && managed.key.incarnation == request.owner_instance
                        && managed.task_id == request.task_id
                        && managed.process.pid == request.pid
                })
            })
            .ok_or(LaunchError::ImageUnavailable)?;
        let valid = self.slots[index].as_ref().is_some_and(|managed| {
            managed.state == ManagedState::Waiting
                && managed.blocked_on == reason
                && !managed.process.completed
                && managed.pending_namespace_mutation
                    == Some(PendingNamespaceMutation {
                        request_id: request.request_id,
                        parent: request.parent,
                        target: request.target,
                        handle: request.handle,
                    })
                && managed
                    .file_handle(request.handle, USER_FILE_RIGHT_MANAGE)
                    .is_some_and(|capability| {
                        capability.path == request.parent
                            && capability.kind == USER_FILE_KIND_DIRECTORY
                            && capability.rights & USER_FILE_RIGHT_MANAGE != 0
                            && is_user_writable_directory(capability.path.as_str())
                    })
        });
        if !valid {
            return Err(LaunchError::InvalidResult);
        }
        if succeeded && reason == BlockReason::PathRemove {
            for managed in self.slots.iter_mut().flatten() {
                for entry in managed.file_handles.iter_mut() {
                    if entry.is_some_and(|capability| {
                        paths_equal(capability.path.as_str(), request.target.as_str())
                    }) {
                        let handle = entry.expect("matched file capability").handle;
                        let removed = managed.handles.unregister(handle, HandleKind::File);
                        debug_assert!(removed);
                        *entry = None;
                    }
                }
            }
        }
        let managed = self.slots[index]
            .as_mut()
            .expect("namespace request owner exists");
        managed.pending_namespace_mutation = None;
        managed.process.context.rax = if succeeded {
            0
        } else {
            syscall::error_code(syscall::SyscallError::Unavailable)
        };
        managed.state = ManagedState::Ready;
        managed.blocked_on = BlockReason::None;
        crate::serial::print(match (reason, succeeded) {
            (BlockReason::DirectoryCreate, true) => "USER_DIRECTORY_CREATE_OK pid=",
            (BlockReason::DirectoryCreate, false) => "USER_DIRECTORY_CREATE_UNAVAILABLE pid=",
            (BlockReason::PathRemove, true) => "USER_PATH_REMOVE_OK pid=",
            (BlockReason::PathRemove, false) => "USER_PATH_REMOVE_UNAVAILABLE pid=",
            _ => "USER_NAMESPACE_COMPLETE pid=",
        });
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
        let handle = managed.endpoints.create(&mut managed.handles);
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
            managed.endpoints.allocate(
                &mut managed.handles,
                EndpointRole::Send {
                    target_pid,
                    target_generation,
                },
            )
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
                managed.endpoints.send_capability(&managed.handles, handle),
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
                || target
                    .endpoints
                    .receive_generation(&target.handles, pending.handle)
                    != Some(target_generation)
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
        let generation = managed
            .endpoints
            .receive_generation(&managed.handles, handle)
            .filter(|_| {
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
            (
                managed.process.pid,
                managed.endpoints.close(&mut managed.handles, handle),
            )
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
            revoked += managed.endpoints.revoke_send_handles(
                &mut managed.handles,
                target_pid,
                target_generation,
            );
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
            managed.endpoints.clear(&mut managed.handles);
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
        let (parent_pid, parent_key) = self.slots[parent_index]
            .as_ref()
            .map(|managed| (managed.process.pid, managed.key))
            .expect("selected process exists");
        let child = self.slots.iter().flatten().find(|managed| {
            managed.process.pid == child_pid && managed.parent_key == Some(parent_key)
        });
        let result = child.map(|managed| {
            if managed.process.completed {
                (managed.key, Some(managed.process.exit_code as u64))
            } else {
                (managed.key, None)
            }
        });
        let parent = self.slots[parent_index]
            .as_mut()
            .expect("selected process exists");
        match result {
            Some((_, Some(status))) => {
                parent.process.context.rax = status;
                parent.state = ManagedState::Ready;
            }
            Some((child_key, None)) => {
                parent.state = ManagedState::Waiting;
                parent.blocked_on = BlockReason::Child(child_key);
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
        let writable = rights & USER_FILE_RIGHT_WRITE != 0;
        let manageable = rights & USER_FILE_RIGHT_MANAGE != 0;
        if rights == 0
            || rights & !USER_FILE_RIGHTS_MASK != 0
            || (writable && manageable)
            || (writable && !is_user_writable_path(path.as_str()))
            || (manageable
                && (rights & USER_FILE_RIGHT_READ == 0
                    || !is_user_writable_directory(path.as_str())))
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
        let Some(request_id) = managed.allocate_request_id() else {
            managed.process.context.rax = syscall::error_code(syscall::SyscallError::Unavailable);
            managed.state = ManagedState::Ready;
            return None;
        };
        managed.state = ManagedState::Waiting;
        managed.blocked_on = BlockReason::FileOpen;
        managed.pending_file_open = Some(PendingFileOpen {
            request_id,
            path,
            rights,
        });
        crate::serial::print("USER_FILE_OPEN_BLOCK pid=");
        crate::serial::print_u64(managed.process.pid as u64);
        crate::serial::print(" path=");
        crate::serial::print(path.as_str());
        crate::serial::print(" rights=");
        crate::serial::print_u64(rights);
        crate::serial::println("");
        Some(FileOpenRequest {
            request_id,
            owner_slot: managed.key.slot,
            owner_instance: managed.key.incarnation,
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
                managed.key.slot == request.owner_slot
                    && managed.key.incarnation == request.owner_instance
                    && managed.task_id == request.task_id
                    && managed.process.pid == request.pid
            })
            .ok_or(LaunchError::ImageUnavailable)?;
        if managed.state != ManagedState::Waiting
            || managed.blocked_on != BlockReason::FileOpen
            || managed.process.completed
            || managed.pending_file_open
                != Some(PendingFileOpen {
                    request_id: request.request_id,
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
                    USER_FILE_KIND_REGULAR | USER_FILE_KIND_DIRECTORY
                ) && match metadata.kind {
                    USER_FILE_KIND_REGULAR => request.rights & USER_FILE_RIGHT_MANAGE == 0,
                    USER_FILE_KIND_DIRECTORY => {
                        request.rights & USER_FILE_RIGHT_READ != 0
                            && request.rights & USER_FILE_RIGHT_WRITE == 0
                    }
                    _ => false,
                }
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
        let Some(capability) = managed
            .file_handle(handle, USER_FILE_RIGHT_READ)
            .copied()
            .filter(|capability| {
                capability.kind == USER_FILE_KIND_REGULAR
                    && capability.rights & USER_FILE_RIGHT_READ != 0
            })
        else {
            managed.process.context.rax =
                syscall::error_code(syscall::SyscallError::InvalidArgument);
            managed.state = ManagedState::Ready;
            return None;
        };
        let Some(request_id) = managed.allocate_request_id() else {
            managed.process.context.rax = syscall::error_code(syscall::SyscallError::Unavailable);
            managed.state = ManagedState::Ready;
            return None;
        };
        managed.state = ManagedState::Waiting;
        managed.blocked_on = BlockReason::FileRead;
        managed.pending_file_read = Some(PendingFileRead {
            request_id,
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
            request_id,
            owner_slot: managed.key.slot,
            owner_instance: managed.key.incarnation,
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
        let stat = managed
            .file_handle(handle, 0)
            .map(|capability| UserFileStat {
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
        let Some(capability) = managed
            .file_handle(handle, USER_FILE_RIGHT_WRITE)
            .copied()
            .filter(|capability| {
                capability.kind == USER_FILE_KIND_REGULAR
                    && capability.rights & USER_FILE_RIGHT_WRITE != 0
                    && is_user_writable_path(capability.path.as_str())
                    && !data.is_empty()
            })
        else {
            managed.process.context.rax =
                syscall::error_code(syscall::SyscallError::InvalidArgument);
            managed.state = ManagedState::Ready;
            crate::serial::print("USER_HANDLE_WRITE_DENIED pid=");
            crate::serial::print_u64(managed.process.pid as u64);
            crate::serial::println("");
            return None;
        };
        let Some(request_id) = managed.allocate_request_id() else {
            managed.process.context.rax = syscall::error_code(syscall::SyscallError::Unavailable);
            managed.state = ManagedState::Ready;
            return None;
        };
        let pending = PendingFileWrite {
            request_id,
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
            request_id,
            owner_slot: managed.key.slot,
            owner_instance: managed.key.incarnation,
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
                managed.key.slot == request.owner_slot
                    && managed.key.incarnation == request.owner_instance
                    && managed.task_id == request.task_id
                    && managed.process.pid == request.pid
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
        if pending.request_id != request.request_id
            || pending.handle != request.handle
            || pending.path != request.path
            || pending.offset != request.offset
            || pending.data != request.data
        {
            return Err(LaunchError::InvalidResult);
        }
        let capability = managed
            .file_handle(request.handle, USER_FILE_RIGHT_WRITE)
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
                .file_handle_mut(request.handle, USER_FILE_RIGHT_WRITE)
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

    fn block_file_handle_truncate(
        &mut self,
        index: usize,
        handle: u64,
    ) -> Option<FileTruncateRequest> {
        let managed = self.slots[index].as_mut().expect("selected process exists");
        let Some(capability) = managed
            .file_handle(handle, USER_FILE_RIGHT_WRITE)
            .copied()
            .filter(|capability| {
                capability.kind == USER_FILE_KIND_REGULAR
                    && capability.rights & USER_FILE_RIGHT_WRITE != 0
                    && is_user_writable_path(capability.path.as_str())
            })
        else {
            managed.process.context.rax =
                syscall::error_code(syscall::SyscallError::InvalidArgument);
            managed.state = ManagedState::Ready;
            crate::serial::print("USER_HANDLE_TRUNCATE_DENIED pid=");
            crate::serial::print_u64(managed.process.pid as u64);
            crate::serial::println("");
            return None;
        };
        let Some(request_id) = managed.allocate_request_id() else {
            managed.process.context.rax = syscall::error_code(syscall::SyscallError::Unavailable);
            managed.state = ManagedState::Ready;
            return None;
        };
        managed.state = ManagedState::Waiting;
        managed.blocked_on = BlockReason::FileTruncate;
        managed.pending_file_truncate = Some(PendingFileTruncate {
            request_id,
            handle,
            path: capability.path,
        });
        crate::serial::print("USER_HANDLE_TRUNCATE_BLOCK pid=");
        crate::serial::print_u64(managed.process.pid as u64);
        crate::serial::print(" handle=0x");
        crate::serial::print_hex(handle);
        crate::serial::println("");
        Some(FileTruncateRequest {
            request_id,
            owner_slot: managed.key.slot,
            owner_instance: managed.key.incarnation,
            task_id: managed.task_id,
            pid: managed.process.pid,
            path: capability.path,
            handle,
        })
    }

    pub fn complete_file_truncate(
        &mut self,
        request: FileTruncateRequest,
        truncated: bool,
    ) -> Result<ProcessUpdate, LaunchError> {
        let managed = self
            .slots
            .iter_mut()
            .flatten()
            .find(|managed| {
                managed.key.slot == request.owner_slot
                    && managed.key.incarnation == request.owner_instance
                    && managed.task_id == request.task_id
                    && managed.process.pid == request.pid
            })
            .ok_or(LaunchError::ImageUnavailable)?;
        if managed.state != ManagedState::Waiting
            || managed.blocked_on != BlockReason::FileTruncate
            || managed.process.completed
            || managed.pending_file_truncate
                != Some(PendingFileTruncate {
                    request_id: request.request_id,
                    handle: request.handle,
                    path: request.path,
                })
        {
            return Err(LaunchError::InvalidResult);
        }
        let valid_capability = managed
            .file_handle(request.handle, USER_FILE_RIGHT_WRITE)
            .is_some_and(|capability| {
                capability.path == request.path
                    && capability.kind == USER_FILE_KIND_REGULAR
                    && capability.rights & USER_FILE_RIGHT_WRITE != 0
                    && is_user_writable_path(capability.path.as_str())
            });
        if !valid_capability {
            return Err(LaunchError::InvalidResult);
        }
        managed.pending_file_truncate = None;
        if truncated {
            let capability = managed
                .file_handle_mut(request.handle, USER_FILE_RIGHT_WRITE)
                .ok_or(LaunchError::InvalidResult)?;
            capability.offset = 0;
            capability.size = 0;
        }
        managed.process.context.rax = if truncated {
            0
        } else {
            syscall::error_code(syscall::SyscallError::Unavailable)
        };
        managed.state = ManagedState::Ready;
        managed.blocked_on = BlockReason::None;
        crate::serial::print(if truncated {
            "USER_HANDLE_TRUNCATE_OK pid="
        } else {
            "USER_HANDLE_TRUNCATE_FAILED pid="
        });
        crate::serial::print_u64(managed.process.pid as u64);
        crate::serial::println("");
        Ok(process_update(managed))
    }

    fn block_file_read(
        &mut self,
        index: usize,
        path: FixedText,
        address: u64,
        capacity: u64,
    ) -> Option<FileReadRequest> {
        let managed = self.slots[index].as_mut().expect("selected process exists");
        let Some(request_id) = managed.allocate_request_id() else {
            managed.process.context.rax = syscall::error_code(syscall::SyscallError::Unavailable);
            managed.state = ManagedState::Ready;
            return None;
        };
        managed.state = ManagedState::Waiting;
        managed.blocked_on = BlockReason::FileRead;
        managed.pending_file_read = Some(PendingFileRead {
            request_id,
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
        Some(FileReadRequest {
            request_id,
            owner_slot: managed.key.slot,
            owner_instance: managed.key.incarnation,
            task_id: managed.task_id,
            pid: managed.process.pid,
            path,
            handle: 0,
            offset: 0,
            capacity,
        })
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
                managed.key.slot == request.owner_slot
                    && managed.key.incarnation == request.owner_instance
                    && managed.task_id == request.task_id
                    && managed.process.pid == request.pid
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
        if pending.request_id != request.request_id
            || pending.path != request.path
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
            let Some(capability) = managed.file_handle_mut(pending.handle, USER_FILE_RIGHT_READ)
            else {
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
        if self.terminate_process_at(index, state, None).is_err() {
            fail("USER_RECLAIM_FAILED");
        }
    }

    fn terminate_process_at(
        &mut self,
        index: usize,
        state: ManagedState,
        forced_exit_code: Option<u8>,
    ) -> Result<TerminalRecord, LaunchError> {
        self.release_endpoints(index);
        let (record, supervisor) = {
            let managed = self.slots[index]
                .as_mut()
                .ok_or(LaunchError::ImageUnavailable)?;
            if let Some(exit_code) = forced_exit_code {
                managed.process.completed = true;
                managed.process.killed = state == ManagedState::Killed;
                managed.process.event = ProcessEvent::Exit;
                managed.process.exit_code = exit_code;
                managed.process.completion_order =
                    COMPLETION_SEQUENCE.fetch_add(1, Ordering::AcqRel) + 1;
            } else if !managed.process.completed {
                return Err(LaunchError::InvalidResult);
            }
            managed.state = state;
            managed.revoke_resources();
            reclaim_process(&mut managed.process).map_err(|_| LaunchError::InvalidResult)?;
            (
                TerminalRecord {
                    key: managed.key,
                    task_id: managed.task_id,
                    pid: managed.process.pid,
                    exit_code: managed.process.exit_code,
                    preemptions: managed.process.preemptions,
                    console_process: managed.console_process,
                },
                managed.supervisor,
            )
        };
        if supervisor {
            let cleaned = self.cleanup_supervised_children(record.key)?;
            crate::serial::print("USER_SUPERVISOR_CHILDREN_CLEANED owner=");
            crate::serial::print_u64(record.pid as u64);
            crate::serial::print(" children=");
            crate::serial::print_u64(cleaned as u64);
            crate::serial::println("");
        }
        self.wake_waiting_parent(record.key, record.pid, record.exit_code);
        Ok(record)
    }

    fn cleanup_supervised_children(&mut self, owner_key: ProcessKey) -> Result<usize, LaunchError> {
        let mut cleaned = 0usize;
        let mut index = 0usize;
        while index < self.slots.len() {
            if self.slots[index]
                .as_ref()
                .is_some_and(|managed| managed.parent_key == Some(owner_key))
            {
                self.release_endpoints(index);
                let managed = self.slots[index].as_mut().expect("supervised child exists");
                if !managed.process.completed {
                    managed.process.completed = true;
                    managed.process.killed = true;
                    managed.process.exit_code = 137;
                    managed.state = ManagedState::Killed;
                    managed.revoke_resources();
                    reclaim_process(&mut managed.process)
                        .map_err(|_| LaunchError::InvalidResult)?;
                }
                self.slots[index] = None;
                cleaned += 1;
            }
            index += 1;
        }
        Ok(cleaned)
    }

    fn wake_waiting_parent(&mut self, child_key: ProcessKey, child_pid: u8, exit_code: u8) {
        for managed in self.slots.iter_mut().flatten() {
            if managed.state == ManagedState::Waiting
                && managed.blocked_on == BlockReason::Child(child_key)
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
        let record = self.terminate_process_at(index, ManagedState::Killed, Some(137))?;
        crate::serial::print("USER_KILLED pid=");
        crate::serial::print_u64(record.pid as u64);
        crate::serial::print(" task=");
        crate::serial::print_u64(task_id as u64);
        crate::serial::println("");
        let update = ProcessUpdate {
            task_id: record.task_id,
            pid: record.pid,
            state: ManagedState::Killed,
            exit_code: record.exit_code,
            preemptions: record.preemptions,
            output: FixedText::empty(),
            console_process: record.console_process,
            console: None,
            vfs_request: None,
            lifecycle_request: None,
            socket_request: None,
        };
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
            .any(|managed| managed.console_process && !managed.process.completed)
    }

    pub fn console_input_ready(&self) -> bool {
        self.slots.iter().flatten().any(|managed| {
            managed.console_process
                && !managed.process.completed
                && managed.state == ManagedState::Waiting
                && managed.blocked_on == BlockReason::Input
        })
    }

    pub fn append_task_snapshots(&self, snapshot: &mut TaskSnapshotSet) {
        for managed in self.slots.iter().flatten() {
            let state = managed_task_state(managed.state);
            let name = if managed.console_process {
                "ring3-shell"
            } else {
                "init-elf"
            };
            let _ = snapshot.push(TaskSnapshot {
                id: managed.task_id,
                name: FixedText::from_str(name),
                class: TaskClass::User,
                state,
                memory_kib: 40,
            });
        }
    }

    pub fn task_snapshots_match(&self, snapshot: &TaskSnapshotSet) -> bool {
        if snapshot.class_len(TaskClass::User) != self.slots.iter().flatten().count() {
            return false;
        }
        self.slots.iter().flatten().all(|managed| {
            snapshot.find(managed.task_id).is_some_and(|task| {
                task.class == TaskClass::User
                    && task.state == managed_task_state(managed.state)
                    && task.name.as_str()
                        == if managed.console_process {
                            "ring3-shell"
                        } else {
                            "init-elf"
                        }
            })
        })
    }

    /// Confirms that a deferred VFS operation still belongs to the exact
    /// process request that was parked. The outer VFS coordinator calls this
    /// before any mutation so a killed or replaced process cannot leave a
    /// stale write behind.
    pub fn vfs_request_active(&self, request: UserVfsRequest) -> bool {
        self.slots.iter().flatten().any(|managed| {
            if managed.state != ManagedState::Waiting || managed.process.completed {
                return false;
            }
            match request {
                UserVfsRequest::Open(request) => {
                    managed.key.slot == request.owner_slot
                        && managed.key.incarnation == request.owner_instance
                        && managed.task_id == request.task_id
                        && managed.process.pid == request.pid
                        && managed.blocked_on == BlockReason::FileOpen
                        && managed.pending_file_open.is_some_and(|pending| {
                            pending.request_id == request.request_id
                                && pending.path == request.path
                                && pending.rights == request.rights
                        })
                }
                UserVfsRequest::Read(request) => {
                    managed.key.slot == request.owner_slot
                        && managed.key.incarnation == request.owner_instance
                        && managed.task_id == request.task_id
                        && managed.process.pid == request.pid
                        && managed.blocked_on == BlockReason::FileRead
                        && managed.pending_file_read.is_some_and(|pending| {
                            pending.request_id == request.request_id
                                && pending.handle == request.handle
                                && pending.path == request.path
                                && pending.offset == request.offset
                                && pending.capacity == request.capacity
                        })
                }
                UserVfsRequest::Write(request) => {
                    managed.key.slot == request.owner_slot
                        && managed.key.incarnation == request.owner_instance
                        && managed.task_id == request.task_id
                        && managed.process.pid == request.pid
                        && managed.blocked_on == BlockReason::FileWrite
                        && managed.pending_file_write.is_some_and(|pending| {
                            pending.request_id == request.request_id
                                && pending.handle == request.handle
                                && pending.path == request.path
                                && pending.offset == request.offset
                                && pending.data == request.data
                        })
                }
                UserVfsRequest::Truncate(request) => {
                    managed.key.slot == request.owner_slot
                        && managed.key.incarnation == request.owner_instance
                        && managed.task_id == request.task_id
                        && managed.process.pid == request.pid
                        && managed.blocked_on == BlockReason::FileTruncate
                        && managed.pending_file_truncate.is_some_and(|pending| {
                            pending.request_id == request.request_id
                                && pending.handle == request.handle
                                && pending.path == request.path
                        })
                }
                UserVfsRequest::ReadDirectory(request) => {
                    managed.key.slot == request.owner_slot
                        && managed.key.incarnation == request.owner_instance
                        && managed.task_id == request.task_id
                        && managed.process.pid == request.pid
                        && managed.blocked_on == BlockReason::DirectoryRead
                        && managed.pending_directory_read.is_some_and(|pending| {
                            pending.request_id == request.request_id
                                && pending.handle == request.handle
                                && pending.path == request.path
                                && pending.cursor == request.cursor
                        })
                }
                UserVfsRequest::CreateDirectory(request) => {
                    namespace_request_active(managed, request, BlockReason::DirectoryCreate)
                }
                UserVfsRequest::RemovePath(request) => {
                    namespace_request_active(managed, request, BlockReason::PathRemove)
                }
            }
        })
    }

    pub fn lifecycle_request_active(&self, request: UserLifecycleRequest) -> bool {
        match request {
            UserLifecycleRequest::Launch(request) => self.slots.iter().flatten().any(|managed| {
                managed.key.slot == request.owner_slot
                    && managed.key.incarnation == request.owner_instance
                    && managed.task_id == request.owner_task_id
                    && managed.process.pid == request.owner_pid
                    && managed.state == ManagedState::Waiting
                    && !managed.process.completed
                    && managed.blocked_on == BlockReason::ProcessLaunch
                    && managed.pending_process_launch
                        == Some(PendingProcessLaunch {
                            request_id: request.request_id,
                            image: request.image,
                            mode: request.mode,
                        })
            }),
        }
    }

    pub fn socket_request_active(&self, request: UserSocketRequest) -> bool {
        self.slots.iter().flatten().any(|managed| {
            managed.key.slot == request.owner_slot
                && managed.key.incarnation == request.owner_instance
                && managed.task_id == request.owner_task_id
                && managed.process.pid == request.owner_pid
                && !managed.process.completed
                && managed
                    .handles
                    .allows(request.handle, HandleKind::Socket, HANDLE_RIGHT_USE)
                && managed.sockets.transport_request_active(
                    socket_owner(managed),
                    request.handle,
                    request.protocol,
                    request.request_id,
                    request.data.len(),
                )
        })
    }

    pub fn complete_socket_request(
        &mut self,
        request: UserSocketRequest,
        response: Option<&[u8]>,
    ) -> Result<ProcessUpdate, LaunchError> {
        if !self.socket_request_active(request) {
            return Err(LaunchError::InvalidResult);
        }
        let managed = self
            .slots
            .iter_mut()
            .flatten()
            .find(|managed| {
                managed.key.slot == request.owner_slot
                    && managed.key.incarnation == request.owner_instance
                    && managed.task_id == request.owner_task_id
                    && managed.process.pid == request.owner_pid
            })
            .ok_or(LaunchError::ImageUnavailable)?;
        let owner = socket_owner(managed);
        match response {
            Some(bytes) => {
                managed
                    .sockets
                    .complete_transport(
                        owner,
                        request.handle,
                        request.protocol,
                        request.request_id,
                        bytes,
                    )
                    .map_err(|_| LaunchError::InvalidResult)?;
                crate::serial::print(match request.protocol {
                    SocketProtocol::Udp => "USER_SOCKET_UDP_COMPLETE pid=",
                    SocketProtocol::TcpStream => "USER_SOCKET_TCP_COMPLETE pid=",
                });
                crate::serial::print_u64(managed.process.pid as u64);
                crate::serial::print(" bytes=");
                crate::serial::print_u64(bytes.len() as u64);
                crate::serial::println("");
            }
            None => {
                managed
                    .sockets
                    .fail_transport(owner, request.handle, request.protocol, request.request_id)
                    .map_err(|_| LaunchError::InvalidResult)?;
                crate::serial::print(match request.protocol {
                    SocketProtocol::Udp => "USER_SOCKET_UDP_TIMEOUT pid=",
                    SocketProtocol::TcpStream => "USER_SOCKET_TCP_ERROR pid=",
                });
                crate::serial::print_u64(managed.process.pid as u64);
                crate::serial::println("");
            }
        }
        Ok(process_update(managed))
    }
}

fn namespace_request_active(
    managed: &ManagedProcess,
    request: NamespaceMutationRequest,
    reason: BlockReason,
) -> bool {
    managed.key.slot == request.owner_slot
        && managed.key.incarnation == request.owner_instance
        && managed.task_id == request.task_id
        && managed.process.pid == request.pid
        && managed.blocked_on == reason
        && managed.pending_namespace_mutation
            == Some(PendingNamespaceMutation {
                request_id: request.request_id,
                parent: request.parent,
                target: request.target,
                handle: request.handle,
            })
}

fn managed_task_state(state: ManagedState) -> TaskState {
    match state {
        ManagedState::Ready => TaskState::Ready,
        ManagedState::Sleeping => TaskState::Sleeping,
        ManagedState::Waiting => TaskState::Waiting,
        ManagedState::Exited => TaskState::Exited,
        ManagedState::Faulted => TaskState::Faulted,
        ManagedState::Killed => TaskState::Killed,
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
        console_process: managed.console_process,
        console: None,
        vfs_request: None,
        lifecycle_request: None,
        socket_request: None,
    }
}

fn managed_state_abi(state: ManagedState) -> u64 {
    match state {
        ManagedState::Ready => USER_PROCESS_STATE_READY,
        ManagedState::Sleeping => USER_PROCESS_STATE_SLEEPING,
        ManagedState::Waiting => USER_PROCESS_STATE_WAITING,
        ManagedState::Exited => USER_PROCESS_STATE_EXITED,
        ManagedState::Faulted => USER_PROCESS_STATE_FAULTED,
        ManagedState::Killed => USER_PROCESS_STATE_KILLED,
    }
}

fn user_process_status_bytes(status: &UserProcessStatus) -> &[u8] {
    unsafe {
        core::slice::from_raw_parts(
            core::ptr::from_ref(status).cast::<u8>(),
            core::mem::size_of::<UserProcessStatus>(),
        )
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

pub fn run_probe(elf_bytes: &'static [u8]) {
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
    let switch_benchmark = paging::benchmark_address_space_switch(first.space, 32)
        .filter(|result| {
            result.min_pair_cycles > 0 && result.average_pair_cycles >= result.min_pair_cycles
        })
        .unwrap_or_else(|| fail("SCHED_CONTEXT_BENCH_FAILED"));
    crate::serial::print("SCHED_CONTEXT_BENCH switches=");
    crate::serial::print_u64(u64::from(switch_benchmark.samples) * 2);
    crate::serial::print(" min_pair_cycles=");
    crate::serial::print_u64(switch_benchmark.min_pair_cycles);
    crate::serial::print(" avg_pair_cycles=");
    crate::serial::print_u64(switch_benchmark.average_pair_cycles);
    crate::serial::println("");
    crate::serial::println("SCHED_CONTEXT_BENCH_OK");
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
                | ProcessEvent::TruncateHandle { .. }
                | ProcessEvent::CreateDirectory { .. }
                | ProcessEvent::RemovePath { .. }
                | ProcessEvent::ProcessLaunch { .. }
                | ProcessEvent::ProcessStatus { .. }
                | ProcessEvent::ProcessKill { .. }
                | ProcessEvent::ProcessReap { .. }
                | ProcessEvent::WaitInput { .. }
                | ProcessEvent::CreateEndpoint
                | ProcessEvent::ConnectEndpoint(_)
                | ProcessEvent::SendEndpoint { .. }
                | ProcessEvent::ReceiveEndpoint { .. }
                | ProcessEvent::CloseEndpoint(_)
                | ProcessEvent::ConsoleWrite { .. }
                | ProcessEvent::ConsoleSetInput { .. }
                | ProcessEvent::ConsoleClear(_)
                | ProcessEvent::SocketOpen { .. }
                | ProcessEvent::SocketConnect { .. }
                | ProcessEvent::SocketBind { .. }
                | ProcessEvent::SocketListen { .. }
                | ProcessEvent::SocketAccept { .. }
                | ProcessEvent::SocketSend { .. }
                | ProcessEvent::SocketReceive { .. }
                | ProcessEvent::SocketStatus { .. }
                | ProcessEvent::SocketShutdown { .. }
                | ProcessEvent::SocketClose(_) => fail("USER_PROBE_BLOCKED"),
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

pub fn active_process_count() -> u8 {
    ACTIVE_PROCESSES.load(Ordering::Acquire)
}

pub fn reclaimed_frame_count() -> u64 {
    RECLAIMED_FRAMES.load(Ordering::Acquire)
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
    path.len() > USER_WRITABLE_PREFIX.len()
        && path
            .get(..USER_WRITABLE_PREFIX.len())
            .is_some_and(|prefix| paths_equal(prefix, USER_WRITABLE_PREFIX))
}

fn is_user_writable_directory(path: &str) -> bool {
    paths_equal(path, USER_WRITABLE_PREFIX.trim_end_matches('/')) || is_user_writable_path(path)
}

fn paths_equal(left: &str, right: &str) -> bool {
    left.len() == right.len()
        && left
            .bytes()
            .zip(right.bytes())
            .all(|(left, right)| left.eq_ignore_ascii_case(&right))
}

fn join_child_path(parent: FixedText, name: FixedText) -> Option<FixedText> {
    let name = name.as_str();
    if name.is_empty() || name == "." || name == ".." || name.contains('/') {
        return None;
    }
    let separator = if parent.as_str() == "/" { "" } else { "/" };
    let length = parent
        .len()
        .checked_add(separator.len())?
        .checked_add(name.len())?;
    if length > USER_PATH_MAX {
        return None;
    }
    let mut target = parent;
    target.push_str(separator);
    target.push_str(name);
    (target.len() == length).then_some(target)
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
    const CANCELED_VFS_TASK: u32 = 0x100b;

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
    let mut saw_one_shot_rejection = false;
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
                    invalid.request_id = invalid.request_id.saturating_add(1);
                    if manager.complete_file_open(invalid, info).is_ok() {
                        fail("USER_FILE_OPEN_IDENTITY_FAILED");
                    }
                    if !matches!(manager.complete_file_open(request, info), Ok(update) if update.state == ManagedState::Ready)
                    {
                        fail("USER_FILE_OPEN_COMPLETION_FAILED");
                    }
                    if manager.complete_file_open(request, info).is_ok() {
                        fail("USER_FILE_OPEN_REPLAY_ACCEPTED");
                    }
                    saw_one_shot_rejection = true;
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
                    invalid.request_id = invalid.request_id.saturating_add(1);
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
                UserVfsRequest::Truncate(_) => fail("USER_UNEXPECTED_FILE_TRUNCATE"),
                UserVfsRequest::ReadDirectory(_) => fail("USER_UNEXPECTED_DIRECTORY_READ"),
                UserVfsRequest::CreateDirectory(_) => fail("USER_UNEXPECTED_DIRECTORY_CREATE"),
                UserVfsRequest::RemovePath(_) => fail("USER_UNEXPECTED_PATH_REMOVE"),
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
        || !saw_one_shot_rejection
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
    crate::serial::println("USER_ASYNC_ONE_SHOT_OK");

    if vfs.find("/USER/APP.TXT").is_some() || manager.spawn_write_init(CANCELED_VFS_TASK).is_err() {
        fail("USER_ASYNC_CANCELLATION_SETUP_FAILED");
    }
    let mut canceled_request = None;
    for tick in 113..=119 {
        let Some(update) = manager.poll(tick) else {
            continue;
        };
        if update.task_id == CANCELED_VFS_TASK {
            if let Some(request) = update.vfs_request {
                canceled_request = Some(request);
                break;
            }
        }
    }
    let Some(request @ UserVfsRequest::Open(open)) = canceled_request else {
        fail("USER_ASYNC_CANCELLATION_REQUEST_FAILED");
    };
    let mut wrong_id = open;
    wrong_id.request_id = wrong_id.request_id.saturating_add(1);
    if open.request_id == 0
        || manager.vfs_request_active(UserVfsRequest::Open(wrong_id))
        || !manager.vfs_request_active(request)
        || !matches!(manager.kill(CANCELED_VFS_TASK), Ok(update) if update.state == ManagedState::Killed)
        || manager.vfs_request_active(request)
        || manager.complete_file_open(open, None).is_ok()
        || vfs.find(open.path.as_str()).is_some()
        || !matches!(manager.wait(CANCELED_VFS_TASK), Ok(result) if result.state == ManagedState::Killed)
        || manager.live_count() != 0
        || active_process_count() != 0
    {
        fail("USER_ASYNC_CANCELLATION_FAILED");
    }
    crate::serial::println("USER_ASYNC_REQUEST_ID_OK");
    crate::serial::println("USER_ASYNC_CANCELLATION_OK");

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
                    invalid.request_id = invalid.request_id.saturating_add(1);
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
                UserVfsRequest::Truncate(_) => fail("USER_UNEXPECTED_FILE_TRUNCATE"),
                UserVfsRequest::ReadDirectory(_) => fail("USER_UNEXPECTED_DIRECTORY_READ"),
                UserVfsRequest::CreateDirectory(_) => fail("USER_UNEXPECTED_DIRECTORY_CREATE"),
                UserVfsRequest::RemovePath(_) => fail("USER_UNEXPECTED_PATH_REMOVE"),
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
        || reclaimed_frame_count() < reclaimed_before + 100
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
        || reclaimed_frame_count() < reclaimed_before + 120
    {
        fail("USER_INPUT_PROBE_FAILED");
    }
    crate::serial::println("USER_INPUT_BLOCK_OK");
    crate::serial::println("USER_INPUT_FILTER_OK");
    crate::serial::println("USER_INPUT_OWNERSHIP_OK");
    crate::serial::println("USER_INPUT_WAKE_OK");
    crate::serial::println("USER_ASYNC_LIFECYCLE_OK");
}

pub fn run_supervisor_cleanup_probe() {
    run_supervisor_cleanup_case(ManagedState::Exited, 0x1100, 0x1101);
    run_supervisor_cleanup_case(ManagedState::Faulted, 0x1102, 0x1103);
    run_supervisor_cleanup_case(ManagedState::Killed, 0x1104, 0x1105);
    crate::serial::println("USER_SUPERVISOR_POLICY_OK");
    crate::serial::println("USER_SUPERVISOR_NO_STALE_TASKS_OK");
    crate::serial::println("USER_SUPERVISOR_NO_STALE_HANDLES_OK");
    crate::serial::println("USER_SUPERVISOR_PENDING_CANCEL_OK");
}

pub fn run_transactional_rollback_probe() {
    let active_before = active_process_count();
    let reclaimed_before = reclaimed_frame_count();
    let mut manager = ProcessManager::new();
    let owner_task = 0x1200;
    manager
        .spawn_shell(owner_task)
        .unwrap_or_else(|_| fail("USER_ROLLBACK_SHELL_FAILED"));
    let owner_index = manager
        .slots
        .iter()
        .position(|slot| {
            slot.as_ref()
                .is_some_and(|managed| managed.task_id == owner_task)
        })
        .unwrap_or_else(|| fail("USER_ROLLBACK_OWNER_MISSING"));
    let supervisor = manager.slots[owner_index]
        .as_ref()
        .map(|managed| managed.process.lifecycle_handle)
        .unwrap_or_else(|| fail("USER_ROLLBACK_OWNER_MISSING"));

    for ordinal in 0..PROCESS_HANDLE_CAPACITY {
        let request = manager
            .block_process_launch(
                owner_index,
                supervisor,
                USER_PROCESS_IMAGE_INIT,
                USER_PROCESS_MODE_HOLD,
            )
            .unwrap_or_else(|| fail("USER_ROLLBACK_FILL_BLOCK_FAILED"));
        manager
            .complete_process_launch(request, Some(0x1210 + ordinal as u32))
            .unwrap_or_else(|_| fail("USER_ROLLBACK_FILL_LAUNCH_FAILED"));
    }
    let full_active = active_process_count();
    if manager.live_count() != MAX_ASYNC_PROCESSES
        || manager
            .block_process_launch(
                owner_index,
                supervisor,
                USER_PROCESS_IMAGE_INIT,
                USER_PROCESS_MODE_HOLD,
            )
            .is_some()
        || active_process_count() != full_active
        || !manager.unified_handle_table_is_authoritative()
    {
        fail("USER_ROLLBACK_FULL_TABLE_FAILED");
    }
    manager
        .kill(owner_task)
        .unwrap_or_else(|_| fail("USER_ROLLBACK_FULL_CLEANUP_FAILED"));
    manager
        .wait(owner_task)
        .unwrap_or_else(|_| fail("USER_ROLLBACK_FULL_REAP_FAILED"));
    if manager.slots.iter().any(Option::is_some) || active_process_count() != active_before {
        fail("USER_ROLLBACK_FULL_LEAKED");
    }

    let owner_task = 0x1220;
    manager
        .spawn_shell(owner_task)
        .unwrap_or_else(|_| fail("USER_ROLLBACK_RESTART_FAILED"));
    let owner_index = manager
        .slots
        .iter()
        .position(|slot| {
            slot.as_ref()
                .is_some_and(|managed| managed.task_id == owner_task)
        })
        .unwrap_or_else(|| fail("USER_ROLLBACK_OWNER_MISSING"));
    let supervisor = manager.slots[owner_index]
        .as_ref()
        .map(|managed| managed.process.lifecycle_handle)
        .unwrap_or_else(|| fail("USER_ROLLBACK_OWNER_MISSING"));
    let failed_request = manager
        .block_process_launch(
            owner_index,
            supervisor,
            USER_PROCESS_IMAGE_INIT,
            USER_PROCESS_MODE_HOLD,
        )
        .unwrap_or_else(|| fail("USER_ROLLBACK_FAILED_LAUNCH_BLOCK_FAILED"));
    let active_with_owner = active_process_count();
    manager
        .complete_process_launch(failed_request, None)
        .unwrap_or_else(|_| fail("USER_ROLLBACK_FAILED_LAUNCH_COMPLETE_FAILED"));
    if manager.live_count() != 1
        || active_process_count() != active_with_owner
        || manager.slots.iter().flatten().count() != 1
        || manager.slots[owner_index]
            .as_ref()
            .is_none_or(|owner| owner.pending_process_launch.is_some() || owner.handles.len() != 2)
    {
        fail("USER_ROLLBACK_FAILED_LAUNCH_LEAKED");
    }

    let request = manager
        .block_process_launch(
            owner_index,
            supervisor,
            USER_PROCESS_IMAGE_INIT,
            USER_PROCESS_MODE_HOLD,
        )
        .unwrap_or_else(|| fail("USER_ROLLBACK_COPYOUT_LAUNCH_BLOCK_FAILED"));
    manager
        .complete_process_launch(request, Some(0x1221))
        .unwrap_or_else(|_| fail("USER_ROLLBACK_COPYOUT_LAUNCH_FAILED"));
    let handle = manager.slots[owner_index]
        .as_ref()
        .map(|owner| owner.process.context.rax)
        .unwrap_or_else(|| fail("USER_ROLLBACK_OWNER_MISSING"));
    manager.complete_controlled_kill(owner_index, handle);
    let child_key = manager.slots[owner_index]
        .as_ref()
        .and_then(|owner| owner.process_capability(handle))
        .map(|capability| capability.target)
        .unwrap_or_else(|| fail("USER_ROLLBACK_HANDLE_MISSING"));
    manager.complete_controlled_reap(
        owner_index,
        handle,
        paging::USER_STACK_GUARD,
        core::mem::size_of::<UserProcessStatus>() as u64,
    );
    if manager.slots[child_key.slot as usize].is_none()
        || manager.slots[owner_index]
            .as_ref()
            .and_then(|owner| owner.process_capability(handle))
            .is_none()
    {
        fail("USER_ROLLBACK_COPYOUT_REMOVED_AUTHORITY");
    }
    manager.complete_controlled_reap(
        owner_index,
        handle,
        paging::USER_DATA + 0x200,
        core::mem::size_of::<UserProcessStatus>() as u64,
    );
    if manager.slots[child_key.slot as usize].is_some()
        || manager.slots[owner_index]
            .as_ref()
            .and_then(|owner| owner.process_capability(handle))
            .is_some()
    {
        fail("USER_ROLLBACK_COPYOUT_COMMIT_FAILED");
    }

    let canceled = manager
        .block_file_open(
            owner_index,
            FixedText::from_str("/USER/CANCEL.TXT"),
            USER_FILE_RIGHT_WRITE,
        )
        .map(UserVfsRequest::Open)
        .unwrap_or_else(|| fail("USER_ROLLBACK_CANCEL_BLOCK_FAILED"));
    manager
        .kill(owner_task)
        .unwrap_or_else(|_| fail("USER_ROLLBACK_CANCEL_KILL_FAILED"));
    if manager.vfs_request_active(canceled) {
        fail("USER_ROLLBACK_CANCEL_STILL_ACTIVE");
    }
    manager
        .wait(owner_task)
        .unwrap_or_else(|_| fail("USER_ROLLBACK_CANCEL_REAP_FAILED"));
    if manager.slots.iter().any(Option::is_some)
        || active_process_count() != active_before
        || reclaimed_frame_count() < reclaimed_before + 70
    {
        fail("USER_ROLLBACK_FINAL_LEAK");
    }
    crate::serial::println("USER_ROLLBACK_FULL_TABLE_OK");
    crate::serial::println("USER_ROLLBACK_LAUNCH_REFUSED_OK");
    crate::serial::println("USER_ROLLBACK_COPYOUT_OK");
    crate::serial::println("USER_ROLLBACK_CANCELLATION_OK");
}

pub fn run_process_generation_stress_probe() {
    const LAUNCHES: usize = 257;
    let active_before = active_process_count();
    let mut manager = ProcessManager::new();
    let owner_task = 0x1300;
    manager
        .spawn_shell(owner_task)
        .unwrap_or_else(|_| fail("USER_GENERATION_STRESS_SHELL_FAILED"));
    let owner_index = manager
        .slots
        .iter()
        .position(|slot| {
            slot.as_ref()
                .is_some_and(|managed| managed.task_id == owner_task)
        })
        .unwrap_or_else(|| fail("USER_GENERATION_STRESS_OWNER_MISSING"));
    let supervisor = manager.slots[owner_index]
        .as_ref()
        .map(|managed| managed.process.lifecycle_handle)
        .unwrap_or_else(|| fail("USER_GENERATION_STRESS_OWNER_MISSING"));
    let mut seen_pids = [false; 256];
    let mut pid_reused = false;
    let mut previous_incarnation = 0u64;
    let mut previous_handle = 0u64;
    let mut first_stale_handle = 0u64;

    for ordinal in 0..LAUNCHES {
        let request = manager
            .block_process_launch(
                owner_index,
                supervisor,
                USER_PROCESS_IMAGE_INIT,
                USER_PROCESS_MODE_HOLD,
            )
            .unwrap_or_else(|| fail("USER_GENERATION_STRESS_BLOCK_FAILED"));
        manager
            .complete_process_launch(request, Some(0x1400 + ordinal as u32))
            .unwrap_or_else(|_| fail("USER_GENERATION_STRESS_LAUNCH_FAILED"));
        let handle = manager.slots[owner_index]
            .as_ref()
            .map(|owner| owner.process.context.rax)
            .unwrap_or_else(|| fail("USER_GENERATION_STRESS_OWNER_MISSING"));
        let capability = manager.slots[owner_index]
            .as_ref()
            .and_then(|owner| owner.process_capability(handle))
            .unwrap_or_else(|| fail("USER_GENERATION_STRESS_HANDLE_FAILED"));
        let child = manager.slots[capability.target.slot as usize]
            .as_ref()
            .unwrap_or_else(|| fail("USER_GENERATION_STRESS_CHILD_MISSING"));
        let pid = child.process.pid as usize;
        pid_reused |= seen_pids[pid];
        seen_pids[pid] = true;
        if capability.target.incarnation <= previous_incarnation
            || handle == previous_handle
            || (first_stale_handle != 0
                && manager.slots[owner_index]
                    .as_ref()
                    .and_then(|owner| owner.process_capability(first_stale_handle))
                    .is_some())
        {
            fail("USER_GENERATION_STRESS_IDENTITY_CONFUSED");
        }
        previous_incarnation = capability.target.incarnation;
        previous_handle = handle;
        manager.complete_controlled_kill(owner_index, handle);
        manager.complete_controlled_reap(
            owner_index,
            handle,
            paging::USER_DATA + 0x200,
            core::mem::size_of::<UserProcessStatus>() as u64,
        );
        if ordinal == 0 {
            first_stale_handle = handle;
        }
        if manager.slots.iter().flatten().count() != 1
            || manager.slots[owner_index].as_ref().is_none_or(|owner| {
                owner.process_capability(handle).is_some() || !owner.handle_table_is_consistent()
            })
        {
            fail("USER_GENERATION_STRESS_REAP_LEAKED");
        }
    }
    if !pid_reused || first_stale_handle == 0 {
        fail("USER_GENERATION_STRESS_NO_PID_REUSE");
    }
    manager
        .kill(owner_task)
        .unwrap_or_else(|_| fail("USER_GENERATION_STRESS_CLEANUP_FAILED"));
    manager
        .wait(owner_task)
        .unwrap_or_else(|_| fail("USER_GENERATION_STRESS_REAP_FAILED"));
    if active_process_count() != active_before || manager.slots.iter().any(Option::is_some) {
        fail("USER_GENERATION_STRESS_FINAL_LEAK");
    }
    crate::serial::println("USER_PROCESS_GENERATION_STRESS_OK launches=257");
    crate::serial::println("USER_PID_REUSE_SAFE_OK");
    crate::serial::println("USER_STALE_PROCESS_HANDLE_REJECTED_OK");
}

fn run_supervisor_cleanup_case(state: ManagedState, owner_task: u32, child_task: u32) {
    let active_before = active_process_count();
    let reclaimed_before = reclaimed_frame_count();
    let mut manager = ProcessManager::new();
    manager
        .spawn_shell(owner_task)
        .unwrap_or_else(|_| fail("USER_SUPERVISOR_PROBE_SPAWN_FAILED"));
    let owner_index = manager
        .slots
        .iter()
        .position(|slot| {
            slot.as_ref()
                .is_some_and(|managed| managed.task_id == owner_task)
        })
        .unwrap_or_else(|| fail("USER_SUPERVISOR_PROBE_OWNER_MISSING"));
    let (owner_key, supervisor) = manager.slots[owner_index]
        .as_ref()
        .map(|managed| (managed.key, managed.process.lifecycle_handle))
        .unwrap_or_else(|| fail("USER_SUPERVISOR_PROBE_OWNER_MISSING"));
    let launch = manager
        .block_process_launch(
            owner_index,
            supervisor,
            USER_PROCESS_IMAGE_INIT,
            USER_PROCESS_MODE_HOLD,
        )
        .unwrap_or_else(|| fail("USER_SUPERVISOR_PROBE_LAUNCH_BLOCK_FAILED"));
    manager
        .complete_process_launch(launch, Some(child_task))
        .unwrap_or_else(|_| fail("USER_SUPERVISOR_PROBE_LAUNCH_FAILED"));
    if !manager
        .slots
        .iter()
        .flatten()
        .any(|managed| managed.task_id == child_task && managed.parent_key == Some(owner_key))
    {
        fail("USER_SUPERVISOR_PROBE_CHILD_MISSING");
    }
    let canceled = manager
        .block_process_launch(
            owner_index,
            supervisor,
            USER_PROCESS_IMAGE_INIT,
            USER_PROCESS_MODE_HOLD,
        )
        .map(UserLifecycleRequest::Launch)
        .unwrap_or_else(|| fail("USER_SUPERVISOR_PROBE_CANCEL_BLOCK_FAILED"));
    match state {
        ManagedState::Killed => {
            if !matches!(manager.kill(owner_task), Ok(update) if update.state == ManagedState::Killed)
            {
                fail("USER_SUPERVISOR_PROBE_KILL_FAILED");
            }
        }
        ManagedState::Exited | ManagedState::Faulted => {
            let managed = manager.slots[owner_index]
                .as_mut()
                .unwrap_or_else(|| fail("USER_SUPERVISOR_PROBE_OWNER_MISSING"));
            managed.process.completed = true;
            managed.process.event = if state == ManagedState::Faulted {
                ProcessEvent::Fault
            } else {
                ProcessEvent::Exit
            };
            managed.process.exit_code = if state == ManagedState::Faulted {
                142
            } else {
                0
            };
            managed.process.completion_order =
                COMPLETION_SEQUENCE.fetch_add(1, Ordering::AcqRel) + 1;
            manager.complete_terminal(owner_index, state);
        }
        _ => fail("USER_SUPERVISOR_PROBE_STATE_INVALID"),
    }
    let owner = manager.slots[owner_index]
        .as_ref()
        .unwrap_or_else(|| fail("USER_SUPERVISOR_PROBE_OWNER_MISSING"));
    let mut snapshot = TaskSnapshotSet::new();
    manager.append_task_snapshots(&mut snapshot);
    if owner.state != state
        || !owner.process.completed
        || !owner.resources_are_revoked()
        || manager.lifecycle_request_active(canceled)
        || manager
            .slots
            .iter()
            .flatten()
            .any(|managed| managed.parent_key == Some(owner_key))
        || manager.live_count() != 0
        || !manager.unified_handle_table_is_authoritative()
        || !manager.task_snapshots_match(&snapshot)
        || snapshot.class_len(TaskClass::User) != 1
        || snapshot
            .find(owner_task)
            .is_none_or(|task| task.state != managed_task_state(state))
        || reclaimed_frame_count() < reclaimed_before + 20
    {
        fail("USER_SUPERVISOR_PROBE_CLEANUP_FAILED");
    }
    let waited = manager
        .wait(owner_task)
        .unwrap_or_else(|_| fail("USER_SUPERVISOR_PROBE_REAP_FAILED"));
    let mut empty_snapshot = TaskSnapshotSet::new();
    manager.append_task_snapshots(&mut empty_snapshot);
    if waited.state != state
        || manager.slots.iter().any(Option::is_some)
        || empty_snapshot.class_len(TaskClass::User) != 0
        || active_process_count() != active_before
    {
        fail("USER_SUPERVISOR_PROBE_STALE_STATE_FAILED");
    }
    crate::serial::print("USER_SUPERVISOR_CLEANUP_OK mode=");
    crate::serial::println(match state {
        ManagedState::Exited => "exit",
        ManagedState::Faulted => "fault",
        ManagedState::Killed => "kill",
        _ => "invalid",
    });
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
    let mut executable_pages = 0u64;
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
        if executable {
            executable_pages = executable_pages
                .checked_add(pages)
                .ok_or(ProcessBuildError::InvalidLayout)?;
            if executable_pages > USER_EXECUTABLE_PAGE_CAPACITY {
                return Err(ProcessBuildError::InvalidLayout);
            }
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
        lifecycle_handle: 0,
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
                process_status_size: core::mem::size_of::<UserProcessStatus>() as u64,
                process_handle_capacity: USER_PROCESS_HANDLE_CAPACITY as u64,
                image_layout_version: USER_IMAGE_LAYOUT_VERSION,
                executable_page_capacity: USER_EXECUTABLE_PAGE_CAPACITY,
                socket_handle_capacity: USER_SOCKET_HANDLE_CAPACITY,
                socket_buffer_capacity: USER_SOCKET_BUFFER_CAPACITY,
                socket_status_size: core::mem::size_of::<UserSocketStatus>() as u64,
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
        Ok(SyscallAction::TruncateHandle { handle }) => {
            frame.rax = 0;
            process.context = *frame;
            process.event = ProcessEvent::TruncateHandle { handle };
            1
        }
        Ok(SyscallAction::CreateDirectory {
            parent,
            name_address,
            name_length,
        }) => {
            if let Some(name) = copy_user_name(process, name_address, name_length) {
                frame.rax = 0;
                process.context = *frame;
                process.event = ProcessEvent::CreateDirectory { parent, name };
                1
            } else {
                frame.rax = syscall::error_code(syscall::SyscallError::InvalidArgument);
                0
            }
        }
        Ok(SyscallAction::RemovePath {
            parent,
            name_address,
            name_length,
        }) => {
            if let Some(name) = copy_user_name(process, name_address, name_length) {
                frame.rax = 0;
                process.context = *frame;
                process.event = ProcessEvent::RemovePath { parent, name };
                1
            } else {
                frame.rax = syscall::error_code(syscall::SyscallError::InvalidArgument);
                0
            }
        }
        Ok(SyscallAction::NetworkConfig {
            output_address,
            output_length,
        }) => {
            let result = network::config().filter(|_| {
                output_length as usize == core::mem::size_of::<UserNetworkConfig>()
                    && valid_user_data_buffer(process, output_address, output_length)
            });
            frame.rax = result
                .filter(|config| {
                    let bytes = unsafe {
                        core::slice::from_raw_parts(
                            core::ptr::from_ref(config).cast::<u8>(),
                            core::mem::size_of::<UserNetworkConfig>(),
                        )
                    };
                    copy_to_user_data(process, output_address, bytes)
                })
                .map(|_| output_length)
                .unwrap_or_else(|| syscall::error_code(syscall::SyscallError::Unavailable));
            0
        }
        Ok(SyscallAction::UdpExchange {
            target,
            port,
            input_address,
            input_length,
            output_address,
            output_capacity,
        }) => {
            frame.rax = complete_network_exchange(
                process,
                false,
                target,
                port,
                input_address,
                input_length,
                output_address,
                output_capacity,
            );
            0
        }
        Ok(SyscallAction::TcpExchange {
            target,
            port,
            input_address,
            input_length,
            output_address,
            output_capacity,
        }) => {
            frame.rax = complete_network_exchange(
                process,
                true,
                target,
                port,
                input_address,
                input_length,
                output_address,
                output_capacity,
            );
            0
        }
        Ok(SyscallAction::SocketOpen { protocol }) => {
            frame.rax = 0;
            process.context = *frame;
            process.event = ProcessEvent::SocketOpen { protocol };
            1
        }
        Ok(SyscallAction::SocketConnect {
            handle,
            target,
            port,
        }) => {
            frame.rax = 0;
            process.context = *frame;
            process.event = ProcessEvent::SocketConnect {
                handle,
                target,
                port,
            };
            1
        }
        Ok(SyscallAction::SocketBind { handle, port }) => {
            frame.rax = 0;
            process.context = *frame;
            process.event = ProcessEvent::SocketBind { handle, port };
            1
        }
        Ok(SyscallAction::SocketListen { handle, backlog }) => {
            frame.rax = 0;
            process.context = *frame;
            process.event = ProcessEvent::SocketListen { handle, backlog };
            1
        }
        Ok(SyscallAction::SocketAccept { handle }) => {
            frame.rax = 0;
            process.context = *frame;
            process.event = ProcessEvent::SocketAccept { handle };
            1
        }
        Ok(SyscallAction::SocketSend {
            handle,
            input_address,
            input_length,
        }) => {
            if let Some(data) = copy_user_bytes(process, input_address, input_length) {
                frame.rax = 0;
                process.context = *frame;
                process.event = ProcessEvent::SocketSend { handle, data };
                1
            } else {
                frame.rax = syscall::error_code(syscall::SyscallError::InvalidArgument);
                0
            }
        }
        Ok(SyscallAction::SocketReceive {
            handle,
            output_address,
            output_capacity,
        }) => {
            if valid_user_data_buffer(process, output_address, output_capacity) {
                frame.rax = 0;
                process.context = *frame;
                process.event = ProcessEvent::SocketReceive {
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
        Ok(SyscallAction::SocketStatus {
            handle,
            output_address,
            output_length,
        }) => {
            if valid_user_data_buffer(process, output_address, output_length) {
                frame.rax = 0;
                process.context = *frame;
                process.event = ProcessEvent::SocketStatus {
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
        Ok(SyscallAction::SocketShutdown { handle, direction }) => {
            frame.rax = 0;
            process.context = *frame;
            process.event = ProcessEvent::SocketShutdown { handle, direction };
            1
        }
        Ok(SyscallAction::SocketClose { handle }) => {
            frame.rax = 0;
            process.context = *frame;
            process.event = ProcessEvent::SocketClose(handle);
            1
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
                    process.event = ProcessEvent::ConsoleWrite { handle, text, kind };
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
                    process.event = ProcessEvent::ConsoleSetInput { handle, text };
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
                process.event = ProcessEvent::ConsoleClear(handle);
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
        Ok(SyscallAction::ProcessLaunch {
            supervisor,
            image,
            mode,
        }) => {
            if syscall::lifecycle_capability_valid(process.lifecycle_handle, supervisor) {
                frame.rax = 0;
                process.context = *frame;
                process.event = ProcessEvent::ProcessLaunch {
                    supervisor,
                    image,
                    mode,
                };
                1
            } else {
                frame.rax = syscall::error_code(syscall::SyscallError::InvalidArgument);
                0
            }
        }
        Ok(SyscallAction::ProcessStatus {
            handle,
            output_address,
            output_length,
        }) => {
            if valid_user_data_buffer(process, output_address, output_length) {
                frame.rax = 0;
                process.context = *frame;
                process.event = ProcessEvent::ProcessStatus {
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
        Ok(SyscallAction::ProcessKill { handle }) => {
            frame.rax = 0;
            process.context = *frame;
            process.event = ProcessEvent::ProcessKill { handle };
            1
        }
        Ok(SyscallAction::ProcessReap {
            handle,
            output_address,
            output_length,
        }) => {
            if valid_user_data_buffer(process, output_address, output_length) {
                frame.rax = 0;
                process.context = *frame;
                process.event = ProcessEvent::ProcessReap {
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

#[allow(clippy::too_many_arguments)]
fn complete_network_exchange(
    process: &UserProcess,
    tcp: bool,
    target: u32,
    port: u16,
    input_address: u64,
    input_length: u64,
    output_address: u64,
    output_capacity: u64,
) -> u64 {
    let Some(input) = copy_user_bytes(process, input_address, input_length) else {
        return syscall::error_code(syscall::SyscallError::InvalidArgument);
    };
    if !valid_user_data_buffer(process, output_address, output_capacity) {
        return syscall::error_code(syscall::SyscallError::InvalidArgument);
    }
    let mut output = [0u8; USER_FILE_READ_MAX];
    let capacity = (output_capacity as usize).min(output.len());
    let result = if tcp {
        network::tcp_exchange(target, port, input.as_slice(), &mut output[..capacity])
    } else {
        network::udp_exchange(target, port, input.as_slice(), &mut output[..capacity])
    };
    result
        .filter(|length| copy_to_user_data(process, output_address, &output[..*length]))
        .map(|length| length as u64)
        .unwrap_or_else(|| syscall::error_code(syscall::SyscallError::Unavailable))
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

fn copy_user_name(process: &UserProcess, address: u64, length: u64) -> Option<FixedText> {
    let name = copy_user_text(process, address, length)?;
    if name.as_str() == "."
        || name.as_str() == ".."
        || !name
            .as_str()
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return None;
    }
    Some(name)
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

    #[test]
    fn namespace_children_stay_beneath_the_owned_directory() {
        let parent = FixedText::from_str("/USER/PROJECTS");
        assert_eq!(
            join_child_path(parent, FixedText::from_str("GENOS"))
                .expect("valid child")
                .as_str(),
            "/USER/PROJECTS/GENOS"
        );
        assert!(join_child_path(parent, FixedText::from_str("..")).is_none());
        assert!(join_child_path(parent, FixedText::from_str("nested/name")).is_none());
        assert!(paths_equal("/USER/NOTE.TXT", "/user/note.txt"));
        assert!(is_user_writable_path("/user/note.txt"));
        assert!(is_user_writable_directory("/user"));
        assert!(join_child_path(
            FixedText::from_str("/USER/ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789ABCDEFGHIJKLMNOPQRST"),
            FixedText::from_str("TOO-LONG")
        )
        .is_none());
    }
}
