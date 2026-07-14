use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// A single-line editable text buffer with a movable cursor. All editing is
/// grapheme-cluster-safe: Backspace/Delete/Left/Right operate on whole
/// graphemes, so multi-scalar emoji never end up half-deleted.
#[derive(Clone, Debug, Default)]
pub struct TextLine {
    text: String,
    /// Byte offset of the cursor within `text`; always on a grapheme boundary.
    cursor: usize,
}

/// What a key did to the line — lets callers distinguish "the text changed"
/// (search must re-query) from "only the cursor moved".
pub enum TextLineEvent {
    Ignored,
    Moved,
    Edited,
}

impl TextLine {
    pub fn new(initial: impl Into<String>) -> TextLine {
        let text = initial.into();
        let cursor = text.len();
        TextLine { text, cursor }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Display width (terminal columns) of the text before the cursor — what
    /// render fns add to the input area's x to place the terminal cursor.
    pub fn cursor_cols(&self) -> u16 {
        self.text[..self.cursor].width() as u16
    }

    fn prev_boundary(&self) -> Option<usize> {
        self.text[..self.cursor]
            .grapheme_indices(true)
            .next_back()
            .map(|(i, _)| i)
    }

    fn next_boundary(&self) -> Option<usize> {
        self.text[self.cursor..]
            .graphemes(true)
            .next()
            .map(|g| self.cursor + g.len())
    }

    pub fn insert(&mut self, c: char) {
        self.text.insert(self.cursor, c);
        self.cursor += c.len_utf8();
        // A combining mark immediately following the insertion point fuses
        // with the just-inserted character into one grapheme cluster; snap
        // the cursor forward to the end of that cluster so it never lands
        // mid-grapheme.
        if !self.text.grapheme_indices(true).any(|(i, _)| i == self.cursor) {
            if let Some((i, g)) = self
                .text
                .grapheme_indices(true)
                .find(|(i, g)| *i <= self.cursor && self.cursor < i + g.len())
            {
                self.cursor = i + g.len();
            }
        }
    }

    pub fn backspace(&mut self) -> bool {
        match self.prev_boundary() {
            Some(start) => {
                self.text.replace_range(start..self.cursor, "");
                self.cursor = start;
                true
            }
            None => false,
        }
    }

    pub fn delete(&mut self) -> bool {
        match self.next_boundary() {
            Some(end) => {
                self.text.replace_range(self.cursor..end, "");
                true
            }
            None => false,
        }
    }

    pub fn left(&mut self) {
        if let Some(i) = self.prev_boundary() {
            self.cursor = i;
        }
    }

    pub fn right(&mut self) {
        if let Some(i) = self.next_boundary() {
            self.cursor = i;
        }
    }

    pub fn home(&mut self) {
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.cursor = self.text.len();
    }

    pub fn on_key(&mut self, key: KeyEvent) -> TextLineEvent {
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            return TextLineEvent::Ignored;
        }
        match key.code {
            KeyCode::Char(c) => {
                self.insert(c);
                TextLineEvent::Edited
            }
            KeyCode::Backspace => {
                if self.backspace() {
                    TextLineEvent::Edited
                } else {
                    TextLineEvent::Moved
                }
            }
            KeyCode::Delete => {
                if self.delete() {
                    TextLineEvent::Edited
                } else {
                    TextLineEvent::Moved
                }
            }
            KeyCode::Left => {
                self.left();
                TextLineEvent::Moved
            }
            KeyCode::Right => {
                self.right();
                TextLineEvent::Moved
            }
            KeyCode::Home => {
                self.home();
                TextLineEvent::Moved
            }
            KeyCode::End => {
                self.end();
                TextLineEvent::Moved
            }
            _ => TextLineEvent::Ignored,
        }
    }
}

impl std::ops::Deref for TextLine {
    type Target = str;
    fn deref(&self) -> &str {
        &self.text
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent};
    use unicode_segmentation::UnicodeSegmentation;

    #[test]
    fn backspace_removes_a_whole_multiscalar_grapheme() {
        // Regional-indicator flag: two scalars, one grapheme.
        let mut l = TextLine::new("hi🇨🇦");
        assert!(l.backspace());
        assert_eq!(l.text(), "hi");
        // ZWJ family emoji: many scalars, one grapheme.
        let mut l = TextLine::new("a👩‍👩‍👧‍👦");
        assert!(l.backspace());
        assert_eq!(l.text(), "a");
    }

    #[test]
    fn left_right_move_by_grapheme_and_clamp() {
        let mut l = TextLine::new("x👋y");
        l.left(); // before 'y'
        l.left(); // before the emoji
        assert_eq!(&l.text()[l.cursor()..], "👋y");
        l.right();
        assert_eq!(&l.text()[l.cursor()..], "y");
        l.right();
        l.right(); // clamped at end
        assert_eq!(l.cursor(), l.text().len());
    }

    #[test]
    fn home_end_delete_and_mid_insert() {
        let mut l = TextLine::new("First");
        l.home();
        assert!(l.delete()); // "irst"
        l.right();
        l.insert('X'); // "iXrst"
        assert_eq!(l.text(), "iXrst");
        l.end();
        assert!(!l.delete()); // nothing after the cursor
    }

    #[test]
    fn on_key_reports_edits_vs_moves_and_ignores_ctrl_chords() {
        use crossterm::event::KeyModifiers;
        let mut l = TextLine::new("");
        assert!(matches!(
            l.on_key(KeyEvent::from(KeyCode::Char('a'))),
            TextLineEvent::Edited
        ));
        assert!(matches!(
            l.on_key(KeyEvent::from(KeyCode::Right)),
            TextLineEvent::Moved
        )); // already at end: clamped, still a move event
        assert!(matches!(
            l.on_key(KeyEvent::from(KeyCode::Backspace)),
            TextLineEvent::Edited
        ));
        assert!(matches!(
            l.on_key(KeyEvent::from(KeyCode::Backspace)),
            TextLineEvent::Moved
        )); // empty: nothing removed
        assert!(matches!(
            l.on_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL)),
            TextLineEvent::Ignored
        ));
    }

    #[test]
    fn cursor_cols_is_display_width_not_bytes() {
        let mut l = TextLine::new("日本"); // 6 bytes, 4 display columns
        assert_eq!(l.cursor_cols(), 4);
        l.left();
        assert_eq!(l.cursor_cols(), 2);
    }

    proptest::proptest! {
        /// The spec §7 grapheme-editing property test: any sequence of edits
        /// leaves the cursor on a grapheme boundary and the text valid.
        #[test]
        fn editing_never_splits_graphemes(s in "\\PC{0,20}", ops in proptest::collection::vec(0..7usize, 0..60)) {
            let mut l = TextLine::new(s);
            for op in ops {
                match op {
                    0 => l.left(),
                    1 => l.right(),
                    2 => { l.backspace(); }
                    3 => { l.delete(); }
                    4 => l.home(),
                    5 => l.end(),
                    _ => l.insert('é'),
                }
                let c = l.cursor();
                proptest::prop_assert!(
                    c == l.text().len() || l.text().grapheme_indices(true).any(|(i, _)| i == c),
                    "cursor {c} not on a grapheme boundary of {:?}", l.text()
                );
            }
        }

        #[test]
        fn backspace_to_empty_never_panics(s in "\\PC{0,20}") {
            let mut l = TextLine::new(s);
            while l.backspace() {}
            proptest::prop_assert!(l.text().is_empty());
        }
    }
}
