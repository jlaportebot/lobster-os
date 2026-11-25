//! Right-click menus and the actions they dispatch.

use super::actions;
use super::model::ViewMode;
use super::state::State;
use super::{nav, ops};
use crate::wm::context_menu;
use crate::wm::hit_region::Hit;

/// First context-menu action id of the Files app (the desktop uses < 100).
const BASE: u32 = 0x4600;
const OPEN: u32 = BASE + 1;
const CUT: u32 = BASE + 2;
const COPY: u32 = BASE + 3;
const RENAME: u32 = BASE + 4;
const DELETE: u32 = BASE + 5;
const NEW_FOLDER: u32 = BASE + 6;
const NEW_FILE: u32 = BASE + 7;
const PASTE: u32 = BASE + 8;
const REFRESH: u32 = BASE + 9;
const VIEW_LIST: u32 = BASE + 10;
const VIEW_ICONS: u32 = BASE + 11;
const RESTORE: u32 = BASE + 12;
const DELETE_PERM: u32 = BASE + 13;
const PROPS: u32 = BASE + 14;
const EMPTY_TRASH: u32 = BASE + 15;
/// Separator entries carry action id 0.
const SEPARATOR: (&str, u32, bool) = ("---", 0, false);

/// Show the menu for a right press at screen (`x`, `y`) on `hit`.
pub fn show(st: &mut State, hit: &Hit, x: u32, y: u32) {
    match actions::item_at_hit(st, hit) {
        Some(i) => {
            if !st.selection.contains(i) {
                st.selection.click(i);
            }
            item_menu(st, x, y);
        }
        None => background_menu(st, x, y),
    }
    crate::wm::dirty::mark();
}

fn item_menu(st: &State, x: u32, y: u32) {
    let mut items: alloc::vec::Vec<(&str, u32, bool)> = alloc::vec::Vec::new();
    if super::nav::in_trash(st) {
        items.extend([("Restore", RESTORE, false), SEPARATOR]);
        items.extend([("Delete permanently", DELETE_PERM, false), SEPARATOR]);
    } else {
        items.extend([("Open", OPEN, false), SEPARATOR]);
        items.extend([("Cut", CUT, false), ("Copy", COPY, false)]);
        if st.selection.count() == 1 {
            items.push(("Rename", RENAME, false));
        }
        items.extend([SEPARATOR, ("Move to Trash", DELETE, false)]);
        items.extend([("Delete permanently", DELETE_PERM, false), SEPARATOR]);
    }
    items.push(("Properties", PROPS, false));
    context_menu::show_with_state(x, y, &items);
}

fn background_menu(st: &mut State, x: u32, y: u32) {
    st.selection.clear();
    let mut items: alloc::vec::Vec<(&str, u32, bool)> = alloc::vec::Vec::new();
    if super::nav::in_trash(st) {
        items.push(("Empty trash", EMPTY_TRASH, st.entries.is_empty()));
    } else {
        items.extend([
            ("New folder", NEW_FOLDER, false),
            ("New file", NEW_FILE, false),
        ]);
        items.push(("Paste", PASTE, st.clipboard.is_none()));
    }
    items.extend([SEPARATOR, ("Refresh", REFRESH, false)]);
    items.push(match st.view {
        ViewMode::List => ("View as icons", VIEW_ICONS, false),
        ViewMode::Icons => ("View as list", VIEW_LIST, false),
    });
    context_menu::show_with_state(x, y, &items);
}

/// Run a Files context-menu action; false when the id is not ours.
pub fn dispatch(id: u32) -> bool {
    if !(OPEN..=EMPTY_TRASH).contains(&id) {
        return false;
    }
    let Some(st) = super::state::get() else {
        return true;
    };
    match id {
        OPEN => actions::open_selection(st),
        CUT => ops::copy_selection(st, true),
        COPY => ops::copy_selection(st, false),
        RENAME => ops::ask_rename(st),
        DELETE => ops::delete_selection(st),
        DELETE_PERM => ops::ask_delete_permanent(st),
        RESTORE => ops::restore_selection(st),
        PROPS => ops::show_properties(st),
        EMPTY_TRASH => ops::ask_empty_trash(st),
        NEW_FOLDER => ops::ask_new(st, true),
        NEW_FILE => ops::ask_new(st, false),
        PASTE => ops::paste(st),
        REFRESH => nav::refresh(st),
        VIEW_LIST => actions::set_view(st, ViewMode::List),
        VIEW_ICONS => actions::set_view(st, ViewMode::Icons),
        _ => {}
    }
    crate::wm::dirty::mark();
    true
}
