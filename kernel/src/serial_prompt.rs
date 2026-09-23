//! Presentation-only state for redrawing the active serial input line.

use crate::console_text::FixedText;

pub struct SerialPrompt {
    active: bool,
    input: FixedText,
}

impl SerialPrompt {
    pub const fn new() -> Self {
        Self {
            active: false,
            input: FixedText::empty(),
        }
    }

    pub fn activate(&mut self) {
        self.active = true;
    }

    pub fn finish_command(&mut self) {
        self.active = false;
        self.input = FixedText::empty();
    }

    pub fn set_input(&mut self, input: FixedText) {
        self.input = input;
    }

    pub fn pending_line(&self) -> Option<&str> {
        self.active.then(|| self.input.as_str())
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
}
