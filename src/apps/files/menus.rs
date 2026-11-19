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
/// Separator entries carry action id 0.
const SEPARATOR: (&str, u32) = ("---", 0);

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
    let mut items: alloc::vec::Vec<(&str, u32)> = alloc::vec![("Open", OPEN), SEPARATOR];
    items.extend([("Cut", CUT), ("Copy", COPY)]);
    if st.selection.count() == 1 {
        items.push(("Rename", RENAME));
    }
    items.extend([SEPARATOR, ("Delete", DELETE)]);
    context_menu::show(x, y, &items);
}

fn background_menu(st: &mut State, x: u32, y: u32) {
    st.selection.clear();
    let mut items: alloc::vec::Vec<(&str, u32)> =
        alloc::vec![("New folder", NEW_FOLDER), ("New file", NEW_FILE)];
    if st.clipboard.is_some() {
        items.push(("Paste", PASTE));
    }
    items.extend([SEPARATOR, ("Refresh", REFRESH)]);
    items.push(match st.view {
        ViewMode::List => ("View as icons", VIEW_ICONS),
        ViewMode::Icons => ("View as list", VIEW_LIST),
    });
    context_menu::show(x, y, &items);
}

/// Run a Files context-menu action; false when the id is not ours.
pub fn dispatch(id: u32) -> bool {
    if !(OPEN..=VIEW_ICONS).contains(&id) {
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
        DELETE => ops::ask_delete(st),
        NEW_FOLDER => ops::ask_new(st, true),
        NEW_FILE => ops::ask_new(st, false),
        PASTE => ops::paste(st),
        REFRESH => nav::refresh(st),
        VIEW_LIST => actions::set_view(st, ViewMode::List),
        _ => actions::set_view(st, ViewMode::Icons),
    }
    crate::wm::dirty::mark();
    true
}
