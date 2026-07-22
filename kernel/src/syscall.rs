pub use genos_abi::{
    USER_ABI_VERSION, USER_PING_REPLY as PING_REPLY,
    USER_SYSCALL_ABI_VERSION as SYSCALL_ABI_VERSION, USER_SYSCALL_EXIT as SYSCALL_EXIT,
    USER_SYSCALL_PING as SYSCALL_PING, USER_SYSCALL_READ_FILE as SYSCALL_READ_FILE,
    USER_SYSCALL_RECEIVE as SYSCALL_RECEIVE, USER_SYSCALL_REPORT as SYSCALL_REPORT,
    USER_SYSCALL_SEND as SYSCALL_SEND, USER_SYSCALL_SLEEP as SYSCALL_SLEEP,
    USER_SYSCALL_SYSTEM_INFO as SYSCALL_SYSTEM_INFO, USER_SYSCALL_WAIT_CHILD as SYSCALL_WAIT_CHILD,
    USER_SYSCALL_WRITE as SYSCALL_WRITE, USER_SYSCALL_YIELD as SYSCALL_YIELD,
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
    Send {
        pid: u8,
        value: u64,
    },
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
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SyscallError {
    UnknownNumber,
    InvalidArgument,
    Unavailable,
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
        SYSCALL_SEND if (1..=u8::MAX as u64).contains(&args[0]) && args[2..] == [0; 4] => {
            Ok(SyscallAction::Send {
                pid: args[0] as u8,
                value: args[1],
            })
        }
        SYSCALL_RECEIVE if args == [0; 6] => Ok(SyscallAction::Receive),
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
        SYSCALL_PING | SYSCALL_ABI_VERSION | SYSCALL_EXIT | SYSCALL_YIELD | SYSCALL_REPORT
        | SYSCALL_WRITE | SYSCALL_SLEEP | SYSCALL_SEND | SYSCALL_RECEIVE | SYSCALL_WAIT_CHILD
        | SYSCALL_SYSTEM_INFO | SYSCALL_READ_FILE => Err(SyscallError::InvalidArgument),
        _ => Err(SyscallError::UnknownNumber),
    }
}

pub const fn error_code(error: SyscallError) -> u64 {
    match error {
        SyscallError::UnknownNumber => u64::MAX,
        SyscallError::InvalidArgument => u64::MAX - 1,
        SyscallError::Unavailable => u64::MAX - 2,
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
            dispatch(SYSCALL_SEND, [7, 0xfeed, 0, 0, 0, 0]),
            Ok(SyscallAction::Send {
                pid: 7,
                value: 0xfeed
            })
        );
        assert_eq!(
            dispatch(SYSCALL_RECEIVE, [0; 6]),
            Ok(SyscallAction::Receive)
        );
        assert_eq!(
            dispatch(SYSCALL_WAIT_CHILD, [8, 0, 0, 0, 0, 0]),
            Ok(SyscallAction::WaitChild { pid: 8 })
        );
        assert_eq!(
            dispatch(SYSCALL_SYSTEM_INFO, [0x6000, 40, 0, 0, 0, 0]),
            Ok(SyscallAction::SystemInfo {
                address: 0x6000,
                length: 40
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
            Err(SyscallError::InvalidArgument)
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
