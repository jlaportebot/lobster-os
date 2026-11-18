//! Pure desktop-icon layout logic: grid snapping, collision, double-click
//! timing, layout persistence and file-type routing.
//!
//! Kept free of drawing and global state so it is host-testable; the icon
//! module (`desktop_icons`) owns the state and feeds it into these helpers.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// Horizontal pitch of the icon grid.
pub const CELL_W: u32 = 96;
/// Vertical pitch of the icon grid (tile + label + gap).
pub const CELL_H: u32 = 98;
/// Left edge of column 0.
pub const ORIGIN_X: u32 = 32;
/// Top edge of row 0.
pub const ORIGIN_Y: u32 = 40;
/// Two presses on the same icon within this window open it.
pub const DOUBLE_CLICK_MS: u64 = 400;
/// Pointer travel (px) after which a held press becomes a drag.
pub const DRAG_THRESHOLD: u32 = 6;

/// A grid cell, counted from the top-left.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    /// Column index.
    pub col: u32,
    /// Row index.
    pub row: u32,
}

impl Cell {
    /// Screen position of the cell's top-left corner.
    pub fn origin(self) -> (u32, u32) {
        (ORIGIN_X + self.col * CELL_W, ORIGIN_Y + self.row * CELL_H)
    }
}

/// Number of whole rows that fit above the bottom `reserved` pixels.
pub fn rows_for(screen_h: u32, reserved: u32) -> u32 {
    (screen_h.saturating_sub(ORIGIN_Y + reserved) / CELL_H).max(1)
}

/// Number of whole columns that fit on the screen.
pub fn cols_for(screen_w: u32) -> u32 {
    (screen_w.saturating_sub(ORIGIN_X) / CELL_W).max(1)
}

/// Nearest grid cell to a point (icon top-left), clamped to the grid.
pub fn snap(x: u32, y: u32, cols: u32, rows: u32) -> Cell {
    let col = (x.saturating_sub(ORIGIN_X) + CELL_W / 2) / CELL_W;
    let row = (y.saturating_sub(ORIGIN_Y) + CELL_H / 2) / CELL_H;
    Cell {
        col: col.min(cols.saturating_sub(1)),
        row: row.min(rows.saturating_sub(1)),
    }
}

/// First free cell scanning down each column, then across.
pub fn first_free(occupied: &[Cell], rows: u32) -> Cell {
    let rows = rows.max(1);
    (0u32..)
        .map(|n| Cell {
            col: n / rows,
            row: n % rows,
        })
        .find(|c| !occupied.contains(c))
        .unwrap_or(Cell { col: 0, row: 0 })
}

/// Accept a drop on `target` only when no other icon sits there.
pub fn resolve_drop(target: Cell, occupied: &[Cell]) -> Option<Cell> {
    if occupied.contains(&target) {
        None
    } else {
        Some(target)
    }
}

/// Double-click detector: remembers the last pressed icon and when.
#[derive(Clone, Copy, Debug, Default)]
pub struct ClickTracker {
    last: Option<(usize, u64)>,
}

impl ClickTracker {
    /// A tracker that has seen no clicks.
    pub const fn new() -> Self {
        Self { last: None }
    }

    /// Record a press on icon `idx` at `now_ms`; true when it completes a
    /// double-click (a completed double-click resets the tracker).
    pub fn press(&mut self, idx: usize, now_ms: u64) -> bool {
        let is_double = match self.last {
            Some((prev, at)) => prev == idx && now_ms.saturating_sub(at) <= DOUBLE_CLICK_MS,
            None => false,
        };
        self.last = if is_double { None } else { Some((idx, now_ms)) };
        is_double
    }

    /// Forget any pending first click (press on empty desktop, drag).
    pub fn reset(&mut self) {
        self.last = None;
    }
}

/// Serialize `(key, cell)` pairs, one `col row key` line each.
pub fn serialize(entries: &[(String, Cell)]) -> String {
    let mut out = String::new();
    for (key, cell) in entries {
        out.push_str(&alloc::format!("{} {} {}\n", cell.col, cell.row, key));
    }
    out
}

/// Parse text written by [`serialize`]; malformed lines are skipped.
pub fn parse(text: &str) -> Vec<(String, Cell)> {
    text.lines().filter_map(parse_line).collect()
}

fn parse_line(line: &str) -> Option<(String, Cell)> {
    let mut parts = line.splitn(3, ' ');
    let col = parts.next()?.parse().ok()?;
    let row = parts.next()?.parse().ok()?;
    let key = parts.next().filter(|k| !k.is_empty())?;
    Some((key.to_string(), Cell { col, row }))
}

/// Which application opens a desktop file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenWith {
    /// Text editor.
    Editor,
    /// Image viewer.
    Images,
    /// File manager, showing the folder.
    Files,
}

/// Extensions the Images app understands.
const IMAGE_EXTS: &[&str] = &["bmp", "png", "jpg", "jpeg", "gif"];

/// Pick the app for a file by extension; unknown types fall back to the
/// editor so any file can at least be inspected.
pub fn open_with(name: &str, is_dir: bool) -> OpenWith {
    if is_dir {
        return OpenWith::Files;
    }
    let ext = name.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    if IMAGE_EXTS.iter().any(|e| ext.eq_ignore_ascii_case(e)) {
        OpenWith::Images
    } else {
        OpenWith::Editor
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(col: u32, row: u32) -> Cell {
        Cell { col, row }
    }

    #[test]
    fn double_click_within_interval_opens() {
        let mut t = ClickTracker::new();
        assert!(!t.press(2, 1000));
        assert!(t.press(2, 1000 + DOUBLE_CLICK_MS));
    }

    #[test]
    fn double_click_too_slow_or_other_icon_does_not_open() {
        let mut t = ClickTracker::new();
        t.press(2, 1000);
        assert!(!t.press(2, 1000 + DOUBLE_CLICK_MS + 1));
        t.press(3, 5000);
        assert!(!t.press(4, 5100));
    }

    #[test]
    fn triple_click_is_not_two_doubles() {
        let mut t = ClickTracker::new();
        t.press(1, 0);
        assert!(t.press(1, 100));
        assert!(!t.press(1, 200));
    }

    #[test]
    fn reset_forgets_first_click() {
        let mut t = ClickTracker::new();
        t.press(1, 0);
        t.reset();
        assert!(!t.press(1, 50));
    }

    #[test]
    fn snap_rounds_to_nearest_cell_and_clamps() {
        assert_eq!(snap(ORIGIN_X, ORIGIN_Y, 10, 8), cell(0, 0));
        assert_eq!(snap(ORIGIN_X + CELL_W / 2 - 1, ORIGIN_Y, 10, 8), cell(0, 0));
        assert_eq!(
            snap(ORIGIN_X + CELL_W / 2, ORIGIN_Y + CELL_H, 10, 8),
            cell(1, 1)
        );
        assert_eq!(snap(0, 0, 10, 8), cell(0, 0));
        assert_eq!(snap(99_999, 99_999, 10, 8), cell(9, 7));
    }

    #[test]
    fn origin_roundtrips_through_snap() {
        let c = cell(3, 2);
        let (x, y) = c.origin();
        assert_eq!(snap(x, y, 10, 8), c);
    }

    #[test]
    fn first_free_fills_columns_then_wraps() {
        assert_eq!(first_free(&[], 3), cell(0, 0));
        assert_eq!(first_free(&[cell(0, 0), cell(0, 1)], 3), cell(0, 2));
        assert_eq!(
            first_free(&[cell(0, 0), cell(0, 1), cell(0, 2)], 3),
            cell(1, 0)
        );
        assert_eq!(first_free(&[cell(0, 1)], 3), cell(0, 0));
    }

    #[test]
    fn drop_on_occupied_cell_is_refused() {
        assert_eq!(resolve_drop(cell(1, 1), &[cell(1, 1)]), None);
        assert_eq!(resolve_drop(cell(1, 1), &[cell(0, 0)]), Some(cell(1, 1)));
    }

    #[test]
    fn grid_size_from_screen() {
        assert_eq!(rows_for(1080, 36), (1080 - ORIGIN_Y - 36) / CELL_H);
        assert_eq!(rows_for(10, 36), 1);
        assert_eq!(cols_for(1920), (1920 - ORIGIN_X) / CELL_W);
    }

    #[test]
    fn layout_roundtrip_keeps_keys_with_spaces() {
        let entries = alloc::vec![
            ("app:Terminal".to_string(), cell(0, 0)),
            ("file:my notes.txt".to_string(), cell(4, 7)),
        ];
        assert_eq!(parse(&serialize(&entries)), entries);
    }

    #[test]
    fn parse_skips_malformed_lines() {
        let parsed = parse("1 2 app:Files\nbogus\n3 x file:a\n4 5\n6 7 file:b\n");
        assert_eq!(
            parsed,
            alloc::vec![
                ("app:Files".to_string(), cell(1, 2)),
                ("file:b".to_string(), cell(6, 7)),
            ]
        );
    }

    #[test]
    fn extension_maps_to_app() {
        assert_eq!(open_with("notes.txt", false), OpenWith::Editor);
        assert_eq!(open_with("main.rs", false), OpenWith::Editor);
        assert_eq!(open_with("README", false), OpenWith::Editor);
        assert_eq!(open_with("photo.BMP", false), OpenWith::Images);
        assert_eq!(open_with("pic.png", false), OpenWith::Images);
        assert_eq!(open_with("Stuff", true), OpenWith::Files);
        assert_eq!(open_with("archive.png", true), OpenWith::Files);
    }
}
