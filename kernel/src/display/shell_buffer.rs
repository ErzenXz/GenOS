pub use crate::console_text::{FixedText, LineKind};

pub const MAX_SCROLLBACK_LINES: usize = 96;

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
}
