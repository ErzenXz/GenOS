//! Bounded copyable console text and line semantics, independent of rendering.

pub const MAX_LINE_BYTES: usize = 160;

/// The serial console accepts printable ASCII as data. Control and extended
/// bytes are displayed as a single '?' so file contents cannot emit escapes.
pub const fn terminal_data_byte(byte: u8) -> u8 {
    if byte >= 0x20 && byte <= 0x7e {
        byte
    } else {
        b'?'
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LineKind {
    Prompt,
    Output,
    Error,
    Status,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FixedText {
    bytes: [u8; MAX_LINE_BYTES],
    len: usize,
}

impl FixedText {
    pub const fn empty() -> Self {
        Self {
            bytes: [0; MAX_LINE_BYTES],
            len: 0,
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(text: &str) -> Self {
        let mut fixed = Self::empty();
        fixed.push_str(text);
        fixed
    }

    pub fn push_str(&mut self, text: &str) {
        for byte in text.bytes() {
            if self.len >= MAX_LINE_BYTES {
                break;
            }
            self.bytes[self.len] = if byte.is_ascii() { byte } else { b'?' };
            self.len += 1;
        }
    }

    pub fn push_u64(&mut self, mut value: u64) {
        let mut buf = [0u8; 20];
        let mut index = buf.len();
        if value == 0 {
            self.push_str("0");
            return;
        }
        while value > 0 {
            index -= 1;
            buf[index] = b'0' + (value % 10) as u8;
            value /= 10;
        }
        for byte in &buf[index..] {
            self.push_byte(*byte);
        }
    }

    pub fn push_hex(&mut self, mut value: u64) {
        let digits = b"0123456789abcdef";
        let mut buf = [0u8; 16];
        for index in (0..16).rev() {
            buf[index] = digits[(value & 0xf) as usize];
            value >>= 4;
        }
        let mut started = false;
        for byte in buf {
            if byte != b'0' || started {
                started = true;
                self.push_byte(byte);
            }
        }
        if !started {
            self.push_str("0");
        }
    }

    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.len]).unwrap_or("")
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn push_byte(&mut self, byte: u8) {
        if self.len < MAX_LINE_BYTES {
            self.bytes[self.len] = byte;
            self.len += 1;
        }
    }
}

#[cfg(test)]
mod terminal_data_tests {
    use super::terminal_data_byte;

    #[test]
    fn every_data_byte_is_printable_and_controls_never_survive() {
        for byte in u8::MIN..=u8::MAX {
            let displayed = terminal_data_byte(byte);
            assert!((0x20..=0x7e).contains(&displayed));
            assert_eq!(
                displayed,
                if (0x20..=0x7e).contains(&byte) {
                    byte
                } else {
                    b'?'
                }
            );
        }
    }
}
