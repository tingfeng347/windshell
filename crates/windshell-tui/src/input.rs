use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use zeroize::Zeroizing;

/// UTF-8 editor shared by ordinary and secret fields. No Debug implementation.
#[derive(Default)]
pub struct Input {
    pub text: Zeroizing<String>,
    pub cursor: usize,
}
impl Input {
    pub fn new(text: &str) -> Self {
        Self {
            text: Zeroizing::new(text.into()),
            cursor: text.len(),
        }
    }
    pub fn insert(&mut self, text: &str, multiline: bool) {
        // Terminals may paste LF, CRLF, or bare CR (e.g. tmux paste-buffer).
        // All remain data, never submit/navigation events.
        let normalized = Zeroizing::new(text.replace("\r\n", "\n"));
        let text = Zeroizing::new(normalized.replace('\r', "\n"));
        let text: Zeroizing<String> = Zeroizing::new(
            text.chars()
                .filter(|c| !c.is_control() || (multiline && *c == '\n'))
                .collect(),
        );
        self.text.insert_str(self.cursor, &text);
        self.cursor += text.len();
    }
    pub fn key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Left => {
                self.cursor = self.text[..self.cursor]
                    .char_indices()
                    .next_back()
                    .map(|(i, _)| i)
                    .unwrap_or(0)
            }
            KeyCode::Right => {
                self.cursor += self.text[self.cursor..]
                    .chars()
                    .next()
                    .map(char::len_utf8)
                    .unwrap_or(0)
            }
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = self.text.len(),
            KeyCode::Backspace if self.cursor > 0 => {
                let start = self.text[..self.cursor]
                    .char_indices()
                    .next_back()
                    .unwrap()
                    .0;
                self.text.replace_range(start..self.cursor, "");
                self.cursor = start;
            }
            KeyCode::Delete if self.cursor < self.text.len() => {
                let end = self.cursor + self.text[self.cursor..].chars().next().unwrap().len_utf8();
                self.text.replace_range(self.cursor..end, "");
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.text.clear();
                self.cursor = 0;
            }
            KeyCode::Char(c)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.insert(&c.to_string(), false)
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn editing_and_paste_preserve_utf8_without_submitting() {
        let mut input = Input::new("服务器");
        input.key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        input.insert("A", false);
        assert_eq!(input.text.as_str(), "服务A器");
        input.key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
        assert_eq!(input.text.as_str(), "服务A");
        input.insert("\r\nEND\r\n", true);
        assert_eq!(input.text.as_str(), "服务A\nEND\n");
    }

    #[test]
    fn terminal_paste_preserves_carriage_return_separated_key_lines() {
        let mut input = Input::default();
        input.insert("BEGIN\rBODY\rEND\r", true);
        assert_eq!(input.text.as_str(), "BEGIN\nBODY\nEND\n");
    }
}
