// Legacy PS/2 input remains compiled while the console is serial-first.
// One admitted BSP owns the state; NMI/emergency entries cannot call this module.

use core::ptr::NonNull;
use kernel::context_slot::ContextSlot;
use kernel::input::{InputEvent, MouseState};
use kernel::input_state::InputState;

use crate::arch;

static mut STATE: InputState = InputState::new();
static ACCESS: ContextSlot<InputState> = ContextSlot::new();
const POLL_BYTE_BUDGET: usize = 8;

// Private callbacks are bounded, never enable IRQs or switch contexts, and
// return owned values. Publication and borrow guards drop before IF restores.
fn with_input<R>(access: impl for<'a> FnOnce(&'a mut InputState) -> R) -> R {
    arch::without_interrupts(|| {
        let pointer = NonNull::new(core::ptr::addr_of_mut!(STATE)).unwrap();
        // SAFETY: BSP admission excludes other CPUs; local IRQs are masked.
        // STATE is permanent/stationary and has no owner reference or external
        // access. Only this private adapter creates a reference. Emergency/NMI
        // entries do not enter Rust input paths. ContextSlot refuses nested
        // publication/access before dereference, and HRTB prevents borrow escape.
        let result = unsafe { ACCESS.run(pointer, || ACCESS.with_current(access)) };
        match result {
            Ok(Ok(Some(value))) => value,
            _ => {
                crate::serial::println("INPUT_ACCESS_REJECTED policy=halt");
                arch::halt_loop();
            }
        }
    })
}

fn require_masked() {
    if arch::interrupts_enabled() {
        arch::disable_interrupts();
        crate::serial::println("INPUT_CONTEXT_REJECTED policy=halt");
        arch::halt_loop();
    }
}

#[allow(dead_code)]
pub fn init(width: i32, height: i32) {
    // Controller configuration is boot-only, before ordinary IRQ delivery.
    require_masked();
    with_input(|state| state.configure_pointer(width, height));
    // SAFETY: the BSP owns PS/2 configuration with IF clear, and no state borrow
    // is held during these bounded controller/ACK waits. This is not runtime IRQ work.
    unsafe { enable_ps2_mouse() };
}

// Read at most one ready byte. IRQ callers cannot consume the other device's
// pending byte; the coordinator poll accepts either source.
fn read_controller(mouse: Option<bool>) -> Option<(u8, u8)> {
    // SAFETY: callers hold the private masked input scope on the admitted BSP.
    // These byte ports belong to the legacy PS/2 controller; no blocking wait.
    unsafe {
        let status = arch::inb(0x64);
        if status & 1 == 0 || mouse.is_some_and(|mouse| mouse != (status & 0x20 != 0)) {
            None
        } else {
            Some((status, arch::inb(0x60)))
        }
    }
}

pub fn keyboard_irq() {
    require_masked();
    consume_irq(false);
}

pub fn mouse_irq() {
    require_masked();
    consume_irq(true);
}

fn consume_irq(mouse: bool) {
    with_input(|state| {
        if let Some((status, byte)) = read_controller(Some(mouse)) {
            state.accept_byte(status, byte);
        }
    });
}

#[allow(dead_code)]
pub fn poll() {
    with_input(|state| {
        for _ in 0..POLL_BYTE_BUDGET {
            let Some((status, byte)) = read_controller(None) else {
                break;
            };
            state.accept_byte(status, byte);
        }
    });
}

#[allow(dead_code)]
pub fn pop_event() -> Option<InputEvent> {
    with_input(InputState::pop)
}

#[allow(dead_code)]
pub fn peek_event() -> Option<InputEvent> {
    with_input(|state| state.peek())
}

#[allow(dead_code)]
pub fn event_depth() -> usize {
    with_input(|state| state.depth())
}

#[allow(dead_code)]
pub fn mouse_state() -> MouseState {
    with_input(|state| state.mouse())
}

#[allow(dead_code)]
unsafe fn enable_ps2_mouse() {
    wait_write();
    arch::outb(0x64, 0xa8);
    wait_write();
    arch::outb(0x64, 0x20);
    wait_read();
    let status = arch::inb(0x60) | 0x02;
    wait_write();
    arch::outb(0x64, 0x60);
    wait_write();
    arch::outb(0x60, status);
    mouse_write(0xf6);
    let _ = mouse_read_ack();
    mouse_write(0xf4);
    let _ = mouse_read_ack();
}

#[allow(dead_code)]
unsafe fn mouse_write(byte: u8) {
    wait_write();
    arch::outb(0x64, 0xd4);
    wait_write();
    arch::outb(0x60, byte);
}

#[allow(dead_code)]
unsafe fn mouse_read_ack() -> u8 {
    wait_read();
    arch::inb(0x60)
}

#[allow(dead_code)]
unsafe fn wait_write() {
    let mut spins = 100_000;
    while spins > 0 && arch::inb(0x64) & 0x02 != 0 {
        spins -= 1;
    }
}

#[allow(dead_code)]
unsafe fn wait_read() {
    let mut spins = 100_000;
    while spins > 0 && arch::inb(0x64) & 0x01 == 0 {
        spins -= 1;
    }
}
