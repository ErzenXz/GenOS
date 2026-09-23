//! Bounded byte-to-key decoder for the active serial terminal.

use crate::input::KeyEvent;

const PREFIX_TIMEOUT_TICKS: u64 = 10;
const PASTE_LIMIT: usize = 512;

#[derive(Clone, Copy, Eq, PartialEq)]
enum Mode {
    Idle,
    Escape,
    Csi,
    Ss3,
    Paste,
    PasteEscape,
    PasteCsi,
}

pub struct SerialKeys {
    mode: Mode,
    parameters: [u8; 8],
    len: usize,
    last_tick: u64,
    paste_bytes: usize,
    drop_paste: bool,
}

impl SerialKeys {
    pub const fn new() -> Self {
        Self {
            mode: Mode::Idle,
            parameters: [0; 8],
            len: 0,
            last_tick: 0,
            paste_bytes: 0,
            drop_paste: false,
        }
    }

    pub fn expire(&mut self, tick: u64) -> Option<KeyEvent> {
        if matches!(self.mode, Mode::Escape | Mode::Csi | Mode::Ss3)
            && tick.saturating_sub(self.last_tick) >= PREFIX_TIMEOUT_TICKS
        {
            self.mode = Mode::Idle;
            self.len = 0;
            return Some(KeyEvent::Cancel);
        }
        None
    }

    pub fn feed(&mut self, byte: u8, tick: u64) -> Option<KeyEvent> {
        self.last_tick = tick;
        if byte == 3 {
            self.mode = Mode::Idle;
            self.len = 0;
            self.drop_paste = false;
            return Some(KeyEvent::Cancel);
        }
        match self.mode {
            Mode::Idle => match byte {
                0x1b => {
                    self.mode = Mode::Escape;
                    None
                }
                b'\r' | b'\n' => Some(KeyEvent::Enter),
                8 | 0x7f => Some(KeyEvent::Backspace),
                b'\t' => Some(KeyEvent::Tab),
                1 => Some(KeyEvent::Home),
                5 => Some(KeyEvent::End),
                4 => Some(KeyEvent::Delete),
                0x20..=0x7e => Some(KeyEvent::Char(byte)),
                _ => None,
            },
            Mode::Escape => {
                self.len = 0;
                self.mode = match byte {
                    b'[' => Mode::Csi,
                    b'O' => Mode::Ss3,
                    _ => Mode::Idle,
                };
                None
            }
            Mode::Ss3 => {
                self.mode = Mode::Idle;
                match byte {
                    b'H' => Some(KeyEvent::Home),
                    b'F' => Some(KeyEvent::End),
                    _ => None,
                }
            }
            Mode::Csi => {
                if (0x30..=0x3f).contains(&byte) {
                    if self.len < self.parameters.len() {
                        self.parameters[self.len] = byte;
                        self.len += 1;
                    } else {
                        self.parameters = [0; 8];
                        self.len = self.parameters.len();
                    }
                    return None;
                }
                self.mode = Mode::Idle;
                let parameters = &self.parameters[..self.len];
                let key = match (parameters, byte) {
                    (b"", b'A') => Some(KeyEvent::ArrowUp),
                    (b"", b'B') => Some(KeyEvent::ArrowDown),
                    (b"", b'C') => Some(KeyEvent::ArrowRight),
                    (b"", b'D') => Some(KeyEvent::ArrowLeft),
                    (b"" | b"1" | b"7", b'H') => Some(KeyEvent::Home),
                    (b"" | b"4" | b"8", b'F') => Some(KeyEvent::End),
                    (b"1" | b"7", b'~') => Some(KeyEvent::Home),
                    (b"4" | b"8", b'~') => Some(KeyEvent::End),
                    (b"3", b'~') => Some(KeyEvent::Delete),
                    (b"200", b'~') => {
                        self.mode = Mode::Paste;
                        self.paste_bytes = 0;
                        self.drop_paste = false;
                        None
                    }
                    _ => None,
                };
                self.len = 0;
                key
            }
            Mode::Paste => {
                if byte == 0x1b {
                    self.mode = Mode::PasteEscape;
                    return None;
                }
                self.paste_byte(byte)
            }
            Mode::PasteEscape => {
                if byte == b'[' {
                    self.mode = Mode::PasteCsi;
                    self.len = 0;
                } else {
                    self.mode = Mode::Paste;
                }
                None
            }
            Mode::PasteCsi => {
                if (0x30..=0x3f).contains(&byte) {
                    if self.len < self.parameters.len() {
                        self.parameters[self.len] = byte;
                        self.len += 1;
                    } else {
                        self.len = self.parameters.len();
                    }
                    return None;
                }
                if self.parameters[..self.len] == *b"201" && byte == b'~' {
                    self.mode = Mode::Idle;
                    self.drop_paste = false;
                } else {
                    self.mode = Mode::Paste;
                }
                self.len = 0;
                None
            }
        }
    }

    fn paste_byte(&mut self, byte: u8) -> Option<KeyEvent> {
        if self.drop_paste {
            return None;
        }
        self.paste_bytes += 1;
        if self.paste_bytes > PASTE_LIMIT {
            self.drop_paste = true;
            return Some(KeyEvent::Cancel);
        }
        match byte {
            b'\r' | b'\n' | b'\t' => Some(KeyEvent::Char(b' ')),
            0x20..=0x7e => Some(KeyEvent::Char(byte)),
            _ => None,
        }
    }
}

impl Default for SerialKeys {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{vec, vec::Vec};

    fn decode(input: &[u8]) -> Vec<KeyEvent> {
        let mut decoder = SerialKeys::new();
        input
            .iter()
            .filter_map(|byte| decoder.feed(*byte, 1))
            .collect()
    }

    #[test]
    fn common_cursor_history_and_delete_sequences_are_single_keys() {
        assert_eq!(
            decode(b"\x1b[A\x1b[B\x1b[C\x1b[D\x1b[H\x1b[F\x1b[3~"),
            vec![
                KeyEvent::ArrowUp,
                KeyEvent::ArrowDown,
                KeyEvent::ArrowRight,
                KeyEvent::ArrowLeft,
                KeyEvent::Home,
                KeyEvent::End,
                KeyEvent::Delete
            ]
        );
        assert_eq!(
            decode(b"\x1bOH\x1bOF\x01\x05\x04"),
            vec![
                KeyEvent::Home,
                KeyEvent::End,
                KeyEvent::Home,
                KeyEvent::End,
                KeyEvent::Delete
            ]
        );
    }

    #[test]
    fn malformed_sequences_and_stale_prefix_never_become_commands() {
        assert_eq!(decode(b"\x1b[999~\x1b[2J\x1bOx"), vec![]);
        let mut decoder = SerialKeys::new();
        assert_eq!(decoder.feed(0x1b, 2), None);
        assert_eq!(decoder.expire(12), Some(KeyEvent::Cancel));
        assert_eq!(decoder.feed(b'X', 12), Some(KeyEvent::Char(b'X')));
    }

    #[test]
    fn bracketed_paste_never_sends_enter_and_overflow_cancels() {
        assert_eq!(
            decode(b"\x1b[200~ab\r\n\x1b[2Jcd\x1b[201~!"),
            vec![
                KeyEvent::Char(b'a'),
                KeyEvent::Char(b'b'),
                KeyEvent::Char(b' '),
                KeyEvent::Char(b' '),
                KeyEvent::Char(b'c'),
                KeyEvent::Char(b'd'),
                KeyEvent::Char(b'!')
            ]
        );
        let mut decoder = SerialKeys::new();
        for byte in b"\x1b[200~" {
            assert_eq!(decoder.feed(*byte, 1), None);
        }
        for _ in 0..PASTE_LIMIT {
            assert_eq!(decoder.feed(b'x', 1), Some(KeyEvent::Char(b'x')));
        }
        assert_eq!(decoder.feed(b'x', 1), Some(KeyEvent::Cancel));
        assert_eq!(decoder.feed(b'\n', 1), None);
        for byte in b"\x1b[201~" {
            assert_eq!(decoder.feed(*byte, 1), None);
        }
        assert_eq!(decoder.feed(b'Z', 1), Some(KeyEvent::Char(b'Z')));
    }
}
