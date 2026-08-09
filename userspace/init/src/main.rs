#![no_std]
#![no_main]

use core::panic::PanicInfo;
use core::ptr::{addr_of, addr_of_mut, read_volatile, write_volatile};

use genos_user_runtime as runtime;

const FAULT_TOKEN: u64 = 0xffff_ffff_ffff_fff0;
/// The kernel selects a program with the top nibble of the launch token; the
/// low bytes carry whatever pids that program needs. Comparing the nibble
/// rather than the full word keeps every dispatch out of the 10-byte
/// `movabs` encoding this image cannot afford.
const TOKEN_MODE_SHIFT: u32 = 60;
const HOLD_TOKEN_MODE: u64 = 0xb;
const SLEEP_TOKEN_MODE: u64 = 0x4;
const CHILD_TOKEN_MODE: u64 = 0x5;
const PARENT_TOKEN_MODE: u64 = 0x6;
const FILE_TOKEN_MODE: u64 = 0x7;
const INPUT_TOKEN_MODE: u64 = 0x9;
const WRITE_TOKEN_MODE: u64 = 0xa;
const FANIN_RECEIVER_TOKEN_MODE: u64 = 0xc;
const FANIN_A_TOKEN_MODE: u64 = 0xd;
const FANIN_B_TOKEN_MODE: u64 = 0xe;
const COORDINATION_MESSAGE: u64 = 0x4745_4e4f_535f_4950;
const FANIN_A1: u64 = 0x4131;
const FANIN_A2: u64 = 0x4132;
const FANIN_B1: u64 = 0x4231;
/// Fan-in deadlines, in timer ticks. Under real 100 Hz preemption the arrival
/// order has to come from these deadlines alone, never from how long a slice
/// happens to last. A queues A1 and takes its fairness denial, B queues B1,
/// only then does the receiver wake and drain, and A retries A2 well after the
/// receiver has parked on the empty queue. Real graphical QEMU runs can spend
/// more than one hundred timer ticks between consecutive Ring 3 syscalls while
/// the desktop is repainting, so the proof deliberately uses second-scale
/// barriers rather than timing that only works in the fast headless harness.
/// Every receive syscall yields; the 2,000-tick retry leaves the receiver ample
/// time to take A1 and B1 and block on its third receive before A2 arrives.
///
/// All four gaps are measured from the receiver's endpoint becoming visible,
/// not from process start: the initial run order is not guaranteed, so both
/// producers may already be retrying their connect when publication happens.
/// Two producers polling on the same `CONNECT_RETRY_TICKS` grid can therefore
/// connect up to one retry gap apart in either order, so B holds back a fixed
/// delay before posting B1. A posts A1 immediately. The receiver's longer
/// barrier keeps both first messages queued until the fairness check is done.
const FANIN_A_TICKS: u64 = 5;
const FANIN_B_TICKS: u64 = 10;
const FANIN_RECEIVER_TICKS: u64 = 800;
const FANIN_A_RETRY_TICKS: u64 = 2_000;
const FANIN_B_CONNECT_TICKS: u64 = 200;
/// Bounded connect retry: 64 attempts ten ticks apart tolerate roughly six
/// seconds of publication delay, and the count makes the loop terminate even
/// if the target never publishes at all.
const CONNECT_ATTEMPTS: u32 = 64;
const CONNECT_RETRY_TICKS: u64 = 10;
const GREETING: &[u8] = b"hello from INIT.ELF in ring 3";
const AWAKENED: &[u8] = b"INIT.ELF woke after deadline";
const COORDINATED: &[u8] = b"parent received child exit + message";
const README_PATH: &[u8] = b"/README.TXT";
const README_CONTENT: &[u8] = b"Welcome to GenOS.\nThis file lives in the V1 RAM disk.\n";
const FILE_COMPLETE: &[u8] = b"INIT.ELF used open/read/stat/close";
const FIRST_READ_BYTES: usize = 17;
const USER_NOTE_PATH: &[u8] = b"/USER/APP.TXT";
const WRITE_FIRST: &[u8] = b"GenOS Ring 3 ";
const WRITE_SECOND: &[u8] = b"writes safely.";
const WRITE_CONTENT: &[u8] = b"GenOS Ring 3 writes safely.";
const WRITE_COMPLETE: &[u8] = b"INIT.ELF wrote and verified /USER/APP.TXT";
const INPUT_PROMPT: &[u8] = b"INIT.ELF waiting for one keyboard event";
const INPUT_BUSY: &[u8] = b"INIT.ELF input channel is busy";
const FANIN_DONE: &[u8] = b"INIT.ELF fan-in A1 B1 A2";

#[repr(C)]
struct ProcessData {
    header: runtime::UserProcessHeader,
    system_info: runtime::UserSystemInfo,
    file_stat: runtime::UserFileStat,
    input_event: runtime::UserInputEvent,
    channel: runtime::UserChannelMessage,
    file_buffer: [u8; 128],
}

#[used]
#[link_section = ".data.process"]
static mut PROCESS_DATA: ProcessData = ProcessData {
    header: runtime::UserProcessHeader::empty(),
    system_info: runtime::UserSystemInfo::empty(),
    file_stat: runtime::UserFileStat::empty(),
    input_event: runtime::UserInputEvent::empty(),
    channel: runtime::UserChannelMessage::empty(),
    file_buffer: [0; 128],
};

/// A live capability is a non-zero handle that is not one of the error codes
/// the kernel returns in the same register. File and endpoint handles are
/// distinct authorities but share this encoding, so both are checked here.
fn handle_ok(handle: u64) -> bool {
    handle != 0 && handle < runtime::ERROR_UNAVAILABLE
}

// Every runtime wrapper expands its own `int 0x80` register setup at the call
// site, so each syscall this image uses more than once gets exactly one
// out-of-line copy here. That is what keeps the text segment inside its single
// 4 KiB page as the program grows.

/// Slice equality would link the compiler-builtins `memcmp`, which costs more
/// than 600 bytes of a 4 KiB image, so the comparison stays here.
#[inline(never)]
fn matches(actual: &[u8], expected: &[u8]) -> bool {
    actual.len() == expected.len() && actual.iter().zip(expected).all(|(got, want)| got == want)
}

#[inline(never)]
fn say(text: &[u8]) -> bool {
    runtime::write(text) == text.len() as u64
}

#[inline(never)]
fn nap(ticks: u64) -> bool {
    runtime::sleep(ticks) == 0
}

#[inline(never)]
fn child_status(pid: u8) -> u64 {
    runtime::wait_child(pid)
}

#[inline(never)]
fn publish() -> u64 {
    runtime::create_endpoint()
}

/// Connect to `pid`'s endpoint, tolerating the window before that endpoint has
/// been published. `ERROR_UNAVAILABLE` is the only retryable answer: every
/// other result, including a live handle, is returned to the caller untouched
/// so the capability check at the call site still sees exactly what the kernel
/// said. The attempt count is fixed, so a target that never publishes ends the
/// loop with the denial rather than hanging the producer.
#[inline(never)]
fn connect_wait(pid: u8) -> u64 {
    let mut attempts = CONNECT_ATTEMPTS;
    loop {
        let handle = runtime::connect_endpoint(pid);
        attempts -= 1;
        if handle != runtime::ERROR_UNAVAILABLE || attempts == 0 || !nap(CONNECT_RETRY_TICKS) {
            return handle;
        }
    }
}

#[inline(never)]
fn post(handle: u64, value: u64) -> u64 {
    runtime::send_endpoint(handle, value)
}

/// One receive plus the identity check every caller needs: the message must be
/// the expected size and name the expected sender and value.
#[inline(never)]
fn expect(handle: u64, sender: u8, value: u64, message: &mut runtime::UserChannelMessage) -> bool {
    runtime::receive_endpoint(handle, message) == runtime::CHANNEL_MESSAGE_SIZE
        && message.sender_pid == sender as u64
        && message.value == value
}

#[inline(never)]
fn unpublish(handle: u64) -> bool {
    runtime::close_endpoint(handle) == 0
}

#[inline(never)]
fn stat_of(handle: u64, stat: &mut runtime::UserFileStat) -> bool {
    runtime::stat_handle(handle, stat) == core::mem::size_of::<runtime::UserFileStat>() as u64
}

#[inline(never)]
fn read_at(handle: u64, buffer: &mut [u8]) -> u64 {
    runtime::read_handle(handle, buffer)
}

#[inline(never)]
fn write_at(handle: u64, data: &[u8]) -> u64 {
    runtime::write_handle(handle, data)
}

#[inline(never)]
fn close_file(handle: u64) -> bool {
    runtime::close_handle(handle) == 0
}

#[inline(never)]
fn open_rights(path: &[u8], rights: u64) -> u64 {
    runtime::open_file_with_rights(path, rights)
}

#[no_mangle]
pub extern "C" fn _start(token: u64) -> ! {
    runtime::exit(run(token))
}

/// Shared entry sequence, then one program per token mode. Each program
/// returns `Some(code)` to exit immediately or `None` to fall through to the
/// shared report-and-exit tail; nothing calls `exit` directly, so the image
/// holds a single copy of that sequence.
fn run(token: u64) -> u8 {
    unsafe {
        write_volatile(addr_of_mut!(PROCESS_DATA.header.token), token);
    }

    if runtime::ping() != runtime::PING_REPLY || runtime::abi_version() != runtime::ABI_VERSION {
        return 255;
    }

    while unsafe { read_volatile(addr_of!(PROCESS_DATA.header.preemptions)) } == 0 {
        core::hint::spin_loop();
    }

    if token == FAULT_TOKEN {
        unsafe {
            write_volatile(runtime::STACK_GUARD as *mut u64, token);
        }
        return 254;
    }

    if !say(GREETING) {
        return 251;
    }

    let mode = token >> TOKEN_MODE_SHIFT;
    let outcome = match mode {
        SLEEP_TOKEN_MODE => sleep_program(),
        CHILD_TOKEN_MODE => child_program(token as u8),
        PARENT_TOKEN_MODE => parent_program(token as u8),
        FANIN_RECEIVER_TOKEN_MODE => fanin_receiver((token >> 8) as u8, token as u8),
        FANIN_A_TOKEN_MODE | FANIN_B_TOKEN_MODE => {
            fanin_producer(token as u8, mode == FANIN_A_TOKEN_MODE)
        }
        FILE_TOKEN_MODE => file_program(),
        WRITE_TOKEN_MODE => write_program(),
        INPUT_TOKEN_MODE => input_program(),
        HOLD_TOKEN_MODE => loop {
            core::hint::spin_loop();
        },
        _ => None,
    };
    if let Some(code) = outcome {
        return code;
    }

    let reported = runtime::report_u64(unsafe { addr_of!(PROCESS_DATA.header.token) });
    if reported == token {
        0
    } else {
        253
    }
}

#[inline(always)]
fn sleep_program() -> Option<u8> {
    (!nap(3) || !say(AWAKENED)).then_some(250)
}

#[inline(always)]
fn child_program(parent: u8) -> Option<u8> {
    if !nap(3) {
        return Some(249);
    }
    let handle = connect_wait(parent);
    if !handle_ok(handle) || post(handle, COORDINATION_MESSAGE) != 0 || !unpublish(handle) {
        return Some(249);
    }
    Some(7)
}

#[inline(always)]
fn parent_program(child: u8) -> Option<u8> {
    let handle = publish();
    let message = unsafe { &mut *addr_of_mut!(PROCESS_DATA.channel) };
    if !handle_ok(handle)
        || child_status(child) != 7
        || !expect(handle, child, COORDINATION_MESSAGE, message)
        || !unpublish(handle)
    {
        return Some(248);
    }
    (!say(COORDINATED)).then_some(247)
}

/// Publishes an endpoint, sleeps long enough for both producers to queue
/// behind it, then drains the exact arrival order. The third receive finds an
/// empty queue and parks until producer A wakes and sends again.
#[inline(always)]
fn fanin_receiver(producer_a: u8, producer_b: u8) -> Option<u8> {
    let handle = publish();
    if !handle_ok(handle) || !nap(FANIN_RECEIVER_TICKS) {
        return Some(228);
    }
    let message = unsafe { &mut *addr_of_mut!(PROCESS_DATA.channel) };
    for (sender, value) in [
        (producer_a, FANIN_A1),
        (producer_b, FANIN_B1),
        (producer_a, FANIN_A2),
    ] {
        if !expect(handle, sender, value, message) {
            return Some(227);
        }
    }
    // Reaping both producers keeps terminal ownership with this process.
    if child_status(producer_a) != 0
        || child_status(producer_b) != 0
        || !unpublish(handle)
        || !say(FANIN_DONE)
    {
        return Some(226);
    }
    None
}

/// Producer A wakes first and proves the fairness rule: its second send is
/// refused while its first message is still queued, and only succeeds after a
/// sleep long enough for the receiver to have drained the queue. Neither
/// producer assumes the receiver has published yet; B alone pays an extra fixed
/// delay after connecting so A1 is queued first however the connect retries
/// interleaved.
#[inline(always)]
fn fanin_producer(receiver: u8, first: bool) -> Option<u8> {
    if !nap(if first { FANIN_A_TICKS } else { FANIN_B_TICKS }) {
        return Some(225);
    }
    let handle = connect_wait(receiver);
    if !handle_ok(handle)
        || (!first && !nap(FANIN_B_CONNECT_TICKS))
        || post(handle, if first { FANIN_A1 } else { FANIN_B1 }) != 0
    {
        return Some(224);
    }
    if first
        && (post(handle, FANIN_A2) != runtime::ERROR_UNAVAILABLE
            || !nap(FANIN_A_RETRY_TICKS)
            || post(handle, FANIN_A2) != 0)
    {
        return Some(223);
    }
    (!unpublish(handle)).then_some(222)
}

#[inline(always)]
fn file_program() -> Option<u8> {
    let info = unsafe { &mut *addr_of_mut!(PROCESS_DATA.system_info) };
    if runtime::system_info(info) != core::mem::size_of::<runtime::UserSystemInfo>() as u64
        || info.abi_version != runtime::ABI_VERSION
        || info.page_size != runtime::PAGE_SIZE
        || info.timer_hz != runtime::TIMER_HZ
        || info.message_capacity != runtime::MESSAGE_CAPACITY
        || info.max_file_read != runtime::FILE_READ_MAX as u64
        || info.file_handle_capacity != runtime::FILE_HANDLE_CAPACITY
        || info.max_file_write != runtime::FILE_WRITE_MAX as u64
        || info.input_event_size != core::mem::size_of::<runtime::UserInputEvent>() as u64
        || info.input_mask != runtime::INPUT_MASK_ALL
        || info.endpoint_handle_capacity != runtime::ENDPOINT_HANDLE_CAPACITY
        || info.channel_message_size != runtime::CHANNEL_MESSAGE_SIZE
    {
        return Some(246);
    }
    let handle = runtime::open_file(README_PATH);
    if !handle_ok(handle) {
        return Some(245);
    }
    let stat = unsafe { &mut *addr_of_mut!(PROCESS_DATA.file_stat) };
    if !stat_of(handle, stat)
        || stat.size != README_CONTENT.len() as u64
        || stat.offset != 0
        || stat.kind != runtime::FILE_KIND_REGULAR
        || stat.rights != runtime::FILE_RIGHT_READ
    {
        return Some(244);
    }
    let buffer = unsafe { &mut *addr_of_mut!(PROCESS_DATA.file_buffer) };
    if read_at(handle, &mut buffer[..FIRST_READ_BYTES]) != FIRST_READ_BYTES as u64
        || !stat_of(handle, stat)
        || stat.offset != FIRST_READ_BYTES as u64
    {
        return Some(243);
    }
    if read_at(handle, &mut buffer[FIRST_READ_BYTES..])
        != (README_CONTENT.len() - FIRST_READ_BYTES) as u64
        || !matches(&buffer[..README_CONTENT.len()], README_CONTENT)
    {
        return Some(242);
    }
    if !close_file(handle) || read_at(handle, &mut buffer[..1]) != runtime::ERROR_INVALID_ARGUMENT {
        return Some(241);
    }
    (!say(FILE_COMPLETE)).then_some(240)
}

#[inline(always)]
fn write_program() -> Option<u8> {
    let rights = runtime::FILE_RIGHT_READ | runtime::FILE_RIGHT_WRITE;
    if open_rights(README_PATH, rights) != runtime::ERROR_INVALID_ARGUMENT {
        return Some(239);
    }
    let handle = open_rights(USER_NOTE_PATH, rights);
    if !handle_ok(handle) {
        return Some(238);
    }
    let stat = unsafe { &mut *addr_of_mut!(PROCESS_DATA.file_stat) };
    if !stat_of(handle, stat)
        || (stat.size != 0 && stat.size != WRITE_CONTENT.len() as u64)
        || stat.offset != 0
        || stat.rights != rights
    {
        return Some(237);
    }
    if write_at(handle, WRITE_FIRST) != WRITE_FIRST.len() as u64
        || write_at(handle, WRITE_SECOND) != WRITE_SECOND.len() as u64
        || !stat_of(handle, stat)
        || stat.size != WRITE_CONTENT.len() as u64
        || stat.offset != WRITE_CONTENT.len() as u64
    {
        return Some(236);
    }
    if !close_file(handle) {
        return Some(235);
    }
    let read_handle = runtime::open_file(USER_NOTE_PATH);
    let buffer = unsafe { &mut *addr_of_mut!(PROCESS_DATA.file_buffer) };
    if !handle_ok(read_handle)
        || write_at(read_handle, b"!") != runtime::ERROR_INVALID_ARGUMENT
        || read_at(read_handle, buffer) != WRITE_CONTENT.len() as u64
        || !matches(&buffer[..WRITE_CONTENT.len()], WRITE_CONTENT)
        || !close_file(read_handle)
    {
        return Some(234);
    }
    (!say(WRITE_COMPLETE)).then_some(233)
}

#[inline(always)]
fn input_program() -> Option<u8> {
    if !say(INPUT_PROMPT) {
        return Some(232);
    }
    let event = unsafe { &mut *addr_of_mut!(PROCESS_DATA.input_event) };
    let result = runtime::wait_input(event, runtime::INPUT_MASK_KEYBOARD);
    if result == runtime::ERROR_UNAVAILABLE {
        return Some(if say(INPUT_BUSY) { 0 } else { 231 });
    }
    if result != core::mem::size_of::<runtime::UserInputEvent>() as u64
        || event.kind != runtime::INPUT_KIND_KEY
        || event.code != runtime::KEY_CHAR
        || !(0x20..=0x7e).contains(&event.value0)
        || event.value1 != 0
    {
        return Some(230);
    }
    let mut message = *b"INIT.ELF received key: ?";
    message[23] = event.value0 as u8;
    (!say(&message)).then_some(229)
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    runtime::exit(252)
}
