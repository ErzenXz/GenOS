pub use genos_abi::{
    USER_ABI_VERSION, USER_PING_REPLY as PING_REPLY,
    USER_SYSCALL_ABI_VERSION as SYSCALL_ABI_VERSION, USER_SYSCALL_EXIT as SYSCALL_EXIT,
    USER_SYSCALL_PING as SYSCALL_PING, USER_SYSCALL_REPORT as SYSCALL_REPORT,
    USER_SYSCALL_WRITE as SYSCALL_WRITE, USER_SYSCALL_YIELD as SYSCALL_YIELD,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SyscallAction {
    Return(u64),
    Exit(u8),
    Yield,
    Report { address: u64, length: u64 },
    Write { address: u64, length: u64 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SyscallError {
    UnknownNumber,
    InvalidArgument,
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
        SYSCALL_PING | SYSCALL_ABI_VERSION | SYSCALL_EXIT | SYSCALL_YIELD | SYSCALL_REPORT
        | SYSCALL_WRITE => Err(SyscallError::InvalidArgument),
        _ => Err(SyscallError::UnknownNumber),
    }
}

pub const fn error_code(error: SyscallError) -> u64 {
    match error {
        SyscallError::UnknownNumber => u64::MAX,
        SyscallError::InvalidArgument => u64::MAX - 1,
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
