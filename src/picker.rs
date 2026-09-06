//! Picker state: what is typed, what is selected, where the list is scrolled.
//!
//! Toolkit-free on purpose - the Wayland backend turns events into calls on this, and the
//! renderer only reads it, so the interesting behaviour is testable without a compositor.

use crate::grid::{self, Grid};

/// Rows of the grid visible at once. The card is sized from this.
pub const VISIBLE_ROWS: f64 = 8.0;
/// Height of the scrolling area, in logical pixels.
pub const VIEWPORT_H: f64 = VISIBLE_ROWS * grid::CELL;

pub struct Picker {
    pub query: String,
    pub grid: Grid,
    /// Selected cell as (row, column).
    pub sel: (usize, usize),
    /// Cell under the pointer, if any.
    pub hover: Option<(usize, usize)>,
    pub scroll: f64,
    /// Fitzpatrick tone applied to emoji that take one; 0 is the default yellow.
    pub tone: u8,
    recents: Vec<String>,
}

impl Picker {
    pub fn new(recents: Vec<String>, tone: u8) -> Picker {
        let grid = Grid::browse(&recents, tone);
        let sel = grid.first_cell();
        Picker {
            query: String::new(),
            grid,
            sel,
            hover: None,
            scroll: 0.0,
            tone,
            recents,
        }
    }

    /// Rebuild the list after the query, tone or recents changed, and put the selection
    /// back at the top - the old position means nothing against new contents.
    pub fn rebuild(&mut self) {
        self.grid = if self.query.trim().is_empty() {
            Grid::browse(&self.recents, self.tone)
        } else {
            Grid::search(self.query.trim(), self.tone)
        };
        self.sel = self.grid.first_cell();
        self.scroll = 0.0;
    }

    /// Append typed text to the query. Takes a whole `&str` rather than a `char` so a
    /// compose sequence or a paste arrives as one edit.
    pub fn insert(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.query.push_str(text);
        self.rebuild();
    }

    /// Delete the last *grapheme*, not the last byte: an emoji typed into the search box
    /// must not come apart into half a surrogate pair's worth of scalar values.
    pub fn backspace(&mut self) {
        if self.query.pop().is_some() {
            self.rebuild();
        }
    }

    pub fn clear_query(&mut self) {
        if !self.query.is_empty() {
            self.query.clear();
            self.rebuild();
        }
    }

    /// Drop the last word, for Ctrl+W.
    pub fn delete_word(&mut self) {
        let trimmed = self.query.trim_end();
        let cut = trimmed.rfind(' ').map_or(0, |i| i + 1);
        if cut != self.query.len() {
            self.query.truncate(cut);
            self.rebuild();
        }
    }

    /// Move the selection and follow it with the viewport.
    pub fn step(&mut self, dr: i32, dc: i32) {
        self.sel = self.grid.step(self.sel, dr, dc);
        self.scroll = self.grid.scroll_to(self.scroll, VIEWPORT_H, self.sel.0);
    }

    /// A page is a viewport's worth of rows, less one for continuity.
    pub fn page(&mut self, down: bool) {
        let rows = (VISIBLE_ROWS as i32 - 1).max(1);
        self.step(if down { rows } else { -rows }, 0);
    }

    /// Scroll without moving the selection, for the wheel.
    pub fn scroll_by(&mut self, dy: f64) {
        let max = (self.grid.height() - VIEWPORT_H).max(0.0);
        self.scroll = (self.scroll + dy).clamp(0.0, max);
    }

    /// Jump to a tab's section. Does nothing for a tab whose section is empty.
    pub fn goto_section(&mut self, i: usize) {
        let Some(Some(row)) = self.grid.sections.get(i).copied() else {
            return;
        };
        let max = (self.grid.height() - VIEWPORT_H).max(0.0);
        self.scroll = self.grid.offset(row).clamp(0.0, max);
        // Selecting the first cell under the header keeps the keyboard in step with what
        // the tab just brought into view.
        self.sel = self.grid.step((row, 0), 1, 0);
    }

    /// Move to the next or previous tab, for Tab / Shift+Tab.
    pub fn cycle_section(&mut self, back: bool) {
        let filled: Vec<usize> = (0..self.grid.sections.len())
            .filter(|i| self.grid.sections[*i].is_some())
            .collect();
        if filled.is_empty() {
            return;
        }
        let here = self.grid.active_section(self.scroll);
        let at = here.and_then(|h| filled.iter().position(|i| *i == h));
        let next = match (at, back) {
            (Some(0), true) => filled.len() - 1,
            (Some(i), true) => i - 1,
            (Some(i), false) => (i + 1) % filled.len(),
            (None, _) => 0,
        };
        self.goto_section(filled[next]);
    }

    /// The emoji the footer should describe and Return should commit.
    pub fn selected(&self) -> Option<&'static str> {
        self.grid.cell(self.sel)
    }

    /// Which cell a point inside the viewport falls on. `y` is measured from the top of
    /// the viewport, `x` from its left edge.
    pub fn cell_at(&self, x: f64, y: f64) -> Option<(usize, usize)> {
        if x < 0.0 || y < 0.0 || y >= VIEWPORT_H {
            return None;
        }
        let row = self.grid.row_at(self.scroll + y);
        let col = (x / grid::CELL) as usize;
        // row_at can land on a header, and the last row of a section is usually short.
        if col < self.grid.cells(row).len() {
            Some((row, col))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_switches_to_results_and_clearing_switches_back() {
        let mut p = Picker::new(vec![], 0);
        let browse_rows = p.grid.rows.len();
        p.insert("cat");
        assert_eq!(p.query, "cat");
        assert!(p.grid.sections.iter().all(Option::is_none));
        assert!(p.grid.rows.len() < browse_rows);
        p.clear_query();
        assert_eq!(p.grid.rows.len(), browse_rows);
    }

    #[test]
    fn backspace_removes_one_character_at_a_time() {
        let mut p = Picker::new(vec![], 0);
        p.insert("ca");
        p.backspace();
        assert_eq!(p.query, "c");
        p.backspace();
        assert_eq!(p.query, "");
        // Backspacing an empty query is a no-op rather than a panic.
        p.backspace();
        assert_eq!(p.query, "");
    }

    #[test]
    fn backspace_does_not_split_a_multibyte_character() {
        let mut p = Picker::new(vec![], 0);
        p.insert("\u{1f600}");
        p.backspace();
        assert_eq!(p.query, "");
    }

    #[test]
    fn delete_word_drops_the_trailing_word() {
        let mut p = Picker::new(vec![], 0);
        p.insert("grinning face");
        p.delete_word();
        assert_eq!(p.query, "grinning ");
        p.delete_word();
        assert_eq!(p.query, "");
    }

    #[test]
    fn a_rebuild_puts_the_selection_on_a_real_cell() {
        let mut p = Picker::new(vec![], 0);
        p.step(5, 0);
        p.insert("cat");
        assert!(p.selected().is_some());
        assert_eq!(p.scroll, 0.0);
    }

    #[test]
    fn moving_down_pulls_the_viewport_along() {
        let mut p = Picker::new(vec![], 0);
        assert_eq!(p.scroll, 0.0);
        for _ in 0..20 {
            p.step(1, 0);
        }
        assert!(p.scroll > 0.0, "viewport should have followed the selection");
        // And the selected row is inside it.
        let top = p.grid.offset(p.sel.0);
        assert!(top >= p.scroll - 0.01 && top < p.scroll + VIEWPORT_H);
    }

    #[test]
    fn scrolling_stops_at_both_ends() {
        let mut p = Picker::new(vec![], 0);
        p.scroll_by(-500.0);
        assert_eq!(p.scroll, 0.0);
        p.scroll_by(1e9);
        assert_eq!(p.scroll, p.grid.height() - VIEWPORT_H);
    }

    #[test]
    fn tabs_jump_to_their_section() {
        let mut p = Picker::new(vec![], 0);
        // Section 0 is recents, empty here, so it should not move anything.
        p.goto_section(0);
        assert_eq!(p.scroll, 0.0);
        p.goto_section(3);
        let row = p.grid.sections[3].expect("group 3 has emoji");
        assert_eq!(p.scroll, p.grid.offset(row));
        assert!(p.selected().is_some());
    }

    #[test]
    fn cycling_tabs_wraps_around() {
        let mut p = Picker::new(vec![], 0);
        let first = p.grid.active_section(p.scroll);
        p.cycle_section(false);
        assert_ne!(p.grid.active_section(p.scroll), first);
        // Walking back from the first filled tab lands on the last one.
        p.cycle_section(true);
        assert_eq!(p.grid.active_section(p.scroll), first);
    }

    #[test]
    fn hit_testing_maps_points_onto_cells() {
        let p = Picker::new(vec![], 0);
        // The very first row of the browse grid is a header, so the top strip has no cell.
        assert_eq!(p.cell_at(-1.0, 10.0), None);
        assert_eq!(p.cell_at(10.0, VIEWPORT_H + 1.0), None);
        // Just below the first header is the first row of cells, column 0.
        let hit = p.cell_at(4.0, grid::HEADER_H + 4.0);
        assert_eq!(hit, Some(p.grid.first_cell()));
        // And one cell to the right.
        let hit = p.cell_at(grid::CELL + 4.0, grid::HEADER_H + 4.0);
        assert_eq!(hit, Some((p.grid.first_cell().0, 1)));
    }

    #[test]
    fn hit_testing_past_a_short_rows_last_cell_selects_nothing() {
        let p = Picker::new(vec![], 0);
        // Far to the right of a 12-column grid there is no cell at all.
        assert_eq!(p.cell_at(grid::CELL * 20.0, grid::HEADER_H + 4.0), None);
    }
}
