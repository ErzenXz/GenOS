// The kernel target disables the red zone. All entries are interrupt gates.
// Preserve every GPR before using scratch registers, clear DF for the SysV
// ABI, and retain the original saved-frame pointer across stack alignment.
.macro genos_push_regs
    push rax
    push rcx
    push rdx
    push rbx
    push rbp
    push rsi
    push rdi
    push r8
    push r9
    push r10
    push r11
    push r12
    push r13
    push r14
    push r15
    // AC is user-controllable. Clear it before Rust so an interrupted user
    // cannot disable SMAP in the kernel. This works even without CLAC support.
    pushfq
    and qword ptr [rsp], -262145
    popfq
.endm

.macro genos_pop_regs
    pop r15
    pop r14
    pop r13
    pop r12
    pop r11
    pop r10
    pop r9
    pop r8
    pop rdi
    pop rsi
    pop rbp
    pop rbx
    pop rdx
    pop rcx
    pop rax
.endm

.macro genos_call_aligned handler
    mov rax, rsp
    and rsp, -16
    sub rsp, 16
    mov [rsp], rax
    call \handler
    mov rsp, [rsp]
.endm

// Timer entry deliberately retains the existing UserContext layout. It does
// not pass through the extra vector/error words used by ExceptionFrame.
.global genos_irq0_stub
genos_irq0_stub:
    cld
    genos_push_regs
    mov rdi, rsp
    genos_call_aligned genos_irq0_rust
    test rax, rax
    jnz genos_leave_userspace
    genos_pop_regs
    iretq

.global genos_irq1_stub
genos_irq1_stub:
    cld
    genos_push_regs
    genos_call_aligned genos_irq1_rust
    genos_pop_regs
    iretq

.global genos_irq12_stub
genos_irq12_stub:
    cld
    genos_push_regs
    genos_call_aligned genos_irq12_rust
    genos_pop_regs
    iretq

// Modern VirtIO MSI-X completion retains its bounded device-specific path.
.global genos_irq48_stub
genos_irq48_stub:
    cld
    genos_push_regs
    genos_call_aligned genos_irq48_rust
    genos_pop_regs
    iretq

// CPU error-code presence is a single Rust constant shared with host tests.
// Do not invoke error-code exception vectors with software INT instructions:
// unlike a CPU exception, INT does not push an architectural error word.
.macro genos_vector number
.global genos_vector_\number
genos_vector_\number:
    .if \number < 32
        .if (({error_code_mask} >> \number) & 1) == 0
            push 0
        .endif
    .else
        push 0
    .endif
    push \number
    jmp genos_vector_common
.endm

.irp number, 0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31,32,33,34,35,36,37,38,39,40,41,42,43,44,45,46,47,48,49,50,51,52,53,54,55,56,57,58,59,60,61,62,63,64,65,66,67,68,69,70,71,72,73,74,75,76,77,78,79,80,81,82,83,84,85,86,87,88,89,90,91,92,93,94,95,96,97,98,99,100,101,102,103,104,105,106,107,108,109,110,111,112,113,114,115,116,117,118,119,120,121,122,123,124,125,126,127,128,129,130,131,132,133,134,135,136,137,138,139,140,141,142,143,144,145,146,147,148,149,150,151,152,153,154,155,156,157,158,159,160,161,162,163,164,165,166,167,168,169,170,171,172,173,174,175,176,177,178,179,180,181,182,183,184,185,186,187,188,189,190,191,192,193,194,195,196,197,198,199,200,201,202,203,204,205,206,207,208,209,210,211,212,213,214,215,216,217,218,219,220,221,222,223,224,225,226,227,228,229,230,231,232,233,234,235,236,237,238,239,240,241,242,243,244,245,246,247,248,249,250,251,252,253,254,255
    genos_vector \number
.endr

genos_vector_common:
    cld
    genos_push_regs
    mov rdi, rsp
    // Sample CR2 before calling Rust. It is meaningful only for vector 14.
    mov rsi, cr2
    genos_call_aligned genos_vector_rust
    test rax, rax
    jnz genos_leave_userspace
    genos_pop_regs
    // Remove both normalized words, including any CPU-supplied error word.
    add rsp, 16
    iretq

.pushsection .rodata
.balign 8
.global genos_vector_table
genos_vector_table:
.irp number, 0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31,32,33,34,35,36,37,38,39,40,41,42,43,44,45,46,47,48,49,50,51,52,53,54,55,56,57,58,59,60,61,62,63,64,65,66,67,68,69,70,71,72,73,74,75,76,77,78,79,80,81,82,83,84,85,86,87,88,89,90,91,92,93,94,95,96,97,98,99,100,101,102,103,104,105,106,107,108,109,110,111,112,113,114,115,116,117,118,119,120,121,122,123,124,125,126,127,128,129,130,131,132,133,134,135,136,137,138,139,140,141,142,143,144,145,146,147,148,149,150,151,152,153,154,155,156,157,158,159,160,161,162,163,164,165,166,167,168,169,170,171,172,173,174,175,176,177,178,179,180,181,182,183,184,185,186,187,188,189,190,191,192,193,194,195,196,197,198,199,200,201,202,203,204,205,206,207,208,209,210,211,212,213,214,215,216,217,218,219,220,221,222,223,224,225,226,227,228,229,230,231,232,233,234,235,236,237,238,239,240,241,242,243,244,245,246,247,248,249,250,251,252,253,254,255
    .quad genos_vector_\number
.endr
.popsection
