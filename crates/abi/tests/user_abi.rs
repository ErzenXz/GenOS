use genos_abi::{
    UserProcessHeader, UserSystemInfo, USER_ABI_VERSION, USER_FILE_READ_MAX, USER_MESSAGE_CAPACITY,
    USER_PAGE_SIZE, USER_TIMER_HZ,
};

#[test]
fn system_info_copy_out_layout_is_stable() {
    assert_eq!(core::mem::size_of::<UserSystemInfo>(), 40);
    assert_eq!(core::mem::align_of::<UserSystemInfo>(), 8);
    assert_eq!(UserSystemInfo::empty().abi_version, 0);
    assert_eq!(USER_ABI_VERSION, 5);
    assert_eq!(USER_MESSAGE_CAPACITY, 4);
    assert_eq!(USER_FILE_READ_MAX, 128);
    assert_eq!(USER_PAGE_SIZE, 4096);
    assert_eq!(USER_TIMER_HZ, 100);
}

#[test]
fn process_header_keeps_kernel_owned_offsets() {
    assert_eq!(core::mem::size_of::<UserProcessHeader>(), 16);
    assert_eq!(core::mem::offset_of!(UserProcessHeader, token), 0);
    assert_eq!(core::mem::offset_of!(UserProcessHeader, preemptions), 8);
}
