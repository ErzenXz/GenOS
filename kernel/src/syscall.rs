pub use genos_abi::{
    USER_ABI_VERSION, USER_PING_REPLY as PING_REPLY,
    USER_SYSCALL_ABI_VERSION as SYSCALL_ABI_VERSION,
    USER_SYSCALL_CLOSE_ENDPOINT as SYSCALL_CLOSE_ENDPOINT,
    USER_SYSCALL_CLOSE_HANDLE as SYSCALL_CLOSE_HANDLE,
    USER_SYSCALL_CONNECT_ENDPOINT as SYSCALL_CONNECT_ENDPOINT,
    USER_SYSCALL_CONSOLE_CLEAR as SYSCALL_CONSOLE_CLEAR,
    USER_SYSCALL_CONSOLE_SET_INPUT as SYSCALL_CONSOLE_SET_INPUT,
    USER_SYSCALL_CONSOLE_WRITE as SYSCALL_CONSOLE_WRITE,
    USER_SYSCALL_CREATE_DIRECTORY as SYSCALL_CREATE_DIRECTORY,
    USER_SYSCALL_CREATE_ENDPOINT as SYSCALL_CREATE_ENDPOINT, USER_SYSCALL_EXIT as SYSCALL_EXIT,
    USER_SYSCALL_NETWORK_CONFIG as SYSCALL_NETWORK_CONFIG,
    USER_SYSCALL_OPEN_FILE as SYSCALL_OPEN_FILE,
    USER_SYSCALL_OPEN_FILE_WITH_RIGHTS as SYSCALL_OPEN_FILE_WITH_RIGHTS,
    USER_SYSCALL_PING as SYSCALL_PING, USER_SYSCALL_PROCESS_KILL as SYSCALL_PROCESS_KILL,
    USER_SYSCALL_PROCESS_LAUNCH as SYSCALL_PROCESS_LAUNCH,
    USER_SYSCALL_PROCESS_REAP as SYSCALL_PROCESS_REAP,
    USER_SYSCALL_PROCESS_STATUS as SYSCALL_PROCESS_STATUS,
    USER_SYSCALL_READ_DIRECTORY as SYSCALL_READ_DIRECTORY,
    USER_SYSCALL_READ_FILE as SYSCALL_READ_FILE, USER_SYSCALL_READ_HANDLE as SYSCALL_READ_HANDLE,
    USER_SYSCALL_RECEIVE as SYSCALL_RECEIVE,
    USER_SYSCALL_RECEIVE_ENDPOINT as SYSCALL_RECEIVE_ENDPOINT,
    USER_SYSCALL_REMOVE_PATH as SYSCALL_REMOVE_PATH, USER_SYSCALL_REPORT as SYSCALL_REPORT,
    USER_SYSCALL_SEND as SYSCALL_SEND, USER_SYSCALL_SEND_ENDPOINT as SYSCALL_SEND_ENDPOINT,
    USER_SYSCALL_SLEEP as SYSCALL_SLEEP, USER_SYSCALL_SOCKET_ACCEPT as SYSCALL_SOCKET_ACCEPT,
    USER_SYSCALL_SOCKET_BIND as SYSCALL_SOCKET_BIND,
    USER_SYSCALL_SOCKET_CLOSE as SYSCALL_SOCKET_CLOSE,
    USER_SYSCALL_SOCKET_CONNECT as SYSCALL_SOCKET_CONNECT,
    USER_SYSCALL_SOCKET_LISTEN as SYSCALL_SOCKET_LISTEN,
    USER_SYSCALL_SOCKET_OPEN as SYSCALL_SOCKET_OPEN,
    USER_SYSCALL_SOCKET_RECEIVE as SYSCALL_SOCKET_RECEIVE,
    USER_SYSCALL_SOCKET_SEND as SYSCALL_SOCKET_SEND,
    USER_SYSCALL_SOCKET_SHUTDOWN as SYSCALL_SOCKET_SHUTDOWN,
    USER_SYSCALL_SOCKET_STATUS as SYSCALL_SOCKET_STATUS,
    USER_SYSCALL_STAT_HANDLE as SYSCALL_STAT_HANDLE,
    USER_SYSCALL_SYSTEM_INFO as SYSCALL_SYSTEM_INFO,
    USER_SYSCALL_TCP_EXCHANGE as SYSCALL_TCP_EXCHANGE,
    USER_SYSCALL_TRUNCATE_HANDLE as SYSCALL_TRUNCATE_HANDLE,
    USER_SYSCALL_UDP_EXCHANGE as SYSCALL_UDP_EXCHANGE,
    USER_SYSCALL_WAIT_CHILD as SYSCALL_WAIT_CHILD, USER_SYSCALL_WAIT_INPUT as SYSCALL_WAIT_INPUT,
    USER_SYSCALL_WRITE as SYSCALL_WRITE, USER_SYSCALL_WRITE_HANDLE as SYSCALL_WRITE_HANDLE,
    USER_SYSCALL_YIELD as SYSCALL_YIELD,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SyscallAction {
    Return(u64),
    Exit(u8),
    Yield,
    Report {
        address: u64,
        length: u64,
    },
    Write {
        address: u64,
        length: u64,
    },
    Sleep {
        ticks: u64,
    },
    /// Legacy direct-PID send, unreachable from ABI 9 onwards.
    Send {
        pid: u8,
        value: u64,
    },
    /// Legacy inbox receive, unreachable from ABI 9 onwards.
    Receive,
    WaitChild {
        pid: u8,
    },
    SystemInfo {
        address: u64,
        length: u64,
    },
    ReadFile {
        path_address: u64,
        path_length: u64,
        output_address: u64,
        output_capacity: u64,
    },
    OpenFile {
        path_address: u64,
        path_length: u64,
    },
    ReadHandle {
        handle: u64,
        output_address: u64,
        output_capacity: u64,
    },
    StatHandle {
        handle: u64,
        output_address: u64,
        output_length: u64,
    },
    CloseHandle {
        handle: u64,
    },
    OpenFileWithRights {
        path_address: u64,
        path_length: u64,
        rights: u64,
    },
    WriteHandle {
        handle: u64,
        input_address: u64,
        input_length: u64,
    },
    TruncateHandle {
        handle: u64,
    },
    WaitInput {
        output_address: u64,
        output_length: u64,
        mask: u64,
    },
    CreateEndpoint,
    ConnectEndpoint {
        pid: u8,
    },
    SendEndpoint {
        handle: u64,
        value: u64,
    },
    ReceiveEndpoint {
        handle: u64,
        output_address: u64,
        output_length: u64,
    },
    CloseEndpoint {
        handle: u64,
    },
    ConsoleWrite {
        handle: u64,
        address: u64,
        length: u64,
        kind: u64,
    },
    ConsoleSetInput {
        handle: u64,
        address: u64,
        length: u64,
    },
    ConsoleClear {
        handle: u64,
    },
    ReadDirectory {
        handle: u64,
        cursor: u64,
        output_address: u64,
        output_length: u64,
    },
    ProcessLaunch {
        supervisor: u64,
        image: u64,
        mode: u64,
    },
    ProcessStatus {
        handle: u64,
        output_address: u64,
        output_length: u64,
    },
    ProcessKill {
        handle: u64,
    },
    ProcessReap {
        handle: u64,
        output_address: u64,
        output_length: u64,
    },
    CreateDirectory {
        parent: u64,
        name_address: u64,
        name_length: u64,
    },
    RemovePath {
        parent: u64,
        name_address: u64,
        name_length: u64,
    },
    NetworkConfig {
        output_address: u64,
        output_length: u64,
    },
    UdpExchange {
        target: u32,
        port: u16,
        input_address: u64,
        input_length: u64,
        output_address: u64,
        output_capacity: u64,
    },
    TcpExchange {
        target: u32,
        port: u16,
        input_address: u64,
        input_length: u64,
        output_address: u64,
        output_capacity: u64,
    },
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
        input_address: u64,
        input_length: u64,
    },
    SocketReceive {
        handle: u64,
        output_address: u64,
        output_capacity: u64,
    },
    SocketStatus {
        handle: u64,
        output_address: u64,
        output_length: u64,
    },
    SocketShutdown {
        handle: u64,
        direction: u64,
    },
    SocketClose {
        handle: u64,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SyscallError {
    UnknownNumber,
    InvalidArgument,
    Unavailable,
}

const CONSOLE_HANDLE_TAG: u64 = 0xc1 << 56;
const LIFECYCLE_HANDLE_TAG: u64 = 0xc2 << 56;

pub const fn console_handle(pid: u8, generation: u64) -> u64 {
    CONSOLE_HANDLE_TAG | ((pid as u64) << 48) | (generation & 0x0000_ffff_ffff_ffff)
}

pub const fn console_capability_valid(owned: u64, presented: u64) -> bool {
    owned != 0 && owned == presented && presented & (0xff << 56) == CONSOLE_HANDLE_TAG
}

pub const fn lifecycle_handle(generation: u64) -> u64 {
    LIFECYCLE_HANDLE_TAG | (generation & 0x00ff_ffff_ffff_ffff)
}

pub const fn lifecycle_capability_valid(owned: u64, presented: u64) -> bool {
    owned != 0 && owned == presented && presented & (0xff << 56) == LIFECYCLE_HANDLE_TAG
}

pub fn dispatch(number: u64, args: [u64; 6]) -> Result<SyscallAction, SyscallError> {
    match number {
        SYSCALL_PING if args == [0; 6] => Ok(SyscallAction::Return(PING_REPLY)),
        SYSCALL_ABI_VERSION if args == [0; 6] => Ok(SyscallAction::Return(USER_ABI_VERSION)),
        SYSCALL_EXIT if args[0] <= u8::MAX as u64 && args[1..] == [0; 5] => {
            Ok(SyscallAction::Exit(args[0] as u8))
        }
        SYSCALL_YIELD if args == [0; 6] => Ok(SyscallAction::Yield),
        SYSCALL_REPORT if args[0] != 0 && args[1] == 8 && args[2..] == [0; 4] => {
            Ok(SyscallAction::Report {
                address: args[0],
                length: args[1],
            })
        }
        SYSCALL_WRITE if args[0] != 0 && (1..=80).contains(&args[1]) && args[2..] == [0; 4] => {
            Ok(SyscallAction::Write {
                address: args[0],
                length: args[1],
            })
        }
        SYSCALL_SLEEP if (1..=10_000).contains(&args[0]) && args[1..] == [0; 5] => {
            Ok(SyscallAction::Sleep { ticks: args[0] })
        }
        SYSCALL_WAIT_CHILD if (1..=u8::MAX as u64).contains(&args[0]) && args[1..] == [0; 5] => {
            Ok(SyscallAction::WaitChild { pid: args[0] as u8 })
        }
        SYSCALL_SYSTEM_INFO
            if args[0] != 0
                && args[1] == core::mem::size_of::<genos_abi::UserSystemInfo>() as u64
                && args[2..] == [0; 4] =>
        {
            Ok(SyscallAction::SystemInfo {
                address: args[0],
                length: args[1],
            })
        }
        SYSCALL_READ_FILE
            if args[0] != 0
                && (1..=64).contains(&args[1])
                && args[2] != 0
                && (1..=genos_abi::USER_FILE_READ_MAX as u64).contains(&args[3])
                && args[4..] == [0; 2] =>
        {
            Ok(SyscallAction::ReadFile {
                path_address: args[0],
                path_length: args[1],
                output_address: args[2],
                output_capacity: args[3],
            })
        }
        SYSCALL_OPEN_FILE if args[0] != 0 && (1..=64).contains(&args[1]) && args[2..] == [0; 4] => {
            Ok(SyscallAction::OpenFile {
                path_address: args[0],
                path_length: args[1],
            })
        }
        SYSCALL_READ_HANDLE
            if args[0] != 0
                && args[1] != 0
                && (1..=genos_abi::USER_FILE_READ_MAX as u64).contains(&args[2])
                && args[3..] == [0; 3] =>
        {
            Ok(SyscallAction::ReadHandle {
                handle: args[0],
                output_address: args[1],
                output_capacity: args[2],
            })
        }
        SYSCALL_STAT_HANDLE
            if args[0] != 0
                && args[1] != 0
                && args[2] == core::mem::size_of::<genos_abi::UserFileStat>() as u64
                && args[3..] == [0; 3] =>
        {
            Ok(SyscallAction::StatHandle {
                handle: args[0],
                output_address: args[1],
                output_length: args[2],
            })
        }
        SYSCALL_CLOSE_HANDLE if args[0] != 0 && args[1..] == [0; 5] => {
            Ok(SyscallAction::CloseHandle { handle: args[0] })
        }
        SYSCALL_OPEN_FILE_WITH_RIGHTS
            if args[0] != 0
                && (1..=64).contains(&args[1])
                && args[2] != 0
                && args[2] & !genos_abi::USER_FILE_RIGHTS_MASK == 0
                && args[3..] == [0; 3] =>
        {
            Ok(SyscallAction::OpenFileWithRights {
                path_address: args[0],
                path_length: args[1],
                rights: args[2],
            })
        }
        SYSCALL_WRITE_HANDLE
            if args[0] != 0
                && args[1] != 0
                && (1..=genos_abi::USER_FILE_WRITE_MAX as u64).contains(&args[2])
                && args[3..] == [0; 3] =>
        {
            Ok(SyscallAction::WriteHandle {
                handle: args[0],
                input_address: args[1],
                input_length: args[2],
            })
        }
        SYSCALL_WAIT_INPUT
            if args[0] != 0
                && args[1] == core::mem::size_of::<genos_abi::UserInputEvent>() as u64
                && args[2] != 0
                && args[2] & !genos_abi::USER_INPUT_MASK_ALL == 0
                && args[3..] == [0; 3] =>
        {
            Ok(SyscallAction::WaitInput {
                output_address: args[0],
                output_length: args[1],
                mask: args[2],
            })
        }
        SYSCALL_CREATE_ENDPOINT if args == [0; 6] => Ok(SyscallAction::CreateEndpoint),
        SYSCALL_CONNECT_ENDPOINT
            if (1..=u8::MAX as u64).contains(&args[0]) && args[1..] == [0; 5] =>
        {
            Ok(SyscallAction::ConnectEndpoint { pid: args[0] as u8 })
        }
        SYSCALL_SEND_ENDPOINT if args[0] != 0 && args[2..] == [0; 4] => {
            Ok(SyscallAction::SendEndpoint {
                handle: args[0],
                value: args[1],
            })
        }
        SYSCALL_RECEIVE_ENDPOINT
            if args[0] != 0
                && args[1] != 0
                && args[2] == core::mem::size_of::<genos_abi::UserChannelMessage>() as u64
                && args[3..] == [0; 3] =>
        {
            Ok(SyscallAction::ReceiveEndpoint {
                handle: args[0],
                output_address: args[1],
                output_length: args[2],
            })
        }
        SYSCALL_CLOSE_ENDPOINT if args[0] != 0 && args[1..] == [0; 5] => {
            Ok(SyscallAction::CloseEndpoint { handle: args[0] })
        }
        SYSCALL_CONSOLE_WRITE
            if args[0] != 0
                && args[1] != 0
                && (1..=genos_abi::USER_CONSOLE_TEXT_MAX as u64).contains(&args[2])
                && args[3] <= genos_abi::USER_CONSOLE_LINE_STATUS
                && args[4..] == [0; 2] =>
        {
            Ok(SyscallAction::ConsoleWrite {
                handle: args[0],
                address: args[1],
                length: args[2],
                kind: args[3],
            })
        }
        SYSCALL_CONSOLE_SET_INPUT
            if args[0] != 0
                && args[2] <= genos_abi::USER_CONSOLE_TEXT_MAX as u64
                && (args[2] == 0 || args[1] != 0)
                && args[3..] == [0; 3] =>
        {
            Ok(SyscallAction::ConsoleSetInput {
                handle: args[0],
                address: args[1],
                length: args[2],
            })
        }
        SYSCALL_CONSOLE_CLEAR if args[0] != 0 && args[1..] == [0; 5] => {
            Ok(SyscallAction::ConsoleClear { handle: args[0] })
        }
        SYSCALL_READ_DIRECTORY
            if args[0] != 0
                && args[2] != 0
                && args[3] == core::mem::size_of::<genos_abi::UserDirectoryEntry>() as u64
                && args[4..] == [0; 2] =>
        {
            Ok(SyscallAction::ReadDirectory {
                handle: args[0],
                cursor: args[1],
                output_address: args[2],
                output_length: args[3],
            })
        }
        SYSCALL_TRUNCATE_HANDLE if args[0] != 0 && args[1..] == [0; 5] => {
            Ok(SyscallAction::TruncateHandle { handle: args[0] })
        }
        SYSCALL_PROCESS_LAUNCH
            if args[0] != 0
                && args[1] == genos_abi::USER_PROCESS_IMAGE_INIT
                && matches!(
                    args[2],
                    genos_abi::USER_PROCESS_MODE_NORMAL | genos_abi::USER_PROCESS_MODE_HOLD
                )
                && args[3..] == [0; 3] =>
        {
            Ok(SyscallAction::ProcessLaunch {
                supervisor: args[0],
                image: args[1],
                mode: args[2],
            })
        }
        SYSCALL_PROCESS_STATUS
            if args[0] != 0
                && args[1] != 0
                && args[2] == core::mem::size_of::<genos_abi::UserProcessStatus>() as u64
                && args[3..] == [0; 3] =>
        {
            Ok(SyscallAction::ProcessStatus {
                handle: args[0],
                output_address: args[1],
                output_length: args[2],
            })
        }
        SYSCALL_PROCESS_KILL if args[0] != 0 && args[1..] == [0; 5] => {
            Ok(SyscallAction::ProcessKill { handle: args[0] })
        }
        SYSCALL_PROCESS_REAP
            if args[0] != 0
                && args[1] != 0
                && args[2] == core::mem::size_of::<genos_abi::UserProcessStatus>() as u64
                && args[3..] == [0; 3] =>
        {
            Ok(SyscallAction::ProcessReap {
                handle: args[0],
                output_address: args[1],
                output_length: args[2],
            })
        }
        SYSCALL_CREATE_DIRECTORY
            if args[0] != 0
                && args[1] != 0
                && (1..=genos_abi::USER_DIRECTORY_NAME_MAX as u64).contains(&args[2])
                && args[3..] == [0; 3] =>
        {
            Ok(SyscallAction::CreateDirectory {
                parent: args[0],
                name_address: args[1],
                name_length: args[2],
            })
        }
        SYSCALL_REMOVE_PATH
            if args[0] != 0
                && args[1] != 0
                && (1..=genos_abi::USER_DIRECTORY_NAME_MAX as u64).contains(&args[2])
                && args[3..] == [0; 3] =>
        {
            Ok(SyscallAction::RemovePath {
                parent: args[0],
                name_address: args[1],
                name_length: args[2],
            })
        }
        SYSCALL_NETWORK_CONFIG
            if args[0] != 0
                && args[1] == core::mem::size_of::<genos_abi::UserNetworkConfig>() as u64
                && args[2..] == [0; 4] =>
        {
            Ok(SyscallAction::NetworkConfig {
                output_address: args[0],
                output_length: args[1],
            })
        }
        SYSCALL_UDP_EXCHANGE | SYSCALL_TCP_EXCHANGE
            if args[0] <= u32::MAX as u64
                && (1..=u16::MAX as u64).contains(&args[1])
                && args[2] != 0
                && (1..=genos_abi::USER_FILE_WRITE_MAX as u64).contains(&args[3])
                && args[4] != 0
                && (1..=genos_abi::USER_FILE_READ_MAX as u64).contains(&args[5]) =>
        {
            let action = if number == SYSCALL_UDP_EXCHANGE {
                SyscallAction::UdpExchange {
                    target: args[0] as u32,
                    port: args[1] as u16,
                    input_address: args[2],
                    input_length: args[3],
                    output_address: args[4],
                    output_capacity: args[5],
                }
            } else {
                SyscallAction::TcpExchange {
                    target: args[0] as u32,
                    port: args[1] as u16,
                    input_address: args[2],
                    input_length: args[3],
                    output_address: args[4],
                    output_capacity: args[5],
                }
            };
            Ok(action)
        }
        SYSCALL_SOCKET_OPEN
            if matches!(
                args[0],
                genos_abi::USER_SOCKET_PROTOCOL_UDP | genos_abi::USER_SOCKET_PROTOCOL_TCP_STREAM
            ) && args[1..] == [0; 5] =>
        {
            Ok(SyscallAction::SocketOpen { protocol: args[0] })
        }
        SYSCALL_SOCKET_CONNECT
            if args[0] != 0
                && args[1] <= u32::MAX as u64
                && args[1] != 0
                && (1..=u16::MAX as u64).contains(&args[2])
                && args[3..] == [0; 3] =>
        {
            Ok(SyscallAction::SocketConnect {
                handle: args[0],
                target: args[1] as u32,
                port: args[2] as u16,
            })
        }
        SYSCALL_SOCKET_BIND
            if args[0] != 0
                && (genos_abi::USER_SOCKET_LISTENER_PORT_MIN..=u16::MAX as u64)
                    .contains(&args[1])
                && args[2..] == [0; 4] =>
        {
            Ok(SyscallAction::SocketBind {
                handle: args[0],
                port: args[1] as u16,
            })
        }
        SYSCALL_SOCKET_LISTEN
            if args[0] != 0
                && (1..=genos_abi::USER_SOCKET_LISTENER_BACKLOG_CAPACITY).contains(&args[1])
                && args[2..] == [0; 4] =>
        {
            Ok(SyscallAction::SocketListen {
                handle: args[0],
                backlog: args[1],
            })
        }
        SYSCALL_SOCKET_ACCEPT if args[0] != 0 && args[1..] == [0; 5] => {
            Ok(SyscallAction::SocketAccept { handle: args[0] })
        }
        SYSCALL_SOCKET_SEND
            if args[0] != 0
                && args[1] != 0
                && (1..=genos_abi::USER_SOCKET_BUFFER_CAPACITY).contains(&args[2])
                && args[3..] == [0; 3] =>
        {
            Ok(SyscallAction::SocketSend {
                handle: args[0],
                input_address: args[1],
                input_length: args[2],
            })
        }
        SYSCALL_SOCKET_RECEIVE
            if args[0] != 0
                && args[1] != 0
                && (1..=genos_abi::USER_SOCKET_BUFFER_CAPACITY).contains(&args[2])
                && args[3..] == [0; 3] =>
        {
            Ok(SyscallAction::SocketReceive {
                handle: args[0],
                output_address: args[1],
                output_capacity: args[2],
            })
        }
        SYSCALL_SOCKET_STATUS
            if args[0] != 0
                && args[1] != 0
                && args[2] == core::mem::size_of::<genos_abi::UserSocketStatus>() as u64
                && args[3..] == [0; 3] =>
        {
            Ok(SyscallAction::SocketStatus {
                handle: args[0],
                output_address: args[1],
                output_length: args[2],
            })
        }
        SYSCALL_SOCKET_SHUTDOWN
            if args[0] != 0
                && matches!(
                    args[1],
                    genos_abi::USER_SOCKET_SHUTDOWN_READ
                        | genos_abi::USER_SOCKET_SHUTDOWN_WRITE
                        | genos_abi::USER_SOCKET_SHUTDOWN_BOTH
                )
                && args[2..] == [0; 4] =>
        {
            Ok(SyscallAction::SocketShutdown {
                handle: args[0],
                direction: args[1],
            })
        }
        SYSCALL_SOCKET_CLOSE if args[0] != 0 && args[1..] == [0; 5] => {
            Ok(SyscallAction::SocketClose { handle: args[0] })
        }
        SYSCALL_PING
        | SYSCALL_ABI_VERSION
        | SYSCALL_EXIT
        | SYSCALL_YIELD
        | SYSCALL_REPORT
        | SYSCALL_WRITE
        | SYSCALL_SLEEP
        | SYSCALL_WAIT_CHILD
        | SYSCALL_SYSTEM_INFO
        | SYSCALL_READ_FILE
        | SYSCALL_OPEN_FILE
        | SYSCALL_READ_HANDLE
        | SYSCALL_STAT_HANDLE
        | SYSCALL_CLOSE_HANDLE
        | SYSCALL_OPEN_FILE_WITH_RIGHTS
        | SYSCALL_WRITE_HANDLE
        | SYSCALL_WAIT_INPUT
        | SYSCALL_CREATE_ENDPOINT
        | SYSCALL_CONNECT_ENDPOINT
        | SYSCALL_SEND_ENDPOINT
        | SYSCALL_RECEIVE_ENDPOINT
        | SYSCALL_CLOSE_ENDPOINT
        | SYSCALL_CONSOLE_WRITE
        | SYSCALL_CONSOLE_SET_INPUT
        | SYSCALL_CONSOLE_CLEAR
        | SYSCALL_READ_DIRECTORY
        | SYSCALL_TRUNCATE_HANDLE
        | SYSCALL_PROCESS_LAUNCH
        | SYSCALL_PROCESS_STATUS
        | SYSCALL_PROCESS_KILL
        | SYSCALL_PROCESS_REAP
        | SYSCALL_CREATE_DIRECTORY
        | SYSCALL_REMOVE_PATH
        | SYSCALL_NETWORK_CONFIG
        | SYSCALL_UDP_EXCHANGE
        | SYSCALL_TCP_EXCHANGE
        | SYSCALL_SOCKET_OPEN
        | SYSCALL_SOCKET_CONNECT
        | SYSCALL_SOCKET_BIND
        | SYSCALL_SOCKET_LISTEN
        | SYSCALL_SOCKET_ACCEPT
        | SYSCALL_SOCKET_SEND
        | SYSCALL_SOCKET_RECEIVE
        | SYSCALL_SOCKET_STATUS
        | SYSCALL_SOCKET_SHUTDOWN
        | SYSCALL_SOCKET_CLOSE => Err(SyscallError::InvalidArgument),
        // `SYSCALL_SEND` and `SYSCALL_RECEIVE` stay reserved but unimplemented.
        _ => Err(SyscallError::UnknownNumber),
    }
}

pub const fn error_code(error: SyscallError) -> u64 {
    match error {
        SyscallError::UnknownNumber => genos_abi::USER_ERROR_UNKNOWN_SYSCALL,
        SyscallError::InvalidArgument => genos_abi::USER_ERROR_INVALID_ARGUMENT,
        SyscallError::Unavailable => genos_abi::USER_ERROR_UNAVAILABLE,
    }
}

pub fn validate_user_buffer(address: u64, length: u64, range_start: u64, range_size: u64) -> bool {
    if length == 0 || address < range_start {
        return false;
    }
    let Some(end) = address.checked_add(length) else {
        return false;
    };
    let Some(range_end) = range_start.checked_add(range_size) else {
        return false;
    };
    end <= range_end
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn console_capabilities_are_tagged_exact_and_process_local() {
        let owned = console_handle(16, 7);
        assert!(console_capability_valid(owned, owned));
        assert!(!console_capability_valid(0, 0));
        assert!(!console_capability_valid(owned, console_handle(17, 7)));
        assert!(!console_capability_valid(owned, console_handle(16, 8)));
        assert!(!console_capability_valid(owned, owned & !(0xff << 56)));
    }

    #[test]
    fn lifecycle_capabilities_are_tagged_and_exact() {
        let owned = lifecycle_handle(19);
        assert!(lifecycle_capability_valid(owned, owned));
        assert!(!lifecycle_capability_valid(0, 0));
        assert!(!lifecycle_capability_valid(owned, lifecycle_handle(20)));
        assert!(!lifecycle_capability_valid(owned, owned & !(0xff << 56)));
    }

    #[test]
    fn known_calls_have_stable_results() {
        assert_eq!(
            dispatch(SYSCALL_PING, [0; 6]),
            Ok(SyscallAction::Return(PING_REPLY))
        );
        assert_eq!(
            dispatch(SYSCALL_ABI_VERSION, [0; 6]),
            Ok(SyscallAction::Return(USER_ABI_VERSION))
        );
        assert_eq!(
            dispatch(SYSCALL_EXIT, [7, 0, 0, 0, 0, 0]),
            Ok(SyscallAction::Exit(7))
        );
        assert_eq!(dispatch(SYSCALL_YIELD, [0; 6]), Ok(SyscallAction::Yield));
        assert_eq!(
            dispatch(SYSCALL_REPORT, [0x4000, 8, 0, 0, 0, 0]),
            Ok(SyscallAction::Report {
                address: 0x4000,
                length: 8
            })
        );
        assert_eq!(
            dispatch(SYSCALL_WRITE, [0x5000, 12, 0, 0, 0, 0]),
            Ok(SyscallAction::Write {
                address: 0x5000,
                length: 12
            })
        );
        assert_eq!(
            dispatch(SYSCALL_SLEEP, [25, 0, 0, 0, 0, 0]),
            Ok(SyscallAction::Sleep { ticks: 25 })
        );
        assert_eq!(
            dispatch(SYSCALL_WAIT_CHILD, [8, 0, 0, 0, 0, 0]),
            Ok(SyscallAction::WaitChild { pid: 8 })
        );
        assert_eq!(
            dispatch(
                SYSCALL_SYSTEM_INFO,
                [
                    0x6000,
                    core::mem::size_of::<genos_abi::UserSystemInfo>() as u64,
                    0,
                    0,
                    0,
                    0,
                ],
            ),
            Ok(SyscallAction::SystemInfo {
                address: 0x6000,
                length: core::mem::size_of::<genos_abi::UserSystemInfo>() as u64,
            })
        );
        assert_eq!(
            dispatch(SYSCALL_READ_FILE, [0x5000, 11, 0x6000, 128, 0, 0]),
            Ok(SyscallAction::ReadFile {
                path_address: 0x5000,
                path_length: 11,
                output_address: 0x6000,
                output_capacity: 128
            })
        );
        assert_eq!(
            dispatch(SYSCALL_OPEN_FILE, [0x5000, 11, 0, 0, 0, 0]),
            Ok(SyscallAction::OpenFile {
                path_address: 0x5000,
                path_length: 11,
            })
        );
        assert_eq!(
            dispatch(SYSCALL_READ_HANDLE, [0x101, 0x6000, 64, 0, 0, 0]),
            Ok(SyscallAction::ReadHandle {
                handle: 0x101,
                output_address: 0x6000,
                output_capacity: 64,
            })
        );
        assert_eq!(
            dispatch(SYSCALL_STAT_HANDLE, [0x101, 0x6000, 32, 0, 0, 0]),
            Ok(SyscallAction::StatHandle {
                handle: 0x101,
                output_address: 0x6000,
                output_length: 32,
            })
        );
        assert_eq!(
            dispatch(SYSCALL_CLOSE_HANDLE, [0x101, 0, 0, 0, 0, 0]),
            Ok(SyscallAction::CloseHandle { handle: 0x101 })
        );
        assert_eq!(
            dispatch(SYSCALL_OPEN_FILE_WITH_RIGHTS, [0x5000, 14, 3, 0, 0, 0]),
            Ok(SyscallAction::OpenFileWithRights {
                path_address: 0x5000,
                path_length: 14,
                rights: 3,
            })
        );
        assert_eq!(
            dispatch(SYSCALL_WRITE_HANDLE, [0x101, 0x6000, 64, 0, 0, 0]),
            Ok(SyscallAction::WriteHandle {
                handle: 0x101,
                input_address: 0x6000,
                input_length: 64,
            })
        );
        assert_eq!(
            dispatch(SYSCALL_WAIT_INPUT, [0x6000, 32, 1, 0, 0, 0]),
            Ok(SyscallAction::WaitInput {
                output_address: 0x6000,
                output_length: 32,
                mask: 1,
            })
        );
        assert_eq!(
            dispatch(SYSCALL_CREATE_ENDPOINT, [0; 6]),
            Ok(SyscallAction::CreateEndpoint)
        );
        assert_eq!(
            dispatch(SYSCALL_CONNECT_ENDPOINT, [3, 0, 0, 0, 0, 0]),
            Ok(SyscallAction::ConnectEndpoint { pid: 3 })
        );
        assert_eq!(
            dispatch(SYSCALL_SEND_ENDPOINT, [0x201, 0xfeed, 0, 0, 0, 0]),
            Ok(SyscallAction::SendEndpoint {
                handle: 0x201,
                value: 0xfeed,
            })
        );
        assert_eq!(
            dispatch(SYSCALL_RECEIVE_ENDPOINT, [0x201, 0x6000, 16, 0, 0, 0]),
            Ok(SyscallAction::ReceiveEndpoint {
                handle: 0x201,
                output_address: 0x6000,
                output_length: 16,
            })
        );
        assert_eq!(
            dispatch(SYSCALL_CLOSE_ENDPOINT, [0x201, 0, 0, 0, 0, 0]),
            Ok(SyscallAction::CloseEndpoint { handle: 0x201 })
        );
        assert_eq!(
            dispatch(SYSCALL_CONSOLE_WRITE, [0xc1, 0x6000, 12, 1, 0, 0]),
            Ok(SyscallAction::ConsoleWrite {
                handle: 0xc1,
                address: 0x6000,
                length: 12,
                kind: 1,
            })
        );
        assert_eq!(
            dispatch(SYSCALL_CONSOLE_SET_INPUT, [0xc1, 0, 0, 0, 0, 0]),
            Ok(SyscallAction::ConsoleSetInput {
                handle: 0xc1,
                address: 0,
                length: 0,
            })
        );
        assert_eq!(
            dispatch(SYSCALL_CONSOLE_CLEAR, [0xc1, 0, 0, 0, 0, 0]),
            Ok(SyscallAction::ConsoleClear { handle: 0xc1 })
        );
        assert_eq!(
            dispatch(SYSCALL_READ_DIRECTORY, [0x101, 3, 0x6000, 96, 0, 0]),
            Ok(SyscallAction::ReadDirectory {
                handle: 0x101,
                cursor: 3,
                output_address: 0x6000,
                output_length: 96,
            })
        );
        assert_eq!(
            dispatch(SYSCALL_TRUNCATE_HANDLE, [0x101, 0, 0, 0, 0, 0]),
            Ok(SyscallAction::TruncateHandle { handle: 0x101 })
        );
        assert_eq!(
            dispatch(SYSCALL_PROCESS_LAUNCH, [0xc201, 1, 1, 0, 0, 0]),
            Ok(SyscallAction::ProcessLaunch {
                supervisor: 0xc201,
                image: 1,
                mode: 1,
            })
        );
        assert_eq!(
            dispatch(SYSCALL_PROCESS_STATUS, [0xd101, 0x6000, 64, 0, 0, 0]),
            Ok(SyscallAction::ProcessStatus {
                handle: 0xd101,
                output_address: 0x6000,
                output_length: 64,
            })
        );
        assert_eq!(
            dispatch(SYSCALL_PROCESS_KILL, [0xd101, 0, 0, 0, 0, 0]),
            Ok(SyscallAction::ProcessKill { handle: 0xd101 })
        );
        assert_eq!(
            dispatch(SYSCALL_PROCESS_REAP, [0xd101, 0x6000, 64, 0, 0, 0]),
            Ok(SyscallAction::ProcessReap {
                handle: 0xd101,
                output_address: 0x6000,
                output_length: 64,
            })
        );
        assert_eq!(
            dispatch(SYSCALL_CREATE_DIRECTORY, [0x101, 0x5000, 5, 0, 0, 0]),
            Ok(SyscallAction::CreateDirectory {
                parent: 0x101,
                name_address: 0x5000,
                name_length: 5,
            })
        );
        assert_eq!(
            dispatch(SYSCALL_REMOVE_PATH, [0x101, 0x5000, 5, 0, 0, 0]),
            Ok(SyscallAction::RemovePath {
                parent: 0x101,
                name_address: 0x5000,
                name_length: 5,
            })
        );
        assert_eq!(
            dispatch(SYSCALL_SOCKET_OPEN, [1, 0, 0, 0, 0, 0]),
            Ok(SyscallAction::SocketOpen { protocol: 1 })
        );
        assert_eq!(
            dispatch(SYSCALL_SOCKET_CONNECT, [0xe701, 0x0a00_0202, 443, 0, 0, 0]),
            Ok(SyscallAction::SocketConnect {
                handle: 0xe701,
                target: 0x0a00_0202,
                port: 443,
            })
        );
        assert_eq!(
            dispatch(SYSCALL_SOCKET_BIND, [0xe701, 18081, 0, 0, 0, 0]),
            Ok(SyscallAction::SocketBind {
                handle: 0xe701,
                port: 18081,
            })
        );
        assert_eq!(
            dispatch(SYSCALL_SOCKET_LISTEN, [0xe701, 2, 0, 0, 0, 0]),
            Ok(SyscallAction::SocketListen {
                handle: 0xe701,
                backlog: 2,
            })
        );
        assert_eq!(
            dispatch(SYSCALL_SOCKET_ACCEPT, [0xe701, 0, 0, 0, 0, 0]),
            Ok(SyscallAction::SocketAccept { handle: 0xe701 })
        );
        assert_eq!(
            dispatch(SYSCALL_SOCKET_SEND, [0xe701, 0x5000, 64, 0, 0, 0]),
            Ok(SyscallAction::SocketSend {
                handle: 0xe701,
                input_address: 0x5000,
                input_length: 64,
            })
        );
        assert_eq!(
            dispatch(SYSCALL_SOCKET_RECEIVE, [0xe701, 0x6000, 128, 0, 0, 0]),
            Ok(SyscallAction::SocketReceive {
                handle: 0xe701,
                output_address: 0x6000,
                output_capacity: 128,
            })
        );
        assert_eq!(
            dispatch(SYSCALL_SOCKET_STATUS, [0xe701, 0x6000, 40, 0, 0, 0]),
            Ok(SyscallAction::SocketStatus {
                handle: 0xe701,
                output_address: 0x6000,
                output_length: 40,
            })
        );
        assert_eq!(
            dispatch(SYSCALL_SOCKET_SHUTDOWN, [0xe701, 3, 0, 0, 0, 0]),
            Ok(SyscallAction::SocketShutdown {
                handle: 0xe701,
                direction: 3,
            })
        );
        assert_eq!(
            dispatch(SYSCALL_SOCKET_CLOSE, [0xe701, 0, 0, 0, 0, 0]),
            Ok(SyscallAction::SocketClose { handle: 0xe701 })
        );
    }

    #[test]
    fn legacy_message_calls_are_reserved_and_unimplemented() {
        assert_eq!(
            dispatch(SYSCALL_SEND, [7, 0xfeed, 0, 0, 0, 0]),
            Err(SyscallError::UnknownNumber)
        );
        assert_eq!(
            dispatch(SYSCALL_SEND, [0; 6]),
            Err(SyscallError::UnknownNumber)
        );
        assert_eq!(
            dispatch(SYSCALL_RECEIVE, [0; 6]),
            Err(SyscallError::UnknownNumber)
        );
    }

    #[test]
    fn syscall_arguments_are_rejected_before_dispatch() {
        assert_eq!(
            dispatch(SYSCALL_PING, [1, 0, 0, 0, 0, 0]),
            Err(SyscallError::InvalidArgument)
        );
        assert_eq!(
            dispatch(SYSCALL_EXIT, [256, 0, 0, 0, 0, 0]),
            Err(SyscallError::InvalidArgument)
        );
        assert_eq!(
            dispatch(SYSCALL_REPORT, [0, 8, 0, 0, 0, 0]),
            Err(SyscallError::InvalidArgument)
        );
        assert_eq!(
            dispatch(SYSCALL_REPORT, [0x4000, 16, 0, 0, 0, 0]),
            Err(SyscallError::InvalidArgument)
        );
        assert_eq!(
            dispatch(SYSCALL_WRITE, [0x4000, 81, 0, 0, 0, 0]),
            Err(SyscallError::InvalidArgument)
        );
        assert_eq!(
            dispatch(SYSCALL_SLEEP, [0; 6]),
            Err(SyscallError::InvalidArgument)
        );
        assert_eq!(
            dispatch(SYSCALL_SEND, [0, 1, 0, 0, 0, 0]),
            Err(SyscallError::UnknownNumber)
        );
        assert_eq!(
            dispatch(SYSCALL_SYSTEM_INFO, [0x6000, 39, 0, 0, 0, 0]),
            Err(SyscallError::InvalidArgument)
        );
        assert_eq!(
            dispatch(SYSCALL_READ_FILE, [0x5000, 65, 0x6000, 128, 0, 0]),
            Err(SyscallError::InvalidArgument)
        );
        assert_eq!(
            dispatch(SYSCALL_READ_FILE, [0x5000, 11, 0x6000, 129, 0, 0]),
            Err(SyscallError::InvalidArgument)
        );
        assert_eq!(
            dispatch(SYSCALL_OPEN_FILE, [0x5000, 0, 0, 0, 0, 0]),
            Err(SyscallError::InvalidArgument)
        );
        assert_eq!(
            dispatch(SYSCALL_READ_HANDLE, [0, 0x6000, 64, 0, 0, 0]),
            Err(SyscallError::InvalidArgument)
        );
        assert_eq!(
            dispatch(SYSCALL_STAT_HANDLE, [0x101, 0x6000, 31, 0, 0, 0]),
            Err(SyscallError::InvalidArgument)
        );
        assert_eq!(
            dispatch(SYSCALL_CLOSE_HANDLE, [0, 0, 0, 0, 0, 0]),
            Err(SyscallError::InvalidArgument)
        );
        assert_eq!(
            dispatch(SYSCALL_OPEN_FILE_WITH_RIGHTS, [0x5000, 14, 8, 0, 0, 0]),
            Err(SyscallError::InvalidArgument)
        );
        assert_eq!(
            dispatch(SYSCALL_WRITE_HANDLE, [0x101, 0x6000, 129, 0, 0, 0]),
            Err(SyscallError::InvalidArgument)
        );
        assert_eq!(
            dispatch(SYSCALL_TRUNCATE_HANDLE, [0, 0, 0, 0, 0, 0]),
            Err(SyscallError::InvalidArgument)
        );
        assert_eq!(
            dispatch(SYSCALL_CREATE_DIRECTORY, [0x101, 0, 5, 0, 0, 0]),
            Err(SyscallError::InvalidArgument)
        );
        assert_eq!(
            dispatch(SYSCALL_REMOVE_PATH, [0x101, 0x5000, 0, 0, 0, 0]),
            Err(SyscallError::InvalidArgument)
        );
        assert_eq!(
            dispatch(SYSCALL_WAIT_INPUT, [0x6000, 31, 1, 0, 0, 0]),
            Err(SyscallError::InvalidArgument)
        );
        assert_eq!(
            dispatch(SYSCALL_WAIT_INPUT, [0x6000, 32, 4, 0, 0, 0]),
            Err(SyscallError::InvalidArgument)
        );
        assert_eq!(
            dispatch(SYSCALL_SOCKET_OPEN, [3, 0, 0, 0, 0, 0]),
            Err(SyscallError::InvalidArgument)
        );
        assert_eq!(
            dispatch(SYSCALL_SOCKET_CONNECT, [0xe701, 0, 443, 0, 0, 0]),
            Err(SyscallError::InvalidArgument)
        );
        assert_eq!(
            dispatch(SYSCALL_SOCKET_BIND, [0xe701, 1023, 0, 0, 0, 0]),
            Err(SyscallError::InvalidArgument)
        );
        assert_eq!(
            dispatch(SYSCALL_SOCKET_LISTEN, [0xe701, 3, 0, 0, 0, 0]),
            Err(SyscallError::InvalidArgument)
        );
        assert_eq!(
            dispatch(SYSCALL_SOCKET_ACCEPT, [0, 0, 0, 0, 0, 0]),
            Err(SyscallError::InvalidArgument)
        );
        assert_eq!(
            dispatch(SYSCALL_SOCKET_SEND, [0xe701, 0x5000, 129, 0, 0, 0]),
            Err(SyscallError::InvalidArgument)
        );
        assert_eq!(
            dispatch(SYSCALL_SOCKET_STATUS, [0xe701, 0x6000, 39, 0, 0, 0]),
            Err(SyscallError::InvalidArgument)
        );
        assert_eq!(
            dispatch(SYSCALL_SOCKET_SHUTDOWN, [0xe701, 0, 0, 0, 0, 0]),
            Err(SyscallError::InvalidArgument)
        );
        assert_eq!(dispatch(99, [0; 6]), Err(SyscallError::UnknownNumber));
    }

    #[test]
    fn user_buffers_must_stay_inside_the_owned_mapping() {
        assert!(validate_user_buffer(0x4000, 8, 0x4000, 0x1000));
        assert!(validate_user_buffer(0x4ff8, 8, 0x4000, 0x1000));
        assert!(!validate_user_buffer(0x3fff, 8, 0x4000, 0x1000));
        assert!(!validate_user_buffer(0x4ff9, 8, 0x4000, 0x1000));
        assert!(!validate_user_buffer(u64::MAX - 3, 8, 0x4000, 0x1000));
        assert!(!validate_user_buffer(0x4000, 0, 0x4000, 0x1000));
    }
}
