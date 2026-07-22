use crate::display::Point;
use genos_abi::{
    UserInputEvent, USER_INPUT_KIND_KEY, USER_INPUT_KIND_POINTER_BUTTON,
    USER_INPUT_KIND_POINTER_MOVE, USER_INPUT_MASK_KEYBOARD, USER_INPUT_MASK_POINTER,
    USER_KEY_ARROW_DOWN, USER_KEY_ARROW_UP, USER_KEY_BACKSPACE, USER_KEY_CHAR, USER_KEY_ENTER,
    USER_KEY_ESCAPE, USER_KEY_TAB, USER_POINTER_BUTTON_LEFT, USER_POINTER_BUTTON_MIDDLE,
    USER_POINTER_BUTTON_RIGHT,
};

pub const EVENT_QUEUE_CAP: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyEvent {
    Char(u8),
    Enter,
    Backspace,
    Escape,
    Tab,
    ArrowUp,
    ArrowDown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MouseButtons {
    pub left: bool,
    pub right: bool,
    pub middle: bool,
}

impl MouseButtons {
    pub const fn empty() -> Self {
        Self {
            left: false,
            right: false,
            middle: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputEvent {
    Key(KeyEvent),
    MouseMove {
        dx: i32,
        dy: i32,
        buttons: MouseButtons,
    },
    MouseButton {
        position: Point,
        buttons: MouseButtons,
    },
}

impl InputEvent {
    pub fn user_mask(self) -> u64 {
        match self {
            Self::Key(_) => USER_INPUT_MASK_KEYBOARD,
            Self::MouseMove { .. } | Self::MouseButton { .. } => USER_INPUT_MASK_POINTER,
        }
    }

    pub fn to_user_event(self) -> UserInputEvent {
        match self {
            Self::Key(key) => {
                let (code, value0) = match key {
                    KeyEvent::Char(byte) => (USER_KEY_CHAR, byte as i64),
                    KeyEvent::Enter => (USER_KEY_ENTER, 0),
                    KeyEvent::Backspace => (USER_KEY_BACKSPACE, 0),
                    KeyEvent::Escape => (USER_KEY_ESCAPE, 0),
                    KeyEvent::Tab => (USER_KEY_TAB, 0),
                    KeyEvent::ArrowUp => (USER_KEY_ARROW_UP, 0),
                    KeyEvent::ArrowDown => (USER_KEY_ARROW_DOWN, 0),
                };
                UserInputEvent {
                    kind: USER_INPUT_KIND_KEY,
                    code,
                    value0,
                    value1: 0,
                }
            }
            Self::MouseMove { dx, dy, buttons } => UserInputEvent {
                kind: USER_INPUT_KIND_POINTER_MOVE,
                code: pointer_button_mask(buttons),
                value0: dx as i64,
                value1: dy as i64,
            },
            Self::MouseButton { position, buttons } => UserInputEvent {
                kind: USER_INPUT_KIND_POINTER_BUTTON,
                code: pointer_button_mask(buttons),
                value0: position.x as i64,
                value1: position.y as i64,
            },
        }
    }
}

fn pointer_button_mask(buttons: MouseButtons) -> u64 {
    let mut mask = 0;
    if buttons.left {
        mask |= USER_POINTER_BUTTON_LEFT;
    }
    if buttons.right {
        mask |= USER_POINTER_BUTTON_RIGHT;
    }
    if buttons.middle {
        mask |= USER_POINTER_BUTTON_MIDDLE;
    }
    mask
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MouseState {
    pub position: Point,
    pub buttons: MouseButtons,
}

impl MouseState {
    pub const fn new(position: Point) -> Self {
        Self {
            position,
            buttons: MouseButtons::empty(),
        }
    }

    pub fn apply_move(&mut self, dx: i32, dy: i32, max_x: i32, max_y: i32, buttons: MouseButtons) {
        let dx = accelerate_axis(dx);
        let dy = accelerate_axis(dy);
        self.position.x = (self.position.x + dx).clamp(0, max_x.max(0));
        self.position.y = (self.position.y - dy).clamp(0, max_y.max(0));
        self.buttons = buttons;
    }
}

fn accelerate_axis(value: i32) -> i32 {
    let magnitude = value.abs();
    let multiplier = if magnitude >= 18 {
        8
    } else if magnitude >= 8 {
        6
    } else if magnitude >= 3 {
        4
    } else {
        3
    };
    value * multiplier
}

#[derive(Clone, Copy)]
pub struct EventQueue {
    events: [Option<InputEvent>; EVENT_QUEUE_CAP],
    head: usize,
    tail: usize,
    len: usize,
    dropped: u64,
}

impl EventQueue {
    pub const fn new() -> Self {
        Self {
            events: [None; EVENT_QUEUE_CAP],
            head: 0,
            tail: 0,
            len: 0,
            dropped: 0,
        }
    }

    pub fn push(&mut self, event: InputEvent) {
        if self.len == EVENT_QUEUE_CAP {
            self.dropped += 1;
            self.events[self.tail] = Some(event);
            self.tail = (self.tail + 1) % EVENT_QUEUE_CAP;
            self.head = self.tail;
        } else {
            self.events[self.tail] = Some(event);
            self.tail = (self.tail + 1) % EVENT_QUEUE_CAP;
            self.len += 1;
        }
    }

    pub fn pop(&mut self) -> Option<InputEvent> {
        if self.len == 0 {
            return None;
        }
        let event = self.events[self.head].take();
        self.head = (self.head + 1) % EVENT_QUEUE_CAP;
        self.len -= 1;
        event
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn dropped(&self) -> u64 {
        self.dropped
    }
}

impl Default for EventQueue {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy)]
pub struct KeyboardDecoder {
    shift_left: bool,
    shift_right: bool,
    caps_lock: bool,
    extended: bool,
}

impl KeyboardDecoder {
    pub const fn new() -> Self {
        Self {
            shift_left: false,
            shift_right: false,
            caps_lock: false,
            extended: false,
        }
    }

    pub fn decode(&mut self, scancode: u8) -> Option<KeyEvent> {
        if scancode == 0xe0 {
            self.extended = true;
            return None;
        }

        let released = scancode & 0x80 != 0;
        let code = scancode & 0x7f;
        if code == 0x2a {
            self.shift_left = !released;
            return None;
        }
        if code == 0x36 {
            self.shift_right = !released;
            return None;
        }
        if released {
            self.extended = false;
            return None;
        }
        if code == 0x3a {
            self.caps_lock = !self.caps_lock;
            return None;
        }

        if self.extended {
            self.extended = false;
            return match code {
                0x48 => Some(KeyEvent::ArrowUp),
                0x50 => Some(KeyEvent::ArrowDown),
                _ => None,
            };
        }

        let shift = self.shift_left || self.shift_right;
        let event = match code {
            0x01 => Some(KeyEvent::Escape),
            0x0f => Some(KeyEvent::Tab),
            0x0e => Some(KeyEvent::Backspace),
            0x1c => Some(KeyEvent::Enter),
            0x02 => Some(KeyEvent::Char(if shift { b'!' } else { b'1' })),
            0x03 => Some(KeyEvent::Char(if shift { b'@' } else { b'2' })),
            0x04 => Some(KeyEvent::Char(if shift { b'#' } else { b'3' })),
            0x05 => Some(KeyEvent::Char(if shift { b'$' } else { b'4' })),
            0x06 => Some(KeyEvent::Char(if shift { b'%' } else { b'5' })),
            0x07 => Some(KeyEvent::Char(if shift { b'^' } else { b'6' })),
            0x08 => Some(KeyEvent::Char(if shift { b'&' } else { b'7' })),
            0x09 => Some(KeyEvent::Char(if shift { b'*' } else { b'8' })),
            0x0a => Some(KeyEvent::Char(if shift { b'(' } else { b'9' })),
            0x0b => Some(KeyEvent::Char(if shift { b')' } else { b'0' })),
            0x0c => Some(KeyEvent::Char(if shift { b'_' } else { b'-' })),
            0x0d => Some(KeyEvent::Char(if shift { b'+' } else { b'=' })),
            0x10 => Some(KeyEvent::Char(b'q')),
            0x11 => Some(KeyEvent::Char(b'w')),
            0x12 => Some(KeyEvent::Char(b'e')),
            0x13 => Some(KeyEvent::Char(b'r')),
            0x14 => Some(KeyEvent::Char(b't')),
            0x15 => Some(KeyEvent::Char(b'y')),
            0x16 => Some(KeyEvent::Char(b'u')),
            0x17 => Some(KeyEvent::Char(b'i')),
            0x18 => Some(KeyEvent::Char(b'o')),
            0x19 => Some(KeyEvent::Char(b'p')),
            0x1a => Some(KeyEvent::Char(if shift { b'{' } else { b'[' })),
            0x1b => Some(KeyEvent::Char(if shift { b'}' } else { b']' })),
            0x1e => Some(KeyEvent::Char(b'a')),
            0x1f => Some(KeyEvent::Char(b's')),
            0x20 => Some(KeyEvent::Char(b'd')),
            0x21 => Some(KeyEvent::Char(b'f')),
            0x22 => Some(KeyEvent::Char(b'g')),
            0x23 => Some(KeyEvent::Char(b'h')),
            0x24 => Some(KeyEvent::Char(b'j')),
            0x25 => Some(KeyEvent::Char(b'k')),
            0x26 => Some(KeyEvent::Char(b'l')),
            0x27 => Some(KeyEvent::Char(if shift { b':' } else { b';' })),
            0x28 => Some(KeyEvent::Char(if shift { b'"' } else { b'\'' })),
            0x29 => Some(KeyEvent::Char(if shift { b'~' } else { b'`' })),
            0x2b => Some(KeyEvent::Char(if shift { b'|' } else { b'\\' })),
            0x2c => Some(KeyEvent::Char(b'z')),
            0x2d => Some(KeyEvent::Char(b'x')),
            0x2e => Some(KeyEvent::Char(b'c')),
            0x2f => Some(KeyEvent::Char(b'v')),
            0x30 => Some(KeyEvent::Char(b'b')),
            0x31 => Some(KeyEvent::Char(b'n')),
            0x32 => Some(KeyEvent::Char(b'm')),
            0x33 => Some(KeyEvent::Char(if shift { b'<' } else { b',' })),
            0x34 => Some(KeyEvent::Char(if shift { b'>' } else { b'.' })),
            0x35 => Some(KeyEvent::Char(if shift { b'?' } else { b'/' })),
            0x39 => Some(KeyEvent::Char(b' ')),
            _ => None,
        };

        match event {
            Some(KeyEvent::Char(byte)) if byte.is_ascii_alphabetic() => {
                let upper = shift ^ self.caps_lock;
                Some(KeyEvent::Char(if upper {
                    byte.to_ascii_uppercase()
                } else {
                    byte
                }))
            }
            _ => event,
        }
    }
}

impl Default for KeyboardDecoder {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy)]
pub struct MouseDecoder {
    packet: [u8; 3],
    index: usize,
    last_buttons: MouseButtons,
}

impl MouseDecoder {
    pub const fn new() -> Self {
        Self {
            packet: [0; 3],
            index: 0,
            last_buttons: MouseButtons::empty(),
        }
    }

    pub fn push_byte(&mut self, byte: u8, position: Point) -> Option<InputEvent> {
        if self.index == 0 && byte & 0x08 == 0 {
            return None;
        }
        self.packet[self.index] = byte;
        self.index += 1;
        if self.index < 3 {
            return None;
        }
        self.index = 0;

        let status = self.packet[0];
        let mut dx = self.packet[1] as i32;
        let mut dy = self.packet[2] as i32;
        if status & 0x10 != 0 {
            dx -= 256;
        }
        if status & 0x20 != 0 {
            dy -= 256;
        }
        let buttons = MouseButtons {
            left: status & 0x01 != 0,
            right: status & 0x02 != 0,
            middle: status & 0x04 != 0,
        };
        let moved = dx != 0 || dy != 0;
        let changed = buttons != self.last_buttons;
        self.last_buttons = buttons;
        if changed && !moved {
            Some(InputEvent::MouseButton { position, buttons })
        } else {
            Some(InputEvent::MouseMove { dx, dy, buttons })
        }
    }
}

impl Default for MouseDecoder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyboard_decodes_letters_and_enter() {
        let mut decoder = KeyboardDecoder::new();
        assert_eq!(decoder.decode(0x23), Some(KeyEvent::Char(b'h')));
        assert_eq!(decoder.decode(0x1c), Some(KeyEvent::Enter));
        assert_eq!(decoder.decode(0x9c), None);
    }

    #[test]
    fn keyboard_tracks_shift_caps_and_arrows() {
        let mut decoder = KeyboardDecoder::new();
        assert_eq!(decoder.decode(0x2a), None);
        assert_eq!(decoder.decode(0x23), Some(KeyEvent::Char(b'H')));
        assert_eq!(decoder.decode(0xaa), None);
        assert_eq!(decoder.decode(0x3a), None);
        assert_eq!(decoder.decode(0x17), Some(KeyEvent::Char(b'I')));
        assert_eq!(decoder.decode(0xe0), None);
        assert_eq!(decoder.decode(0x48), Some(KeyEvent::ArrowUp));
    }

    #[test]
    fn event_queue_wraps_and_counts_drops() {
        let mut queue = EventQueue::new();
        for _ in 0..(EVENT_QUEUE_CAP + 2) {
            queue.push(InputEvent::Key(KeyEvent::Char(b'x')));
        }
        assert_eq!(queue.len(), EVENT_QUEUE_CAP);
        assert_eq!(queue.dropped(), 2);
    }

    #[test]
    fn mouse_packet_decodes_motion_and_buttons() {
        let mut decoder = MouseDecoder::new();
        assert_eq!(decoder.push_byte(0x29, Point::new(10, 10)), None);
        assert_eq!(decoder.push_byte(5, Point::new(10, 10)), None);
        assert_eq!(
            decoder.push_byte(253, Point::new(10, 10)),
            Some(InputEvent::MouseMove {
                dx: 5,
                dy: -3,
                buttons: MouseButtons {
                    left: true,
                    right: false,
                    middle: false,
                }
            })
        );
    }

    #[test]
    fn mouse_state_accelerates_motion() {
        let mut state = MouseState::new(Point::new(10, 10));
        state.apply_move(3, -3, 100, 100, MouseButtons::empty());
        assert_eq!(state.position, Point::new(22, 22));
    }

    #[test]
    fn user_events_preserve_key_pointer_and_button_data() {
        let key = InputEvent::Key(KeyEvent::Char(b'G'));
        assert_eq!(key.user_mask(), USER_INPUT_MASK_KEYBOARD);
        assert_eq!(
            key.to_user_event(),
            UserInputEvent {
                kind: USER_INPUT_KIND_KEY,
                code: USER_KEY_CHAR,
                value0: b'G' as i64,
                value1: 0,
            }
        );

        let movement = InputEvent::MouseMove {
            dx: -4,
            dy: 7,
            buttons: MouseButtons {
                left: true,
                right: false,
                middle: true,
            },
        };
        assert_eq!(movement.user_mask(), USER_INPUT_MASK_POINTER);
        assert_eq!(
            movement.to_user_event(),
            UserInputEvent {
                kind: USER_INPUT_KIND_POINTER_MOVE,
                code: USER_POINTER_BUTTON_LEFT | USER_POINTER_BUTTON_MIDDLE,
                value0: -4,
                value1: 7,
            }
        );

        assert_eq!(
            InputEvent::MouseButton {
                position: Point::new(320, 240),
                buttons: MouseButtons {
                    left: false,
                    right: true,
                    middle: false,
                },
            }
            .to_user_event(),
            UserInputEvent {
                kind: USER_INPUT_KIND_POINTER_BUTTON,
                code: USER_POINTER_BUTTON_RIGHT,
                value0: 320,
                value1: 240,
            }
        );
    }
}
