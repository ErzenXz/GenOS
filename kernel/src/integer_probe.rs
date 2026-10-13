//! Six real Ring 3 processes exercise native GPR/flag entry and reuse.
//! Timer progress is published only after two actual, validated captures inside
//! a MOV/LEA/JRCXZ loop. Scheduling and entry/return use the production paths.

use core::arch::global_asm;

use kernel::integer_state::{self as oracle, FIRST, INPUT, PROGRESS, STRIDE};

use super::{build_process, reclaim_process, run_slice, ProcessEvent, UserProcess};
use crate::{memory, paging};

global_asm!(
    include_str!("integer_probe.S"),
    user_code = const paging::USER_CODE,
    user_data = const paging::USER_DATA,
    abi_version = const genos_abi::USER_SYSCALL_ABI_VERSION,
    yield_call = const genos_abi::USER_SYSCALL_YIELD,
    exit_call = const genos_abi::USER_SYSCALL_EXIT,
);

unsafe extern "C" {
    static genos_integer_probe_elf: u8;
    static genos_integer_probe_elf_end: u8;
    static genos_integer_timer_start: u8;
    static genos_integer_timer_end: u8;
}

pub(super) fn run() -> bool {
    let begin = core::ptr::addr_of!(genos_integer_probe_elf);
    let end = core::ptr::addr_of!(genos_integer_probe_elf_end);
    // SAFETY: ordered linked assembly symbols delimit one immutable kernel-
    // lifetime ELF in rodata. Only the production loader executes its copy.
    let image = unsafe { core::slice::from_raw_parts(begin, end as usize - begin as usize) };
    let user_address =
        |symbol: *const u8| paging::USER_CODE + (symbol as u64 - begin as u64 - 4096);
    let timer_start = user_address(core::ptr::addr_of!(genos_integer_timer_start));
    let timer_end = user_address(core::ptr::addr_of!(genos_integer_timer_end));
    let baseline = memory::allocated_frames();
    let mut total_captures = 0;
    for round in 0..2 {
        let seeds = [2 + round * 8, 4 + round * 8, 7 + round * 8];
        let mut processes: [Option<UserProcess>; 3] = [None, None, None];
        let mut captures = [0u64; 3];
        let mut passed = true;
        for (slot, seed) in seeds.iter().enumerate() {
            match build_process(130 + slot as u8, *seed, image) {
                Ok(process) => {
                    let words = oracle::pattern(*seed);
                    let mut input = [0u8; 128];
                    for (index, word) in words[..15].iter().chain([&words[16]]).enumerate() {
                        input[index * 8..index * 8 + 8].copy_from_slice(&word.to_le_bytes());
                    }
                    passed &=
                        paging::copy_to_user(process.space, paging::USER_DATA + INPUT, &input);
                    processes[slot] = Some(process);
                }
                Err(_) => passed = false,
            }
        }
        if passed {
            for _ in 0..48 {
                if processes.iter().flatten().all(|process| process.completed) {
                    break;
                }
                for (slot, process) in processes.iter_mut().enumerate() {
                    let Some(process) = process else { continue };
                    if process.completed {
                        continue;
                    }
                    run_slice(process);
                    if !matches!(
                        process.event,
                        ProcessEvent::Preempt
                            | ProcessEvent::Yield
                            | ProcessEvent::Exit
                            | ProcessEvent::Fault
                    ) {
                        passed = false;
                        break;
                    }
                    if process.event == ProcessEvent::Preempt
                        && (timer_start..timer_end).contains(&process.context.rip)
                    {
                        let mut expected = oracle::pattern(seeds[slot]);
                        // RCX is the flag-neutral progress scratch. A timer can
                        // arrive before/after MOV or LEA; every other word must
                        // match exactly, including AC/DF and arithmetic flags.
                        passed &= matches!(process.context.rcx, 0..=2 | u64::MAX)
                            || process.context.rcx == u64::MAX - 1;
                        expected[13] = process.context.rcx;
                        let actual = oracle::timer_words(&process.context);
                        if actual != expected {
                            for (index, (actual, expected)) in
                                actual.iter().zip(expected).enumerate()
                            {
                                if *actual != expected {
                                    crate::serial::print("USER_INTEGER_CAPTURE_MISMATCH word=");
                                    crate::serial::print_u64(index as u64);
                                    crate::serial::print(" actual=0x");
                                    crate::serial::print_hex(*actual);
                                    crate::serial::print(" expected=0x");
                                    crate::serial::print_hex(expected);
                                    crate::serial::println("");
                                }
                            }
                        }
                        passed &= actual == expected
                            && process.context.cs
                                == u64::from(kernel::cpu_context::USER_CODE_SELECTOR)
                            && process.context.ss == u64::from(kernel::segments::USER_DATA);
                        captures[slot] = (captures[slot] + 1).min(2);
                        passed &= paging::copy_to_user(
                            process.space,
                            paging::USER_DATA + PROGRESS + seeds[slot] * 8,
                            &captures[slot].to_le_bytes(),
                        );
                    }
                    if !passed {
                        break;
                    }
                }
                if !passed {
                    break;
                }
            }
            for (slot, process) in processes.iter().flatten().enumerate() {
                passed &= captures[slot] == 2 && observed(process, seeds[slot]);
                total_captures += captures[slot];
            }
        }
        // Every owner is reclaimed even if construction or a comparison failed.
        let mut reclaimed = true;
        for process in processes.iter_mut().flatten() {
            reclaimed &= reclaim_process(process).is_ok();
        }
        reclaimed &= memory::allocated_frames() == baseline;
        passed &= reclaimed;
        if !passed {
            crate::serial::println("USER_INTEGER_MISMATCH");
            if reclaimed {
                crate::serial::println("USER_INTEGER_REJECTED reclaimed=true");
            }
            return false;
        }
    }
    if total_captures != 12 {
        return false;
    }
    for marker in [
        "USER_INTEGER_FRESH_OK processes=6 gprs=15 rsp=initial flags=0x202",
        "USER_INTEGER_DIRECT_OK processes=6 preserved=14 rax=unknown-syscall flags=exact",
        "USER_INTEGER_YIELD_OK processes=6 gprs=15 args=zero flags=exact",
        "USER_INTEGER_TIMER_OK processes=6 captures=12 flags=arithmetic,df,ac,id",
        "USER_INTEGER_REUSE_OK rounds=2 faults=2 vector=6 exits=4 reclaimed=true",
    ] {
        crate::serial::println(marker);
    }
    true
}

fn observed(process: &UserProcess, seed: u64) -> bool {
    let mut samples = [0u8; STRIDE * 8];
    if !paging::copy_from_user(process.space, paging::USER_DATA + FIRST, &mut samples) {
        return false;
    }
    let mut expected = [oracle::pattern(seed); 8];
    expected[0] = oracle::initial(seed);
    expected[2][14] = genos_abi::USER_ERROR_UNKNOWN_SYSCALL;
    expected[3] = oracle::yielding(seed, false);
    expected[4] = oracle::yielding(seed, true);
    expected[5][13] = u64::MAX - 1;
    expected[6][13] = 0;
    let completion = if seed & 1 == 1 {
        process.event == ProcessEvent::Fault
            && process.fault_vector == 6
            && process.fault_error == 0
            && process.fault_address == 0
    } else {
        process.event == ProcessEvent::Exit && process.exit_code == 0
    };
    let mut passed =
        process.completed && completion && process.yields == 1 && process.preemptions >= 2;
    for (index, (words, bytes)) in expected
        .iter()
        .zip(samples.chunks_exact(STRIDE))
        .enumerate()
    {
        if !oracle::matches(words, bytes) {
            crate::serial::print("USER_INTEGER_SAMPLE_MISMATCH pid=");
            crate::serial::print_u64(u64::from(process.pid));
            crate::serial::print(" sample=");
            crate::serial::print_u64(index as u64);
            crate::serial::println("");
            passed = false;
        }
    }
    passed
}
