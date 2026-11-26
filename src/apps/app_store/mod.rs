//! App Store (O4.6): browse the signed package catalogue, install, update
//! and remove apps as visible jobs without freezing the desktop.
//!
//! * [`model`], [`job`] and [`layout`] are pure and host-tested.
//! * [`backend`] connects jobs to `crate::pkg`; [`render`] draws with the
//!   shared controls in `crate::wm::controls`; [`actions`] performs what
//!   the controls ask for.
//! * This module wires the window, input and the public entry points.
//!
//! The longest blocking step of a job is the package download inside
//! `crate::net::fetch_url`; every other stage is short. The stage name is
//! drawn one frame before its step runs.

pub mod actions;
pub mod backend;
pub mod job;
pub mod layout;
pub mod model;
mod render;
pub mod state;

use crate::wm::controls::{drag, scroll, Track, KEY_NONE};
use crate::wm::hit_region::{self, Hit};
use crate::wm::window;
use actions::Action;

/// Title of the App Store window.
pub const TITLE: &str = "App Store";

const SC_BACKSPACE: u8 = 0x0E;
const SC_TAB: u8 = 0x0F;
const SC_ENTER: u8 = 0x1C;
const SC_SPACE: u8 = 0x39;
const SC_UP: u8 = 0x48;
const SC_PAGE_UP: u8 = 0x49;
const SC_DOWN: u8 = 0x50;
const SC_PAGE_DOWN: u8 = 0x51;
/// Longest search text the field accepts.
const MAX_QUERY_LEN: usize = 60;
/// Pixels one arrow-key press scrolls the list.
const SCROLL_STEP: i32 = 34;

/// Initialise the App Store application.
pub fn init() {
    crate::println!("[app_store] App Store initialized");
}

/// Open (or focus) the App Store window; returns its id.
pub fn open_window() -> u32 {
    if let Some(id) = window::find_by_title(TITLE) {
        window::focus(id);
        return id;
    }
    let screen = crate::display::fb::get_info();
    let panel_h = crate::wm::panel::panel_height();
    let (x, y, w, h) = layout::initial_window(screen.width, screen.height, panel_h);
    window::create(x, y, w, h, TITLE).unwrap_or(0)
}

/// Render hook used by `wm::tick`.
pub fn render_window(fb: &crate::display::fb::FramebufferInfo) {
    let Some(win_id) = window::find_by_title(TITLE) else {
        return;
    };
    let Some(win) = window::get(win_id) else {
        return;
    };
    actions::tick();
    let focused = window::focused_id() == win_id;
    let target = crate::wm::scene::window_target(win_id, fb);
    let (cx, cy, cw, ch) = crate::wm::window_frame::draw_frame(
        &target, win_id, win.x, win.y, win.w, win.h, TITLE, focused,
    );
    hit_region::set_handler(win_id, on_hit);
    let st = state::get();
    let mut ui =
        crate::wm::controls::Ui::new(&target, win_id, (cx, cy), (cw, ch), state::controls());
    render::frame(&mut ui, (cw, ch), st);
    ui.finish();
    if st.busy() {
        st.stage_shown = true;
        crate::wm::dirty::mark();
    }
}

fn on_hit(hit: &Hit) -> bool {
    window::focus(hit.win_id);
    state::controls().focus = hit.region.tag;
    state::controls().focus_visible = false;
    let Some(action) = Action::from_key(hit.region.tag) else {
        return false;
    };
    if action == Action::ListScroll {
        begin_drag(hit);
    } else {
        actions::perform(action);
    }
    crate::wm::dirty::mark();
    true
}

fn begin_drag(hit: &Hit) {
    let key = Action::ListScroll.key();
    let Some(track) = state::controls().track(key) else {
        return;
    };
    let grab = grab_offset(&track, hit.screen_y as i32);
    on_scroll_drag(
        key,
        track.value_at(hit.screen_x as i32, hit.screen_y as i32, grab),
    );
    drag::begin(key, track, grab, on_scroll_drag);
}

fn on_scroll_drag(_key: u32, value: u32) {
    state::get().scroll = value as i32;
    crate::wm::dirty::mark();
}

/// Where inside the scrollbar thumb a press landed.
fn grab_offset(track: &Track, press_y: i32) -> i32 {
    let Track::Vertical {
        y,
        h,
        view_h,
        content_h,
    } = *track
    else {
        return 0;
    };
    let (offset, len) = scroll::thumb(h, view_h, content_h, state::get().scroll);
    let top = y + offset as i32;
    if press_y >= top && press_y < top + len as i32 {
        press_y - top
    } else {
        (len / 2) as i32
    }
}

/// Keyboard input while the App Store window is focused. True when consumed.
pub fn handle_key(scancode: u8, ch: Option<char>) -> bool {
    let consumed = key_in_search(scancode, ch) || key_navigation(scancode);
    if consumed {
        crate::wm::dirty::mark();
    }
    consumed
}

fn key_in_search(scancode: u8, ch: Option<char>) -> bool {
    let st = state::get();
    let search = Action::Search.key();
    let on_button = matches!(scancode, SC_ENTER | SC_SPACE) && state::controls().focus != search;
    if scancode == SC_BACKSPACE && state::controls().focus == search {
        st.query.pop();
        st.scroll = 0;
        return true;
    }
    let Some(c) = ch.filter(|c| (c.is_ascii_graphic() || *c == ' ') && !on_button) else {
        return false;
    };
    if st.query.len() < MAX_QUERY_LEN {
        st.query.push(c);
    }
    state::controls().focus = search;
    st.scroll = 0;
    true
}

fn key_navigation(scancode: u8) -> bool {
    let st = state::get();
    match scancode {
        SC_TAB => {
            state::controls().move_focus(crate::usb::hid::shift_held());
        }
        SC_UP => st.scroll -= SCROLL_STEP,
        SC_DOWN => st.scroll += SCROLL_STEP,
        SC_PAGE_UP => st.scroll -= SCROLL_STEP * 6,
        SC_PAGE_DOWN => st.scroll += SCROLL_STEP * 6,
        SC_ENTER | SC_SPACE => return activate_focus(),
        _ => return false,
    }
    true
}

fn activate_focus() -> bool {
    let focus = state::controls().focus;
    if focus == KEY_NONE {
        return false;
    }
    match Action::from_key(focus) {
        Some(Action::Search | Action::ListScroll) | None => false,
        Some(action) => {
            actions::perform(action);
            true
        }
    }
}

/// Serial diagnostics: one line describing the window state.
pub fn describe() -> alloc::string::String {
    let st = state::get();
    let job = st.job.as_ref().map_or_else(
        || alloc::string::String::from("none"),
        |j| alloc::format!("{:?}:{:?}", j.op, j.stage),
    );
    alloc::format!(
        "store: section={} query='{}' entries={} updates={} selected={:?} offline={} job={} notice='{}'",
        st.section.title(),
        st.query,
        st.entries.len(),
        model::update_count(&st.entries),
        st.selected,
        st.offline,
        job,
        st.notice
    )
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::actions::Action;

    #[test]
    fn control_keys_round_trip() {
        for action in [
            Action::Section(7),
            Action::Search,
            Action::Row(300),
            Action::RowButton(2),
            Action::Refresh,
            Action::DetailsPrimary,
            Action::DetailsRemove,
            Action::DetailsClose,
            Action::DismissJob,
            Action::ListScroll,
        ] {
            assert_eq!(Action::from_key(action.key()), Some(action));
        }
        assert_eq!(Action::from_key(0), None);
    }
}
