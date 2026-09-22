// A self-contained ELF fixture. Only the production ELF loader executes these
// bytes, mapped at USER_CODE in Ring 3; the kernel never calls this code.
.pushsection .rodata
.balign 16
.global genos_xstate_probe_elf
genos_xstate_probe_elf:
    .byte 0x7f, 0x45, 0x4c, 0x46, 2, 1, 1
    .zero 9
    .short 2, 62
    .long 1
    .quad {user_code}, 64, 0
    .long 0
    .short 64, 56, 2, 0, 0, 0
    .long 1, 5
    .quad 4096, {user_code}, {user_code}
    .quad genos_xstate_probe_code_end - genos_xstate_probe_code
    .quad genos_xstate_probe_code_end - genos_xstate_probe_code, 4096
    .long 1, 6
    .quad 8192, {user_data}, {user_data}, 16, 4096, 4096
    .zero 4096 - (. - genos_xstate_probe_elf)
genos_xstate_probe_code:
    mov r12, rdi
    movabs rbx, {user_data}
    // Snapshot genuinely observed initial registers before installing a unique
    // x87/SSE pattern. Kernel validation compares only architectural fields.
    fxsave64 [rbx + 1024]
    fxrstor64 [rbx + 512]
    xor edi, edi
    mov eax, {abi_version}
    int 0x80
    cmp eax, {abi_value}
    jne 2f
1:
    pause
    cmp qword ptr [rbx + 8], 2
    jb 1b
    // A non-yielding syscall must preserve state in addition to preemption.
    mov eax, {ping}
    int 0x80
    fxsave64 [rbx + 1536]
    // Exercise all eight MMX aliases too, with distinct values. MMX changes
    // the x87 tags/exponents; the verifier has a separate expected image.
    movq mm0, [rbx + 544]
    movq mm1, [rbx + 560]
    movq mm2, [rbx + 576]
    movq mm3, [rbx + 592]
    movq mm4, [rbx + 608]
    movq mm5, [rbx + 624]
    movq mm6, [rbx + 640]
    movq mm7, [rbx + 656]
    mov eax, {yield_call}
    int 0x80
    fxsave64 [rbx + 2048]
    mov qword ptr [rbx + 2560], 1
    test r12b, 1
    jnz 2f
    xor edi, edi
    mov eax, {exit_call}
    int 0x80
2:
    // One adversarial peer dies with its state loaded. Its successor must
    // observe fresh initial state, never this peer's private register values.
    ud2
genos_xstate_probe_code_end:
    .zero 8192 - (. - genos_xstate_probe_elf)
    .quad 0, 0
.global genos_xstate_probe_elf_end
genos_xstate_probe_elf_end:
.popsection
