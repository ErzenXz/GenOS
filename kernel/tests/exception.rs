use core::mem::{align_of, offset_of, size_of};
use kernel::exception::{has_error_code, ist_index, pic_eoi_policy, process_local, ExceptionFrame};

#[test]
fn frame_matches_the_assembly_stack_exactly() {
    assert_eq!(size_of::<ExceptionFrame>(), 22 * 8);
    assert_eq!(align_of::<ExceptionFrame>(), 8);
    assert_eq!(offset_of!(ExceptionFrame, registers), 0);
    assert_eq!(offset_of!(ExceptionFrame, vector), 120);
    assert_eq!(offset_of!(ExceptionFrame, error), 128);
    assert_eq!(offset_of!(ExceptionFrame, rip), 136);
    assert_eq!(offset_of!(ExceptionFrame, cs), 144);
    assert_eq!(offset_of!(ExceptionFrame, rflags), 152);
    assert_eq!(offset_of!(ExceptionFrame, rsp), 160);
    assert_eq!(offset_of!(ExceptionFrame, ss), 168);
}

#[test]
fn every_vector_has_an_explicit_error_word_policy() {
    let cpu_error_vectors = [8, 10, 11, 12, 13, 14, 17, 21, 29, 30];
    for vector in 0..256 {
        assert_eq!(has_error_code(vector), cpu_error_vectors.contains(&vector));
    }
    assert!(!has_error_code(u64::MAX));
}

#[test]
fn user_fault_recovery_never_accepts_a_kernel_or_machine_fault() {
    let recoverable = [0, 1, 3, 4, 5, 6, 7, 10, 11, 12, 13, 14, 16, 17, 19, 21];
    for vector in 0..256 {
        for cs in [0x08, 0x09, 0x0a, 0x33] {
            assert_eq!(
                process_local(vector, cs),
                cs == 0x33 && recoverable.contains(&vector)
            );
        }
    }
    assert!(!process_local(u64::MAX, 0x33));
}

#[test]
fn asynchronous_fatal_entries_do_not_share_the_ordinary_irq_stack() {
    let stacks = [
        ist_index(32),
        ist_index(8),
        ist_index(2),
        ist_index(18),
        ist_index(1),
    ];
    for (index, stack) in stacks.iter().enumerate() {
        assert!((1..=7).contains(stack));
        assert!(!stacks[..index].contains(stack));
    }
}

#[test]
fn spurious_pic_interrupts_do_not_acknowledge_an_unasserted_isr_bit() {
    assert_eq!(pic_eoi_policy(7, false), (false, false));
    assert_eq!(pic_eoi_policy(15, false), (true, false));
    assert_eq!(pic_eoi_policy(7, true), (true, false));
    assert_eq!(pic_eoi_policy(15, true), (true, true));
    for irq in 0..16 {
        assert_eq!(pic_eoi_policy(irq, true), (true, irq >= 8));
    }
    assert_eq!(pic_eoi_policy(16, true), (false, false));
}
