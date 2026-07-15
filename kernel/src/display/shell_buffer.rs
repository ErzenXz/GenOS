pub const MAX_LINE_BYTES: usize = 160;
pub const MAX_SCROLLBACK_LINES: usize = 96;

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ShellLine {
    pub kind: LineKind,
    pub text: FixedText,
}

impl ShellLine {
    pub const fn empty() -> Self {
        Self {
            kind: LineKind::Output,
            text: FixedText::empty(),
        }
    }

    pub fn new(kind: LineKind, text: &str) -> Self {
        Self {
            kind,
            text: FixedText::from_str(text),
        }
    }
}

pub struct ShellBuffer {
    lines: [ShellLine; MAX_SCROLLBACK_LINES],
    len: usize,
}

impl ShellBuffer {
    pub const fn new() -> Self {
        Self {
            lines: [ShellLine::empty(); MAX_SCROLLBACK_LINES],
            len: 0,
        }
    }

    pub fn clear(&mut self) {
        self.len = 0;
    }

    pub fn push(&mut self, line: ShellLine) {
        if self.len < MAX_SCROLLBACK_LINES {
            self.lines[self.len] = line;
            self.len += 1;
        } else {
            let mut index = 1;
            while index < MAX_SCROLLBACK_LINES {
                self.lines[index - 1] = self.lines[index];
                index += 1;
            }
            self.lines[MAX_SCROLLBACK_LINES - 1] = line;
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn line(&self, index: usize) -> Option<&ShellLine> {
        self.lines.get(index).filter(|_| index < self.len)
    }

    pub fn visible_start(&self, max_lines: usize) -> usize {
        self.len.saturating_sub(max_lines)
    }

    pub fn wrap_count(text_len: usize, columns: usize) -> usize {
        if columns == 0 {
            0
        } else {
            text_len.max(1).div_ceil(columns)
        }
    }
}

impl Default for ShellBuffer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
pub fn classify_command(command: &str) -> LineKind {
    let command = trim(command);
    if command.is_empty() {
        LineKind::Status
    } else {
        let name = split_once_space(command).0;
        match name {
            "help" | "clear" | "mem" | "ls" | "cat" | "echo" | "uname" | "reboot" | "shutdown"
            | "about" | "whoami" | "ui" => LineKind::Prompt,
            _ => LineKind::Error,
        }
    }
}

#[cfg(test)]
fn split_once_space(text: &str) -> (&str, &str) {
    if let Some(index) = text.find(' ') {
        (&text[..index], trim(&text[index + 1..]))
    } else {
        (text, "")
    }
}

#[cfg(test)]
fn trim(mut text: &str) -> &str {
    while text.as_bytes().first() == Some(&b' ') {
        text = &text[1..];
    }
    while text.as_bytes().last() == Some(&b' ') {
        text = &text[..text.len() - 1];
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrollback_drops_oldest_line() {
        let mut buffer = ShellBuffer::new();
        for i in 0..(MAX_SCROLLBACK_LINES + 3) {
            let mut text = FixedText::from_str("line ");
            text.push_u64(i as u64);
            buffer.push(ShellLine {
                kind: LineKind::Output,
                text,
            });
        }
        assert_eq!(buffer.len(), MAX_SCROLLBACK_LINES);
        assert_eq!(buffer.line(0).unwrap().text.as_str(), "line 3");
    }

    #[test]
    fn wrapping_counts_continuation_rows() {
        assert_eq!(ShellBuffer::wrap_count(0, 12), 1);
        assert_eq!(ShellBuffer::wrap_count(12, 12), 1);
        assert_eq!(ShellBuffer::wrap_count(13, 12), 2);
    }

    #[test]
    fn command_classification_marks_unknown_as_error() {
        assert_eq!(classify_command("help"), LineKind::Prompt);
        assert_eq!(classify_command("wat"), LineKind::Error);
        assert_eq!(classify_command("   "), LineKind::Status);
    }
}
