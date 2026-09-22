//! Real Ring 3 x87/SSE isolation fixture, compiled only for validation images.

use core::arch::global_asm;

use kernel::xstate::State;

use super::{build_process, reclaim_process, run_slice, ProcessEvent, UserProcess};
use crate::{memory, paging};

global_asm!(
    include_str!("xstate_probe.S"),
    user_code = const paging::USER_CODE,
    user_data = const paging::USER_DATA,
    abi_version = const genos_abi::USER_SYSCALL_ABI_VERSION,
    abi_value = const genos_abi::USER_ABI_VERSION,
    ping = const genos_abi::USER_SYSCALL_PING,
    yield_call = const genos_abi::USER_SYSCALL_YIELD,
    exit_call = const genos_abi::USER_SYSCALL_EXIT,
);

unsafe extern "C" {
    static genos_xstate_probe_elf: u8;
    static genos_xstate_probe_elf_end: u8;
}

pub(super) fn run() -> bool {
    let begin = core::ptr::addr_of!(genos_xstate_probe_elf);
    let end = core::ptr::addr_of!(genos_xstate_probe_elf_end);
    // SAFETY: the linked assembly emits these ordered symbols around one
    // immutable ELF fixture; the whole interval is kernel-lifetime rodata.
    let image = unsafe { core::slice::from_raw_parts(begin, end as usize - begin as usize) };
    let baseline = memory::allocated_frames();
    let mut preemptions = 0;
    // Rebuild the same three process identities in the same slots after their
    // exit/fault and reclamation. Fresh initial snapshots must still be zero.
    for round in 0..2 {
        let seeds = [2 + round * 8, 4 + round * 8, 7 + round * 8];
        let mut processes: [Option<UserProcess>; 3] = [None, None, None];
        let mut passed = true;
        for (slot, seed) in seeds.iter().enumerate() {
            match build_process(120 + slot as u8, *seed, image) {
                Ok(process) => {
                    let pattern = State::probe_pattern(*seed);
                    // SAFETY: this inactive process owns the mapped data page;
                    // offset 512 and the complete image fit the 4096-byte page.
                    // No Ring 3 execution or references to these bytes exist.
                    unsafe {
                        core::ptr::copy_nonoverlapping(
                            pattern.bytes().as_ptr(),
                            (process.data_frame + 512) as *mut u8,
                            512,
                        );
                    }
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
                for process in processes.iter_mut().flatten() {
                    if !process.completed {
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
                    }
                }
                if !passed {
                    break;
                }
            }
            for (slot, process) in processes.iter().flatten().enumerate() {
                preemptions += process.preemptions;
                passed &= observed(process, seeds[slot]);
            }
        }
        // Cleanup is unconditional, including partially constructed fixtures.
        for process in processes.iter_mut().flatten() {
            passed &= reclaim_process(process).is_ok();
        }
        passed &= memory::allocated_frames() == baseline;
        if !passed {
            return false;
        }
    }
    crate::serial::println(
        "USER_XSTATE_OK processes=6 rounds=2 components=x87,mmx,xmm0-15,mxcsr syscalls=direct,yield faults=2 fresh=6 reclaimed=true",
    );
    crate::serial::print("USER_XSTATE_PREEMPTIONS count=");
    crate::serial::print_u64(preemptions);
    crate::serial::println("");
    true
}

fn observed(process: &UserProcess, seed: u64) -> bool {
    let pattern = State::probe_pattern(seed);
    let mmx_pattern = State::probe_mmx_pattern(seed);
    // SAFETY: run_slice returned to kernel mappings with this process inactive;
    // its data page is live and private until the unconditional reclamation.
    // The fixture uses only these bounded offsets inside that page.
    let data = unsafe { core::slice::from_raw_parts(process.data_frame as *const u8, 4096) };
    let completion = if seed & 1 == 1 {
        process.event == ProcessEvent::Fault && process.fault_vector == 6
    } else {
        process.event == ProcessEvent::Exit && process.exit_code == 0
    };
    process.completed
        && completion
        && process.preemptions >= 2
        && process.yields == 1
        && data[2560..2568] == 1u64.to_le_bytes()
        && State::initial().matches_saved(&data[1024..1536])
        && pattern.matches_saved(&data[1536..2048])
        && mmx_pattern.matches_saved(&data[2048..2560])
        && mmx_pattern.matches_saved(process.xstate.bytes())
}
