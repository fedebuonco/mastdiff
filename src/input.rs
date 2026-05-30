//! Single-line text input widget with a byte-accurate cursor.
//!
//! [`TextInput`] is used for the project-file filter bar and the search query
//! box. All cursor movement and editing operations work correctly with
//! multi-byte UTF-8 codepoints; the cursor is always maintained at a valid
//! char boundary.

/// Simple editable single-line text field with a byte-accurate cursor.
#[derive(Debug, Default, Clone)]
pub struct TextInput {
    pub text: String,
    cursor: usize, // byte offset
}

impl TextInput {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// Display column (char count) of the cursor — used for terminal cursor positioning.
    pub fn cursor_col(&self) -> usize {
        self.text[..self.cursor].chars().count()
    }

    pub fn push(&mut self, c: char) {
        self.text.insert(self.cursor, c);
        self.cursor += c.len_utf8();
    }

    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let prev = prev_char_boundary(&self.text, self.cursor);
        self.text.remove(prev);
        self.cursor = prev;
    }

    pub fn delete_forward(&mut self) {
        if self.cursor < self.text.len() {
            self.text.remove(self.cursor);
        }
    }

    pub fn move_left(&mut self) {
        if self.cursor > 0 {
            self.cursor = prev_char_boundary(&self.text, self.cursor);
        }
    }

    pub fn move_right(&mut self) {
        if self.cursor < self.text.len() {
            let ch = self.text[self.cursor..].chars().next().unwrap();
            self.cursor += ch.len_utf8();
        }
    }

    pub fn move_home(&mut self) {
        self.cursor = 0;
    }

    pub fn move_end(&mut self) {
        self.cursor = self.text.len();
    }

    pub fn kill_to_end(&mut self) {
        self.text.truncate(self.cursor);
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
    }

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }
}

fn prev_char_boundary(s: &str, mut pos: usize) -> usize {
    pos -= 1;
    while !s.is_char_boundary(pos) {
        pos -= 1;
    }
    pos
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> TextInput {
        let mut ti = TextInput::new();
        for c in s.chars() { ti.push(c); }
        ti
    }

    // ── push / content ────────────────────────────────────────────────────

    #[test]
    fn push_builds_string() {
        let ti = t("hello");
        assert_eq!(ti.as_str(), "hello");
    }

    #[test]
    fn push_updates_cursor_to_end() {
        let ti = t("abc");
        assert_eq!(ti.cursor_col(), 3);
    }

    #[test]
    fn push_multibyte_char() {
        let mut ti = TextInput::new();
        ti.push('é'); // 2 bytes
        assert_eq!(ti.as_str(), "é");
        assert_eq!(ti.cursor_col(), 1);
    }

    // ── backspace ─────────────────────────────────────────────────────────

    #[test]
    fn backspace_removes_last_char() {
        let mut ti = t("abc");
        ti.backspace();
        assert_eq!(ti.as_str(), "ab");
    }

    #[test]
    fn backspace_at_start_is_noop() {
        let mut ti = TextInput::new();
        ti.backspace(); // should not panic
        assert_eq!(ti.as_str(), "");
    }

    #[test]
    fn backspace_multibyte() {
        let mut ti = t("aé");
        ti.backspace();
        assert_eq!(ti.as_str(), "a");
    }

    #[test]
    fn push_then_backspace_restores_original() {
        let mut ti = t("hello");
        ti.push('!');
        ti.backspace();
        assert_eq!(ti.as_str(), "hello");
    }

    // ── delete_forward ────────────────────────────────────────────────────

    #[test]
    fn delete_forward_removes_char_at_cursor() {
        let mut ti = t("abc");
        ti.move_home();
        ti.delete_forward();
        assert_eq!(ti.as_str(), "bc");
    }

    #[test]
    fn delete_forward_at_end_is_noop() {
        let mut ti = t("abc");
        ti.delete_forward();
        assert_eq!(ti.as_str(), "abc");
    }

    // ── cursor movement ───────────────────────────────────────────────────

    #[test]
    fn move_left_decrements_col() {
        let mut ti = t("abc");
        ti.move_left();
        assert_eq!(ti.cursor_col(), 2);
    }

    #[test]
    fn move_left_at_start_is_noop() {
        let mut ti = t("abc");
        ti.move_home();
        ti.move_left();
        assert_eq!(ti.cursor_col(), 0);
    }

    #[test]
    fn move_right_increments_col() {
        let mut ti = t("abc");
        ti.move_home();
        ti.move_right();
        assert_eq!(ti.cursor_col(), 1);
    }

    #[test]
    fn move_right_at_end_is_noop() {
        let mut ti = t("abc");
        ti.move_right();
        assert_eq!(ti.cursor_col(), 3);
    }

    #[test]
    fn home_moves_to_start() {
        let mut ti = t("abc");
        ti.move_home();
        assert_eq!(ti.cursor_col(), 0);
    }

    #[test]
    fn end_moves_to_end() {
        let mut ti = t("abc");
        ti.move_home();
        ti.move_end();
        assert_eq!(ti.cursor_col(), 3);
    }

    // ── insert at cursor position ─────────────────────────────────────────

    #[test]
    fn push_at_middle_inserts_correctly() {
        let mut ti = t("ac");
        ti.move_home();
        ti.move_right(); // cursor after 'a'
        ti.push('b');
        assert_eq!(ti.as_str(), "abc");
    }

    // ── kill_to_end ───────────────────────────────────────────────────────

    #[test]
    fn kill_to_end_removes_from_cursor() {
        let mut ti = t("hello world");
        ti.move_home();
        for _ in 0..5 { ti.move_right(); } // after "hello"
        ti.kill_to_end();
        assert_eq!(ti.as_str(), "hello");
    }

    #[test]
    fn kill_to_end_at_end_is_noop() {
        let mut ti = t("abc");
        ti.kill_to_end();
        assert_eq!(ti.as_str(), "abc");
    }

    // ── clear ─────────────────────────────────────────────────────────────

    #[test]
    fn clear_empties_text_and_resets_cursor() {
        let mut ti = t("something");
        ti.clear();
        assert_eq!(ti.as_str(), "");
        assert_eq!(ti.cursor_col(), 0);
    }

    // ── cursor_col with multibyte ─────────────────────────────────────────

    #[test]
    fn cursor_col_counts_chars_not_bytes() {
        let mut ti = TextInput::new();
        ti.push('a');
        ti.push('é'); // 2 bytes, 1 char
        ti.push('b');
        assert_eq!(ti.cursor_col(), 3); // 3 chars total
    }
}


