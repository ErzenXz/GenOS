//! Fixed-capacity shell line editing; no terminal bytes or syscalls here.

pub struct Editor<const N: usize, const H: usize> {
    line: [u8; N],
    len: usize,
    cursor: usize,
    history: [[u8; N]; H],
    history_lens: [usize; H],
    history_len: usize,
    history_cursor: usize,
    draft: [u8; N],
    draft_len: usize,
    draft_cursor: usize,
}

impl<const N: usize, const H: usize> Editor<N, H> {
    pub const fn new() -> Self {
        Self {
            line: [0; N],
            len: 0,
            cursor: 0,
            history: [[0; N]; H],
            history_lens: [0; H],
            history_len: 0,
            history_cursor: 0,
            draft: [0; N],
            draft_len: 0,
            draft_cursor: 0,
        }
    }

    pub fn line(&self) -> &[u8] {
        &self.line[..self.len]
    }
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn insert(&mut self, byte: u8) -> bool {
        if !(0x20..=0x7e).contains(&byte) || self.len == N {
            return false;
        }
        self.line
            .copy_within(self.cursor..self.len, self.cursor + 1);
        self.line[self.cursor] = byte;
        self.cursor += 1;
        self.len += 1;
        self.history_cursor = self.history_len;
        true
    }

    pub fn backspace(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }
        self.cursor -= 1;
        self.line
            .copy_within(self.cursor + 1..self.len, self.cursor);
        self.len -= 1;
        self.line[self.len] = 0;
        self.history_cursor = self.history_len;
        true
    }

    pub fn delete(&mut self) -> bool {
        if self.cursor == self.len {
            return false;
        }
        self.line
            .copy_within(self.cursor + 1..self.len, self.cursor);
        self.len -= 1;
        self.line[self.len] = 0;
        self.history_cursor = self.history_len;
        true
    }

    pub fn left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }
    pub fn right(&mut self) {
        self.cursor = (self.cursor + 1).min(self.len);
    }
    pub fn home(&mut self) {
        self.cursor = 0;
    }
    pub fn end(&mut self) {
        self.cursor = self.len;
    }

    pub fn clear_line(&mut self) {
        self.line = [0; N];
        self.len = 0;
        self.cursor = 0;
        self.history_cursor = self.history_len;
        self.draft_len = 0;
        self.draft_cursor = 0;
    }

    pub fn load(&mut self, text: &[u8]) -> bool {
        if text.len() > N || text.iter().any(|byte| !(0x20..=0x7e).contains(byte)) {
            return false;
        }
        self.line = [0; N];
        self.line[..text.len()].copy_from_slice(text);
        self.len = text.len();
        self.cursor = self.len;
        true
    }

    pub fn remember(&mut self) {
        if self.len == 0 || H == 0 {
            return;
        }
        let index = if self.history_len < H {
            let index = self.history_len;
            self.history_len += 1;
            index
        } else {
            for index in 1..H {
                self.history[index - 1] = self.history[index];
                self.history_lens[index - 1] = self.history_lens[index];
            }
            H - 1
        };
        self.history[index] = self.line;
        self.history_lens[index] = self.len;
        self.history_cursor = self.history_len;
    }

    pub fn history_up(&mut self) -> bool {
        if self.history_len == 0 || self.history_cursor == 0 {
            return false;
        }
        if self.history_cursor == self.history_len {
            self.draft = self.line;
            self.draft_len = self.len;
            self.draft_cursor = self.cursor;
        }
        self.history_cursor -= 1;
        self.line = self.history[self.history_cursor];
        self.len = self.history_lens[self.history_cursor];
        self.cursor = self.len;
        true
    }

    pub fn history_down(&mut self) -> bool {
        if self.history_cursor == self.history_len {
            return false;
        }
        self.history_cursor += 1;
        if self.history_cursor == self.history_len {
            self.line = self.draft;
            self.len = self.draft_len;
            self.cursor = self.draft_cursor;
        } else {
            self.line = self.history[self.history_cursor];
            self.len = self.history_lens[self.history_cursor];
            self.cursor = self.len;
        }
        true
    }

    pub fn complete_command(&mut self, words: &[u8]) -> bool {
        if self.line().contains(&b' ') || self.cursor != self.len {
            return false;
        }
        let mut matched = 0usize;
        let mut common: &[u8] = &[];
        for word in words
            .split(|byte| *byte == 0)
            .filter(|word| !word.is_empty())
        {
            if word.starts_with(self.line()) {
                if matched == 0 {
                    common = word;
                } else {
                    let mut shared = 0;
                    while shared < common.len()
                        && shared < word.len()
                        && common[shared] == word[shared]
                    {
                        shared += 1;
                    }
                    common = &common[..shared];
                }
                matched += 1;
            }
        }
        if matched == 0 || common.len() <= self.len || common.len() > N {
            return false;
        }
        self.load(common)
    }
}

impl<const N: usize, const H: usize> Default for Editor<N, H> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insertion_deletion_cursor_and_capacity_are_bounded() {
        let mut editor: Editor<5, 2> = Editor::new();
        for byte in b"abcd" {
            assert!(editor.insert(*byte));
        }
        editor.left();
        editor.left();
        assert!(editor.insert(b'X'));
        assert_eq!(editor.line(), b"abXcd");
        assert_eq!(editor.cursor(), 3);
        assert!(!editor.insert(b'!'));
        assert!(editor.delete());
        assert_eq!(editor.line(), b"abXd");
        assert!(editor.backspace());
        assert_eq!(editor.line(), b"abd");
        editor.home();
        assert!(!editor.backspace());
        editor.end();
        assert!(!editor.delete());
    }

    #[test]
    fn history_restores_draft_and_oldest_record_is_evicted() {
        let mut editor: Editor<12, 2> = Editor::new();
        for command in [b"uname".as_slice(), b"help", b"mem"] {
            assert!(editor.load(command));
            editor.remember();
            editor.clear_line();
        }
        assert!(editor.load(b"echo"));
        editor.left();
        assert!(editor.history_up());
        assert_eq!(editor.line(), b"mem");
        assert!(editor.history_up());
        assert_eq!(editor.line(), b"help");
        assert!(!editor.history_up());
        assert!(editor.history_down());
        assert_eq!(editor.line(), b"mem");
        assert!(editor.history_down());
        assert_eq!(editor.line(), b"echo");
        assert_eq!(editor.cursor(), 3);
    }

    #[test]
    fn completion_extends_only_shared_command_prefix() {
        let words: &[u8] = b"help\0hello\0mem\0";
        let mut editor: Editor<8, 1> = Editor::new();
        assert!(editor.load(b"h"));
        assert!(editor.complete_command(words));
        assert_eq!(editor.line(), b"hel");
        assert!(editor.load(b"mem"));
        assert!(!editor.complete_command(words));
        assert!(editor.load(b"m"));
        assert!(editor.complete_command(words));
        assert_eq!(editor.line(), b"mem");
    }
}
