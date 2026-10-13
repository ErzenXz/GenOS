#![no_main]

use std::sync::OnceLock;

static TARGET: OnceLock<String> = OnceLock::new();

/// # Safety
/// The pinned libFuzzer runtime calls this synchronously with `size` readable,
/// immutable bytes at `data`, live for this callback. No input borrow escapes.
/// Zero-length input requires no pointer dereference. This host-only entry never
/// supplies a hardware pointer, executes privileged code or crosses into a VM.
#[no_mangle]
pub unsafe extern "C" fn LLVMFuzzerTestOneInput(data: *const u8, size: usize) -> i32 {
    assert!(size <= 65_536);
    let bytes = if size == 0 {
        &[]
    } else {
        assert!(!data.is_null());
        // SAFETY: the runtime's callback contract above supplies the original
        // allocation and bounds; usize is capped before slice construction.
        unsafe { std::slice::from_raw_parts(data, size) }
    };
    let target = TARGET.get_or_init(|| std::env::var("GENOS_FUZZ_TARGET").expect("fuzz target"));
    #[cfg(feature = "fault-oracle")]
    if target == "feedback-oracle" {
        // Deliberately injected harness failure, excluded from normal binaries.
        // Success requires a discovered crash and a separate retained replay.
        if bytes.len() >= 5
            && bytes[0] == b'G'
            && bytes[1] == b'E'
            && bytes[2] == b'N'
            && bytes[3] == b'O'
            && bytes[4] == b'S'
        {
            panic!("GENOS_COVERAGE_ORACLE");
        }
        return 0;
    }
    std::hint::black_box(genos_parser_stress::exercise(target, bytes));
    0
}
