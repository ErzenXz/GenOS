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
    mov [rbx + 64], ds
    mov [rbx + 66], es
    mov [rbx + 68], fs
    mov [rbx + 70], gs
    // Snapshot genuinely observed initial registers before installing a unique
    // x87/SSE pattern. Kernel validation compares only architectural fields.
    fxsave64 [rbx + 1024]
    fxrstor64 [rbx + 512]
    mov ds, [rbx + 96]
    mov es, [rbx + 98]
    mov fs, [rbx + 100]
    mov gs, [rbx + 102]
    // Base-zero behavior is observed through both segment overrides.
    mov rax, fs:[rbx]
    cmp rax, [rbx]
    jne 2f
    mov rax, gs:[rbx]
    cmp rax, [rbx]
    jne 2f
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
    mov [rbx + 72], ds
    mov [rbx + 74], es
    mov [rbx + 76], fs
    mov [rbx + 78], gs
    // Exercise all eight MMX aliases with values different from x87. Their
    // reserved upper sixteen saved bits are not part of this phase's oracle.
    movq mm0, [rbx + 128]
    movq mm1, [rbx + 136]
    movq mm2, [rbx + 144]
    movq mm3, [rbx + 152]
    movq mm4, [rbx + 160]
    movq mm5, [rbx + 168]
    movq mm6, [rbx + 176]
    movq mm7, [rbx + 184]
    mov eax, {yield_call}
    int 0x80
    fxsave64 [rbx + 2048]
    mov [rbx + 80], ds
    mov [rbx + 82], es
    mov [rbx + 84], fs
    mov [rbx + 86], gs
    mov qword ptr [rbx + 2560], 1
    test r12b, 1
    jnz 3f
    xor edi, edi
    mov eax, {exit_call}
    int 0x80
3:
    // An unmasked x87 invalid operation records a pending #MF. Non-waiting
    // FXSAVE/FXRSTOR, direct/yield syscalls and preemption must preserve it
    // without raising #MF in kernel mode or contaminating another process.
    fninit
    mov word ptr [rbx + 104], 0x037e
    fldcw [rbx + 104]
    fldz
    fldz
    fdivp st(1), st(0)
    mov eax, {ping}
    int 0x80
    fxsave64 [rbx + 2576]
    mov eax, {yield_call}
    int 0x80
4:
    pause
    cmp qword ptr [rbx + 8], 3
    jb 4b
    fxsave64 [rbx + 3088]
    mov [rbx + 88], ds
    mov [rbx + 90], es
    mov [rbx + 92], fs
    mov [rbx + 94], gs
    mov rax, fs:[rbx]
    cmp rax, gs:[rbx]
    jne 2f
    fwait
2:
    // Reaching this sentinel instead of the selected #MF/exit fails the oracle.
    ud2
genos_xstate_probe_code_end:
    .zero 8192 - (. - genos_xstate_probe_elf)
    .quad 0, 0
.global genos_xstate_probe_elf_end
genos_xstate_probe_elf_end:
.popsection
