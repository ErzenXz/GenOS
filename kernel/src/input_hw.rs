use kernel::display::Point;
use kernel::input::{EventQueue, InputEvent, KeyboardDecoder, MouseDecoder, MouseState};

use crate::arch;

static mut QUEUE: EventQueue = EventQueue::new();
static mut KEYBOARD: KeyboardDecoder = KeyboardDecoder::new();
static mut MOUSE_DECODER: MouseDecoder = MouseDecoder::new();
static mut MOUSE_STATE: MouseState = MouseState::new(Point::new(640, 400));
static mut MAX_X: i32 = 1279;
static mut MAX_Y: i32 = 799;

pub fn init(width: i32, height: i32) {
    unsafe {
        MAX_X = width.saturating_sub(1).max(0);
        MAX_Y = height.saturating_sub(1).max(0);
        MOUSE_STATE = MouseState::new(Point::new(width / 2, height / 2));
        enable_ps2_mouse();
    }
}

pub fn keyboard_irq() {
    unsafe {
        let status = arch::inb(0x64);
        if status & 1 != 0 && status & 0x20 == 0 {
            let scancode = arch::inb(0x60);
            if let Some(key) = (*core::ptr::addr_of_mut!(KEYBOARD)).decode(scancode) {
                (*core::ptr::addr_of_mut!(QUEUE)).push(InputEvent::Key(key));
            }
        }
    }
}

pub fn mouse_irq() {
    unsafe {
        let status = arch::inb(0x64);
        if status & 1 != 0 && status & 0x20 != 0 {
            let byte = arch::inb(0x60);
            if let Some(event) =
                (*core::ptr::addr_of_mut!(MOUSE_DECODER)).push_byte(byte, MOUSE_STATE.position)
            {
                apply_mouse_event(event);
                (*core::ptr::addr_of_mut!(QUEUE)).push(event);
            }
        }
    }
}

pub fn poll() {
    unsafe {
        let mut guard = 0;
        while arch::inb(0x64) & 1 != 0 && guard < 8 {
            let status = arch::inb(0x64);
            if status & 0x20 != 0 {
                let byte = arch::inb(0x60);
                if let Some(event) =
                    (*core::ptr::addr_of_mut!(MOUSE_DECODER)).push_byte(byte, MOUSE_STATE.position)
                {
                    apply_mouse_event(event);
                    (*core::ptr::addr_of_mut!(QUEUE)).push(event);
                }
            } else {
                let scancode = arch::inb(0x60);
                if let Some(key) = (*core::ptr::addr_of_mut!(KEYBOARD)).decode(scancode) {
                    (*core::ptr::addr_of_mut!(QUEUE)).push(InputEvent::Key(key));
                }
            }
            guard += 1;
        }
    }
}

pub fn pop_event() -> Option<InputEvent> {
    unsafe { (*core::ptr::addr_of_mut!(QUEUE)).pop() }
}

pub fn event_depth() -> usize {
    unsafe { (*core::ptr::addr_of!(QUEUE)).len() }
}

pub fn mouse_state() -> MouseState {
    unsafe { MOUSE_STATE }
}

unsafe fn apply_mouse_event(event: InputEvent) {
    match event {
        InputEvent::MouseMove { dx, dy, buttons } => {
            (*core::ptr::addr_of_mut!(MOUSE_STATE)).apply_move(dx, dy, MAX_X, MAX_Y, buttons);
        }
        InputEvent::MouseButton { buttons, .. } => {
            MOUSE_STATE.buttons = buttons;
        }
        InputEvent::Key(_) => {}
    }
}

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

unsafe fn mouse_write(byte: u8) {
    wait_write();
    arch::outb(0x64, 0xd4);
    wait_write();
    arch::outb(0x60, byte);
}

unsafe fn mouse_read_ack() -> u8 {
    wait_read();
    arch::inb(0x60)
}

unsafe fn wait_write() {
    let mut spins = 100_000;
    while spins > 0 && arch::inb(0x64) & 0x02 != 0 {
        spins -= 1;
    }
}

unsafe fn wait_read() {
    let mut spins = 100_000;
    while spins > 0 && arch::inb(0x64) & 0x01 == 0 {
        spins -= 1;
    }
}
