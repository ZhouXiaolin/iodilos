//! Pure editing model: a `'\n'`-separated buffer + char-index cursor.

/// An editable prompt buffer with a char-index cursor.
///
/// `'\n'` marks a hard line break (Shift/Alt+Enter). The cursor is a char index
/// in `0..=len` and is kept in bounds by every operation. Char-index (not byte)
/// so UTF-8 grapheme boundaries — 中文, emoji — never panic.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PromptModel {
    buffer: String,
    cursor: usize,
}

impl PromptModel {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn buffer(&self) -> &str {
        &self.buffer
    }

    pub fn cursor_char(&self) -> usize {
        self.cursor
    }

    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    fn len_chars(&self) -> usize {
        self.buffer.chars().count()
    }

    /// Byte offset of the char at `char_idx` (or buffer end if out of range).
    fn byte_at_char(&self, char_idx: usize) -> usize {
        self.buffer
            .char_indices()
            .nth(char_idx)
            .map(|(b, _)| b)
            .unwrap_or_else(|| self.buffer.len())
    }

    pub fn insert_char(&mut self, ch: char) {
        let byte = self.byte_at_char(self.cursor);
        self.buffer.insert(byte, ch);
        self.cursor += 1;
    }

    /// Convenience for hard line breaks (Shift/Alt+Enter).
    pub fn newline(&mut self) {
        self.insert_char('\n');
    }

    /// Insert a multi-char string at the cursor (helper for tests / paste).
    pub fn insert_str(&mut self, s: &str) {
        for ch in s.chars() {
            self.insert_char(ch);
        }
    }

    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let start = self.byte_at_char(self.cursor - 1);
        let end = self.byte_at_char(self.cursor);
        self.buffer.drain(start..end);
        self.cursor -= 1;
    }

    pub fn move_left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn move_right(&mut self) {
        self.cursor = (self.cursor + 1).min(self.len_chars());
    }

    /// Take the buffer (returns its contents), reset to empty.
    pub fn submit(&mut self) -> String {
        self.cursor = 0;
        std::mem::take(&mut self.buffer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_is_empty_cursor_zero() {
        let m = PromptModel::new();
        assert!(m.is_empty());
        assert_eq!(m.buffer(), "");
        assert_eq!(m.cursor_char(), 0);
    }

    #[test]
    fn insert_advances_cursor() {
        let mut m = PromptModel::new();
        m.insert_char('a');
        m.insert_char('b');
        assert_eq!(m.buffer(), "ab");
        assert_eq!(m.cursor_char(), 2);
    }

    #[test]
    fn insert_into_middle() {
        let mut m = PromptModel::new();
        m.insert_str("ac");
        m.move_left();
        m.insert_char('b');
        assert_eq!(m.buffer(), "abc");
        assert_eq!(m.cursor_char(), 2);
    }

    #[test]
    fn backspace_removes_before_cursor() {
        let mut m = PromptModel::new();
        m.insert_str("abc");
        m.move_left();
        m.backspace(); // removes 'b'
        assert_eq!(m.buffer(), "ac");
        assert_eq!(m.cursor_char(), 1);
    }

    #[test]
    fn backspace_at_start_is_noop() {
        let mut m = PromptModel::new();
        m.backspace();
        assert_eq!(m.cursor_char(), 0);
    }

    #[test]
    fn newline_inserts_hard_break() {
        let mut m = PromptModel::new();
        m.insert_str("ab");
        m.move_left(); // cursor between a and b
        m.newline();
        assert_eq!(m.buffer(), "a\nb");
        assert_eq!(m.cursor_char(), 2); // right after the '\n'
    }

    #[test]
    fn backspace_across_newline() {
        let mut m = PromptModel::new();
        m.insert_str("a\nb"); // cursor at end
        m.backspace(); // removes 'b'
        m.backspace(); // removes '\n'
        assert_eq!(m.buffer(), "a");
    }

    #[test]
    fn move_left_right_clamp() {
        let mut m = PromptModel::new();
        m.move_left();
        assert_eq!(m.cursor_char(), 0);
        m.insert_str("ab");
        m.move_right(); // already at end
        assert_eq!(m.cursor_char(), 2);
        m.move_left();
        m.move_left();
        assert_eq!(m.cursor_char(), 0);
    }

    #[test]
    fn handles_unicode() {
        let mut m = PromptModel::new();
        m.insert_str("你好");
        assert_eq!(m.cursor_char(), 2);
        m.move_left();
        m.insert_char('世');
        assert_eq!(m.buffer(), "你世好");
        assert_eq!(m.cursor_char(), 2);
    }

    #[test]
    fn handles_emoji() {
        let mut m = PromptModel::new();
        m.insert_char('🦀');
        assert_eq!(m.buffer(), "🦀");
        assert_eq!(m.cursor_char(), 1);
        m.backspace();
        assert!(m.is_empty());
    }

    #[test]
    fn submit_returns_and_clears() {
        let mut m = PromptModel::new();
        m.insert_str("hello");
        let out = m.submit();
        assert_eq!(out, "hello");
        assert!(m.is_empty());
        assert_eq!(m.cursor_char(), 0);
    }
}
