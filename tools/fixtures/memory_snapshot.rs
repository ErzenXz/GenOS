//! Disposable normal Ring 3 shell fixture for tools/test_memory_snapshot.py.
#![no_std]
#![no_main]

use core::ptr::{addr_of_mut, write_volatile};
use genos_user_runtime as runtime;

const PATH: &[u8] = b"/MEMORY.STATUS";

#[repr(C)]
struct ProbeData {
    before: [u8; 512],
    first: [u8; 512],
    current: [u8; 512],
    chunk: [u8; 64],
    line: [u8; 80],
    stat: runtime::UserFileStat,
    child_status: runtime::UserProcessStatus,
    input: runtime::UserInputEvent,
}

#[repr(C)]
struct ProcessData {
    header: runtime::UserProcessHeader,
    probe: ProbeData,
}

#[used]
#[link_section = ".data.process"]
static mut DATA: ProcessData = ProcessData {
    header: runtime::UserProcessHeader::empty(),
    probe: ProbeData {
        before: [0; 512],
        first: [0; 512],
        current: [0; 512],
        chunk: [0; 64],
        line: [0; 80],
        stat: runtime::UserFileStat::empty(),
        child_status: runtime::UserProcessStatus::empty(),
        input: runtime::UserInputEvent::empty(),
    },
};
const _: () = assert!(core::mem::size_of::<ProcessData>() <= 4096);

fn emit(console: u64, text: &[u8]) -> bool {
    runtime::console_write(console, text, runtime::CONSOLE_LINE_STATUS) == text.len() as u64
}

fn valid_handle(value: u64) -> bool {
    value != 0 && value < runtime::ERROR_TIMED_OUT
}

fn read_tail(
    handle: u64,
    output: &mut [u8; 512],
    mut used: usize,
    chunk: &mut [u8; 64],
) -> Option<usize> {
    for _ in 0..=512 {
        let count = runtime::read_handle(handle, &mut chunk[..7]);
        if count == 0 {
            return Some(used);
        }
        if count > 7 || used + count as usize > output.len() {
            return None;
        }
        output[used..used + count as usize].copy_from_slice(&chunk[..count as usize]);
        used += count as usize;
    }
    None
}

fn capture(output: &mut [u8; 512], chunk: &mut [u8; 64]) -> Option<usize> {
    let handle = runtime::open_file(PATH);
    if !valid_handle(handle) {
        return None;
    }
    let read = read_tail(handle, output, 0, chunk);
    if runtime::close_handle(handle) != 0 {
        return None;
    }
    read
}

fn live_frames(report: &[u8]) -> Option<u64> {
    let start = report.windows(6).position(|bytes| bytes == b" live=")? + 6;
    let mut value = 0u64;
    let mut count = 0;
    for byte in &report[start..] {
        if !byte.is_ascii_digit() {
            break;
        }
        value = value
            .checked_mul(10)?
            .checked_add(u64::from(*byte - b'0'))?;
        count += 1;
    }
    (count != 0).then_some(value)
}

fn emit_live(console: u64, line: &mut [u8; 80], prefix: &[u8], value: u64) -> bool {
    line[..prefix.len()].copy_from_slice(prefix);
    let mut number = value;
    let mut digits = [0u8; 20];
    let mut len = 0;
    loop {
        digits[len] = b'0' + (number % 10) as u8;
        len += 1;
        number /= 10;
        if number == 0 {
            break;
        }
    }
    for i in 0..len {
        line[prefix.len() + i] = digits[len - i - 1];
    }
    emit(console, &line[..prefix.len() + len])
}

fn probe(data: &mut ProbeData, console: u64, supervisor: u64) -> Option<()> {
    if !emit(console, b"MEMORY_SNAPSHOT_BEGIN") {
        return None;
    }
    let first = runtime::open_file(PATH);
    if !valid_handle(first)
        || runtime::stat_handle(first, &mut data.stat)
            != core::mem::size_of::<runtime::UserFileStat>() as u64
        || data.stat.offset != 0
        || data.stat.rights != runtime::FILE_RIGHT_READ
    {
        return None;
    }
    let original_size = data.stat.size;
    if runtime::read_handle(first, &mut data.chunk[..7]) != 7 {
        return None;
    }
    data.first[..7].copy_from_slice(&data.chunk[..7]);
    let before_len = capture(&mut data.before, &mut data.chunk)?;
    let before = live_frames(&data.before[..before_len])?;
    if original_size != before_len as u64
        || !emit_live(
            console,
            &mut data.line,
            b"MEMORY_SNAPSHOT_LIVE phase=before frames=",
            before,
        )
    {
        return None;
    }

    let child = runtime::process_launch(
        supervisor,
        runtime::PROCESS_IMAGE_INIT,
        runtime::PROCESS_MODE_HOLD,
    );
    if !valid_handle(child) {
        return None;
    }
    let during_len = capture(&mut data.current, &mut data.chunk)?;
    let during = live_frames(&data.current[..during_len])?;
    if during <= before
        || data.current[..during_len] == data.before[..before_len]
        || !emit_live(
            console,
            &mut data.line,
            b"MEMORY_SNAPSHOT_LIVE phase=during frames=",
            during,
        )
    {
        return None;
    }
    if runtime::stat_handle(first, &mut data.stat)
        != core::mem::size_of::<runtime::UserFileStat>() as u64
        || data.stat.size != original_size
        || data.stat.offset != 7
    {
        return None;
    }
    let first_len = read_tail(first, &mut data.first, 7, &mut data.chunk)?;
    if first_len != before_len
        || data.first[..first_len] != data.before[..before_len]
        || runtime::stat_handle(first, &mut data.stat)
            != core::mem::size_of::<runtime::UserFileStat>() as u64
        || data.stat.size != original_size
        || data.stat.offset != original_size
        || runtime::read_handle(first, &mut data.chunk) != 0
        || !emit(console, b"MEMORY_SNAPSHOT_FIRST_STABLE")
    {
        return None;
    }
    if runtime::close_handle(first) != 0
        || runtime::read_handle(first, &mut data.chunk) != runtime::ERROR_INVALID_ARGUMENT
        || runtime::stat_handle(first, &mut data.stat) != runtime::ERROR_INVALID_ARGUMENT
        || runtime::process_kill(child) != 0
        || runtime::process_reap(child, &mut data.child_status)
            != core::mem::size_of::<runtime::UserProcessStatus>() as u64
        || data.child_status.state != runtime::PROCESS_KILLED
    {
        return None;
    }
    let after_len = capture(&mut data.current, &mut data.chunk)?;
    let after = live_frames(&data.current[..after_len])?;
    if after != before
        || !emit_live(
            console,
            &mut data.line,
            b"MEMORY_SNAPSHOT_LIVE phase=after frames=",
            after,
        )
    {
        return None;
    }

    // Fill the complete file budget, reject one extra open, close/reuse a slot,
    // and repeat. Leaked snapshot slots cannot survive these two cycles.
    for _ in 0..2 {
        let mut handles = [0; runtime::FILE_HANDLE_CAPACITY as usize];
        for handle in &mut handles {
            *handle = runtime::open_file(PATH);
            if !valid_handle(*handle) {
                return None;
            }
        }
        if runtime::open_file(PATH) != runtime::ERROR_UNAVAILABLE {
            return None;
        }
        let stale = handles[0];
        if runtime::close_handle(stale) != 0 {
            return None;
        }
        handles[0] = runtime::open_file(PATH);
        if !valid_handle(handles[0])
            || handles[0] == stale
            || runtime::read_handle(stale, &mut data.chunk) != runtime::ERROR_INVALID_ARGUMENT
        {
            return None;
        }
        for handle in handles {
            let count = runtime::read_handle(handle, &mut data.chunk);
            if count == 0
                || count > data.chunk.len() as u64
                || !data.chunk[..count as usize].starts_with(b"memory allocator\n")
                || runtime::close_handle(handle) != 0
            {
                return None;
            }
        }
    }
    if !emit(console, b"MEMORY_SNAPSHOT_HANDLES_REUSED") || !emit(console, b"MEMORY_SNAPSHOT_READY")
    {
        return None;
    }
    Some(())
}

#[no_mangle]
pub extern "C" fn _start(console: u64, supervisor: u64) -> ! {
    // SAFETY: this sole process entry initializes only its header token; the
    // kernel updates the disjoint preemption word when returning to Ring 3.
    unsafe {
        write_volatile(addr_of_mut!(DATA.header.token), console);
    }
    // SAFETY: this single-threaded fixture owns the separate probe payload.
    // All syscall output buffers live in its mapped data page. The kernel's
    // asynchronous preemption writes touch only the disjoint header above.
    let data = unsafe { &mut *addr_of_mut!(DATA.probe) };
    if probe(data, console, supervisor).is_none() {
        let _ = emit(console, b"MEMORY_SNAPSHOT_FAILED");
        runtime::exit(1);
    }
    loop {
        if runtime::wait_input(&mut data.input, runtime::INPUT_MASK_KEYBOARD)
            != core::mem::size_of::<runtime::UserInputEvent>() as u64
        {
            runtime::exit(2);
        }
    }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    runtime::exit(250)
}
