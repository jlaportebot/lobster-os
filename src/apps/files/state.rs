//! Mutable state of the Files window, kept between frames.

use super::dnd::DragState;
use super::history::History;
use super::layout::Layout;
use super::model::{Entry, Sort, ViewMode};
use super::selection::Selection;
use super::typeahead::TypeAhead;
use crate::wm::controls::{ControlState, Rect};
use crate::wm::desktop_layout::ClickTracker;
use alloc::string::String;
use alloc::vec::Vec;

/// Items copied or cut, waiting for a paste.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Clipboard {
    pub paths: Vec<String>,
    /// Cut (move on paste) rather than copy.
    pub cut: bool,
}

/// What an open dialog was asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pending {
    NewFolder,
    NewFile,
    Rename(String),
    /// Permanent deletion of these paths.
    Delete(Vec<String>),
    EmptyTrash,
}

/// Where a drag would drop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropTarget {
    Item(usize),
    Crumb(usize),
    Place(usize),
}

/// Rows of the Properties panel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PropsPanel {
    pub title: String,
    pub rows: Vec<(String, String)>,
}

/// Shape of a dialog waiting for its turn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ask {
    /// Text prompt with a default value.
    Prompt(String),
    Confirm,
    Alert,
}

/// A dialog waiting until the mouse button is released and no other
/// dialog is open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Queued {
    pub ask: Ask,
    pub title: String,
    pub message: String,
    pub pending: Option<Pending>,
}

/// Everything the window remembers.
pub struct State {
    pub history: History,
    pub entries: Vec<Entry>,
    pub selection: Selection,
    pub sort: Sort,
    pub view: ViewMode,
    pub scroll: i32,
    /// Height of all items, measured in the last frame.
    pub content_h: u32,
    /// Window layout of the last frame (for hit-testing presses).
    pub layout: Option<Layout>,
    /// Grid columns of the last frame (1 in the list view).
    pub cols: usize,
    pub clipboard: Option<Clipboard>,
    pub queue: Vec<Queued>,
    /// Dialog currently on screen whose answer we are waiting for.
    pub awaiting: Option<Option<Pending>>,
    pub clicks: ClickTracker,
    /// Item to scroll into view on the next frame.
    pub reveal: Option<usize>,
    /// The trash holds something (drives its icon).
    pub trash_full: bool,
    pub props: Option<PropsPanel>,
    pub drag: DragState,
    pub drop: Option<DropTarget>,
    /// Breadcrumb segments and places of the last frame (client coords)
    /// with the index they stand for, for drop hit-testing.
    pub crumb_rects: Vec<(Rect, usize)>,
    pub place_rects: Vec<(Rect, usize)>,
    /// Screen position of the client area's top-left corner.
    pub client_origin: (u32, u32),
    pub typeahead: TypeAhead,
}

impl State {
    /// State of a freshly opened window, positioned at `start`.
    #[must_use]
    pub fn new(start: &str) -> Self {
        Self {
            history: History::new(start),
            entries: Vec::new(),
            selection: Selection::new(),
            sort: Sort::DEFAULT,
            view: ViewMode::List,
            scroll: 0,
            content_h: 0,
            layout: None,
            cols: 1,
            clipboard: None,
            queue: Vec::new(),
            awaiting: None,
            clicks: ClickTracker::new(),
            reveal: None,
            trash_full: false,
            props: None,
            drag: DragState::Idle,
            drop: None,
            crumb_rects: Vec::new(),
            place_rects: Vec::new(),
            client_origin: (0, 0),
            typeahead: TypeAhead::new(),
        }
    }

    /// Folder being shown.
    #[must_use]
    pub fn cwd(&self) -> &str {
        self.history.current()
    }

    /// Entry at `i`.
    #[must_use]
    pub fn entry(&self, i: usize) -> Option<&Entry> {
        self.entries.get(i)
    }

    /// Full path of item `i`.
    #[must_use]
    pub fn path_of(&self, i: usize) -> Option<String> {
        let entry = self.entries.get(i)?;
        Some(super::paths::join(self.cwd(), &entry.name))
    }

    /// Full paths of the selected items, in list order.
    #[must_use]
    pub fn selected_paths(&self) -> Vec<String> {
        self.selection
            .indices()
            .into_iter()
            .filter_map(|i| self.path_of(i))
            .collect()
    }

    /// Whether a sibling called `name` is in the current listing.
    #[must_use]
    pub fn has_name(&self, name: &str) -> bool {
        self.entries.iter().any(|e| e.name == name)
    }

    /// Index of the entry called `name`.
    #[must_use]
    pub fn index_of(&self, name: &str) -> Option<usize> {
        self.entries.iter().position(|e| e.name == name)
    }
}

static mut STATE: Option<State> = None;

/// The window state, created on first use at `start`.
pub fn get_or_init(start: &str) -> &'static mut State {
    // SAFETY: only the UI thread (input handlers and the render pass) calls
    // this, never reentrantly across a held borrow.
    let slot = unsafe { &mut *core::ptr::addr_of_mut!(STATE) };
    slot.get_or_insert_with(|| State::new(start))
}

/// The window state if the app has been opened.
pub fn get() -> Option<&'static mut State> {
    // SAFETY: same single-thread discipline as `get_or_init`.
    unsafe { (*core::ptr::addr_of_mut!(STATE)).as_mut() }
}

static mut CONTROLS: ControlState = ControlState::new();

/// Focus and track state of the controls (separate from [`State`] so a
/// frame can borrow both at once).
pub fn controls() -> &'static mut ControlState {
    // SAFETY: same single-thread discipline as `get_or_init`.
    unsafe { &mut *core::ptr::addr_of_mut!(CONTROLS) }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    use crate::apps::files::model::Entry;

    fn entry(name: &str) -> Entry {
        Entry {
            name: name.into(),
            is_dir: false,
            size: 1,
            mtime: 0,
        }
    }

    #[test]
    fn selected_paths_are_joined_with_the_folder_in_list_order() {
        let mut st = State::new("/home/user");
        st.entries = alloc::vec![entry("a"), entry("b"), entry("c")];
        st.selection.click(2);
        st.selection.toggle(0);
        assert_eq!(st.selected_paths(), ["/home/user/a", "/home/user/c"]);
    }

    #[test]
    fn names_are_looked_up_by_exact_match() {
        let mut st = State::new("/");
        st.entries = alloc::vec![entry("a.txt")];
        assert!(st.has_name("a.txt") && !st.has_name("A.txt"));
        assert_eq!(st.index_of("a.txt"), Some(0));
        assert_eq!(st.path_of(0), Some("/a.txt".into()));
    }
}
