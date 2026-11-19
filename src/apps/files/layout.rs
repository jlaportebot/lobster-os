//! Geometry of the Files window: toolbar, sidebar, item area, list columns
//! and icon grid, plus keyboard cursor movement. Pure maths, tested at every
//! window size the desktop offers.

use super::model::ViewMode;
use crate::wm::controls::Rect;

/// Preferred window size.
pub const PREFERRED_W: u32 = 960;
pub const PREFERRED_H: u32 = 640;
/// Height of the toolbar strip.
pub const TOOLBAR_H: u32 = 48;
/// Square toolbar button edge.
pub const NAV_BTN: u32 = 32;
/// Gap between toolbar controls and padding at the window edges.
pub const GAP: u32 = 8;
/// Gap between the three navigation buttons.
const NAV_GAP: u32 = 4;
/// Width of the places sidebar.
pub const SIDEBAR_W: u32 = 172;
/// Height of the status bar.
pub const STATUS_H: u32 = 28;
/// Height of the column header row.
pub const HEADER_H: u32 = 28;
/// Width of the scrollbar column.
pub const SCROLL_W: u32 = 12;
/// Width of the "New folder" button.
const NEW_FOLDER_W: u32 = 108;
/// Width of the list / icons switch.
const TOGGLE_W: u32 = 128;
/// Height of one place in the sidebar.
pub const PLACE_H: u32 = 36;
/// Gap below each place.
const PLACE_GAP: u32 = 2;

/// Height of one list row.
pub const ROW_H: u32 = 30;
/// Space below the last list row.
const LIST_PAD: u32 = 8;
/// Width of one icon-grid cell and its height.
pub const CELL_W: u32 = 112;
pub const CELL_H: u32 = 112;
/// Padding around the icon grid.
pub const GRID_PAD: u32 = 8;

/// Column widths of the list view.
const SIZE_W: u32 = 90;
const MODIFIED_W: u32 = 150;
const TYPE_W: u32 = 96;
/// Narrowest the Name column may get before other columns are dropped.
const MIN_NAME_W: u32 = 200;

/// Where everything sits, in client-area coordinates.
#[derive(Clone, Copy, Debug)]
pub struct Layout {
    pub toolbar: Rect,
    pub back: Rect,
    pub forward: Rect,
    pub up: Rect,
    pub crumbs: Rect,
    pub new_folder: Rect,
    pub view_toggle: Rect,
    pub sidebar: Rect,
    /// Column header (zero height in the icon view).
    pub header: Rect,
    /// Scrolling item area.
    pub body: Rect,
    pub scrollbar: Rect,
    pub status: Rect,
}

/// Lay out a client area of `w` x `h` px.
#[must_use]
pub fn compute(w: u32, h: u32, view: ViewMode) -> Layout {
    let toolbar = Rect::new(0, 0, w, TOOLBAR_H.min(h));
    let btn_y = ((TOOLBAR_H - NAV_BTN) / 2) as i32;
    let nav = |i: u32| Rect::new((GAP + i * (NAV_BTN + NAV_GAP)) as i32, btn_y, NAV_BTN, NAV_BTN);
    let toggle_x = w.saturating_sub(GAP + TOGGLE_W) as i32;
    let new_x = toggle_x - (GAP + NEW_FOLDER_W) as i32;
    let crumbs_x = (GAP + 3 * (NAV_BTN + NAV_GAP)) as i32;
    let crumbs_w = (new_x - GAP as i32 - crumbs_x).max(0) as u32;
    let sidebar_w = SIDEBAR_W.min(w / 3);
    let main_w = w - sidebar_w;
    let status_y = h.saturating_sub(STATUS_H);
    let header_h = if view == ViewMode::List { HEADER_H } else { 0 };
    let body_y = TOOLBAR_H + header_h;
    let body_h = status_y.saturating_sub(body_y);
    let bar_w = SCROLL_W.min(main_w);
    Layout {
        toolbar,
        back: nav(0),
        forward: nav(1),
        up: nav(2),
        crumbs: Rect::new(crumbs_x, btn_y, crumbs_w, NAV_BTN),
        new_folder: Rect::new(new_x, btn_y, NEW_FOLDER_W, NAV_BTN),
        view_toggle: Rect::new(toggle_x, btn_y, TOGGLE_W, NAV_BTN),
        sidebar: Rect::new(0, TOOLBAR_H as i32, sidebar_w, status_y.saturating_sub(TOOLBAR_H)),
        header: Rect::new(sidebar_w as i32, TOOLBAR_H as i32, main_w - bar_w, header_h),
        body: Rect::new(sidebar_w as i32, body_y as i32, main_w - bar_w, body_h),
        scrollbar: Rect::new((w - bar_w) as i32, body_y as i32, bar_w, body_h),
        status: Rect::new(0, status_y as i32, w, STATUS_H.min(h)),
    }
}

/// Rectangle of sidebar place `i`.
#[must_use]
pub fn place_rect(l: &Layout, i: usize) -> Rect {
    let y = l.sidebar.y + (GAP + i as u32 * (PLACE_H + PLACE_GAP)) as i32;
    Rect::new(GAP as i32, y, l.sidebar.w.saturating_sub(2 * GAP), PLACE_H)
}

/// One list column: its left edge and width inside the item area.
pub type Column = (i32, u32);

/// The list columns that fit a view `view_w` px wide.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Columns {
    pub name: Column,
    pub size: Option<Column>,
    pub modified: Option<Column>,
    pub kind: Option<Column>,
}

/// Divide `view_w` into columns, dropping Type, then Modified, then Size
/// when the Name column would get too narrow.
#[must_use]
pub fn columns(view_w: u32) -> Columns {
    let wanted = [Some(SIZE_W), Some(MODIFIED_W), Some(TYPE_W)];
    let mut keep = [true; 3];
    let used = |keep: &[bool; 3]| -> u32 {
        wanted
            .iter()
            .zip(keep)
            .filter(|(_, k)| **k)
            .map(|(w, _)| w.unwrap_or(0))
            .sum()
    };
    for drop in [2usize, 1, 0] {
        if view_w.saturating_sub(used(&keep)) >= MIN_NAME_W {
            break;
        }
        keep[drop] = false;
    }
    let mut right = view_w;
    let mut place = |w: u32, kept: bool| {
        kept.then(|| {
            right = right.saturating_sub(w);
            (right as i32, w)
        })
    };
    let kind = place(TYPE_W, keep[2]);
    let modified = place(MODIFIED_W, keep[1]);
    let size = place(SIZE_W, keep[0]);
    Columns {
        name: (0, right),
        size,
        modified,
        kind,
    }
}

/// Number of grid columns for a view `view_w` px wide (at least one).
#[must_use]
pub fn grid_cols(view_w: u32) -> usize {
    ((view_w.saturating_sub(GRID_PAD)) / CELL_W).max(1) as usize
}

/// Columns the current view uses for keyboard movement.
#[must_use]
pub fn view_cols(view: ViewMode, view_w: u32) -> usize {
    match view {
        ViewMode::List => 1,
        ViewMode::Icons => grid_cols(view_w),
    }
}

/// Rectangle of item `i` in content coordinates.
#[must_use]
pub fn item_rect(view: ViewMode, i: usize, cols: usize, view_w: u32) -> Rect {
    match view {
        ViewMode::List => Rect::new(0, (i as u32 * ROW_H) as i32, view_w, ROW_H),
        ViewMode::Icons => {
            let (col, row) = ((i % cols) as u32, (i / cols) as u32);
            let x = GRID_PAD + col * CELL_W;
            Rect::new(x as i32, (GRID_PAD + row * CELL_H) as i32, CELL_W, CELL_H)
        }
    }
}

/// Total height of `n` items.
#[must_use]
pub fn content_h(view: ViewMode, n: usize, cols: usize) -> u32 {
    match view {
        ViewMode::List => n as u32 * ROW_H + LIST_PAD,
        ViewMode::Icons => n.div_ceil(cols.max(1)) as u32 * CELL_H + 2 * GRID_PAD,
    }
}

/// Item under the content-coordinate point (`x`, `y`), if any.
#[must_use]
pub fn index_at(view: ViewMode, x: i32, y: i32, n: usize, cols: usize) -> Option<usize> {
    let hit = match view {
        ViewMode::List => (y >= 0).then(|| y as usize / ROW_H as usize)?,
        ViewMode::Icons => {
            let (gx, gy) = (x - GRID_PAD as i32, y - GRID_PAD as i32);
            if gx < 0 || gy < 0 || gx as usize / CELL_W as usize >= cols {
                return None;
            }
            gy as usize / CELL_H as usize * cols + gx as usize / CELL_W as usize
        }
    };
    (hit < n).then_some(hit)
}

/// Keyboard cursor movements.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Move {
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
}

/// Item the cursor moves to from `cur`; `page` is the number of rows a
/// page jump covers. `None` when there are no items.
#[must_use]
pub fn move_target(cur: Option<usize>, mv: Move, n: usize, cols: usize, page: usize) -> Option<usize> {
    let last = n.checked_sub(1)?;
    let Some(cur) = cur else {
        let backwards = matches!(mv, Move::Up | Move::Left | Move::End | Move::PageUp);
        return Some(if backwards { last } else { 0 });
    };
    let cols = cols.max(1);
    let target = match mv {
        Move::Up => cur.saturating_sub(cols),
        Move::Down => (cur + cols).min(last),
        Move::Left if cols > 1 => cur.saturating_sub(1),
        Move::Right if cols > 1 => (cur + 1).min(last),
        Move::Left | Move::Right => cur,
        Move::Home => 0,
        Move::End => last,
        Move::PageUp => cur.saturating_sub(page * cols),
        Move::PageDown => (cur + page * cols).min(last),
    };
    Some(target)
}

/// Scroll offset that brings the span (`top`, `h`) fully into a view
/// `view_h` px tall, changing `scroll` as little as possible.
#[must_use]
pub fn scroll_to_show(scroll: i32, top: i32, h: u32, view_h: u32) -> i32 {
    if top < scroll {
        return top;
    }
    let bottom = top + h as i32;
    if bottom > scroll + view_h as i32 {
        return bottom - view_h as i32;
    }
    scroll
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    fn inside(r: Rect, w: u32, h: u32) -> bool {
        r.x >= 0 && r.y >= 0 && r.right() <= w as i32 && r.bottom() <= h as i32
    }

    fn disjoint(a: Rect, b: Rect) -> bool {
        a.w == 0 || a.h == 0 || b.w == 0 || b.h == 0 || a.intersect(&b).is_none()
    }

    fn check(w: u32, h: u32, view: ViewMode) {
        let l = compute(w, h, view);
        let all = [
            l.toolbar,
            l.back,
            l.forward,
            l.up,
            l.crumbs,
            l.new_folder,
            l.view_toggle,
            l.sidebar,
            l.header,
            l.body,
            l.scrollbar,
            l.status,
        ];
        for r in all {
            assert!(inside(r, w, h), "{r:?} escapes {w}x{h}");
        }
        for pair in [
            [l.sidebar, l.header],
            [l.sidebar, l.body],
            [l.header, l.body],
            [l.body, l.scrollbar],
            [l.toolbar, l.body],
            [l.toolbar, l.sidebar],
            [l.body, l.status],
            [l.sidebar, l.status],
            [l.back, l.forward],
            [l.forward, l.up],
            [l.up, l.crumbs],
            [l.crumbs, l.new_folder],
            [l.new_folder, l.view_toggle],
        ] {
            assert!(disjoint(pair[0], pair[1]), "{:?} overlaps {:?}", pair[0], pair[1]);
        }
    }

    #[test]
    fn layout_fits_the_preferred_window_in_both_views() {
        check(PREFERRED_W - 4, PREFERRED_H - 28, ViewMode::List);
        check(PREFERRED_W - 4, PREFERRED_H - 28, ViewMode::Icons);
    }

    #[test]
    fn layout_fits_an_800x600_screen() {
        check(796, 548, ViewMode::List);
        check(796, 548, ViewMode::Icons);
    }

    #[test]
    fn layout_survives_a_tiny_window() {
        check(420, 300, ViewMode::List);
        check(300, 200, ViewMode::Icons);
    }

    #[test]
    fn breadcrumb_bar_gets_the_leftover_toolbar_width() {
        let l = compute(956, 612, ViewMode::List);
        assert!(l.crumbs.w > 300, "{:?}", l.crumbs);
    }

    #[test]
    fn icon_view_has_no_header() {
        assert_eq!(compute(956, 612, ViewMode::Icons).header.h, 0);
        assert_eq!(compute(956, 612, ViewMode::List).header.h, HEADER_H);
    }

    #[test]
    fn places_stack_without_overlap_below_the_toolbar() {
        let l = compute(956, 612, ViewMode::List);
        let a = place_rect(&l, 0);
        let b = place_rect(&l, 1);
        assert!(a.y >= TOOLBAR_H as i32);
        assert!(b.y >= a.bottom());
        assert!(inside(place_rect(&l, 7), 956, 612));
    }

    #[test]
    fn all_columns_fit_a_wide_list() {
        let c = columns(770);
        assert!(c.size.is_some() && c.modified.is_some() && c.kind.is_some());
        let (kx, kw) = c.kind.unwrap_or_default();
        assert_eq!(kx as u32 + kw, 770);
        assert!(c.name.1 >= MIN_NAME_W);
    }

    #[test]
    fn narrow_lists_drop_type_then_modified() {
        let c = columns(450);
        assert!(c.kind.is_none() && c.modified.is_some());
        let c = columns(336);
        assert!(c.kind.is_none() && c.modified.is_none() && c.size.is_some());
        let c = columns(230);
        assert!(c.size.is_none());
        assert_eq!(c.name, (0, 230));
    }

    #[test]
    fn columns_never_overlap() {
        for w in [230, 336, 450, 612, 770, 1200] {
            let c = columns(w);
            let mut edge = c.name.0 + c.name.1 as i32;
            for col in [c.size, c.modified, c.kind].into_iter().flatten() {
                assert!(col.0 >= edge, "{col:?} overlaps at width {w}");
                edge = col.0 + col.1 as i32;
            }
            assert!(edge <= w as i32);
        }
    }

    #[test]
    fn list_rows_hit_test_to_their_index() {
        assert_eq!(index_at(ViewMode::List, 10, 0, 5, 1), Some(0));
        assert_eq!(index_at(ViewMode::List, 10, ROW_H as i32 - 1, 5, 1), Some(0));
        assert_eq!(index_at(ViewMode::List, 10, ROW_H as i32, 5, 1), Some(1));
        assert_eq!(index_at(ViewMode::List, 10, 5 * ROW_H as i32, 5, 1), None);
        assert_eq!(index_at(ViewMode::List, 10, -3, 5, 1), None);
    }

    #[test]
    fn list_item_rects_agree_with_hit_testing() {
        for i in 0..5 {
            let r = item_rect(ViewMode::List, i, 1, 600);
            assert_eq!(index_at(ViewMode::List, r.x + 5, r.y + 5, 5, 1), Some(i));
            assert_eq!(index_at(ViewMode::List, r.right() - 1, r.bottom() - 1, 5, 1), Some(i));
        }
    }

    #[test]
    fn grid_cells_tile_without_overlap_and_hit_test_back() {
        let view_w = 600;
        let cols = grid_cols(view_w);
        assert_eq!(cols, 5);
        let n = 13;
        for i in 0..n {
            let r = item_rect(ViewMode::Icons, i, cols, view_w);
            assert!(r.right() <= view_w as i32, "cell {i} escapes");
            assert_eq!(index_at(ViewMode::Icons, r.x + 3, r.y + 3, n, cols), Some(i));
            for j in i + 1..n {
                let other = item_rect(ViewMode::Icons, j, cols, view_w);
                assert!(r.intersect(&other).is_none(), "{i} overlaps {j}");
            }
        }
    }

    #[test]
    fn grid_gaps_and_empty_cells_hit_nothing() {
        let cols = 4;
        assert_eq!(index_at(ViewMode::Icons, 2, 2, 8, cols), None);
        let past_last = item_rect(ViewMode::Icons, 7, cols, 600);
        assert_eq!(
            index_at(ViewMode::Icons, past_last.x + 1, past_last.bottom() + 1, 8, cols),
            None
        );
        let right_of_grid = (GRID_PAD + 4 * CELL_W + 1) as i32;
        assert_eq!(index_at(ViewMode::Icons, right_of_grid, 20, 8, cols), None);
    }

    #[test]
    fn content_height_covers_every_row() {
        assert_eq!(content_h(ViewMode::List, 10, 1), 10 * ROW_H + 8);
        assert_eq!(content_h(ViewMode::Icons, 13, 5), 3 * CELL_H + 2 * GRID_PAD);
        assert_eq!(content_h(ViewMode::Icons, 0, 5), 2 * GRID_PAD);
    }

    #[test]
    fn grid_never_has_zero_columns() {
        assert_eq!(grid_cols(0), 1);
        assert_eq!(grid_cols(CELL_W), 1);
    }

    #[test]
    fn list_cursor_moves_by_one_row_and_clamps() {
        assert_eq!(move_target(Some(2), Move::Down, 5, 1, 8), Some(3));
        assert_eq!(move_target(Some(4), Move::Down, 5, 1, 8), Some(4));
        assert_eq!(move_target(Some(0), Move::Up, 5, 1, 8), Some(0));
        assert_eq!(move_target(Some(2), Move::Left, 5, 1, 8), Some(2));
        assert_eq!(move_target(Some(2), Move::Home, 5, 1, 8), Some(0));
        assert_eq!(move_target(Some(2), Move::End, 5, 1, 8), Some(4));
        assert_eq!(move_target(Some(1), Move::PageDown, 5, 1, 8), Some(4));
    }

    #[test]
    fn grid_cursor_moves_by_rows_and_columns() {
        assert_eq!(move_target(Some(1), Move::Down, 10, 4, 3), Some(5));
        assert_eq!(move_target(Some(5), Move::Up, 10, 4, 3), Some(1));
        assert_eq!(move_target(Some(5), Move::Right, 10, 4, 3), Some(6));
        assert_eq!(move_target(Some(5), Move::Left, 10, 4, 3), Some(4));
        assert_eq!(move_target(Some(8), Move::Down, 10, 4, 3), Some(9));
    }

    #[test]
    fn cursor_starts_at_an_end_when_nothing_is_selected() {
        assert_eq!(move_target(None, Move::Down, 5, 1, 8), Some(0));
        assert_eq!(move_target(None, Move::Up, 5, 1, 8), Some(4));
        assert_eq!(move_target(None, Move::Down, 0, 1, 8), None);
    }

    #[test]
    fn scrolling_reveals_an_item_with_minimal_movement() {
        assert_eq!(scroll_to_show(0, 100, 30, 400), 0);
        assert_eq!(scroll_to_show(0, 390, 30, 400), 20);
        assert_eq!(scroll_to_show(200, 150, 30, 400), 150);
        assert_eq!(scroll_to_show(50, 100, 30, 400), 50);
    }
}
