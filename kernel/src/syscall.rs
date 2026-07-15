pub const USER_ABI_VERSION: u64 = 1;
pub const SYSCALL_PING: u64 = 0;
pub const SYSCALL_ABI_VERSION: u64 = 1;
pub const SYSCALL_EXIT: u64 = 2;
pub const PING_REPLY: u64 = 0x4745_4e4f_535f_4f4b;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SyscallAction {
    Return(u64),
    Exit(u8),
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
        SYSCALL_PING | SYSCALL_ABI_VERSION | SYSCALL_EXIT => Err(SyscallError::InvalidArgument),
        _ => Err(SyscallError::UnknownNumber),
    }
}

pub const fn error_code(error: SyscallError) -> u64 {
    match error {
        SyscallError::UnknownNumber => u64::MAX,
        SyscallError::InvalidArgument => u64::MAX - 1,
    }
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
        assert_eq!(dispatch(99, [0; 6]), Err(SyscallError::UnknownNumber));
    }
}
