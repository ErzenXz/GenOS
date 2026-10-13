// Validation-only fixed-address Ring 3 ELF, admitted by the production loader.
// Snapshots use a balanced user-stack PUSH/POP to save RAX without modifying
// any register or flag. All accesses are aligned; AC never tests misalignment.
// No error-code word, privilege transition or kernel call is synthesized here.
.pushsection .rodata
.balign 16
.global genos_integer_probe_elf
genos_integer_probe_elf:
    .byte 0x7f, 0x45, 0x4c, 0x46, 2, 1, 1
    .zero 9
    .short 2, 62
    .long 1
    .quad {user_code}, 64, 0
    .long 0
    .short 64, 56, 2, 0, 0, 0
    .long 1, 5
    .quad 4096, {user_code}, {user_code}
    .quad genos_integer_code_end - genos_integer_code
    .quad genos_integer_code_end - genos_integer_code, 4096
    .long 1, 6
    .quad 8192, {user_data}, {user_data}, 16, 4096, 4096
    .zero 4096 - (. - genos_integer_probe_elf)

.macro integer_snapshot offset
    push rax
    movabs rax, {user_data} + 64 + \offset * 160
    mov [rax], r15
    mov [rax + 8], r14
    mov [rax + 16], r13
    mov [rax + 24], r12
    mov [rax + 32], r11
    mov [rax + 40], r10
    mov [rax + 48], r9
    mov [rax + 56], r8
    mov [rax + 64], rdi
    mov [rax + 72], rsi
    mov [rax + 80], rbp
    mov [rax + 88], rbx
    mov [rax + 96], rdx
    mov [rax + 104], rcx
    pop qword ptr [rax + 112]
    mov [rax + 120], rsp
    pushfq
    pop qword ptr [rax + 128]
    mov [rax + 136], cs
    mov [rax + 138], ss
    mov rax, [rax + 112]
.endm

.macro integer_pattern
    movabs rax, {user_data} + 1536
    mov r15, [rax]
    mov r14, [rax + 8]
    mov r13, [rax + 16]
    mov r12, [rax + 24]
    mov r11, [rax + 32]
    mov r10, [rax + 40]
    mov r9, [rax + 48]
    mov r8, [rax + 56]
    mov rdi, [rax + 64]
    mov rsi, [rax + 72]
    mov rbp, [rax + 80]
    mov rbx, [rax + 88]
    mov rdx, [rax + 96]
    mov rcx, [rax + 104]
    push qword ptr [rax + 120]
    popfq
    mov rax, [rax + 112]
.endm

genos_integer_code:
    integer_snapshot 0
    movabs rax, {user_data}
    mov [rax], rdi
    // Arm real PIT scheduling through the existing ABI handshake.
    mov eax, {abi_version}
    xor edi, edi
    int 0x80
    integer_pattern
    integer_snapshot 1
    // Deliberately unknown number permits nonzero patterns in all arguments;
    // RAX must become the exact ABI error while the other registers survive.
    int 0x80
    integer_snapshot 2
    integer_pattern
    mov edi, 0
    mov esi, 0
    mov edx, 0
    mov r10d, 0
    mov r8d, 0
    mov r9d, 0
    mov eax, {yield_call}
    integer_snapshot 3
    int 0x80
    integer_snapshot 4
    integer_pattern
    mov rcx, -2
    integer_snapshot 5
.global genos_integer_timer_start
genos_integer_timer_start:
    nop
    mov rcx, [rbx]
    lea rcx, [rcx - 2]
    jrcxz 1f
    jmp genos_integer_timer_start
1:
.global genos_integer_timer_end
genos_integer_timer_end:
    integer_snapshot 6
    movabs rax, {user_data}
    test byte ptr [rax], 1
    jnz 2f
    integer_pattern
    integer_snapshot 7
    mov edi, 0
    mov esi, 0
    mov edx, 0
    mov r10d, 0
    mov r8d, 0
    mov r9d, 0
    mov eax, {exit_call}
    int 0x80
2:
    integer_pattern
    integer_snapshot 7
    ud2
genos_integer_code_end:
    .zero 8192 - (. - genos_integer_probe_elf)
    .quad 0, 0
.global genos_integer_probe_elf_end
genos_integer_probe_elf_end:
.purgem integer_snapshot
.purgem integer_pattern
.popsection
