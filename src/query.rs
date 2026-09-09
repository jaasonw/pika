//! Single-line search text with a byte-offset cursor.
//!
//! The picker handles word-wise editing and leaves plain Left and Right for grid navigation.
//! The cursor is always on a `char` boundary.

#[derive(Default, Debug)]
pub struct Query {
    text: String,
    cursor: usize,
}

impl Query {
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// Insert typed text at the cursor. Takes a whole `&str` rather than a `char` so a
    /// compose sequence or a paste arrives as one edit.
    pub fn insert(&mut self, s: &str) -> bool {
        if s.is_empty() {
            return false;
        }
        self.text.insert_str(self.cursor, s);
        self.cursor += s.len();
        true
    }

    /// Delete the character before the cursor.
    ///
    /// A `char`, not a grapheme: an emoji with a skin-tone modifier typed into the search
    /// box comes apart into its parts. Nothing is searchable by emoji, so this has not been
    /// worth a Unicode segmentation dependency.
    pub fn backspace(&mut self) -> bool {
        let Some(prev) = self.prev_boundary(self.cursor) else {
            return false;
        };
        self.text.replace_range(prev..self.cursor, "");
        self.cursor = prev;
        true
    }

    /// Delete the character after the cursor.
    pub fn delete(&mut self) -> bool {
        let Some(next) = self.next_boundary(self.cursor) else {
            return false;
        };
        self.text.replace_range(self.cursor..next, "");
        true
    }

    /// Delete the previous word for Ctrl+W and Ctrl+Backspace.
    pub fn delete_word_back(&mut self) -> bool {
        let start = self.word_start();
        if start == self.cursor {
            return false;
        }
        self.text.replace_range(start..self.cursor, "");
        self.cursor = start;
        true
    }

    /// Delete the next word for Ctrl+Delete.
    pub fn delete_word_forward(&mut self) -> bool {
        let end = self.word_end();
        if end == self.cursor {
            return false;
        }
        self.text.replace_range(self.cursor..end, "");
        true
    }

    pub fn clear(&mut self) -> bool {
        if self.text.is_empty() {
            return false;
        }
        self.text.clear();
        self.cursor = 0;
        true
    }

    pub fn word_left(&mut self) -> bool {
        let start = self.word_start();
        let moved = start != self.cursor;
        self.cursor = start;
        moved
    }

    pub fn word_right(&mut self) -> bool {
        let end = self.word_end();
        let moved = end != self.cursor;
        self.cursor = end;
        moved
    }

    pub fn home(&mut self) -> bool {
        let moved = self.cursor != 0;
        self.cursor = 0;
        moved
    }

    pub fn end(&mut self) -> bool {
        let moved = self.cursor != self.text.len();
        self.cursor = self.text.len();
        moved
    }

    /// Put the cursor at a byte offset, snapping to the nearest boundary at or below it.
    /// Used for click-to-position, where the offset comes out of a Pango layout.
    pub fn set_cursor(&mut self, at: usize) {
        let at = at.min(self.text.len());
        self.cursor = (0..=at)
            .rev()
            .find(|i| self.text.is_char_boundary(*i))
            .unwrap_or(0);
    }

    /// Start of the word before the cursor: skip any run of spaces, then the word itself.
    fn word_start(&self) -> usize {
        let head = &self.text[..self.cursor];
        let trimmed = head.trim_end_matches(' ');
        match trimmed.rfind(' ') {
            Some(i) => i + 1,
            None => 0,
        }
    }

    /// End of the word after the cursor, by the same rule in the other direction.
    fn word_end(&self) -> usize {
        let tail = &self.text[self.cursor..];
        let lead = tail.len() - tail.trim_start_matches(' ').len();
        match tail[lead..].find(' ') {
            Some(i) => self.cursor + lead + i,
            None => self.text.len(),
        }
    }

    fn prev_boundary(&self, from: usize) -> Option<usize> {
        (0..from).rev().find(|i| self.text.is_char_boundary(*i))
    }

    fn next_boundary(&self, from: usize) -> Option<usize> {
        (from + 1..=self.text.len()).find(|i| self.text.is_char_boundary(*i))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(text: &str) -> Query {
        let mut q = Query::default();
        q.insert(text);
        q
    }

    #[test]
    fn typing_appends_at_the_cursor() {
        let mut q = q("cat");
        assert_eq!((q.text(), q.cursor()), ("cat", 3));
        q.home();
        q.insert("a ");
        assert_eq!((q.text(), q.cursor()), ("a cat", 2));
    }

    #[test]
    fn backspace_and_delete_work_from_the_middle() {
        let mut q = q("cat");
        q.set_cursor(2);
        assert!(q.backspace());
        assert_eq!((q.text(), q.cursor()), ("ct", 1));
        assert!(q.delete());
        assert_eq!((q.text(), q.cursor()), ("c", 1));
        assert!(!q.delete());
        assert_eq!(q.cursor(), 1);
    }

    #[test]
    fn edits_at_the_ends_report_that_they_did_nothing() {
        let mut q = Query::default();
        assert!(!q.backspace());
        assert!(!q.delete());
        assert!(!q.word_left());
        assert!(!q.word_right());
        assert!(!q.home());
        assert!(!q.end());
        assert!(!q.clear());
    }

    #[test]
    fn multibyte_characters_are_not_split() {
        let mut q = q("a\u{1f600}b");
        q.set_cursor(1 + "\u{1f600}".len());
        assert!(q.backspace());
        assert_eq!(q.text(), "ab");
    }

    #[test]
    fn word_motion_skips_the_spaces_before_the_word() {
        let mut q = q("grinning face");
        assert!(q.word_left());
        assert_eq!(q.cursor(), 9);
        assert!(q.word_left());
        assert_eq!(q.cursor(), 0);
        assert!(!q.word_left());

        assert!(q.word_right());
        assert_eq!(q.cursor(), 8);
        assert!(q.word_right());
        assert_eq!(q.cursor(), 13);
        assert!(!q.word_right());
    }

    #[test]
    fn deleting_a_word_takes_its_trailing_spaces_with_it() {
        let mut q = q("grinning face");
        assert!(q.delete_word_back());
        assert_eq!((q.text(), q.cursor()), ("grinning ", 9));
        assert!(q.delete_word_back());
        assert_eq!((q.text(), q.cursor()), ("", 0));
        assert!(!q.delete_word_back());
    }

    #[test]
    fn deleting_a_word_forward_leaves_the_cursor_put() {
        let mut q = q("grinning face");
        q.home();
        assert!(q.delete_word_forward());
        assert_eq!((q.text(), q.cursor()), (" face", 0));
        assert!(q.delete_word_forward());
        assert_eq!(q.text(), "");
    }

    #[test]
    fn home_and_end_report_whether_they_moved() {
        let mut q = q("cat");
        assert!(q.home());
        assert!(!q.home());
        assert!(q.end());
        assert!(!q.end());
        assert_eq!(q.cursor(), 3);
    }

    #[test]
    fn set_cursor_snaps_onto_a_character_boundary() {
        let mut q = q("a\u{1f600}b");
        q.set_cursor(2);
        assert_eq!(q.cursor(), 1);
        q.set_cursor(999);
        assert_eq!(q.cursor(), q.text().len());
    }
}
