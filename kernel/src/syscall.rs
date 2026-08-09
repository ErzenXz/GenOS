pub use genos_abi::{
    USER_ABI_VERSION, USER_PING_REPLY as PING_REPLY,
    USER_SYSCALL_ABI_VERSION as SYSCALL_ABI_VERSION,
    USER_SYSCALL_CLOSE_ENDPOINT as SYSCALL_CLOSE_ENDPOINT,
    USER_SYSCALL_CLOSE_HANDLE as SYSCALL_CLOSE_HANDLE,
    USER_SYSCALL_CONNECT_ENDPOINT as SYSCALL_CONNECT_ENDPOINT,
    USER_SYSCALL_CONSOLE_CLEAR as SYSCALL_CONSOLE_CLEAR,
    USER_SYSCALL_CONSOLE_SET_INPUT as SYSCALL_CONSOLE_SET_INPUT,
    USER_SYSCALL_CONSOLE_WRITE as SYSCALL_CONSOLE_WRITE,
    USER_SYSCALL_CREATE_ENDPOINT as SYSCALL_CREATE_ENDPOINT, USER_SYSCALL_EXIT as SYSCALL_EXIT,
    USER_SYSCALL_OPEN_FILE as SYSCALL_OPEN_FILE,
    USER_SYSCALL_OPEN_FILE_WITH_RIGHTS as SYSCALL_OPEN_FILE_WITH_RIGHTS,
    USER_SYSCALL_PING as SYSCALL_PING, USER_SYSCALL_READ_DIRECTORY as SYSCALL_READ_DIRECTORY,
    USER_SYSCALL_READ_FILE as SYSCALL_READ_FILE, USER_SYSCALL_READ_HANDLE as SYSCALL_READ_HANDLE,
    USER_SYSCALL_RECEIVE as SYSCALL_RECEIVE,
    USER_SYSCALL_RECEIVE_ENDPOINT as SYSCALL_RECEIVE_ENDPOINT,
    USER_SYSCALL_REPORT as SYSCALL_REPORT, USER_SYSCALL_SEND as SYSCALL_SEND,
    USER_SYSCALL_SEND_ENDPOINT as SYSCALL_SEND_ENDPOINT, USER_SYSCALL_SLEEP as SYSCALL_SLEEP,
    USER_SYSCALL_STAT_HANDLE as SYSCALL_STAT_HANDLE,
    USER_SYSCALL_SYSTEM_INFO as SYSCALL_SYSTEM_INFO, USER_SYSCALL_WAIT_CHILD as SYSCALL_WAIT_CHILD,
    USER_SYSCALL_WAIT_INPUT as SYSCALL_WAIT_INPUT, USER_SYSCALL_WRITE as SYSCALL_WRITE,
    USER_SYSCALL_WRITE_HANDLE as SYSCALL_WRITE_HANDLE, USER_SYSCALL_YIELD as SYSCALL_YIELD,
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
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SyscallError {
    UnknownNumber,
    InvalidArgument,
    Unavailable,
}

const CONSOLE_HANDLE_TAG: u64 = 0xc1 << 56;

pub const fn console_handle(pid: u8, generation: u64) -> u64 {
    CONSOLE_HANDLE_TAG | ((pid as u64) << 48) | (generation & 0x0000_ffff_ffff_ffff)
}

pub const fn console_capability_valid(owned: u64, presented: u64) -> bool {
    owned != 0 && owned == presented && presented & (0xff << 56) == CONSOLE_HANDLE_TAG
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
        | SYSCALL_READ_DIRECTORY => Err(SyscallError::InvalidArgument),
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
            dispatch(SYSCALL_SYSTEM_INFO, [0x6000, 104, 0, 0, 0, 0]),
            Ok(SyscallAction::SystemInfo {
                address: 0x6000,
                length: 104
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
            dispatch(SYSCALL_OPEN_FILE_WITH_RIGHTS, [0x5000, 14, 4, 0, 0, 0]),
            Err(SyscallError::InvalidArgument)
        );
        assert_eq!(
            dispatch(SYSCALL_WRITE_HANDLE, [0x101, 0x6000, 129, 0, 0, 0]),
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
