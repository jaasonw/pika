//! Picker state: what is typed, what is selected, where the list is scrolled.
//!
//! Toolkit-free on purpose - the Wayland backend turns events into calls on this, and the
//! renderer only reads it, so the interesting behaviour is testable without a compositor.

use crate::grid::{self, Grid};
use crate::query::Query;
use crate::store;

/// Rows of the grid visible at once. The card is sized from this.
pub const VISIBLE_ROWS: f64 = 8.0;
/// Height of the scrolling area, in logical pixels.
pub const VIEWPORT_H: f64 = VISIBLE_ROWS * grid::CELL;

/// Which face of the card is showing. Settings is a mode rather than a second surface:
/// only one layer surface can hold the keyboard grab, so the GTK build had to hide the
/// picker to show settings and re-present it afterwards. Drawing both into the same card
/// removes that dance entirely.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Mode {
    Browse,
    Settings,
}

/// A row of the settings mode, in display order.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Setting {
    Insert,
    AlwaysCopy,
    Tone,
    RecentLimit,
    ClearRecents,
    ResetPaste,
    Back,
}

pub const SETTINGS: [Setting; 7] = [
    Setting::Insert,
    Setting::AlwaysCopy,
    Setting::Tone,
    Setting::RecentLimit,
    Setting::ClearRecents,
    Setting::ResetPaste,
    Setting::Back,
];

/// What a settings row wants done. `Picker` cannot reach the store, so it names the change
/// and the backend applies it - which also keeps every mutation in one place.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Action {
    ToggleInsert,
    ToggleAlwaysCopy,
    SetTone(u8),
    SetRecentLimit(usize),
    ClearRecents,
    ResetPaste,
    Close,
}

/// How far one Left/Right press moves the recents cap. The bounds are the store's, so the
/// row cannot offer a value `set_recent_limit` would silently clamp away.
const LIMIT_STEP: usize = 12;

pub struct Picker {
    pub mode: Mode,
    /// Selected settings row, when `mode` is Settings.
    pub setting: usize,
    /// Settings row under the pointer, drawn apart from the keyboard selection so the two
    /// do not fight over the same highlight.
    pub hover_setting: Option<usize>,
    /// Set once the recents list has been cleared this session, so the row can say so.
    pub cleared_recents: bool,
    /// Set once the portal token has been dropped this session.
    pub reset_paste: bool,
    pub query: Query,
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
            mode: Mode::Browse,
            setting: 0,
            hover_setting: None,
            cleared_recents: false,
            reset_paste: false,
            query: Query::default(),
            grid,
            sel,
            hover: None,
            scroll: 0.0,
            tone,
            recents,
        }
    }

    pub fn open_settings(&mut self) {
        self.mode = Mode::Settings;
        self.setting = 0;
        self.hover = None;
        self.hover_setting = None;
    }

    /// Leave settings, rebuilding the grid so a tone or recents change is reflected.
    pub fn close_settings(&mut self, recents: Vec<String>, tone: u8) {
        self.mode = Mode::Browse;
        self.hover_setting = None;
        self.recents = recents;
        self.tone = tone;
        self.rebuild();
    }

    /// Move between settings rows, clamping rather than wrapping.
    pub fn step_setting(&mut self, d: i32) {
        let last = SETTINGS.len() - 1;
        self.setting = (self.setting as i32 + d).clamp(0, last as i32) as usize;
    }

    /// Left/Right on the selected row. Only the two rows holding a value respond.
    pub fn adjust_setting(&self, d: i32, tone: u8, limit: usize) -> Option<Action> {
        match SETTINGS[self.setting] {
            Setting::Tone => {
                let next = (tone as i32 + d).rem_euclid(6) as u8;
                Some(Action::SetTone(next))
            }
            Setting::RecentLimit => {
                let (lo, hi) = store::RECENT_LIMIT_RANGE;
                let next = (limit as i32 + LIMIT_STEP as i32 * d).clamp(lo as i32, hi as i32);
                Some(Action::SetRecentLimit(next as usize))
            }
            _ => None,
        }
    }

    /// Enter or Space on the selected row.
    pub fn activate_setting(&self) -> Option<Action> {
        match SETTINGS[self.setting] {
            Setting::Insert => Some(Action::ToggleInsert),
            Setting::AlwaysCopy => Some(Action::ToggleAlwaysCopy),
            // Both value rows are driven by Left/Right and, for the tones, by clicking
            // the swatch directly. Enter has nothing left to mean on either.
            Setting::Tone | Setting::RecentLimit => None,
            Setting::ClearRecents => Some(Action::ClearRecents),
            Setting::ResetPaste => Some(Action::ResetPaste),
            Setting::Back => Some(Action::Close),
        }
    }

    /// Rebuild the list after the query, tone or recents changed, and put the selection
    /// back at the top - the old position means nothing against new contents.
    pub fn rebuild(&mut self) {
        let query = self.query.text().trim();
        self.grid = if query.is_empty() {
            Grid::browse(&self.recents, self.tone)
        } else {
            Grid::search(query, self.tone)
        };
        self.sel = self.grid.first_cell();
        self.scroll = 0.0;
    }

    /// Run an edit on the search box, rebuilding the grid only if the text changed.
    /// Cursor motion returns false from the closure and costs nothing.
    pub fn edit(&mut self, f: impl FnOnce(&mut Query) -> bool) {
        if f(&mut self.query) {
            self.rebuild();
        }
    }

    pub fn insert(&mut self, text: &str) {
        self.edit(|q| q.insert(text));
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
        assert_eq!(p.query.text(), "cat");
        assert!(p.grid.sections.iter().all(Option::is_none));
        assert!(p.grid.rows.len() < browse_rows);
        p.edit(|q| q.clear());
        assert_eq!(p.grid.rows.len(), browse_rows);
    }

    #[test]
    fn backspace_removes_one_character_at_a_time() {
        let mut p = Picker::new(vec![], 0);
        p.insert("ca");
        p.edit(|q| q.backspace());
        assert_eq!(p.query.text(), "c");
        p.edit(|q| q.backspace());
        assert_eq!(p.query.text(), "");
        // Backspacing an empty query is a no-op rather than a panic.
        p.edit(|q| q.backspace());
        assert_eq!(p.query.text(), "");
    }

    #[test]
    fn backspace_does_not_split_a_multibyte_character() {
        let mut p = Picker::new(vec![], 0);
        p.insert("\u{1f600}");
        p.edit(|q| q.backspace());
        assert_eq!(p.query.text(), "");
    }

    #[test]
    fn delete_word_drops_the_trailing_word() {
        let mut p = Picker::new(vec![], 0);
        p.insert("grinning face");
        p.edit(|q| q.delete_word_back());
        assert_eq!(p.query.text(), "grinning ");
        p.edit(|q| q.delete_word_back());
        assert_eq!(p.query.text(), "");
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
    fn settings_rows_clamp_rather_than_wrap() {
        let mut p = Picker::new(vec![], 0);
        p.open_settings();
        assert_eq!(p.mode, Mode::Settings);
        p.step_setting(-1);
        assert_eq!(p.setting, 0);
        for _ in 0..20 {
            p.step_setting(1);
        }
        assert_eq!(p.setting, SETTINGS.len() - 1);
    }

    #[test]
    fn the_recents_cap_stays_inside_the_range_the_store_accepts() {
        let (lo, hi) = store::RECENT_LIMIT_RANGE;
        let mut p = Picker::new(vec![], 0);
        p.open_settings();
        p.setting = SETTINGS.iter().position(|s| *s == Setting::RecentLimit).unwrap();
        assert_eq!(p.adjust_setting(-1, 0, lo), Some(Action::SetRecentLimit(lo)));
        assert_eq!(p.adjust_setting(1, 0, hi), Some(Action::SetRecentLimit(hi)));
        let mid = lo + LIMIT_STEP * 2;
        assert_eq!(
            p.adjust_setting(1, 0, mid),
            Some(Action::SetRecentLimit(mid + LIMIT_STEP))
        );
    }

    #[test]
    fn the_tone_row_wraps_in_both_directions() {
        let mut p = Picker::new(vec![], 0);
        p.open_settings();
        p.setting = SETTINGS.iter().position(|s| *s == Setting::Tone).unwrap();
        // Enter does nothing; the row is driven by the arrows and by clicking a swatch.
        assert_eq!(p.activate_setting(), None);
        assert_eq!(p.adjust_setting(-1, 0, 12), Some(Action::SetTone(5)));
        assert_eq!(p.adjust_setting(1, 5, 12), Some(Action::SetTone(0)));
    }

    #[test]
    fn closing_settings_picks_up_the_new_tone_and_recents() {
        let mut p = Picker::new(vec![], 0);
        p.open_settings();
        p.close_settings(vec!["\u{1f680}".to_string()], 3);
        assert_eq!(p.mode, Mode::Browse);
        assert_eq!(p.tone, 3);
        // Recents is now a real section rather than an empty tab slot.
        assert!(p.grid.sections[0].is_some());
    }

    #[test]
    fn hit_testing_past_a_short_rows_last_cell_selects_nothing() {
        let p = Picker::new(vec![], 0);
        // Far to the right of a 12-column grid there is no cell at all.
        assert_eq!(p.cell_at(grid::CELL * 20.0, grid::HEADER_H + 4.0), None);
    }
}
