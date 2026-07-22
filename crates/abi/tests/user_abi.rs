use genos_abi::{
    UserFileStat, UserProcessHeader, UserSystemInfo, USER_ABI_VERSION, USER_ERROR_INVALID_ARGUMENT,
    USER_ERROR_UNAVAILABLE, USER_ERROR_UNKNOWN_SYSCALL, USER_FILE_HANDLE_CAPACITY,
    USER_FILE_KIND_DIRECTORY, USER_FILE_KIND_REGULAR, USER_FILE_READ_MAX, USER_FILE_RIGHT_READ,
    USER_MESSAGE_CAPACITY, USER_PAGE_SIZE, USER_TIMER_HZ,
};

#[test]
fn system_info_copy_out_layout_is_stable() {
    assert_eq!(core::mem::size_of::<UserSystemInfo>(), 48);
    assert_eq!(core::mem::align_of::<UserSystemInfo>(), 8);
    assert_eq!(UserSystemInfo::empty().abi_version, 0);
    assert_eq!(USER_ABI_VERSION, 6);
    assert_eq!(USER_MESSAGE_CAPACITY, 4);
    assert_eq!(USER_FILE_READ_MAX, 128);
    assert_eq!(USER_PAGE_SIZE, 4096);
    assert_eq!(USER_TIMER_HZ, 100);
    assert_eq!(USER_FILE_HANDLE_CAPACITY, 4);
}

#[test]
fn file_stat_and_capability_constants_are_stable() {
    assert_eq!(core::mem::size_of::<UserFileStat>(), 32);
    assert_eq!(core::mem::align_of::<UserFileStat>(), 8);
    assert_eq!(core::mem::offset_of!(UserFileStat, size), 0);
    assert_eq!(core::mem::offset_of!(UserFileStat, offset), 8);
    assert_eq!(core::mem::offset_of!(UserFileStat, kind), 16);
    assert_eq!(core::mem::offset_of!(UserFileStat, rights), 24);
    assert_eq!(USER_FILE_KIND_REGULAR, 1);
    assert_eq!(USER_FILE_KIND_DIRECTORY, 2);
    assert_eq!(USER_FILE_RIGHT_READ, 1);
    assert_eq!(USER_ERROR_UNKNOWN_SYSCALL, u64::MAX);
    assert_eq!(USER_ERROR_INVALID_ARGUMENT, u64::MAX - 1);
    assert_eq!(USER_ERROR_UNAVAILABLE, u64::MAX - 2);
}

#[test]
fn process_header_keeps_kernel_owned_offsets() {
    assert_eq!(core::mem::size_of::<UserProcessHeader>(), 16);
    assert_eq!(core::mem::offset_of!(UserProcessHeader, token), 0);
    assert_eq!(core::mem::offset_of!(UserProcessHeader, preemptions), 8);
}
