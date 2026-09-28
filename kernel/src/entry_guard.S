// Single-BSP entry ownership. Callers saved all GPRs and cleared DF/AC; IF=0.
// EAX=vector, RDI=saved frame, EDX=0 (UserContext) or 16 (ExceptionFrame).
// Helpers clobber GPRs/flags only, never FP state. No Rust runs on terminal IST.
.text
.global genos_entry_claim
genos_entry_claim:
    cmp byte ptr [rip + genos_entry_active], 0
    jne genos_entry_nested
    cmp eax, 8
    je genos_entry_double_fault
    mov dword ptr [rip + genos_entry_parent], eax
    mov byte ptr [rip + genos_entry_active], 1
    lea rsi, [rip + genos_entry_config]
    mov rax, [rsi + {config_terminal}]
    mov ecx, 5
1:
    mov rdx, [rsi + {config_slots}]
    mov [rdx], rax
    add rsi, 8
    loop 1b
    ret

.global genos_entry_release
genos_entry_release:
    lea rsi, [rip + genos_entry_config]
    mov ecx, 5
1:
    mov rdx, [rsi + {config_slots}]
    mov rax, [rsi + {config_tops}]
    mov [rdx], rax
    add rsi, 8
    loop 1b
    // Last write: a new owner can enter only after all original tops are back.
    mov byte ptr [rip + genos_entry_active], 0
    ret

genos_entry_nested:
    mov r15d, dword ptr [rip + genos_entry_parent]
    lea rsi, [rip + genos_nested_message]
    jmp genos_entry_terminal

genos_entry_double_fault:
    mov dword ptr [rip + genos_entry_parent], 8
    mov byte ptr [rip + genos_entry_active], 1
    mov r15d, 255
    lea rbx, [rip + genos_entry_config]
    mov r8, [rbx + {config_terminal}]
    mov r9d, 5
1:
    mov r10, [rbx + {config_slots}]
    mov [r10], r8
    add rbx, 8
    dec r9d
    jnz 1b
    lea rsi, [rip + genos_double_message]

genos_entry_terminal:
    // No return, allocation, locks, global subsystem state or typed frame borrow.
    // The existing 16-KiB double-fault stack holds only hardware/assembly frames.
    cli
    mov r12d, eax
    mov r13, rdi
    mov r14d, edx
    mov rbp, 0
    cmp eax, 14
    jne 1f
    mov rbp, cr2
1:
    call genos_terminal_string
    mov rax, r12
    call genos_terminal_hex
    lea rsi, [rip + genos_parent_message]
    call genos_terminal_string
    mov rax, r15
    call genos_terminal_hex
    lea rsi, [rip + genos_error_message]
    call genos_terminal_string
    mov rax, 0
    test r14, r14
    jz 2f
    mov rax, [r13 + {exception_error}]
2:
    call genos_terminal_hex
    lea rsi, [rip + genos_rip_message]
    call genos_terminal_string
    mov rax, [r13 + r14 + {user_rip}]
    call genos_terminal_hex
    lea rsi, [rip + genos_cs_message]
    call genos_terminal_string
    mov rax, [r13 + r14 + {user_cs}]
    call genos_terminal_hex
    lea rsi, [rip + genos_rsp_message]
    call genos_terminal_string
    mov rax, [r13 + r14 + {user_rsp}]
    call genos_terminal_hex
    lea rsi, [rip + genos_cr2_message]
    call genos_terminal_string
    mov rax, rbp
    call genos_terminal_hex
    lea rsi, [rip + genos_frame_message]
    call genos_terminal_string
    mov rax, r13
    call genos_terminal_hex
    lea rsi, [rip + genos_halt_message]
    call genos_terminal_string
genos_terminal_stop:
    cli
    hlt
    jmp genos_terminal_stop

// Read only linked NUL-terminated literals. At most three fixed call levels;
// bounded UART polling stops deliberately if the transmitter is unavailable.
genos_terminal_string:
    lodsb
    test al, al
    jz 1f
    call genos_terminal_byte
    jmp genos_terminal_string
1:
    ret

genos_terminal_hex:
    mov r10, rax
    mov r11d, 16
1:
    rol r10, 4
    mov eax, r10d
    and eax, 15
    add al, '0'
    cmp al, '9'
    jbe 2f
    add al, 39
2:
    call genos_terminal_byte
    dec r11d
    jnz 1b
    ret

genos_terminal_byte:
    mov r8b, al
    mov ecx, 65536
    mov edx, 0x3fd
1:
    in al, dx
    test al, 0x20
    jnz 2f
    loop 1b
    jmp genos_terminal_stop
2:
    mov edx, 0x3f8
    mov al, r8b
    out dx, al
    ret

.pushsection .rodata
genos_nested_message: .asciz "\nEXCEPTION_TERMINAL kind=nested vector=0x"
genos_double_message: .asciz "\nEXCEPTION_TERMINAL kind=double-fault vector=0x"
genos_parent_message: .asciz " parent=0x"
genos_error_message: .asciz " error=0x"
genos_rip_message: .asciz " rip=0x"
genos_cs_message: .asciz " cs=0x"
genos_rsp_message: .asciz " rsp=0x"
genos_cr2_message: .asciz " cr2=0x"
genos_frame_message: .asciz " frame=0x"
genos_halt_message: .asciz "\nEXCEPTION_TERMINAL_HALT\n"
.popsection

.pushsection .bss
.balign 8
.global genos_entry_config
genos_entry_config: .zero {config_bytes}
genos_entry_parent: .long 0
genos_entry_active: .byte 0
.popsection
