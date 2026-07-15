use core::arch::global_asm;
use core::sync::atomic::{AtomicU64, Ordering};

use crate::{arch, input_hw, userspace};

const PIC1_COMMAND: u16 = 0x20;
const PIC1_DATA: u16 = 0x21;
const PIC2_COMMAND: u16 = 0xa0;
const PIC2_DATA: u16 = 0xa1;
const PIC_EOI: u8 = 0x20;
const ENABLE_HARDWARE_INTERRUPTS: bool = true;

static TICKS: AtomicU64 = AtomicU64::new(0);
static FALLBACK_TICKS: AtomicU64 = AtomicU64::new(0);
static FALLBACK_SPINS: AtomicU64 = AtomicU64::new(0);
static KEYBOARD_IRQS: AtomicU64 = AtomicU64::new(0);
static MOUSE_IRQS: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy)]
pub struct InterruptStats {
    pub ticks: u64,
    pub keyboard_irqs: u64,
    pub mouse_irqs: u64,
}

global_asm!(
    r#"
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

    .global genos_irq0_stub
genos_irq0_stub:
    cld
    genos_push_regs
    genos_call_aligned genos_irq0_rust
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

    .global genos_fault_df_stub
genos_fault_df_stub:
    cld
    genos_push_regs
    mov rdi, 8
    mov rsi, [rsp + 120]
    mov rdx, [rsp + 128]
    xor rcx, rcx
    genos_call_aligned genos_fault_rust
1:
    hlt
    jmp 1b

    .global genos_fault_gp_stub
genos_fault_gp_stub:
    cld
    genos_push_regs
    mov rdi, 13
    mov rsi, [rsp + 120]
    mov rdx, [rsp + 128]
    xor rcx, rcx
    genos_call_aligned genos_fault_rust
2:
    hlt
    jmp 2b

    .global genos_fault_pf_stub
genos_fault_pf_stub:
    cld
    genos_push_regs
    mov rdi, 14
    mov rsi, [rsp + 120]
    mov rdx, [rsp + 128]
    mov rcx, cr2
    genos_call_aligned genos_fault_rust
3:
    hlt
    jmp 3b
"#
);

extern "C" {
    fn genos_irq0_stub();
    fn genos_irq1_stub();
    fn genos_irq12_stub();
    fn genos_fault_df_stub();
    fn genos_fault_gp_stub();
    fn genos_fault_pf_stub();
}

pub fn init() {
    arch::disable_interrupts();
    unsafe {
        arch::set_idt_handler(8, genos_fault_df_stub);
        arch::set_idt_handler(13, genos_fault_gp_stub);
        arch::set_idt_handler(14, genos_fault_pf_stub);
        arch::set_idt_handler(32, genos_irq0_stub);
        arch::set_idt_handler(33, genos_irq1_stub);
        arch::set_idt_handler(44, genos_irq12_stub);
        arch::set_user_idt_handler(userspace::SYSCALL_VECTOR, userspace::syscall_handler());
        remap_pic();
        init_pit_100hz();
    }
    crate::serial::println("IRQ_READY");
}

pub fn enable() {
    if ENABLE_HARDWARE_INTERRUPTS {
        arch::enable_interrupts();
        crate::serial::println("IRQ_HARDWARE_ON");
    } else {
        crate::serial::println("IRQ_POLLING_SAFE_MODE");
    }
}

pub fn ticks() -> u64 {
    let hardware = TICKS.load(Ordering::Relaxed);
    if hardware > 0 {
        hardware
    } else {
        FALLBACK_TICKS.load(Ordering::Relaxed)
    }
}

pub fn poll_fallback_tick() -> u64 {
    if TICKS.load(Ordering::Relaxed) == 0 {
        let spins = FALLBACK_SPINS.fetch_add(1, Ordering::Relaxed) + 1;
        if spins & 0x0fff == 0 {
            FALLBACK_TICKS.fetch_add(1, Ordering::Relaxed);
        }
    }
    ticks()
}

pub fn stats() -> InterruptStats {
    InterruptStats {
        ticks: ticks(),
        keyboard_irqs: KEYBOARD_IRQS.load(Ordering::Relaxed),
        mouse_irqs: MOUSE_IRQS.load(Ordering::Relaxed),
    }
}

#[no_mangle]
extern "C" fn genos_irq0_rust() {
    TICKS.fetch_add(1, Ordering::Relaxed);
    unsafe { pic_eoi(0) };
}

#[no_mangle]
extern "C" fn genos_irq1_rust() {
    KEYBOARD_IRQS.fetch_add(1, Ordering::Relaxed);
    input_hw::keyboard_irq();
    unsafe { pic_eoi(1) };
}

#[no_mangle]
extern "C" fn genos_irq12_rust() {
    MOUSE_IRQS.fetch_add(1, Ordering::Relaxed);
    input_hw::mouse_irq();
    unsafe { pic_eoi(12) };
}

#[no_mangle]
extern "C" fn genos_fault_rust(vector: u64, error: u64, rip: u64, cr2: u64) -> ! {
    match vector {
        8 => crate::serial::println("FAULT_DF"),
        13 => crate::serial::println("FAULT_GP"),
        14 => crate::serial::println("FAULT_PF"),
        _ => crate::serial::println("FAULT_CPU"),
    }
    crate::serial::print("vector=");
    crate::serial::print_u64(vector);
    crate::serial::print(" error=0x");
    crate::serial::print_hex(error);
    crate::serial::print(" rip=0x");
    crate::serial::print_hex(rip);
    crate::serial::print(" cr2=0x");
    crate::serial::print_hex(cr2);
    crate::serial::println("");
    arch::halt_loop();
}

unsafe fn remap_pic() {
    arch::outb(PIC1_COMMAND, 0x11);
    io_wait();
    arch::outb(PIC2_COMMAND, 0x11);
    io_wait();
    arch::outb(PIC1_DATA, 0x20);
    io_wait();
    arch::outb(PIC2_DATA, 0x28);
    io_wait();
    arch::outb(PIC1_DATA, 4);
    io_wait();
    arch::outb(PIC2_DATA, 2);
    io_wait();
    arch::outb(PIC1_DATA, 0x01);
    io_wait();
    arch::outb(PIC2_DATA, 0x01);
    io_wait();

    arch::outb(PIC1_DATA, 0b1111_1000);
    arch::outb(PIC2_DATA, 0b1110_1111);
}

unsafe fn init_pit_100hz() {
    let divisor: u16 = 11932;
    arch::outb(0x43, 0x36);
    arch::outb(0x40, (divisor & 0xff) as u8);
    arch::outb(0x40, (divisor >> 8) as u8);
}

unsafe fn pic_eoi(irq: u8) {
    if irq >= 8 {
        arch::outb(PIC2_COMMAND, PIC_EOI);
    }
    arch::outb(PIC1_COMMAND, PIC_EOI);
}

unsafe fn io_wait() {
    arch::outb(0x80, 0);
}
