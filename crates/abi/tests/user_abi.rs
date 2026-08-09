use genos_abi::{
    UserChannelMessage, UserDirectoryEntry, UserFileStat, UserInputEvent, UserProcessHeader,
    UserSystemInfo, USER_ABI_VERSION, USER_CHANNEL_MESSAGE_SIZE, USER_CONSOLE_LINE_ERROR,
    USER_CONSOLE_LINE_OUTPUT, USER_CONSOLE_LINE_PROMPT, USER_CONSOLE_LINE_STATUS,
    USER_CONSOLE_TEXT_MAX, USER_ENDPOINT_HANDLE_CAPACITY, USER_ENDPOINT_QUEUE_CAPACITY,
    USER_ERROR_INVALID_ARGUMENT, USER_ERROR_UNAVAILABLE, USER_ERROR_UNKNOWN_SYSCALL,
    USER_FILE_HANDLE_CAPACITY, USER_FILE_KIND_DIRECTORY, USER_FILE_KIND_REGULAR,
    USER_FILE_READ_MAX, USER_FILE_RIGHTS_MASK, USER_FILE_RIGHT_READ, USER_FILE_RIGHT_WRITE,
    USER_FILE_WRITE_MAX, USER_INPUT_KIND_KEY, USER_INPUT_KIND_POINTER_BUTTON,
    USER_INPUT_KIND_POINTER_MOVE, USER_INPUT_MASK_ALL, USER_INPUT_MASK_KEYBOARD,
    USER_INPUT_MASK_POINTER, USER_KEY_ARROW_DOWN, USER_KEY_ARROW_UP, USER_KEY_BACKSPACE,
    USER_KEY_CHAR, USER_KEY_ENTER, USER_KEY_ESCAPE, USER_KEY_TAB, USER_MESSAGE_CAPACITY,
    USER_PAGE_SIZE, USER_POINTER_BUTTON_LEFT, USER_POINTER_BUTTON_MIDDLE,
    USER_POINTER_BUTTON_RIGHT, USER_SYSCALL_CLOSE_ENDPOINT, USER_SYSCALL_CONNECT_ENDPOINT,
    USER_SYSCALL_CONSOLE_CLEAR, USER_SYSCALL_CONSOLE_SET_INPUT, USER_SYSCALL_CONSOLE_WRITE,
    USER_SYSCALL_CREATE_ENDPOINT, USER_SYSCALL_RECEIVE_ENDPOINT, USER_SYSCALL_SEND_ENDPOINT,
    USER_TIMER_HZ, USER_WRITABLE_PREFIX,
};

#[test]
fn system_info_copy_out_layout_is_stable() {
    assert_eq!(core::mem::size_of::<UserSystemInfo>(), 104);
    assert_eq!(core::mem::align_of::<UserSystemInfo>(), 8);
    assert_eq!(core::mem::offset_of!(UserSystemInfo, abi_version), 0);
    assert_eq!(
        core::mem::offset_of!(UserSystemInfo, file_handle_capacity),
        40
    );
    assert_eq!(core::mem::offset_of!(UserSystemInfo, max_file_write), 48);
    assert_eq!(core::mem::offset_of!(UserSystemInfo, input_event_size), 56);
    assert_eq!(core::mem::offset_of!(UserSystemInfo, input_mask), 64);
    assert_eq!(
        core::mem::offset_of!(UserSystemInfo, endpoint_handle_capacity),
        72
    );
    assert_eq!(
        core::mem::offset_of!(UserSystemInfo, channel_message_size),
        80
    );
    assert_eq!(UserSystemInfo::empty().abi_version, 0);
    assert_eq!(UserSystemInfo::empty().endpoint_handle_capacity, 0);
    assert_eq!(UserSystemInfo::empty().channel_message_size, 0);
    assert_eq!(UserSystemInfo::empty().directory_entry_size, 0);
    assert_eq!(
        core::mem::offset_of!(UserSystemInfo, directory_entry_size),
        88
    );
    assert_eq!(core::mem::offset_of!(UserSystemInfo, max_path_length), 96);
    assert_eq!(core::mem::size_of::<UserDirectoryEntry>(), 96);
    assert_eq!(core::mem::align_of::<UserDirectoryEntry>(), 8);
    assert_eq!(core::mem::offset_of!(UserDirectoryEntry, name), 32);
    assert_eq!(UserDirectoryEntry::empty().name_length, 0);
    assert_eq!(USER_ABI_VERSION, 11);
    assert_eq!(USER_MESSAGE_CAPACITY, 4);
    assert_eq!(USER_FILE_READ_MAX, 128);
    assert_eq!(USER_PAGE_SIZE, 4096);
    assert_eq!(USER_TIMER_HZ, 100);
    assert_eq!(USER_FILE_HANDLE_CAPACITY, 4);
    assert_eq!(USER_FILE_WRITE_MAX, 128);
}

#[test]
fn channel_message_layout_and_endpoint_constants_are_stable() {
    assert_eq!(core::mem::size_of::<UserChannelMessage>(), 16);
    assert_eq!(core::mem::align_of::<UserChannelMessage>(), 8);
    assert_eq!(core::mem::offset_of!(UserChannelMessage, sender_pid), 0);
    assert_eq!(core::mem::offset_of!(UserChannelMessage, value), 8);
    assert_eq!(UserChannelMessage::empty().sender_pid, 0);
    assert_eq!(UserChannelMessage::empty().value, 0);
    assert_eq!(USER_CHANNEL_MESSAGE_SIZE, 16);
    assert_eq!(USER_ENDPOINT_HANDLE_CAPACITY, 4);
    assert_eq!(USER_ENDPOINT_QUEUE_CAPACITY, 4);
    assert_eq!(USER_SYSCALL_CREATE_ENDPOINT, 19);
    assert_eq!(USER_SYSCALL_CONNECT_ENDPOINT, 20);
    assert_eq!(USER_SYSCALL_SEND_ENDPOINT, 21);
    assert_eq!(USER_SYSCALL_RECEIVE_ENDPOINT, 22);
    assert_eq!(USER_SYSCALL_CLOSE_ENDPOINT, 23);
    assert_eq!(USER_SYSCALL_CONSOLE_WRITE, 24);
    assert_eq!(USER_SYSCALL_CONSOLE_SET_INPUT, 25);
    assert_eq!(USER_SYSCALL_CONSOLE_CLEAR, 26);
    assert_eq!(genos_abi::USER_SYSCALL_READ_DIRECTORY, 27);
    assert_eq!(USER_CONSOLE_LINE_OUTPUT, 0);
    assert_eq!(USER_CONSOLE_LINE_PROMPT, 1);
    assert_eq!(USER_CONSOLE_LINE_ERROR, 2);
    assert_eq!(USER_CONSOLE_LINE_STATUS, 3);
    assert_eq!(USER_CONSOLE_TEXT_MAX, 80);
}

#[test]
fn input_event_layout_and_constants_are_stable() {
    assert_eq!(core::mem::size_of::<UserInputEvent>(), 32);
    assert_eq!(core::mem::align_of::<UserInputEvent>(), 8);
    assert_eq!(core::mem::offset_of!(UserInputEvent, kind), 0);
    assert_eq!(core::mem::offset_of!(UserInputEvent, code), 8);
    assert_eq!(core::mem::offset_of!(UserInputEvent, value0), 16);
    assert_eq!(core::mem::offset_of!(UserInputEvent, value1), 24);
    assert_eq!(UserInputEvent::empty().kind, 0);
    assert_eq!(USER_INPUT_MASK_KEYBOARD, 1);
    assert_eq!(USER_INPUT_MASK_POINTER, 2);
    assert_eq!(USER_INPUT_MASK_ALL, 3);
    assert_eq!(USER_INPUT_KIND_KEY, 1);
    assert_eq!(USER_INPUT_KIND_POINTER_MOVE, 2);
    assert_eq!(USER_INPUT_KIND_POINTER_BUTTON, 3);
    assert_eq!(USER_KEY_CHAR, 1);
    assert_eq!(USER_KEY_ENTER, 2);
    assert_eq!(USER_KEY_BACKSPACE, 3);
    assert_eq!(USER_KEY_ESCAPE, 4);
    assert_eq!(USER_KEY_TAB, 5);
    assert_eq!(USER_KEY_ARROW_UP, 6);
    assert_eq!(USER_KEY_ARROW_DOWN, 7);
    assert_eq!(USER_POINTER_BUTTON_LEFT, 1);
    assert_eq!(USER_POINTER_BUTTON_RIGHT, 2);
    assert_eq!(USER_POINTER_BUTTON_MIDDLE, 4);
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
    assert_eq!(USER_FILE_RIGHT_WRITE, 2);
    assert_eq!(USER_FILE_RIGHTS_MASK, 3);
    assert_eq!(USER_WRITABLE_PREFIX, "/USER/");
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
