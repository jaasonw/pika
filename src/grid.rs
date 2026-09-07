//! The picker's contents and geometry, with no toolkit in sight.
//!
//! The layout *is* the arithmetic below: the whole view is built eagerly - ~160 rows of
//! `&'static str`, microseconds - and every position is derived rather than measured.

use crate::emoji;

/// Cells across the grid, and the size of one cell in logical pixels.
pub const COLUMNS: usize = 12;
pub const CELL: f64 = 44.0;
/// A section header's row height, taller than a cell to give the title some air.
pub const HEADER_H: f64 = 30.0;

/// One line of the scrolling list.
#[derive(Clone, Debug, PartialEq)]
pub enum Row {
    Header(String),
    Cells(Vec<&'static str>),
}

impl Row {
    pub fn height(&self) -> f64 {
        match self {
            Row::Header(_) => HEADER_H,
            Row::Cells(_) => CELL,
        }
    }
}

/// The built list, plus where each tab points into it.
pub struct Grid {
    pub rows: Vec<Row>,
    /// Row index of each tab's header: recents first, then one per emoji group. `None`
    /// where a section came out empty (no recents yet) or while searching.
    pub sections: Vec<Option<usize>>,
    /// Running offset of every row, with a final entry for the total height, so a scroll
    /// position maps to a row by binary search rather than a walk.
    offsets: Vec<f64>,
}

impl Grid {
    /// Build the browse view: recents, then every group in tab order.
    pub fn browse(recents: &[String], tone: u8) -> Grid {
        let mut b = Builder::default();
        let cells: Vec<&'static str> = recents
            .iter()
            .filter_map(|ch| emoji::find(ch).map(|e| e.ch))
            .collect();
        b.section("recents", &cells);
        for group in emoji::GROUPS {
            let cells: Vec<&'static str> = emoji::by_group(group).map(|e| e.toned(tone)).collect();
            b.section(group, &cells);
        }
        b.finish()
    }

    /// Build the results view. A search has no meaningful sections, but the tab bar still
    /// wants one slot per tab so that none of them light up.
    pub fn search(query: &str, tone: u8) -> Grid {
        let hits: Vec<&'static str> = emoji::search(query)
            .iter()
            .map(|e| e.toned(tone))
            .collect();
        let mut b = Builder::default();
        b.section("results", &hits);
        let mut g = b.finish();
        g.sections = vec![None; emoji::GROUPS.len() + 1];
        g
    }

    /// Total height of every row, in logical pixels.
    pub fn height(&self) -> f64 {
        self.offsets.last().copied().unwrap_or(0.0)
    }

    /// Where row `r` starts.
    pub fn offset(&self, r: usize) -> f64 {
        self.offsets.get(r).copied().unwrap_or(0.0)
    }

    /// The first row at or below `y`. Used to start drawing at the top of the viewport
    /// instead of walking the list from row zero every frame.
    pub fn row_at(&self, y: f64) -> usize {
        match self
            .offsets
            .binary_search_by(|o| o.partial_cmp(&y).expect("offsets are finite"))
        {
            Ok(i) => i,
            // `binary_search` gives the insertion point; the row containing `y` is the one
            // before it. Saturating covers a negative scroll position.
            Err(i) => i.saturating_sub(1),
        }
    }

    pub fn cells(&self, r: usize) -> &[&'static str] {
        match self.rows.get(r) {
            Some(Row::Cells(c)) => c,
            _ => &[],
        }
    }

    /// The emoji at `(row, col)`, if that cell holds one.
    pub fn cell(&self, (r, c): (usize, usize)) -> Option<&'static str> {
        self.cells(r).get(c).copied()
    }

    /// Move the selection by `(dr, dc)`, skipping headers and clamping at the ends.
    ///
    /// Column is preserved across rows where it can be: stepping down from column 7 onto a
    /// short final row lands on that row's last cell rather than snapping to zero.
    pub fn step(&self, (r, c): (usize, usize), dr: i32, dc: i32) -> (usize, usize) {
        if dc != 0 {
            let width = self.cells(r).len();
            let next = c as i32 + dc;
            if next >= 0 && (next as usize) < width {
                return (r, next as usize);
            }
            // Off the end of a row: carry into the neighbouring one, at the near edge.
            let into = if next < 0 { -1 } else { 1 };
            return match self.next_cell_row(r, into) {
                Some(nr) if into < 0 => (nr, self.cells(nr).len().saturating_sub(1)),
                Some(nr) => (nr, 0),
                None => (r, c),
            };
        }
        if dr != 0 {
            let mut row = r;
            for _ in 0..dr.unsigned_abs() {
                match self.next_cell_row(row, dr.signum()) {
                    Some(nr) => row = nr,
                    // Already at the last row of cells; hold rather than wrap.
                    None => break,
                }
            }
            let width = self.cells(row).len();
            return (row, c.min(width.saturating_sub(1)));
        }
        (r, c)
    }

    /// The next row of cells in `dir`, skipping over headers.
    fn next_cell_row(&self, from: usize, dir: i32) -> Option<usize> {
        let mut r = from as i64;
        loop {
            r += dir as i64;
            if r < 0 || r as usize >= self.rows.len() {
                return None;
            }
            if matches!(self.rows[r as usize], Row::Cells(_)) {
                return Some(r as usize);
            }
        }
    }

    /// The first cell in the list, for resetting the selection after a rebuild.
    pub fn first_cell(&self) -> (usize, usize) {
        let r = self
            .rows
            .iter()
            .position(|row| matches!(row, Row::Cells(_)))
            .unwrap_or(0);
        (r, 0)
    }

    /// A scroll offset that brings row `r` into a viewport `height` tall, moving as little
    /// as possible: rows already visible leave `scroll` alone.
    pub fn scroll_to(&self, scroll: f64, height: f64, r: usize) -> f64 {
        let top = self.offset(r);
        let bottom = top + self.rows.get(r).map_or(0.0, Row::height);
        if top < scroll {
            top
        } else if bottom > scroll + height {
            bottom - height
        } else {
            scroll
        }
        .clamp(0.0, (self.height() - height).max(0.0))
    }

    /// Which tab covers scroll position `y`: the last section starting at or above it.
    pub fn active_section(&self, y: f64) -> Option<usize> {
        let mut active = None;
        for (i, start) in self.sections.iter().enumerate() {
            let Some(start) = start else { continue };
            if self.offset(*start) <= y + 1.0 {
                active = Some(i);
            }
        }
        active
    }
}

/// Accumulates rows and section starts while a grid is built.
#[derive(Default)]
struct Builder {
    rows: Vec<Row>,
    sections: Vec<Option<usize>>,
}

impl Builder {
    fn section(&mut self, title: &str, cells: &[&'static str]) {
        if cells.is_empty() {
            // The tab still needs its slot, so it can be drawn inactive.
            self.sections.push(None);
            return;
        }
        self.sections.push(Some(self.rows.len()));
        self.rows.push(Row::Header(title.to_uppercase()));
        for chunk in cells.chunks(COLUMNS) {
            self.rows.push(Row::Cells(chunk.to_vec()));
        }
    }

    fn finish(self) -> Grid {
        let mut offsets = Vec::with_capacity(self.rows.len() + 1);
        let mut y = 0.0;
        for row in &self.rows {
            offsets.push(y);
            y += row.height();
        }
        offsets.push(y);
        Grid {
            rows: self.rows,
            sections: self.sections,
            offsets,
        }
    }
}

/// The tab bar's icon for a group.
pub fn group_icon(group: &str) -> &'static str {
    match group {
        "Smileys & Emotion" => "\u{1f600}",
        "People & Body" => "\u{1f44b}",
        "Animals & Nature" => "\u{1f43b}",
        "Food & Drink" => "\u{1f34e}",
        "Travel & Places" => "\u{2708}\u{fe0f}",
        "Activities" => "\u{26bd}",
        "Objects" => "\u{1f4a1}",
        "Symbols" => "\u{1f523}",
        "Flags" => "\u{1f6a9}",
        _ => "\u{2753}",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A grid with two short sections, so row indices are easy to reason about:
    /// 0 header, 1 cells(2), 2 header, 3 cells(12), 4 cells(1).
    fn fixture() -> Grid {
        let mut b = Builder::default();
        b.section("one", &["a", "b"]);
        b.section(
            "two",
            &[
                "c", "d", "e", "f", "g", "h", "i", "j", "k", "l", "m", "n", "o",
            ],
        );
        b.finish()
    }

    #[test]
    fn sections_chunk_into_rows_of_twelve() {
        let g = fixture();
        assert_eq!(g.rows.len(), 5);
        assert_eq!(g.cells(1).len(), 2);
        assert_eq!(g.cells(3).len(), 12);
        assert_eq!(g.cells(4).len(), 1);
        assert_eq!(g.sections, vec![Some(0), Some(2)]);
    }

    #[test]
    fn an_empty_section_keeps_its_tab_slot_but_adds_no_rows() {
        let mut b = Builder::default();
        b.section("recents", &[]);
        b.section("one", &["a"]);
        let g = b.finish();
        assert_eq!(g.sections, vec![None, Some(0)]);
        assert_eq!(g.rows.len(), 2);
    }

    #[test]
    fn offsets_account_for_taller_headers() {
        let g = fixture();
        assert_eq!(g.offset(0), 0.0);
        assert_eq!(g.offset(1), HEADER_H);
        assert_eq!(g.offset(2), HEADER_H + CELL);
        assert_eq!(g.height(), HEADER_H * 2.0 + CELL * 3.0);
    }

    #[test]
    fn row_at_finds_the_row_containing_a_scroll_position() {
        let g = fixture();
        assert_eq!(g.row_at(0.0), 0);
        assert_eq!(g.row_at(HEADER_H - 1.0), 0);
        assert_eq!(g.row_at(HEADER_H), 1);
        assert_eq!(g.row_at(HEADER_H + 1.0), 1);
        // Past the end clamps to the last row rather than panicking.
        assert!(g.row_at(g.height() + 500.0) <= g.rows.len());
    }

    #[test]
    fn stepping_skips_headers() {
        let g = fixture();
        // Row 1 is the only row of section one; down lands on row 3, past the header.
        assert_eq!(g.step((1, 0), 1, 0), (3, 0));
        assert_eq!(g.step((3, 0), -1, 0), (1, 0));
    }

    #[test]
    fn stepping_down_onto_a_short_row_clamps_the_column() {
        let g = fixture();
        // Row 3 is full; row 4 holds one cell.
        assert_eq!(g.step((3, 7), 1, 0), (4, 0));
    }

    #[test]
    fn stepping_off_a_row_edge_carries_into_the_neighbour() {
        let g = fixture();
        // Right off the end of row 1 (2 cells) lands at the start of row 3.
        assert_eq!(g.step((1, 1), 0, 1), (3, 0));
        // Left off the start of row 3 lands on the last cell of row 1.
        assert_eq!(g.step((3, 0), 0, -1), (1, 1));
    }

    #[test]
    fn stepping_holds_at_the_ends_rather_than_wrapping() {
        let g = fixture();
        assert_eq!(g.step((1, 0), -1, 0), (1, 0));
        assert_eq!(g.step((4, 0), 1, 0), (4, 0));
        assert_eq!(g.step((1, 0), 0, -1), (1, 0));
        assert_eq!(g.step((4, 0), 0, 1), (4, 0));
    }

    #[test]
    fn scroll_to_moves_only_when_the_row_is_off_screen() {
        let g = fixture();
        let h = CELL * 2.0;
        // Row 1 sits at HEADER_H; a viewport already showing it does not move.
        assert_eq!(g.scroll_to(HEADER_H, h, 1), HEADER_H);
        // A row above the viewport scrolls up to meet it.
        assert_eq!(g.scroll_to(g.height(), h, 0), 0.0);
        // A row below scrolls just far enough to show its bottom.
        let want = g.offset(4) + CELL - h;
        assert_eq!(g.scroll_to(0.0, h, 4), want);
    }

    #[test]
    fn scroll_to_never_leaves_a_gap_past_the_end() {
        let g = fixture();
        let h = g.height() + 100.0;
        // A viewport taller than the content pins to the top.
        assert_eq!(g.scroll_to(0.0, h, 4), 0.0);
    }

    #[test]
    fn active_section_is_the_last_one_started() {
        let g = fixture();
        assert_eq!(g.active_section(0.0), Some(0));
        assert_eq!(g.active_section(g.offset(2)), Some(1));
        assert_eq!(g.active_section(g.height()), Some(1));
    }

    #[test]
    fn first_cell_skips_the_leading_header() {
        assert_eq!(fixture().first_cell(), (1, 0));
    }

    #[test]
    fn the_real_browse_grid_is_built_and_navigable() {
        let g = Grid::browse(&[], 0);
        assert!(g.rows.len() > 100, "expected the full table, got {}", g.rows.len());
        // One tab slot per group, plus recents, which is empty here.
        assert_eq!(g.sections.len(), emoji::GROUPS.len() + 1);
        assert_eq!(g.sections[0], None);
        let start = g.first_cell();
        assert!(g.cell(start).is_some());
        // Walking down from the top must never land on a header.
        let mut at = start;
        for _ in 0..50 {
            at = g.step(at, 1, 0);
            assert!(g.cell(at).is_some(), "landed on a non-cell at {at:?}");
        }
    }

    #[test]
    fn searching_clears_the_tab_bar() {
        let g = Grid::search("cat", 0);
        assert_eq!(g.sections.len(), emoji::GROUPS.len() + 1);
        assert!(g.sections.iter().all(Option::is_none));
        assert!(g.active_section(0.0).is_none());
    }
}
