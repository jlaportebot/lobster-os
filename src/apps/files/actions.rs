//! Presses on the window: toolbar, places, breadcrumbs, headers and items.

use super::layout;
use super::model::ViewMode;
use super::nav;
use super::state::State;
use super::targets::Target;
use crate::usb::hid;
use crate::wm::controls::{drag, scroll, Track};
use crate::wm::hit_region::Hit;

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
        Target::NewFolder => super::ops::ask_new(st, true),
        Target::ViewList => set_view(st, ViewMode::List),
        Target::ViewIcons => set_view(st, ViewMode::Icons),
        Target::Place(i) => open_place(st, i),
        Target::Crumb(i) => open_crumb(st, i),
        Target::Header(key) => {
            st.sort = st.sort.after_click(key);
            nav::resort(st);
        }
        Target::Body => click_item(st, item_at(st, hit.local_x, hit.local_y), Mods::held()),
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
    if let Some(place) = nav::places().get(i) {
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
