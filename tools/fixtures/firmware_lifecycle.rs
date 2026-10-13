// Injected only into a retained private copy of pinned uefi 0.36.1 boot.rs.
// The real firmware table is never modified. Production retry/reset behavior
// remains unchanged. The harness replaces only these three mode constants.
const GENOS_FIXTURE_MODE: u8 = 0;
const GENOS_FIXTURE_STAGE: u8 = 0;
const GENOS_FIXTURE_GROW_PAGES: usize = 0;

static GENOS_FIXTURE_ATTEMPTS: core::sync::atomic::AtomicUsize =
    core::sync::atomic::AtomicUsize::new(0);
static GENOS_FIXTURE_MAP_CALLS: core::sync::atomic::AtomicUsize =
    core::sync::atomic::AtomicUsize::new(0);
static GENOS_FIXTURE_GROWN: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);
static GENOS_FIXTURE_ARMED: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);
static GENOS_FIXTURE_UART: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);

fn genos_fixture_emit(bytes: &[u8]) {
    // SAFETY: this x86_64-only disposable loader runs on the admitted firmware
    // BSP. COM1 port operations have no Rust memory aliases, perform bounded
    // polling and do not call firmware/allocate. AL/DX are the declared clobbers.
    // A missing UART halts rather than emitting a partial successful proof.
    unsafe {
        if !GENOS_FIXTURE_UART.swap(true, Ordering::SeqCst) {
            for (port, value) in [
                (0x3f9u16, 0u8),
                (0x3fb, 0x80),
                (0x3f8, 3),
                (0x3f9, 0),
                (0x3fb, 3),
                (0x3fa, 0xc7),
                (0x3fc, 0x0b),
            ] {
                core::arch::asm!("out dx, al", in("dx") port, in("al") value, options(nomem, nostack));
            }
        }
        for &byte in bytes {
            let mut ready = false;
            for _ in 0..100_000 {
                let status: u8;
                core::arch::asm!("in al, dx", in("dx") 0x3fdu16, out("al") status, options(nomem, nostack));
                if status & 0x20 != 0 {
                    ready = true;
                    break;
                }
            }
            if !ready {
                core::arch::asm!("cli", options(nomem, nostack));
                loop {
                    core::arch::asm!("hlt", options(nomem, nostack));
                }
            }
            core::arch::asm!("out dx, al", in("dx") 0x3f8u16, in("al") byte, options(nomem, nostack));
        }
    }
}

fn genos_fixture_number(mut number: usize) {
    let mut bytes = [0u8; 20];
    let mut start = bytes.len();
    loop {
        start -= 1;
        bytes[start] = b'0' + (number % 10) as u8;
        number /= 10;
        if number == 0 {
            break;
        }
    }
    genos_fixture_emit(&bytes[start..]);
}

fn genos_fixture_status(status: Status) {
    genos_fixture_emit(if status == Status::SUCCESS {
        b"success"
    } else if status == Status::INVALID_PARAMETER {
        b"invalid-parameter"
    } else if status == Status::BUFFER_TOO_SMALL {
        b"buffer-too-small"
    } else if status == Status::OUT_OF_RESOURCES {
        b"out-of-resources"
    } else if status == Status::NOT_FOUND {
        b"not-found"
    } else {
        b"unexpected"
    });
}

pub fn genos_fixture_arm_allocation() {
    assert_eq!(GENOS_FIXTURE_MODE, 5);
    assert!(!GENOS_FIXTURE_ARMED.swap(true, Ordering::SeqCst));
    genos_fixture_emit(b"\r\nFIRMWARE_ALLOCATION_ARMED stage=");
    genos_fixture_number(usize::from(GENOS_FIXTURE_STAGE));
    genos_fixture_emit(b"\r\n");
}

fn genos_fixture_page_failure() -> bool {
    GENOS_FIXTURE_MODE == 5
        && GENOS_FIXTURE_STAGE != 4
        && GENOS_FIXTURE_ARMED.swap(false, Ordering::SeqCst)
}

fn genos_fixture_pool_failure() -> bool {
    GENOS_FIXTURE_MODE == 5
        && GENOS_FIXTURE_STAGE == 4
        && GENOS_FIXTURE_ARMED.swap(false, Ordering::SeqCst)
}

fn genos_fixture_allocation_result(status: Status) {
    assert!(status == Status::OUT_OF_RESOURCES || status == Status::NOT_FOUND);
    genos_fixture_emit(b"\r\nFIRMWARE_ALLOCATION_DENIED stage=");
    genos_fixture_number(usize::from(GENOS_FIXTURE_STAGE));
    genos_fixture_emit(b" status=");
    genos_fixture_status(status);
    genos_fixture_emit(b"\r\n");
}

fn genos_fixture_grow(buf: &mut [u8]) {
    if !matches!(GENOS_FIXTURE_MODE, 3 | 4) || GENOS_FIXTURE_GROWN.swap(true, Ordering::SeqCst) {
        return;
    }
    // This function runs before the first ExitBootServices attempt. The
    // subsequent retries can only retrieve the map and attempt exit again.
    let before = get_memory_map(buf).expect("fixture initial map");
    assert!(before.desc_size >= 40 && before.map_size <= buf.len());
    let required_pages = GENOS_FIXTURE_GROW_PAGES * 2 + 2;
    let start = buf[..before.map_size]
        .chunks_exact(before.desc_size)
        .find_map(|raw| {
            let kind = u32::from_le_bytes(raw[..4].try_into().unwrap());
            let pages = u64::from_le_bytes(raw[24..32].try_into().unwrap());
            let start = u64::from_le_bytes(raw[8..16].try_into().unwrap());
            let attributes = u64::from_le_bytes(raw[32..40].try_into().unwrap());
            (kind == 7
                && pages >= required_pages as u64
                && start != 0
                && start.is_multiple_of(4096)
                && attributes & (1 << 63) == 0
                && pages
                    .checked_mul(4096)
                    .and_then(|size| start.checked_add(size))
                    .is_some())
            .then_some(start)
        })
        .expect("fixture conventional RAM span");
    for index in 0..GENOS_FIXTURE_GROW_PAGES {
        let address = start.checked_add(((index * 2 + 1) * 4096) as u64).unwrap();
        // Allocate alternating interior pages so real firmware must split its
        // conventional-memory descriptors. The map buffer is LOADER_DATA and
        // cannot be inside the conventional span being selected here.
        let pointer = allocate_pages(AllocateType::Address(address), MemoryType::LOADER_DATA, 1)
            .expect("fixture exact interior allocation");
        assert_eq!(pointer.as_ptr() as u64, address);
        // Deliberately retained in this disposable VM; never freed after exit.
    }
    genos_fixture_emit(b"\r\nFIRMWARE_MAP_GROWN pages=");
    genos_fixture_number(GENOS_FIXTURE_GROW_PAGES);
    genos_fixture_emit(b" before=");
    genos_fixture_number(before.map_size);
    genos_fixture_emit(b" capacity=");
    genos_fixture_number(buf.len());
    genos_fixture_emit(b" stride=");
    genos_fixture_number(before.desc_size);
    genos_fixture_emit(b"\r\n");
}

fn genos_fixture_exit_key(key: usize) -> (usize, usize) {
    let attempt = GENOS_FIXTURE_ATTEMPTS.fetch_add(1, Ordering::SeqCst) + 1;
    let stale = GENOS_FIXTURE_MODE == 2 || GENOS_FIXTURE_MODE == 1 && attempt == 1;
    (if stale { key.wrapping_add(1) } else { key }, attempt)
}

fn genos_fixture_exit_result(status: Status, attempt: usize) {
    if GENOS_FIXTURE_MODE == 2 || GENOS_FIXTURE_MODE == 1 && attempt == 1 {
        assert_eq!(status, Status::INVALID_PARAMETER);
    }
    genos_fixture_emit(b"\r\nFIRMWARE_EXIT_RESULT attempt=");
    genos_fixture_number(attempt);
    genos_fixture_emit(b" status=");
    genos_fixture_status(status);
    genos_fixture_emit(b"\r\n");
}

fn genos_fixture_map_result(status: Status, size: usize, capacity: usize) {
    let call = GENOS_FIXTURE_MAP_CALLS.fetch_add(1, Ordering::SeqCst) + 1;
    genos_fixture_emit(b"\r\nFIRMWARE_MAP_RESULT call=");
    genos_fixture_number(call);
    genos_fixture_emit(b" status=");
    genos_fixture_status(status);
    genos_fixture_emit(b" size=");
    genos_fixture_number(size);
    genos_fixture_emit(b" capacity=");
    genos_fixture_number(capacity);
    genos_fixture_emit(b"\r\n");
}
