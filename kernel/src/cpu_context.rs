//! Native saved CPU frame and pure return admission for the single-BSP profile.
//!
//! Assembly owns the live frame. These values carry no mapping authority: the
//! coordinator supplies bounds from the owned executable/stack and separately
//! verifies the active address-space root before accepting a captured frame.

pub const USER_CODE_SELECTOR: u16 = 0x33;

/// Fifteen saved GPRs followed by the five IRET words, in entry stack order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct UserContext {
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub r11: u64,
    pub r10: u64,
    pub r9: u64,
    pub r8: u64,
    pub rdi: u64,
    pub rsi: u64,
    pub rbp: u64,
    pub rbx: u64,
    pub rdx: u64,
    pub rcx: u64,
    pub rax: u64,
    pub rip: u64,
    pub cs: u64,
    pub rflags: u64,
    pub rsp: u64,
    pub ss: u64,
}

pub const USER_CONTEXT_CS_OFFSET: usize = core::mem::offset_of!(UserContext, cs);
pub const USER_CONTEXT_RIP_OFFSET: usize = core::mem::offset_of!(UserContext, rip);
pub const USER_CONTEXT_RSP_OFFSET: usize = core::mem::offset_of!(UserContext, rsp);

#[derive(Clone, Copy)]
pub struct ReturnBounds {
    pub executable_start: u64,
    pub executable_end: u64,
    pub stack_bottom: u64,
    pub stack_top: u64,
}

impl UserContext {
    pub const fn initial(token: u64, entry: u64, stack_top: u64) -> Self {
        Self {
            r15: 0,
            r14: 0,
            r13: 0,
            r12: 0,
            r11: 0,
            r10: 0,
            r9: 0,
            r8: 0,
            rdi: token,
            rsi: 0,
            rbp: 0,
            rbx: 0,
            rdx: 0,
            rcx: 0,
            rax: 0,
            rip: entry,
            cs: USER_CODE_SELECTOR as u64,
            rflags: 0x202,
            rsp: stack_top,
            ss: crate::segments::USER_DATA as u64,
        }
    }

    /// Preserve the current native policy: executable end and stack bottom
    /// are excluded, while the empty stack's top is a valid initial RSP.
    pub const fn valid_return(&self, bounds: ReturnBounds) -> bool {
        self.cs == USER_CODE_SELECTOR as u64
            && self.ss == crate::segments::USER_DATA as u64
            && self.rip >= bounds.executable_start
            && self.rip < bounds.executable_end
            && self.rsp > bounds.stack_bottom
            && self.rsp <= bounds.stack_top
            && crate::exception::user_flags_valid(self.rflags)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOUNDS: ReturnBounds = ReturnBounds {
        executable_start: 0x4000,
        executable_end: 0x6000,
        stack_bottom: 0x9000,
        stack_top: 0xb000,
    };

    fn initial() -> UserContext {
        UserContext::initial(0x1234, BOUNDS.executable_start, BOUNDS.stack_top)
    }

    #[test]
    fn assembly_layout_matches_all_twenty_saved_words() {
        assert_eq!(core::mem::size_of::<UserContext>(), 20 * 8);
        assert_eq!(core::mem::align_of::<UserContext>(), 8);
        let offsets = [
            core::mem::offset_of!(UserContext, r15),
            core::mem::offset_of!(UserContext, r14),
            core::mem::offset_of!(UserContext, r13),
            core::mem::offset_of!(UserContext, r12),
            core::mem::offset_of!(UserContext, r11),
            core::mem::offset_of!(UserContext, r10),
            core::mem::offset_of!(UserContext, r9),
            core::mem::offset_of!(UserContext, r8),
            core::mem::offset_of!(UserContext, rdi),
            core::mem::offset_of!(UserContext, rsi),
            core::mem::offset_of!(UserContext, rbp),
            core::mem::offset_of!(UserContext, rbx),
            core::mem::offset_of!(UserContext, rdx),
            core::mem::offset_of!(UserContext, rcx),
            core::mem::offset_of!(UserContext, rax),
            USER_CONTEXT_RIP_OFFSET,
            USER_CONTEXT_CS_OFFSET,
            core::mem::offset_of!(UserContext, rflags),
            USER_CONTEXT_RSP_OFFSET,
            core::mem::offset_of!(UserContext, ss),
        ];
        for (word, offset) in offsets.into_iter().enumerate() {
            assert_eq!(offset, word * 8);
        }
    }

    #[test]
    fn fresh_frame_contains_only_the_declared_token_entry_and_return_state() {
        let frame = initial();
        assert!(frame.valid_return(BOUNDS));
        assert_eq!(frame.rdi, 0x1234);
        assert_eq!(frame.rflags, 0x202);
        assert_eq!(
            [
                frame.r15, frame.r14, frame.r13, frame.r12, frame.r11, frame.r10, frame.r9,
                frame.r8, frame.rsi, frame.rbp, frame.rbx, frame.rdx, frame.rcx, frame.rax
            ],
            [0; 14]
        );
    }

    #[test]
    fn return_bounds_reject_guards_outside_and_empty_ranges() {
        for (rip, allowed) in [
            (0x3fff, false),
            (0x4000, true),
            (0x5fff, true),
            (0x6000, false),
            (u64::MAX, false),
        ] {
            assert_eq!(
                UserContext { rip, ..initial() }.valid_return(BOUNDS),
                allowed
            );
        }
        for (rsp, allowed) in [
            (0, false),
            (0x9000, false),
            (0x9001, true),
            (0xb000, true),
            (0xb001, false),
            (u64::MAX, false),
        ] {
            assert_eq!(
                UserContext { rsp, ..initial() }.valid_return(BOUNDS),
                allowed
            );
        }
        for bounds in [
            ReturnBounds {
                executable_end: BOUNDS.executable_start,
                ..BOUNDS
            },
            ReturnBounds {
                executable_start: BOUNDS.executable_end,
                ..BOUNDS
            },
            ReturnBounds {
                stack_bottom: BOUNDS.stack_top,
                ..BOUNDS
            },
            ReturnBounds {
                stack_top: BOUNDS.stack_bottom,
                ..BOUNDS
            },
        ] {
            assert!(!initial().valid_return(bounds));
        }
    }

    #[test]
    fn selector_admission_requires_exact_native_descriptors() {
        for selector in 0..=u16::MAX as u64 {
            assert_eq!(
                UserContext {
                    cs: selector,
                    ..initial()
                }
                .valid_return(BOUNDS),
                selector == USER_CODE_SELECTOR as u64
            );
            assert_eq!(
                UserContext {
                    ss: selector,
                    ..initial()
                }
                .valid_return(BOUNDS),
                selector == crate::segments::USER_DATA as u64
            );
        }
        assert!(!UserContext {
            cs: 0x1_0033,
            ..initial()
        }
        .valid_return(BOUNDS));
        assert!(!UserContext {
            ss: 0x1_002b,
            ..initial()
        }
        .valid_return(BOUNDS));
    }

    #[test]
    fn each_flag_bit_obeys_native_return_policy() {
        for bit in 0..64 {
            let flag = 1u64 << bit;
            let frame = UserContext {
                rflags: 0x202 | flag,
                ..initial()
            };
            assert_eq!(
                frame.valid_return(BOUNDS),
                crate::exception::USER_FLAGS_ALLOWED & flag != 0
            );
            let frame = UserContext {
                rflags: 0x202 & !flag,
                ..initial()
            };
            assert_eq!(
                frame.valid_return(BOUNDS),
                crate::exception::USER_FLAGS_REQUIRED & flag == 0
            );
        }
    }
}
