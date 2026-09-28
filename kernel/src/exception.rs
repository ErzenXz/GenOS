//! Pure x86-64 entry contracts shared by assembly, dispatch, and host tests.
//!
//! An interrupt gate owns the frame until IRETQ or the existing process-exit
//! trampoline consumes it. No allocation or device access belongs here.

/// Architectural exceptions that push an error word, including AMD #VC/#SX.
/// Reserved vectors still receive a synthetic zero, never a guessed CPU word.
pub const ERROR_CODE_MASK: u32 = (1 << 8)
    | (1 << 10)
    | (1 << 11)
    | (1 << 12)
    | (1 << 13)
    | (1 << 14)
    | (1 << 17)
    | (1 << 21)
    | (1 << 29)
    | (1 << 30);

/// Stack order after the common entry saves GPRs. Long-mode interrupt entry
/// supplies SS/RSP; the stub supplies vector and, when needed, a zero error.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(C)]
pub struct ExceptionFrame {
    /// r15, r14, r13, r12, r11, r10, r9, r8, rdi, rsi, rbp, rbx, rdx, rcx, rax.
    pub registers: [u64; 15],
    pub vector: u64,
    pub error: u64,
    pub rip: u64,
    pub cs: u64,
    pub rflags: u64,
    pub rsp: u64,
    pub ss: u64,
}

/// Boot publishes these five TSS IST field addresses and their original tops.
/// Entry assembly owns updates afterwards. The double-fault stack is reserved
/// for assembly-only terminal reporting; no Rust frame ever lives there.
#[repr(C)]
pub struct EntryConfig {
    pub slots: [u64; 5],
    pub tops: [u64; 5],
    pub terminal_top: u64,
}

/// Accepted user flags for the native preemptible 64-bit profile. IOPL, NT,
/// VM, VIF/VIP and reserved bits never cross a return boundary. AC/DF/TF and
/// arithmetic flags remain private; entry clears live AC/DF before kernel Rust.
pub const USER_FLAGS_ALLOWED: u64 = 0x0025_0fd7;
pub const USER_FLAGS_REQUIRED: u64 = (1 << 1) | (1 << 9);

pub const fn user_flags_valid(flags: u64) -> bool {
    flags & USER_FLAGS_REQUIRED == USER_FLAGS_REQUIRED && flags & !USER_FLAGS_ALLOWED == 0
}

pub const fn has_error_code(vector: u64) -> bool {
    vector < 32 && ERROR_CODE_MASK & (1 << vector) != 0
}

/// Machine-level events, reserved vectors and unknown privilege levels must
/// never be downgraded to a successful process-local recovery.
pub const fn process_local(vector: u64, cs: u64) -> bool {
    cs & 3 == 3
        && matches!(
            vector,
            0 | 1 | 3 | 4 | 5 | 6 | 7 | 10 | 11 | 12 | 13 | 14 | 16 | 17 | 19 | 21
        )
}

/// Dedicated stacks prevent these exceptional entries from resetting the
/// ordinary IRQ stack while its saved frame is still live. Syscalls use RSP0.
pub const fn ist_index(vector: usize) -> u16 {
    match vector {
        8 => 2,
        2 => 3,
        18 => 4,
        1 => 5,
        _ => 1,
    }
}

/// IRQ7 has no EOI when spurious. A spurious slave IRQ15 still acknowledges
/// the master's cascade, but must not acknowledge an unasserted slave ISR bit.
pub const fn pic_eoi_policy(irq: u8, in_service: bool) -> (bool, bool) {
    match irq {
        7 if !in_service => (false, false),
        15 if !in_service => (true, false),
        0..=7 => (true, false),
        8..=15 => (true, true),
        _ => (false, false),
    }
}

#[cfg(test)]
mod tests;
