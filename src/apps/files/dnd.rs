//! Drag-and-move inside the window: the press / drag / drop state machine,
//! the movement threshold and drop-target hit-testing.

use super::layout;
use super::model::{Entry, ViewMode};
use super::selection::Selection;
use crate::wm::controls::Rect;

/// Pointer travel (px) before a press becomes a drag.
pub const DRAG_THRESHOLD_PX: u32 = 6;

/// Whether the pointer moved far enough from `start` to count as a drag.
#[must_use]
pub fn exceeds_threshold(start: (u32, u32), now: (u32, u32)) -> bool {
    let dx = i64::from(now.0) - i64::from(start.0);
    let dy = i64::from(now.1) - i64::from(start.1);
    dx * dx + dy * dy > i64::from(DRAG_THRESHOLD_PX * DRAG_THRESHOLD_PX)
}

/// Where a drag stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum DragState {
    #[default]
    Idle,
    /// Button down on an item, not moved far enough yet.
    Pressed {
        index: usize,
        start: (u32, u32),
        /// Collapse a multi-selection to this item if it ends as a click.
        collapse: bool,
    },
    Dragging,
}

/// What a pointer update did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DragEvent {
    None,
    /// The press turned into a drag.
    Started,
    /// Released without dragging; `Some(i)` asks to collapse the selection.
    Click(Option<usize>),
    /// Released after dragging: drop on the current target.
    Drop,
}

impl DragState {
    /// A press on item `index` at screen position `start`.
    pub fn press(&mut self, index: usize, start: (u32, u32), collapse: bool) {
        *self = DragState::Pressed {
            index,
            start,
            collapse,
        };
    }

    /// Whether items are being dragged right now.
    #[must_use]
    pub fn is_dragging(&self) -> bool {
        *self == DragState::Dragging
    }

    /// Feed the pointer position and left-button state.
    pub fn update(&mut self, pos: (u32, u32), down: bool) -> DragEvent {
        match (*self, down) {
            (DragState::Idle, _) => DragEvent::None,
            (DragState::Pressed { start, .. }, true) if exceeds_threshold(start, pos) => {
                *self = DragState::Dragging;
                DragEvent::Started
            }
            (DragState::Pressed { .. }, true) | (DragState::Dragging, true) => DragEvent::None,
            (
                DragState::Pressed {
                    index, collapse, ..
                },
                false,
            ) => {
                *self = DragState::Idle;
                DragEvent::Click(collapse.then_some(index))
            }
            (DragState::Dragging, false) => {
                *self = DragState::Idle;
                DragEvent::Drop
            }
        }
    }
}

/// Folder item under the content-coordinate point that accepts a drop:
/// a folder that is not itself being dragged.
#[must_use]
pub fn item_drop_target(
    view: ViewMode,
    point: (i32, i32),
    entries: &[Entry],
    selection: &Selection,
    cols: usize,
) -> Option<usize> {
    let i = layout::index_at(view, point.0, point.1, entries.len(), cols)?;
    (entries[i].is_dir && !selection.contains(i)).then_some(i)
}

/// Index tag of the rectangle containing `point` (breadcrumb segments and
/// sidebar places).
#[must_use]
pub fn rect_target(point: (i32, i32), rects: &[(Rect, usize)]) -> Option<usize> {
    rects
        .iter()
        .find(|(r, _)| r.contains(point.0, point.1))
        .map(|(_, tag)| *tag)
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    fn entry(name: &str, is_dir: bool) -> Entry {
        Entry {
            name: name.into(),
            is_dir,
            size: 0,
            mtime: 0,
        }
    }

    #[test]
    fn small_movements_stay_clicks() {
        assert!(!exceeds_threshold((100, 100), (103, 104)));
        assert!(!exceeds_threshold((100, 100), (106, 100)));
        assert!(exceeds_threshold((100, 100), (107, 100)));
        assert!(exceeds_threshold((100, 100), (100, 90)));
    }

    #[test]
    fn a_press_released_in_place_is_a_click() {
        let mut d = DragState::Idle;
        d.press(2, (50, 50), false);
        assert_eq!(d.update((52, 51), true), DragEvent::None);
        assert_eq!(d.update((52, 51), false), DragEvent::Click(None));
        assert_eq!(d, DragState::Idle);
    }

    #[test]
    fn a_click_on_a_multi_selection_collapses_to_that_item() {
        let mut d = DragState::Idle;
        d.press(4, (50, 50), true);
        assert_eq!(d.update((50, 50), false), DragEvent::Click(Some(4)));
    }

    #[test]
    fn moving_past_the_threshold_starts_a_drag_that_ends_in_a_drop() {
        let mut d = DragState::Idle;
        d.press(1, (50, 50), false);
        assert_eq!(d.update((70, 50), true), DragEvent::Started);
        assert!(d.is_dragging());
        assert_eq!(d.update((90, 60), true), DragEvent::None);
        assert_eq!(d.update((90, 60), false), DragEvent::Drop);
        assert_eq!(d.update((90, 60), false), DragEvent::None);
    }

    #[test]
    fn dragging_back_to_the_start_still_drops() {
        let mut d = DragState::Idle;
        d.press(1, (50, 50), false);
        d.update((80, 80), true);
        d.update((50, 50), true);
        assert_eq!(d.update((50, 50), false), DragEvent::Drop);
    }

    #[test]
    fn only_unselected_folders_accept_drops() {
        let entries = [
            entry("docs", true),
            entry("a.txt", false),
            entry("pics", true),
        ];
        let mut sel = Selection::new();
        sel.click(2);
        let at = |i: usize| (10, (i as u32 * layout::ROW_H + 5) as i32);
        let hit = |i: usize| item_drop_target(ViewMode::List, at(i), &entries, &sel, 1);
        assert_eq!(hit(0), Some(0));
        assert_eq!(hit(1), None);
        assert_eq!(hit(2), None);
        assert_eq!(
            item_drop_target(ViewMode::List, (10, 900), &entries, &sel, 1),
            None
        );
    }

    #[test]
    fn grid_tiles_are_drop_targets_too() {
        let entries = [entry("a.txt", false), entry("docs", true)];
        let sel = Selection::new();
        let cell = layout::item_rect(ViewMode::Icons, 1, 4, 600);
        let p = (cell.x + 4, cell.y + 4);
        assert_eq!(
            item_drop_target(ViewMode::Icons, p, &entries, &sel, 4),
            Some(1)
        );
    }

    #[test]
    fn rectangles_resolve_to_their_tag() {
        let rects = [(Rect::new(0, 0, 50, 20), 7), (Rect::new(60, 0, 50, 20), 9)];
        assert_eq!(rect_target((10, 10), &rects), Some(7));
        assert_eq!(rect_target((70, 5), &rects), Some(9));
        assert_eq!(rect_target((55, 5), &rects), None);
    }
}
