//! Owned PS/2 decoding and bounded event storage, without hardware access.
//!
//! The hardware adapter must serialize all access, including observations.
//! This object alone supplies neither IRQ exclusion nor an SMP lock.

use crate::geometry::Point;
use crate::input::{EventQueue, InputEvent, KeyboardDecoder, MouseDecoder, MouseState};

pub struct InputState {
    queue: EventQueue,
    keyboard: KeyboardDecoder,
    mouse_decoder: MouseDecoder,
    mouse: MouseState,
    max_x: i32,
    max_y: i32,
}

impl InputState {
    pub const fn new() -> Self {
        Self {
            queue: EventQueue::new(),
            keyboard: KeyboardDecoder::new(),
            mouse_decoder: MouseDecoder::new(),
            mouse: MouseState::new(Point::new(640, 400)),
            max_x: 1279,
            max_y: 799,
        }
    }

    pub fn configure_pointer(&mut self, width: i32, height: i32) {
        self.max_x = width.saturating_sub(1).max(0);
        self.max_y = height.saturating_sub(1).max(0);
        self.mouse = MouseState::new(Point::new(
            (width / 2).clamp(0, self.max_x),
            (height / 2).clamp(0, self.max_y),
        ));
    }

    /// Consume only a byte whose controller status says output is available.
    /// Bit 5 selects the mouse; keyboard modifier and partial mouse-packet state
    /// persist independently across interleaved deliveries.
    pub fn accept_byte(&mut self, status: u8, byte: u8) {
        if status & 1 == 0 {
            return;
        }
        if status & 0x20 == 0 {
            if let Some(key) = self.keyboard.decode(byte) {
                self.queue.push(InputEvent::Key(key));
            }
        } else if let Some(event) = self.mouse_decoder.push_byte(byte, self.mouse.position) {
            match event {
                InputEvent::MouseMove { dx, dy, buttons } => {
                    self.mouse
                        .apply_move(dx, dy, self.max_x, self.max_y, buttons);
                }
                InputEvent::MouseButton { buttons, .. } => self.mouse.buttons = buttons,
                InputEvent::Key(_) => unreachable!(),
            }
            self.queue.push(event);
        }
    }

    pub fn pop(&mut self) -> Option<InputEvent> {
        self.queue.pop()
    }

    pub fn peek(&self) -> Option<InputEvent> {
        self.queue.peek()
    }

    pub fn depth(&self) -> usize {
        self.queue.len()
    }

    pub fn dropped(&self) -> u64 {
        self.queue.dropped()
    }

    pub fn mouse(&self) -> MouseState {
        self.mouse
    }
}

impl Default for InputState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::{KeyEvent, MouseButtons, EVENT_QUEUE_CAP};

    #[test]
    fn absent_bytes_leave_decoders_queue_and_pointer_unchanged() {
        let mut state = InputState::new();
        state.accept_byte(0, 0x2a);
        state.accept_byte(0x20, 0x09);
        state.accept_byte(1, 0x1e);
        assert_eq!(state.pop(), Some(InputEvent::Key(KeyEvent::Char(b'a'))));
        assert_eq!(state.mouse(), MouseState::new(Point::new(640, 400)));
        assert_eq!((state.depth(), state.dropped()), (0, 0));
    }

    #[test]
    fn interleaved_keyboard_and_mouse_preserve_independent_partial_state() {
        let mut state = InputState::new();
        state.accept_byte(1, 0x2a);
        state.accept_byte(0x21, 0x08);
        state.accept_byte(1, 0x1e);
        state.accept_byte(0x21, 1);
        state.accept_byte(1, 0xaa);
        state.accept_byte(0x21, 2);
        state.accept_byte(1, 0x30);
        assert_eq!(state.depth(), 3);
        assert_eq!(state.pop(), Some(InputEvent::Key(KeyEvent::Char(b'A'))));
        assert_eq!(
            state.pop(),
            Some(InputEvent::MouseMove {
                dx: 1,
                dy: 2,
                buttons: MouseButtons::empty(),
            })
        );
        assert_eq!(state.mouse().position, Point::new(643, 394));
        assert_eq!(state.pop(), Some(InputEvent::Key(KeyEvent::Char(b'b'))));
        assert_eq!(state.pop(), None);
    }

    #[test]
    fn overflow_keeps_newest_events_in_order_without_stalling_pointer_updates() {
        let mut state = InputState::new();
        for _ in 0..EVENT_QUEUE_CAP {
            state.accept_byte(1, 0x1e);
        }
        state.accept_byte(1, 0x30);
        for byte in [0x08, 1, 0] {
            state.accept_byte(0x21, byte);
        }
        assert_eq!((state.depth(), state.dropped()), (EVENT_QUEUE_CAP, 2));
        assert_eq!(state.peek(), Some(InputEvent::Key(KeyEvent::Char(b'a'))));
        assert_eq!(state.depth(), EVENT_QUEUE_CAP);
        for _ in 0..EVENT_QUEUE_CAP - 2 {
            assert_eq!(state.pop(), Some(InputEvent::Key(KeyEvent::Char(b'a'))));
        }
        assert_eq!(state.pop(), Some(InputEvent::Key(KeyEvent::Char(b'b'))));
        assert!(matches!(
            state.pop(),
            Some(InputEvent::MouseMove { dx: 1, dy: 0, .. })
        ));
        assert_eq!(state.mouse().position, Point::new(643, 400));
        assert_eq!(state.pop(), None);
    }

    #[test]
    fn movement_clamps_and_button_events_use_the_current_position() {
        let mut state = InputState::new();
        state.configure_pointer(10, 8);
        for byte in [0x28, 127, 129] {
            state.accept_byte(0x21, byte);
        }
        assert_eq!(state.mouse().position, Point::new(9, 7));
        state.pop();
        for byte in [0x09, 0, 0] {
            state.accept_byte(0x21, byte);
        }
        let buttons = MouseButtons {
            left: true,
            right: false,
            middle: false,
        };
        assert_eq!(
            state.pop(),
            Some(InputEvent::MouseButton {
                position: Point::new(9, 7),
                buttons
            })
        );
        assert_eq!(state.mouse().buttons, buttons);
    }

    #[test]
    fn degenerate_bounds_start_and_remain_at_the_only_valid_position() {
        for extent in [i32::MIN, -1, 0, 1] {
            let mut state = InputState::new();
            state.configure_pointer(extent, extent);
            assert_eq!(state.mouse().position, Point::new(0, 0));
            for byte in [0x08, 127, 127] {
                state.accept_byte(0x21, byte);
            }
            assert_eq!(state.mouse().position, Point::new(0, 0));
        }
    }

    #[test]
    fn movement_near_integer_bounds_clamps_without_wrapping() {
        let mut state = InputState::new();
        state.max_x = i32::MAX;
        state.max_y = i32::MAX;
        state.mouse.position = Point::new(i32::MAX - 1, i32::MAX - 1);
        for byte in [0x28, 127, 129] {
            state.accept_byte(0x21, byte);
        }
        assert_eq!(state.mouse.position, Point::new(i32::MAX, i32::MAX));
        state.mouse.position = Point::new(0, 0);
        for byte in [0x18, 129, 127] {
            state.accept_byte(0x21, byte);
        }
        assert_eq!(state.mouse.position, Point::new(0, 0));
    }
}
