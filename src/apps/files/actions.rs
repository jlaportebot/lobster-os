//! Presses on the window: toolbar, places, breadcrumbs, headers and items.

use super::dnd::{self, DragEvent, DragState};
use super::layout;
use super::model::ViewMode;
use super::nav;
use super::state::{DropTarget, State};
use super::targets::Target;
use crate::usb::hid;
use crate::wm::controls::{drag, scroll, Track};
use crate::wm::hit_region::Hit;
use alloc::string::String;

/// Open the item at `i`: folders are entered, files go to the app their
/// type maps to.
pub fn open_index(st: &mut State, i: usize) {
    let (Some(path), Some(entry)) = (st.path_of(i), st.entry(i)) else {
        return;
    };
    if entry.is_dir {
        nav::navigate(st, &path);
        return;
    }
    crate::wm::desktop_files::open_path(&path, false);
}

/// Open what Enter acts on: the keyboard cursor, else the only selection.
pub fn open_selection(st: &mut State) {
    let one = st
        .selection
        .lead()
        .or_else(|| st.selection.indices().first().copied());
    if let Some(i) = one {
        open_index(st, i);
    }
}

fn now_ms() -> u64 {
    crate::arch::ArchTimer::uptime_ms()
}

/// Item under a press at client-local (`lx`, `ly`), if any.
fn item_at(st: &State, lx: u32, ly: u32) -> Option<usize> {
    let l = st.layout?;
    let (x, y) = (lx as i32 - l.body.x, ly as i32 - l.body.y + st.scroll);
    layout::index_at(st.view, x, y, st.entries.len(), st.cols)
}

/// Item under a screen position (used by the right-click menu).
#[must_use]
pub fn item_at_hit(st: &State, hit: &Hit) -> Option<usize> {
    item_at(st, hit.local_x, hit.local_y)
}

/// Modifier keys held during a click.
#[derive(Clone, Copy, Debug, Default)]
pub struct Mods {
    pub ctrl: bool,
    pub shift: bool,
}

impl Mods {
    /// The modifiers held right now.
    #[must_use]
    pub fn held() -> Self {
        Self {
            ctrl: hid::ctrl_held(),
            shift: hid::shift_held(),
        }
    }
}

/// A click on item `hit` (`None` = empty space) with `mods` held.
pub fn click_item(st: &mut State, hit: Option<usize>, mods: Mods) {
    let Some(i) = hit else {
        if !mods.ctrl && !mods.shift {
            st.selection.clear();
        }
        st.clicks.reset();
        return;
    };
    st.reveal = Some(i);
    if mods.ctrl {
        st.selection.toggle(i);
    } else if mods.shift {
        st.selection.extend_to(i);
    } else {
        st.selection.click(i);
        if st.clicks.press(i, now_ms()) {
            open_index(st, i);
        }
    }
}

/// Dispatch a left press on a registered region. True when consumed.
pub fn press(st: &mut State, hit: &Hit) -> bool {
    let Some(target) = Target::from_key(hit.region.tag) else {
        return false;
    };
    match target {
        Target::Back => nav::go_back(st),
        Target::Forward => nav::go_forward(st),
        Target::Up => nav::go_up(st),
        Target::NewFolder if nav::in_trash(st) => super::ops::ask_empty_trash(st),
        Target::NewFolder => super::ops::ask_new(st, true),
        Target::PropsOk => st.props = None,
        Target::Backdrop => {}
        Target::ViewList => set_view(st, ViewMode::List),
        Target::ViewIcons => set_view(st, ViewMode::Icons),
        Target::Place(i) => open_place(st, i),
        Target::Crumb(i) => open_crumb(st, i),
        Target::Header(key) => {
            st.sort = st.sort.after_click(key);
            nav::resort(st);
        }
        Target::Body => body_press(st, hit),
        Target::Scrollbar => begin_scroll_drag(st, hit),
    }
    crate::wm::dirty::mark();
    true
}

/// Switch between the list and the icon grid, keeping the cursor visible.
pub fn set_view(st: &mut State, view: ViewMode) {
    if st.view == view {
        return;
    }
    st.view = view;
    st.reveal = st.selection.lead();
}

fn open_place(st: &mut State, i: usize) {
    if let Some(place) = nav::places(st.trash_full).get(i) {
        nav::open_place(st, place);
    }
}

fn open_crumb(st: &mut State, i: usize) {
    if let Some(crumb) = super::paths::crumbs(st.cwd()).get(i) {
        let path = crumb.path.clone();
        nav::navigate(st, &path);
    }
}

fn begin_scroll_drag(st: &mut State, hit: &Hit) {
    let Some(track) = super::state::controls().track(Target::Scrollbar.key()) else {
        return;
    };
    let grab = grab_offset(st, &track, hit.screen_y as i32);
    let value = track.value_at(hit.screen_x as i32, hit.screen_y as i32, grab);
    on_drag(Target::Scrollbar.key(), value);
    drag::begin(Target::Scrollbar.key(), track, grab, on_drag);
}

/// Where inside the thumb a press landed; presses elsewhere grab its middle.
fn grab_offset(st: &State, track: &Track, press_y: i32) -> i32 {
    let Track::Vertical {
        y,
        h,
        view_h,
        content_h,
    } = *track
    else {
        return 0;
    };
    let (offset, len) = scroll::thumb(h, view_h, content_h, st.scroll);
    let top = y + offset as i32;
    if press_y >= top && press_y < top + len as i32 {
        press_y - top
    } else {
        (len / 2) as i32
    }
}

/// Scrollbar drag: the pointer chose scroll offset `value`.
pub fn on_drag(_key: u32, value: u32) {
    if let Some(st) = super::state::get() {
        st.scroll = value as i32;
    }
}

fn body_press(st: &mut State, hit: &Hit) {
    let item = item_at(st, hit.local_x, hit.local_y);
    let mods = Mods::held();
    let plain = !mods.ctrl && !mods.shift;
    let start = (hit.screen_x, hit.screen_y);
    let keep_multi =
        item.is_some_and(|i| plain && st.selection.count() > 1 && st.selection.contains(i));
    if let (Some(i), true) = (item, keep_multi) {
        st.drag.press(i, start, true);
        if st.clicks.press(i, now_ms()) {
            open_index(st, i);
        }
        return;
    }
    click_item(st, item, mods);
    if let (Some(i), true, false) = (item, plain, nav::in_trash(st)) {
        st.drag.press(i, start, false);
    }
}

/// Where the pointer at screen (`x`, `y`) would drop dragged items.
fn drop_target_at(st: &State, x: u32, y: u32) -> Option<DropTarget> {
    let (ox, oy) = st.client_origin;
    let point = (x as i32 - ox as i32, y as i32 - oy as i32);
    if let Some(i) = dnd::rect_target(point, &st.crumb_rects) {
        return Some(DropTarget::Crumb(i));
    }
    if let Some(i) = dnd::rect_target(point, &st.place_rects) {
        return Some(DropTarget::Place(i));
    }
    let l = st.layout?;
    if !l.body.contains(point.0, point.1) {
        return None;
    }
    let content = (point.0 - l.body.x, point.1 - l.body.y + st.scroll);
    dnd::item_drop_target(st.view, content, &st.entries, &st.selection, st.cols)
        .map(DropTarget::Item)
}

fn drop_dir(st: &State, target: DropTarget) -> Option<String> {
    match target {
        DropTarget::Item(i) => st.path_of(i),
        DropTarget::Crumb(i) => super::paths::crumbs(st.cwd())
            .get(i)
            .map(|c| c.path.clone()),
        DropTarget::Place(i) => nav::places(st.trash_full).get(i).map(|p| p.path.clone()),
    }
}

fn drop_selection(st: &mut State, target: DropTarget) {
    let Some(dir) = drop_dir(st, target) else {
        return;
    };
    if dir == nav::trash_dirs().files {
        super::ops::delete_selection(st);
        return;
    }
    super::ops::move_selection(st, &dir);
}

/// Per-frame drag handling: movement threshold, drop-target tracking and
/// the drop itself when the button is released.
pub fn drag_tick(st: &mut State) {
    let pos = crate::wm::input::cursor_pos();
    let down = crate::wm::input::mouse_buttons().0;
    match st.drag.update(pos, down) {
        DragEvent::Click(Some(i)) => st.selection.click(i),
        DragEvent::Drop => {
            if let Some(target) = st.drop.take() {
                drop_selection(st, target);
            }
        }
        _ => {}
    }
    st.drop = st
        .drag
        .is_dragging()
        .then(|| drop_target_at(st, pos.0, pos.1))
        .flatten();
    if st.drag != DragState::Idle {
        crate::wm::dirty::mark();
    }
}
