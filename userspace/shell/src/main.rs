#![no_std]
#![no_main]

use core::panic::PanicInfo;
use core::ptr::{addr_of, addr_of_mut, read_volatile, write_volatile};

use genos_user_runtime as runtime;

const LINE_CAPACITY: usize = runtime::CONSOLE_TEXT_MAX;
const READY: &[u8] = b"SHELL.ELF ready - filesystem, network, and process control run in Ring 3";
const HELP: &[u8] =
    b"help clear echo uname net ls cat stat touch write append mkdir rm run ps kill wait";
const UNAME: &[u8] = b"GenOS v0.42 ring3-shell x86_64 ABI 15";
const UNKNOWN: &[u8] = b"unknown userspace command";
const DIRECTORY_ERROR: &[u8] = b"directory unavailable";
const FILE_ERROR: &[u8] = b"file unavailable";
const MUTATION_ERROR: &[u8] = b"file change denied; use /USER/FILE";
const MUTATION_PROOF_PATH: &[u8] = b"/USER/SHELL.TXT";
const MUTATION_PROOF_FIRST: &[u8] = b"Ring 3 shell file mutation";
const MUTATION_PROOF_APPEND: &[u8] = b" is ready.";
const MUTATION_PROOF_EXPECTED: &[u8] = b"Ring 3 shell file mutation is ready.";
const JOB_CAPACITY: usize = runtime::PROCESS_HANDLE_CAPACITY as usize;
const HISTORY_CAPACITY: usize = 8;
const NAMESPACE_PROOF_DIRECTORY: &[u8] = b"ABI14";

#[derive(Clone, Copy, Eq, PartialEq)]
enum StorageMode {
    Writable,
    ReadOnly,
    Unavailable,
}

#[derive(Clone, Copy)]
#[repr(C)]
struct Job {
    id: u64,
    handle: u64,
}

impl Job {
    const fn empty() -> Self {
        Self { id: 0, handle: 0 }
    }
}

#[repr(C)]
struct ShellData {
    header: runtime::UserProcessHeader,
    event: runtime::UserInputEvent,
    system_info: runtime::UserSystemInfo,
    network_config: runtime::UserNetworkConfig,
    file_stat: runtime::UserFileStat,
    directory_entry: runtime::UserDirectoryEntry,
    process_status: runtime::UserProcessStatus,
    jobs: [Job; JOB_CAPACITY],
    next_job_id: u64,
    file_buffer: [u8; runtime::FILE_READ_MAX],
    line: [u8; LINE_CAPACITY],
    len: usize,
    history: [[u8; LINE_CAPACITY]; HISTORY_CAPACITY],
    history_lens: [usize; HISTORY_CAPACITY],
    history_len: usize,
    history_cursor: usize,
}

#[used]
#[link_section = ".data.process"]
static mut DATA: ShellData = ShellData {
    header: runtime::UserProcessHeader::empty(),
    event: runtime::UserInputEvent::empty(),
    system_info: runtime::UserSystemInfo::empty(),
    network_config: runtime::UserNetworkConfig::empty(),
    file_stat: runtime::UserFileStat::empty(),
    directory_entry: runtime::UserDirectoryEntry::empty(),
    process_status: runtime::UserProcessStatus::empty(),
    jobs: [Job::empty(); JOB_CAPACITY],
    next_job_id: 1,
    file_buffer: [0; runtime::FILE_READ_MAX],
    line: [0; LINE_CAPACITY],
    len: 0,
    history: [[0; LINE_CAPACITY]; HISTORY_CAPACITY],
    history_lens: [0; HISTORY_CAPACITY],
    history_len: 0,
    history_cursor: 0,
};

#[no_mangle]
pub extern "C" fn _start(console: u64, supervisor: u64) -> ! {
    unsafe {
        write_volatile(addr_of_mut!(DATA.header.token), console);
    }
    if runtime::ping() != runtime::PING_REPLY || runtime::abi_version() != runtime::ABI_VERSION {
        runtime::exit(255);
    }
    while unsafe { read_volatile(addr_of!(DATA.header.preemptions)) } == 0 {
        core::hint::spin_loop();
    }
    let info = unsafe { &mut *addr_of_mut!(DATA.system_info) };
    if runtime::system_info(info) != core::mem::size_of::<runtime::UserSystemInfo>() as u64
        || info.image_layout_version != runtime::IMAGE_LAYOUT_VERSION
        || info.executable_page_capacity != runtime::EXECUTABLE_PAGE_CAPACITY
    {
        runtime::exit(246);
    }
    let root = runtime::open_file(b"/");
    if handle_error(root) {
        runtime::exit(249);
    }
    let entry = unsafe { &mut *addr_of_mut!(DATA.directory_entry) };
    if runtime::read_directory(root, 0, entry)
        != core::mem::size_of::<runtime::UserDirectoryEntry>() as u64
        || runtime::close_handle(root) != 0
    {
        runtime::exit(249);
    }
    let Some(storage_mode) = prove_storage_status(console) else {
        runtime::exit(243);
    };
    match storage_mode {
        StorageMode::ReadOnly => {
            if !prove_read_only_storage(console) {
                runtime::exit(248);
            }
        }
        StorageMode::Writable | StorageMode::Unavailable => {
            if !prove_file_mutation(console, storage_mode == StorageMode::Writable) {
                runtime::exit(248);
            }
            if !prove_namespace_mutation() {
                runtime::exit(245);
            }
        }
    }
    if !prove_history() {
        runtime::exit(244);
    }
    if !prove_network(console) {
        runtime::exit(242);
    }
    if !prove_process_control(supervisor) {
        runtime::exit(247);
    }
    if runtime::console_write(console, READY, runtime::CONSOLE_LINE_STATUS) != READY.len() as u64 {
        runtime::exit(254);
    }

    loop {
        let event = unsafe { &mut *addr_of_mut!(DATA.event) };
        if runtime::wait_input(event, runtime::INPUT_MASK_KEYBOARD)
            != core::mem::size_of::<runtime::UserInputEvent>() as u64
        {
            runtime::exit(253);
        }
        if event.kind != runtime::INPUT_KIND_KEY {
            continue;
        }
        match event.code {
            runtime::KEY_CHAR if (0x20..=0x7e).contains(&event.value0) => push(event.value0 as u8),
            runtime::KEY_BACKSPACE => backspace(),
            runtime::KEY_ENTER => execute(console, supervisor),
            runtime::KEY_ARROW_UP => history_up(),
            runtime::KEY_ARROW_DOWN => history_down(),
            _ => continue,
        }
        let line = unsafe { &DATA.line[..DATA.len] };
        if runtime::console_set_input(console, line) != line.len() as u64 {
            runtime::exit(252);
        }
    }
}

fn prove_storage_status(console: u64) -> Option<StorageMode> {
    if runtime::open_file_with_rights(
        b"/STORAGE.STATUS",
        runtime::FILE_RIGHT_READ | runtime::FILE_RIGHT_WRITE,
    ) != runtime::ERROR_INVALID_ARGUMENT
    {
        return None;
    }
    let handle = runtime::open_file(b"/STORAGE.STATUS");
    if handle_error(handle) {
        return None;
    }
    let buffer = unsafe { &mut *addr_of_mut!(DATA.file_buffer) };
    let read = runtime::read_handle(handle, buffer) as usize;
    if read > buffer.len() || runtime::close_handle(handle) != 0 {
        return None;
    }
    let status = &buffer[..read];
    let (message, storage_mode) = if matches(status, b"state=error") {
        (
            b"storage failure visible" as &[u8],
            StorageMode::Unavailable,
        )
    } else if matches(status, b"state=readonly") {
        (b"storage read-only visible" as &[u8], StorageMode::ReadOnly)
    } else if matches(status, b"state=healthy") || matches(status, b"state=recovered") {
        (b"storage status visible" as &[u8], StorageMode::Writable)
    } else {
        return None;
    };
    if runtime::console_write(console, message, runtime::CONSOLE_LINE_STATUS)
        != message.len() as u64
    {
        return None;
    }

    let temp = runtime::open_file(b"/TMP/SESSION.TXT");
    if handle_error(temp) {
        return None;
    }
    let read = runtime::read_handle(temp, buffer) as usize;
    if read > buffer.len()
        || &buffer[..read] != b"session-only RAM data"
        || runtime::close_handle(temp) != 0
    {
        return None;
    }
    let message = b"RAMFS temp visible";
    (runtime::console_write(console, message, runtime::CONSOLE_LINE_STATUS) == message.len() as u64)
        .then_some(storage_mode)
}

fn prove_network(console: u64) -> bool {
    let config = unsafe { &mut *addr_of_mut!(DATA.network_config) };
    let configured = runtime::network_config(config);
    if configured == runtime::ERROR_UNAVAILABLE {
        return true;
    }
    if configured != core::mem::size_of::<runtime::UserNetworkConfig>() as u64
        || config.address == 0
        || config.gateway == 0
        || config.dns == 0
    {
        return false;
    }

    let query = [
        0x47, 0x45, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x07, b'e', b'x',
        b'a', b'm', b'p', b'l', b'e', 0x03, b'c', b'o', b'm', 0x00, 0x00, 0x01, 0x00, 0x01,
    ];
    let buffer = unsafe { &mut *addr_of_mut!(DATA.file_buffer) };
    let dns_len = runtime::udp_exchange(config.dns, 53, &query, buffer);
    if handle_error(dns_len) || dns_len as usize > buffer.len() {
        return true;
    }
    let Some(_resolved) = dns_first_a(&buffer[..dns_len as usize]) else {
        return true;
    };
    let dns_message = b"network DNS resolved";
    if runtime::console_write(console, dns_message, runtime::CONSOLE_LINE_STATUS)
        != dns_message.len() as u64
    {
        return false;
    }

    let request = b"GET / HTTP/1.0\r\nHost: genos.test\r\nConnection: close\r\n\r\n";
    let http_len = runtime::tcp_exchange(config.gateway, 18080, request, buffer);
    if handle_error(http_len)
        || http_len as usize > buffer.len()
        || !buffer[..http_len as usize].starts_with(b"HTTP/1.0 200")
        || !contains_bytes(&buffer[..http_len as usize], b"GENOS_OK")
    {
        return true;
    }
    let http_message = b"network HTTP complete";
    if runtime::console_write(console, http_message, runtime::CONSOLE_LINE_STATUS)
        != http_message.len() as u64
    {
        return false;
    }

    if runtime::tcp_exchange(config.gateway, 1, request, buffer) != runtime::ERROR_UNAVAILABLE {
        return false;
    }
    let timeout_message = b"network timeout handled";
    if runtime::console_write(console, timeout_message, runtime::CONSOLE_LINE_STATUS)
        != timeout_message.len() as u64
    {
        return false;
    }
    let diagnostics = b"network diagnostics ready";
    runtime::console_write(console, diagnostics, runtime::CONSOLE_LINE_STATUS)
        == diagnostics.len() as u64
}

fn dns_first_a(response: &[u8]) -> Option<u32> {
    if response.len() < 12
        || response[0..2] != [0x47, 0x45]
        || response[2] & 0x80 == 0
        || u16::from_be_bytes([response[4], response[5]]) != 1
    {
        return None;
    }
    let answers = u16::from_be_bytes([response[6], response[7]]);
    let mut cursor = skip_dns_name(response, 12)?.checked_add(4)?;
    for _ in 0..answers {
        cursor = skip_dns_name(response, cursor)?;
        let header_end = cursor
            .checked_add(10)
            .filter(|end| *end <= response.len())?;
        let kind = u16::from_be_bytes([response[cursor], response[cursor + 1]]);
        let class = u16::from_be_bytes([response[cursor + 2], response[cursor + 3]]);
        let data_len = usize::from(u16::from_be_bytes([
            response[cursor + 8],
            response[cursor + 9],
        ]));
        let data_end = header_end
            .checked_add(data_len)
            .filter(|end| *end <= response.len())?;
        if kind == 1 && class == 1 && data_len == 4 {
            return Some(u32::from_be_bytes(
                response[header_end..data_end].try_into().ok()?,
            ));
        }
        cursor = data_end;
    }
    None
}

fn skip_dns_name(packet: &[u8], mut cursor: usize) -> Option<usize> {
    for _ in 0..128 {
        let length = *packet.get(cursor)?;
        cursor += 1;
        if length == 0 {
            return Some(cursor);
        }
        if length & 0xc0 == 0xc0 {
            packet.get(cursor)?;
            return Some(cursor + 1);
        }
        if length & 0xc0 != 0 || length > 63 {
            return None;
        }
        cursor = cursor
            .checked_add(length as usize)
            .filter(|end| *end <= packet.len())?;
    }
    None
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

fn push(byte: u8) {
    unsafe {
        if DATA.len < LINE_CAPACITY {
            DATA.line[DATA.len] = byte;
            DATA.len += 1;
        }
        DATA.history_cursor = DATA.history_len;
    }
}

fn backspace() {
    unsafe {
        DATA.len = DATA.len.saturating_sub(1);
        DATA.history_cursor = DATA.history_len;
    }
}

fn history_up() {
    unsafe {
        if DATA.history_len == 0 {
            return;
        }
        DATA.history_cursor = DATA.history_cursor.saturating_sub(1);
        let index = DATA.history_cursor;
        DATA.len = DATA.history_lens[index];
        DATA.line[..DATA.len].copy_from_slice(&DATA.history[index][..DATA.len]);
    }
}

fn history_down() {
    unsafe {
        if DATA.history_cursor + 1 < DATA.history_len {
            DATA.history_cursor += 1;
            let index = DATA.history_cursor;
            DATA.len = DATA.history_lens[index];
            DATA.line[..DATA.len].copy_from_slice(&DATA.history[index][..DATA.len]);
        } else {
            DATA.history_cursor = DATA.history_len;
            DATA.len = 0;
        }
    }
}

fn remember_history(line: &[u8]) {
    if line.is_empty() {
        return;
    }
    unsafe {
        let index = if DATA.history_len < HISTORY_CAPACITY {
            let index = DATA.history_len;
            DATA.history_len += 1;
            index
        } else {
            let mut index = 1usize;
            while index < HISTORY_CAPACITY {
                DATA.history[index - 1] = DATA.history[index];
                DATA.history_lens[index - 1] = DATA.history_lens[index];
                index += 1;
            }
            HISTORY_CAPACITY - 1
        };
        DATA.history[index] = [0; LINE_CAPACITY];
        DATA.history[index][..line.len()].copy_from_slice(line);
        DATA.history_lens[index] = line.len();
        DATA.history_cursor = DATA.history_len;
    }
}

fn prove_history() -> bool {
    remember_history(b"uname");
    remember_history(b"echo history");
    history_up();
    let newest = unsafe { matches(&DATA.line[..DATA.len], b"echo history") };
    history_up();
    let oldest = unsafe { matches(&DATA.line[..DATA.len], b"uname") };
    history_down();
    let forward = unsafe { matches(&DATA.line[..DATA.len], b"echo history") };
    history_down();
    let cleared = unsafe { DATA.len == 0 && DATA.history_cursor == DATA.history_len };
    unsafe {
        DATA.history = [[0; LINE_CAPACITY]; HISTORY_CAPACITY];
        DATA.history_lens = [0; HISTORY_CAPACITY];
        DATA.history_len = 0;
        DATA.history_cursor = 0;
    }
    newest && oldest && forward && cleared
}

fn execute(console: u64, supervisor: u64) {
    let len = unsafe { DATA.len };
    let mut command = [0u8; LINE_CAPACITY];
    unsafe { command[..len].copy_from_slice(&DATA.line[..len]) };
    let line = &command[..len];
    if len != 0 {
        remember_history(line);
        let mut prompt = [0u8; LINE_CAPACITY];
        prompt[0] = b'/';
        prompt[1] = b'>';
        prompt[2] = b' ';
        let copy = len.min(LINE_CAPACITY - 3);
        prompt[3..3 + copy].copy_from_slice(&line[..copy]);
        let _ = runtime::console_write(console, &prompt[..3 + copy], runtime::CONSOLE_LINE_PROMPT);
    }
    if matches(line, b"help") {
        let _ = runtime::console_write(console, HELP, runtime::CONSOLE_LINE_OUTPUT);
    } else if matches(line, b"uname") {
        let _ = runtime::console_write(console, UNAME, runtime::CONSOLE_LINE_OUTPUT);
    } else if matches(line, b"net") {
        network_status(console);
    } else if matches(line, b"clear") {
        let _ = runtime::console_clear(console);
    } else if line.len() >= 5 && matches(&line[..5], b"echo ") {
        let _ = runtime::console_write(console, &line[5..], runtime::CONSOLE_LINE_OUTPUT);
    } else if matches(line, b"ls") {
        list_directory(console, b"/");
    } else if line.len() > 3 && matches(&line[..3], b"ls ") {
        list_directory(console, &line[3..]);
    } else if line.len() > 4 && matches(&line[..4], b"cat ") {
        print_file(console, &line[4..]);
    } else if line.len() > 5 && matches(&line[..5], b"stat ") {
        stat_path(console, &line[5..]);
    } else if line.len() > 6 && matches(&line[..6], b"touch ") {
        touch_file(console, &line[6..]);
    } else if line.len() > 6 && matches(&line[..6], b"write ") {
        let (path, text) = split_argument(&line[6..]);
        if path.is_empty() || text.is_empty() {
            let _ = runtime::console_write(
                console,
                b"usage: write /USER/FILE TEXT",
                runtime::CONSOLE_LINE_ERROR,
            );
        } else {
            change_file(console, path, text, true);
        }
    } else if line.len() > 7 && matches(&line[..7], b"append ") {
        let (path, text) = split_argument(&line[7..]);
        if path.is_empty() || text.is_empty() {
            let _ = runtime::console_write(
                console,
                b"usage: append /USER/FILE TEXT",
                runtime::CONSOLE_LINE_ERROR,
            );
        } else {
            change_file(console, path, text, false);
        }
    } else if line.len() > 6 && matches(&line[..6], b"mkdir ") {
        mutate_namespace(console, &line[6..], true);
    } else if line.len() > 3 && matches(&line[..3], b"rm ") {
        mutate_namespace(console, &line[3..], false);
    } else if matches(line, b"run init") {
        launch_job(console, supervisor, runtime::PROCESS_MODE_NORMAL);
    } else if matches(line, b"run init hold") {
        launch_job(console, supervisor, runtime::PROCESS_MODE_HOLD);
    } else if matches(line, b"ps") {
        list_jobs(console);
    } else if line.len() > 5 && matches(&line[..5], b"kill ") {
        control_job(console, &line[5..], false);
    } else if line.len() > 5 && matches(&line[..5], b"wait ") {
        control_job(console, &line[5..], true);
    } else if !line.is_empty() {
        let _ = runtime::console_write(console, UNKNOWN, runtime::CONSOLE_LINE_ERROR);
    }
    unsafe {
        DATA.len = 0;
        DATA.history_cursor = DATA.history_len;
    }
}

fn network_status(console: u64) {
    let config = unsafe { &mut *addr_of_mut!(DATA.network_config) };
    let result = runtime::network_config(config);
    let message: &[u8] = if result == core::mem::size_of::<runtime::UserNetworkConfig>() as u64 {
        b"network online - DHCP configuration available"
    } else {
        b"network unavailable"
    };
    let _ = runtime::console_write(console, message, runtime::CONSOLE_LINE_OUTPUT);
}

fn prove_process_control(supervisor: u64) -> bool {
    let status = unsafe { &mut *addr_of_mut!(DATA.process_status) };
    if runtime::process_launch(
        supervisor ^ 1,
        runtime::PROCESS_IMAGE_INIT,
        runtime::PROCESS_MODE_HOLD,
    ) != runtime::ERROR_INVALID_ARGUMENT
    {
        return false;
    }
    let handle = runtime::process_launch(
        supervisor,
        runtime::PROCESS_IMAGE_INIT,
        runtime::PROCESS_MODE_HOLD,
    );
    if handle_error(handle)
        || runtime::process_status(handle ^ 1, status) != runtime::ERROR_INVALID_ARGUMENT
        || runtime::process_status(handle, status) != process_status_size()
        || status.state == runtime::PROCESS_EXITED
        || status.state == runtime::PROCESS_FAULTED
        || status.state == runtime::PROCESS_KILLED
        || runtime::process_reap(handle, status) != runtime::ERROR_UNAVAILABLE
        || runtime::process_kill(handle) != 0
        || runtime::process_status(handle, status) != process_status_size()
        || status.state != runtime::PROCESS_KILLED
        || status.exit_code != 137
        || runtime::process_reap(handle, status) != process_status_size()
        || runtime::process_status(handle, status) != runtime::ERROR_INVALID_ARGUMENT
    {
        return false;
    }
    true
}

fn launch_job(console: u64, supervisor: u64, mode: u64) {
    let jobs = unsafe { read_volatile(addr_of!(DATA.jobs)) };
    let slot = jobs.iter().position(|job| job.id == 0);
    let Some(slot) = slot else {
        let _ = runtime::console_write(console, b"job table full", runtime::CONSOLE_LINE_ERROR);
        return;
    };
    let handle = runtime::process_launch(supervisor, runtime::PROCESS_IMAGE_INIT, mode);
    if handle_error(handle) {
        let _ = runtime::console_write(
            console,
            b"process launch failed",
            runtime::CONSOLE_LINE_ERROR,
        );
        return;
    }
    let id = unsafe { read_volatile(addr_of!(DATA.next_job_id)) };
    unsafe {
        DATA.next_job_id = DATA.next_job_id.saturating_add(1);
        DATA.jobs[slot] = Job { id, handle };
    }
    let status = unsafe { &mut *addr_of_mut!(DATA.process_status) };
    let mut output = [0u8; LINE_CAPACITY];
    let mut len = 0usize;
    append(&mut output, &mut len, b"job ");
    append_u64(&mut output, &mut len, id);
    if runtime::process_status(handle, status) == process_status_size() {
        append(&mut output, &mut len, b" started task=");
        append_u64(&mut output, &mut len, status.task_id);
        append(&mut output, &mut len, b" pid=");
        append_u64(&mut output, &mut len, status.runtime_pid);
    } else {
        append(&mut output, &mut len, b" started");
    }
    let _ = runtime::console_write(console, &output[..len], runtime::CONSOLE_LINE_STATUS);
}

fn list_jobs(console: u64) {
    let mut any = false;
    let jobs = unsafe { read_volatile(addr_of!(DATA.jobs)) };
    for job in jobs.iter().filter(|job| job.id != 0) {
        any = true;
        let status = unsafe { &mut *addr_of_mut!(DATA.process_status) };
        let result = runtime::process_status(job.handle, status);
        let mut output = [0u8; LINE_CAPACITY];
        let mut len = 0usize;
        append(&mut output, &mut len, b"job ");
        append_u64(&mut output, &mut len, job.id);
        if result == process_status_size() {
            append(&mut output, &mut len, b" task=");
            append_u64(&mut output, &mut len, status.task_id);
            append(&mut output, &mut len, b" pid=");
            append_u64(&mut output, &mut len, status.runtime_pid);
            append(&mut output, &mut len, b" ");
            append(&mut output, &mut len, process_state(status.state));
        } else {
            append(&mut output, &mut len, b" unavailable");
        }
        let _ = runtime::console_write(console, &output[..len], runtime::CONSOLE_LINE_OUTPUT);
    }
    if !any {
        let _ = runtime::console_write(console, b"no jobs", runtime::CONSOLE_LINE_OUTPUT);
    }
}

fn control_job(console: u64, argument: &[u8], reap: bool) {
    let Some(id) = parse_u64(argument) else {
        let _ = runtime::console_write(console, b"job id required", runtime::CONSOLE_LINE_ERROR);
        return;
    };
    let jobs = unsafe { read_volatile(addr_of!(DATA.jobs)) };
    let slot = jobs.iter().position(|job| job.id == id);
    let Some(slot) = slot else {
        let _ = runtime::console_write(console, b"job not found", runtime::CONSOLE_LINE_ERROR);
        return;
    };
    let handle = unsafe { DATA.jobs[slot].handle };
    let result = if reap {
        let status = unsafe { &mut *addr_of_mut!(DATA.process_status) };
        runtime::process_reap(handle, status)
    } else {
        runtime::process_kill(handle)
    };
    if reap && result == process_status_size() {
        unsafe { DATA.jobs[slot] = Job::empty() };
        write_job_result(console, b"reaped job ", id, runtime::CONSOLE_LINE_STATUS);
    } else if !reap && result == 0 {
        write_job_result(console, b"killed job ", id, runtime::CONSOLE_LINE_STATUS);
    } else if result == runtime::ERROR_UNAVAILABLE {
        let text = if reap {
            b"job still running" as &[u8]
        } else {
            b"job already finished"
        };
        let _ = runtime::console_write(console, text, runtime::CONSOLE_LINE_ERROR);
    } else {
        let _ = runtime::console_write(console, b"job control failed", runtime::CONSOLE_LINE_ERROR);
    }
}

fn write_job_result(console: u64, prefix: &[u8], id: u64, kind: u64) {
    let mut output = [0u8; LINE_CAPACITY];
    let mut len = 0usize;
    append(&mut output, &mut len, prefix);
    append_u64(&mut output, &mut len, id);
    let _ = runtime::console_write(console, &output[..len], kind);
}

fn process_status_size() -> u64 {
    core::mem::size_of::<runtime::UserProcessStatus>() as u64
}

fn process_state(state: u64) -> &'static [u8] {
    match state {
        runtime::PROCESS_READY => b"ready",
        runtime::PROCESS_SLEEPING => b"sleeping",
        runtime::PROCESS_WAITING => b"waiting",
        runtime::PROCESS_EXITED => b"exited",
        runtime::PROCESS_FAULTED => b"fault",
        runtime::PROCESS_KILLED => b"killed",
        _ => b"unknown",
    }
}

fn parse_u64(bytes: &[u8]) -> Option<u64> {
    let mut start = 0usize;
    let mut end = bytes.len();
    while start < end && bytes[start] == b' ' {
        start += 1;
    }
    while end > start && bytes[end - 1] == b' ' {
        end -= 1;
    }
    let bytes = &bytes[start..end];
    if bytes.is_empty() {
        return None;
    }
    let mut value = 0u64;
    for byte in bytes {
        if !byte.is_ascii_digit() {
            return None;
        }
        value = value.checked_mul(10)?.checked_add((byte - b'0') as u64)?;
    }
    (value != 0).then_some(value)
}

fn prove_file_mutation(console: u64, persistent_available: bool) -> bool {
    let existing = runtime::open_file(MUTATION_PROOF_PATH);
    let restored = if !handle_error(existing) {
        let buffer = unsafe { &mut *addr_of_mut!(DATA.file_buffer) };
        let count = runtime::read_handle(existing, buffer);
        let restored = count == MUTATION_PROOF_EXPECTED.len() as u64
            && &buffer[..count as usize] == MUTATION_PROOF_EXPECTED
            && runtime::close_handle(existing) == 0;
        if !restored {
            return false;
        }
        true
    } else if existing == runtime::ERROR_UNAVAILABLE {
        false
    } else {
        return false;
    };
    let handle = runtime::open_file_with_rights(
        MUTATION_PROOF_PATH,
        runtime::FILE_RIGHT_READ | runtime::FILE_RIGHT_WRITE,
    );
    if handle_error(handle) {
        return false;
    }
    let mutated = runtime::truncate_handle(handle) == 0
        && runtime::write_handle(handle, MUTATION_PROOF_FIRST) == MUTATION_PROOF_FIRST.len() as u64;
    let closed = runtime::close_handle(handle) == 0;
    if !mutated || !closed {
        return false;
    }
    let handle = runtime::open_file_with_rights(
        MUTATION_PROOF_PATH,
        runtime::FILE_RIGHT_READ | runtime::FILE_RIGHT_WRITE,
    );
    if handle_error(handle) {
        return false;
    }
    let appended = advance_to_end(handle)
        && runtime::write_handle(handle, MUTATION_PROOF_APPEND)
            == MUTATION_PROOF_APPEND.len() as u64;
    let closed = runtime::close_handle(handle) == 0;
    if !appended || !closed {
        return false;
    }
    let handle = runtime::open_file(MUTATION_PROOF_PATH);
    if handle_error(handle) {
        return false;
    }
    if runtime::truncate_handle(handle) != runtime::ERROR_INVALID_ARGUMENT {
        let _ = runtime::close_handle(handle);
        return false;
    }
    let buffer = unsafe { &mut *addr_of_mut!(DATA.file_buffer) };
    let count = runtime::read_handle(handle, buffer);
    let verified = count == MUTATION_PROOF_EXPECTED.len() as u64
        && &buffer[..count as usize] == MUTATION_PROOF_EXPECTED;
    let closed = runtime::close_handle(handle) == 0;
    let message: &[u8] = if restored && persistent_available {
        b"durable file restored" as &[u8]
    } else if persistent_available {
        b"durable file committed"
    } else {
        b"session file written"
    };
    verified
        && closed
        && runtime::console_write(console, message, runtime::CONSOLE_LINE_STATUS)
            == message.len() as u64
}

fn prove_read_only_storage(console: u64) -> bool {
    let handle = runtime::open_file(MUTATION_PROOF_PATH);
    if handle_error(handle) {
        return false;
    }
    let buffer = unsafe { &mut *addr_of_mut!(DATA.file_buffer) };
    let count = runtime::read_handle(handle, buffer);
    let restored = count == MUTATION_PROOF_EXPECTED.len() as u64
        && &buffer[..count as usize] == MUTATION_PROOF_EXPECTED
        && runtime::close_handle(handle) == 0;
    if !restored {
        return false;
    }

    let writable = runtime::open_file_with_rights(
        MUTATION_PROOF_PATH,
        runtime::FILE_RIGHT_READ | runtime::FILE_RIGHT_WRITE,
    );
    let manageable = runtime::open_file_with_rights(
        b"/USER",
        runtime::FILE_RIGHT_READ | runtime::FILE_RIGHT_MANAGE,
    );
    let message = b"durable file restored read-only";
    writable == runtime::ERROR_UNAVAILABLE
        && manageable == runtime::ERROR_UNAVAILABLE
        && runtime::console_write(console, message, runtime::CONSOLE_LINE_STATUS)
            == message.len() as u64
}

fn prove_namespace_mutation() -> bool {
    let parent = runtime::open_file_with_rights(
        b"/USER",
        runtime::FILE_RIGHT_READ | runtime::FILE_RIGHT_MANAGE,
    );
    if handle_error(parent) {
        return false;
    }
    let stat = unsafe { &mut *addr_of_mut!(DATA.file_stat) };
    let valid_parent = runtime::stat_handle(parent, stat)
        == core::mem::size_of::<runtime::UserFileStat>() as u64
        && stat.kind == runtime::FILE_KIND_DIRECTORY
        && stat.rights & runtime::FILE_RIGHT_MANAGE != 0;
    let proved = valid_parent
        && runtime::create_directory(parent ^ 1, NAMESPACE_PROOF_DIRECTORY)
            == runtime::ERROR_INVALID_ARGUMENT
        && runtime::create_directory(parent, NAMESPACE_PROOF_DIRECTORY) == 0
        && runtime::remove_path(parent, NAMESPACE_PROOF_DIRECTORY) == 0
        && runtime::remove_path(parent, NAMESPACE_PROOF_DIRECTORY) == runtime::ERROR_UNAVAILABLE;
    let closed = runtime::close_handle(parent) == 0;
    proved && closed
}

fn stat_path(console: u64, path: &[u8]) {
    let mut absolute = [0u8; runtime::PATH_MAX];
    let path = absolute_path(path, &mut absolute);
    let handle = if path.is_empty() {
        runtime::ERROR_INVALID_ARGUMENT
    } else {
        runtime::open_file(path)
    };
    if handle_error(handle) {
        let _ = runtime::console_write(console, FILE_ERROR, runtime::CONSOLE_LINE_ERROR);
        return;
    }
    let stat = unsafe { &mut *addr_of_mut!(DATA.file_stat) };
    if runtime::stat_handle(handle, stat) != core::mem::size_of::<runtime::UserFileStat>() as u64 {
        let _ = runtime::close_handle(handle);
        let _ = runtime::console_write(console, FILE_ERROR, runtime::CONSOLE_LINE_ERROR);
        return;
    }
    let mut output = [0u8; LINE_CAPACITY];
    let mut len = 0usize;
    append(&mut output, &mut len, path);
    append(
        &mut output,
        &mut len,
        if stat.kind == runtime::FILE_KIND_DIRECTORY {
            b" dir "
        } else {
            b" file "
        },
    );
    append_u64(&mut output, &mut len, stat.size);
    append(&mut output, &mut len, b" B");
    let closed = runtime::close_handle(handle) == 0;
    if closed {
        let _ = runtime::console_write(console, &output[..len], runtime::CONSOLE_LINE_OUTPUT);
    } else {
        let _ = runtime::console_write(console, FILE_ERROR, runtime::CONSOLE_LINE_ERROR);
    }
}

fn mutate_namespace(console: u64, path: &[u8], create: bool) {
    let mut absolute = [0u8; runtime::PATH_MAX];
    let path = absolute_path(path, &mut absolute);
    let Some((parent_path, name)) = split_parent(path) else {
        let _ = runtime::console_write(console, MUTATION_ERROR, runtime::CONSOLE_LINE_ERROR);
        return;
    };
    let parent = runtime::open_file_with_rights(
        parent_path,
        runtime::FILE_RIGHT_READ | runtime::FILE_RIGHT_MANAGE,
    );
    if handle_error(parent) {
        let _ = runtime::console_write(console, MUTATION_ERROR, runtime::CONSOLE_LINE_ERROR);
        return;
    }
    let result = if create {
        runtime::create_directory(parent, name)
    } else {
        runtime::remove_path(parent, name)
    };
    let closed = runtime::close_handle(parent) == 0;
    if result == 0 && closed {
        let status = if create {
            b"directory created" as &[u8]
        } else {
            b"path removed"
        };
        let _ = runtime::console_write(console, status, runtime::CONSOLE_LINE_STATUS);
    } else {
        let _ = runtime::console_write(console, MUTATION_ERROR, runtime::CONSOLE_LINE_ERROR);
    }
}

fn split_parent(path: &[u8]) -> Option<(&[u8], &[u8])> {
    let slash = path.iter().rposition(|byte| *byte == b'/')?;
    let name = path.get(slash + 1..)?;
    if name.is_empty() {
        return None;
    }
    let parent = if slash == 0 {
        b"/" as &[u8]
    } else {
        &path[..slash]
    };
    Some((parent, name))
}

fn touch_file(console: u64, path: &[u8]) {
    let mut absolute = [0u8; runtime::PATH_MAX];
    let path = absolute_path(path, &mut absolute);
    let handle = if path.is_empty() {
        runtime::ERROR_INVALID_ARGUMENT
    } else {
        runtime::open_file_with_rights(path, runtime::FILE_RIGHT_READ | runtime::FILE_RIGHT_WRITE)
    };
    if handle_error(handle) || runtime::close_handle(handle) != 0 {
        let _ = runtime::console_write(console, MUTATION_ERROR, runtime::CONSOLE_LINE_ERROR);
    } else {
        let _ = runtime::console_write(console, b"file touched", runtime::CONSOLE_LINE_STATUS);
    }
}

fn change_file(console: u64, path: &[u8], text: &[u8], truncate: bool) {
    let mut absolute = [0u8; runtime::PATH_MAX];
    let path = absolute_path(path, &mut absolute);
    let handle = if path.is_empty() {
        runtime::ERROR_INVALID_ARGUMENT
    } else {
        runtime::open_file_with_rights(path, runtime::FILE_RIGHT_READ | runtime::FILE_RIGHT_WRITE)
    };
    if handle_error(handle) {
        let _ = runtime::console_write(console, MUTATION_ERROR, runtime::CONSOLE_LINE_ERROR);
        return;
    }
    let prepared = if truncate {
        runtime::truncate_handle(handle) == 0
    } else {
        advance_to_end(handle)
    };
    let written = prepared && runtime::write_handle(handle, text) == text.len() as u64;
    let closed = runtime::close_handle(handle) == 0;
    if written && closed {
        let status = if truncate {
            b"file written" as &[u8]
        } else {
            b"file appended" as &[u8]
        };
        let _ = runtime::console_write(console, status, runtime::CONSOLE_LINE_STATUS);
    } else {
        let _ = runtime::console_write(console, FILE_ERROR, runtime::CONSOLE_LINE_ERROR);
    }
}

fn advance_to_end(handle: u64) -> bool {
    loop {
        let buffer = unsafe { &mut *addr_of_mut!(DATA.file_buffer) };
        let count = runtime::read_handle(handle, buffer);
        if handle_error(count) {
            return false;
        }
        if count == 0 {
            return true;
        }
    }
}

fn split_argument(input: &[u8]) -> (&[u8], &[u8]) {
    let Some(space) = input.iter().position(|byte| *byte == b' ') else {
        return (input, &[]);
    };
    let mut text_start = space;
    while input.get(text_start) == Some(&b' ') {
        text_start += 1;
    }
    (&input[..space], &input[text_start..])
}

fn list_directory(console: u64, path: &[u8]) {
    let mut absolute = [0u8; runtime::PATH_MAX];
    let path = absolute_path(path, &mut absolute);
    if path.is_empty() {
        let _ = runtime::console_write(console, DIRECTORY_ERROR, runtime::CONSOLE_LINE_ERROR);
        return;
    }
    let handle = runtime::open_file(path);
    if handle_error(handle) {
        let _ = runtime::console_write(console, DIRECTORY_ERROR, runtime::CONSOLE_LINE_ERROR);
        return;
    }
    let mut cursor = 0u64;
    loop {
        let entry = unsafe { &mut *addr_of_mut!(DATA.directory_entry) };
        let result = runtime::read_directory(handle, cursor, entry);
        if result == 0 {
            break;
        }
        if result != core::mem::size_of::<runtime::UserDirectoryEntry>() as u64
            || entry.name_length == 0
            || entry.name_length as usize > runtime::DIRECTORY_NAME_MAX
        {
            let _ = runtime::console_write(console, DIRECTORY_ERROR, runtime::CONSOLE_LINE_ERROR);
            break;
        }
        let mut output = [0u8; LINE_CAPACITY];
        let name_len = entry.name_length as usize;
        output[..name_len].copy_from_slice(&entry.name[..name_len]);
        let mut len = name_len;
        if entry.kind == runtime::FILE_KIND_DIRECTORY && len < output.len() {
            output[len] = b'/';
            len += 1;
        } else if entry.kind == runtime::FILE_KIND_REGULAR {
            append(&mut output, &mut len, b"  ");
            append_u64(&mut output, &mut len, entry.size);
            append(&mut output, &mut len, b" B");
        }
        let _ = runtime::console_write(console, &output[..len], runtime::CONSOLE_LINE_OUTPUT);
        cursor = cursor.saturating_add(1);
    }
    let _ = runtime::close_handle(handle);
}

fn print_file(console: u64, path: &[u8]) {
    let mut absolute = [0u8; runtime::PATH_MAX];
    let path = absolute_path(path, &mut absolute);
    if path.is_empty() {
        let _ = runtime::console_write(console, FILE_ERROR, runtime::CONSOLE_LINE_ERROR);
        return;
    }
    let handle = runtime::open_file(path);
    if handle_error(handle) {
        let _ = runtime::console_write(console, FILE_ERROR, runtime::CONSOLE_LINE_ERROR);
        return;
    }
    let mut output = [0u8; LINE_CAPACITY];
    let mut output_len = 0usize;
    loop {
        let buffer = unsafe { &mut *addr_of_mut!(DATA.file_buffer) };
        let count = runtime::read_handle(handle, buffer);
        if count == runtime::ERROR_INVALID_ARGUMENT || count == runtime::ERROR_UNAVAILABLE {
            let _ = runtime::console_write(console, FILE_ERROR, runtime::CONSOLE_LINE_ERROR);
            break;
        }
        if count == 0 {
            if output_len != 0 {
                let _ = runtime::console_write(
                    console,
                    &output[..output_len],
                    runtime::CONSOLE_LINE_OUTPUT,
                );
            }
            break;
        }
        for byte in buffer.iter().take(count as usize) {
            if *byte == b'\n' || output_len == output.len() {
                if output_len != 0 {
                    let _ = runtime::console_write(
                        console,
                        &output[..output_len],
                        runtime::CONSOLE_LINE_OUTPUT,
                    );
                    output_len = 0;
                }
                if *byte == b'\n' {
                    continue;
                }
            }
            output[output_len] = *byte;
            output_len += 1;
        }
    }
    let _ = runtime::close_handle(handle);
}

fn handle_error(value: u64) -> bool {
    value == runtime::ERROR_INVALID_ARGUMENT || value == runtime::ERROR_UNAVAILABLE
}

fn absolute_path<'a>(path: &'a [u8], output: &'a mut [u8; runtime::PATH_MAX]) -> &'a [u8] {
    if path.first() == Some(&b'/') {
        return path;
    }
    if path.is_empty() || path.len() >= output.len() {
        return &[];
    }
    output[0] = b'/';
    output[1..1 + path.len()].copy_from_slice(path);
    &output[..1 + path.len()]
}

fn append(output: &mut [u8; LINE_CAPACITY], len: &mut usize, bytes: &[u8]) {
    for byte in bytes {
        if *len == output.len() {
            return;
        }
        output[*len] = *byte;
        *len += 1;
    }
}

fn append_u64(output: &mut [u8; LINE_CAPACITY], len: &mut usize, mut value: u64) {
    let mut digits = [0u8; 20];
    let mut count = 0usize;
    loop {
        digits[count] = b'0' + (value % 10) as u8;
        count += 1;
        value /= 10;
        if value == 0 {
            break;
        }
    }
    while count != 0 {
        count -= 1;
        append(output, len, &digits[count..count + 1]);
    }
}

fn matches(actual: &[u8], expected: &[u8]) -> bool {
    actual.len() == expected.len() && actual.iter().zip(expected).all(|(a, b)| a == b)
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    runtime::exit(250)
}
