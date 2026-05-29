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
