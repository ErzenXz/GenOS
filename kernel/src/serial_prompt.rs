//! Presentation-only state for redrawing the active serial input line.

use crate::console_text::FixedText;
use crate::input::KeyEvent;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputEcho {
    None,
    Char(u8),
    EraseLast,
    Redraw,
}

pub struct SerialPrompt {
    active: bool,
    input: FixedText,
    cursor: usize,
    redraw: bool,
    seek_end: bool,
}

impl SerialPrompt {
    pub const fn new() -> Self {
        Self {
            active: false,
            input: FixedText::empty(),
            cursor: 0,
            redraw: false,
            seek_end: false,
        }
    }

    pub fn activate(&mut self) {
        self.active = true;
    }

    pub fn finish_command(&mut self) {
        self.active = false;
        self.input = FixedText::empty();
        self.cursor = 0;
        self.redraw = false;
        self.seek_end = false;
    }

    pub fn note_key(&mut self, key: KeyEvent) -> InputEcho {
        if !self.active {
            return InputEcho::None;
        }
        let len = self.input.len();
        match key {
            KeyEvent::Char(byte) if len < genos_abi::USER_CONSOLE_TEXT_MAX => {
                let echo = if self.cursor == len {
                    InputEcho::Char(byte)
                } else {
                    InputEcho::Redraw
                };
                self.cursor += 1;
                self.redraw = echo == InputEcho::Redraw;
                echo
            }
            KeyEvent::Backspace if self.cursor > 0 => {
                let echo = if self.cursor == len {
                    InputEcho::EraseLast
                } else {
                    InputEcho::Redraw
                };
                self.cursor -= 1;
                self.redraw = echo == InputEcho::Redraw;
                echo
            }
            KeyEvent::Delete if self.cursor < len => {
                self.redraw = true;
                InputEcho::Redraw
            }
            KeyEvent::ArrowLeft if self.cursor > 0 => {
                self.cursor -= 1;
                self.redraw = true;
                InputEcho::Redraw
            }
            KeyEvent::ArrowRight if self.cursor < len => {
                self.cursor += 1;
                self.redraw = true;
                InputEcho::Redraw
            }
            KeyEvent::Home
            | KeyEvent::End
            | KeyEvent::ArrowUp
            | KeyEvent::ArrowDown
            | KeyEvent::Tab => {
                self.cursor = match key {
                    KeyEvent::Home => 0,
                    _ => len,
                };
                self.seek_end = key != KeyEvent::Home;
                self.redraw = true;
                InputEcho::Redraw
            }
            _ => InputEcho::None,
        }
    }

    /// Returns whether an editing key needs a full redraw after the shell has
    /// published its authoritative input line.
    pub fn set_input(&mut self, input: FixedText) -> bool {
        self.input = input;
        self.cursor = if self.seek_end {
            input.len()
        } else {
            self.cursor.min(input.len())
        };
        self.seek_end = false;
        let redraw = self.active && self.redraw;
        self.redraw = false;
        redraw
    }

    pub fn pending_line(&self) -> Option<&str> {
        self.active.then(|| self.input.as_str())
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }
}

impl Default for SerialPrompt {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn background_output_preserves_owned_input_and_command_output_does_not_redraw() {
        let mut prompt = SerialPrompt::new();
        assert_eq!(prompt.pending_line(), None);
        prompt.activate();
        prompt.set_input(FixedText::from_str("write /USER/A.TXT hello"));
        assert_eq!(prompt.pending_line(), Some("write /USER/A.TXT hello"));
        prompt.finish_command();
        assert_eq!(prompt.pending_line(), None);
        prompt.activate();
        assert_eq!(prompt.pending_line(), Some(""));
    }

    #[test]
    fn middle_edits_request_redraw_and_keep_cursor_inside_authoritative_line() {
        let mut prompt = SerialPrompt::new();
        prompt.activate();
        prompt.set_input(FixedText::from_str("abcd"));
        assert_eq!(prompt.note_key(KeyEvent::Home), InputEcho::Redraw);
        assert!(prompt.set_input(FixedText::from_str("abcd")));
        assert_eq!(prompt.cursor(), 0);
        assert_eq!(prompt.note_key(KeyEvent::Char(b'X')), InputEcho::Redraw);
        assert!(prompt.set_input(FixedText::from_str("Xabcd")));
        assert_eq!(prompt.cursor(), 1);
        assert_eq!(prompt.note_key(KeyEvent::Backspace), InputEcho::Redraw);
        assert!(prompt.set_input(FixedText::from_str("abcd")));
        assert_eq!(prompt.cursor(), 0);
        assert_eq!(prompt.note_key(KeyEvent::End), InputEcho::Redraw);
        assert!(prompt.set_input(FixedText::from_str("abcd")));
        assert_eq!(prompt.cursor(), 4);
        assert_eq!(prompt.note_key(KeyEvent::Char(b'!')), InputEcho::Char(b'!'));
    }
}
