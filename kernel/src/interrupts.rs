use core::arch::global_asm;
use core::sync::atomic::{AtomicU64, Ordering};

use kernel::exception::{pic_eoi_policy, process_local, ExceptionFrame};

use crate::{arch, input_hw, userspace};

const PIC1_COMMAND: u16 = 0x20;
const PIC1_DATA: u16 = 0x21;
const PIC2_COMMAND: u16 = 0xa0;
const PIC2_DATA: u16 = 0xa1;
const PIC_EOI: u8 = 0x20;
const PIC_READ_ISR: u8 = 0x0b;
const ENABLE_HARDWARE_INTERRUPTS: bool = true;

static TICKS: AtomicU64 = AtomicU64::new(0);
static FALLBACK_TICKS: AtomicU64 = AtomicU64::new(0);
static FALLBACK_SPINS: AtomicU64 = AtomicU64::new(0);
static KEYBOARD_IRQS: AtomicU64 = AtomicU64::new(0);
static MOUSE_IRQS: AtomicU64 = AtomicU64::new(0);
static NETWORK_IRQS: AtomicU64 = AtomicU64::new(0);

#[allow(dead_code)]
#[derive(Clone, Copy)]
pub struct InterruptStats {
    pub ticks: u64,
    pub keyboard_irqs: u64,
    pub mouse_irqs: u64,
    pub network_irqs: u64,
}

global_asm!(
    include_str!("interrupt_entry.S"),
    error_code_mask = const kernel::exception::ERROR_CODE_MASK,
);

extern "C" {
    fn genos_irq0_stub();
    fn genos_irq1_stub();
    fn genos_irq12_stub();
    fn genos_irq48_stub();
    static genos_vector_table: [unsafe extern "C" fn(); 256];
}

pub fn vector_handler(vector: usize) -> unsafe extern "C" fn() {
    // SAFETY: the linked assembly emits exactly 256 immutable function pointers.
    // Array indexing checks the caller's vector before reading the table.
    unsafe { genos_vector_table[vector] }
}

pub fn init() {
    arch::disable_interrupts();
    // SAFETY: BSP initialization, with maskable interrupts disabled, owns IDT
    // mutation and the legacy PIC/PIT. Every other vector was installed by arch.
    unsafe {
        arch::set_idt_handler(32, genos_irq0_stub);
        arch::set_idt_handler(33, genos_irq1_stub);
        arch::set_idt_handler(44, genos_irq12_stub);
        arch::set_idt_handler(crate::network_device::VIRTIO_MSIX_VECTOR, genos_irq48_stub);
        arch::set_user_idt_handler(userspace::SYSCALL_VECTOR, userspace::syscall_handler());
        remap_pic();
        init_pit_100hz();
    }
    crate::serial::println("EXCEPTION_ENTRY_READY vectors=256 fatal_ist=dedicated");
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

#[allow(dead_code)]
// Retained for the deferred graphical diagnostics surface.
pub fn stats() -> InterruptStats {
    InterruptStats {
        ticks: ticks(),
        keyboard_irqs: KEYBOARD_IRQS.load(Ordering::Relaxed),
        mouse_irqs: MOUSE_IRQS.load(Ordering::Relaxed),
        network_irqs: NETWORK_IRQS.load(Ordering::Relaxed),
    }
}

#[no_mangle]
extern "C" fn genos_irq0_rust(frame: *mut userspace::UserContext) -> u64 {
    TICKS.fetch_add(1, Ordering::Relaxed);
    let preempted = userspace::timer_preempt(frame);
    unsafe { pic_eoi(0) };
    u64::from(preempted)
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
extern "C" fn genos_irq48_rust() {
    NETWORK_IRQS.fetch_add(1, Ordering::Relaxed);
    crate::network_device::record_virtio_interrupt();
    // SAFETY: this MSI-X vector is routed to the BSP local APIC; acknowledge
    // its identity-mapped EOI register without touching protocol state.
    unsafe { core::ptr::write_volatile(0xfee0_00b0 as *mut u32, 0) };
}

#[no_mangle]
extern "C" fn genos_vector_rust(frame: &ExceptionFrame, sampled_cr2: u64) -> u64 {
    // Entry owns an aligned immutable frame on its TSS stack. Interrupt gates
    // clear IF. Fatal/NMI/debug stacks do not overlap the ordinary IRQ stack.
    let cr2 = if frame.vector == 14 { sampled_cr2 } else { 0 };
    if (32..48).contains(&frame.vector) {
        unexpected_pic_irq((frame.vector - 32) as u8);
        return 0;
    }
    print_fault(frame, cr2);
    if process_local(frame.vector, frame.cs)
        && userspace::terminate_current_fault(frame.vector as u8, frame.error, frame.rip, cr2)
    {
        return 1;
    }
    crate::serial::println("EXCEPTION_FATAL_HALT");
    arch::disable_interrupts();
    arch::halt_loop();
}

fn print_fault(frame: &ExceptionFrame, cr2: u64) {
    crate::serial::print("EXCEPTION_FRAME vector=");
    crate::serial::print_u64(frame.vector);
    crate::serial::print(" cpl=");
    crate::serial::print_u64(frame.cs & 3);
    for (label, value) in [
        (" error=0x", frame.error),
        (" rip=0x", frame.rip),
        (" cs=0x", frame.cs),
        (" rflags=0x", frame.rflags),
        (" rsp=0x", frame.rsp),
        (" ss=0x", frame.ss),
        (" cr2=0x", cr2),
    ] {
        crate::serial::print(label);
        crate::serial::print_hex(value);
    }
    crate::serial::println("");
}

fn unexpected_pic_irq(irq: u8) {
    // SAFETY: only the remapped PIC range reaches this function, so irq < 16.
    // IF is clear on the BSP; command/mask accesses cannot race normal IRQ work.
    unsafe {
        let in_service = match irq {
            7 => {
                arch::outb(PIC1_COMMAND, PIC_READ_ISR);
                arch::inb(PIC1_COMMAND) & 0x80 != 0
            }
            15 => {
                arch::outb(PIC2_COMMAND, PIC_READ_ISR);
                arch::inb(PIC2_COMMAND) & 0x80 != 0
            }
            _ => true,
        };
        if in_service {
            // An unowned real IRQ is masked before EOI to prevent a storm.
            let port = if irq < 8 { PIC1_DATA } else { PIC2_DATA };
            let mask = arch::inb(port);
            arch::outb(port, mask | (1 << (irq & 7)));
        }
        let (master, slave) = pic_eoi_policy(irq, in_service);
        if slave {
            arch::outb(PIC2_COMMAND, PIC_EOI);
        }
        if master {
            arch::outb(PIC1_COMMAND, PIC_EOI);
        }
    }
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
