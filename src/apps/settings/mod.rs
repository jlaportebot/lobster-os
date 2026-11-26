//! Settings application (O4.1): a sidebar of pages whose controls change
//! the live system and persist to `/sd/etc/settings.conf`.
//!
//! * [`model`], [`layout`], [`resolution`], [`users`], [`format`], [`store`]
//!   are pure and host-tested.
//! * [`render`] and [`pages`] draw with the shared controls in
//!   `crate::wm::controls`; [`actions`] performs what the controls ask for.
//! * This module wires the window, input and the public entry points.

pub mod accounts;
pub mod actions;
pub mod display;
pub mod format;
pub mod layout;
pub mod model;
mod pages;
pub mod passwords;
mod render;
pub mod resolution;
pub mod sound;
pub mod state;
pub mod store;
pub mod system;
pub mod users;

pub use model::Page;

use crate::wm::controls::{drag, scroll, Track, KEY_NONE};
use crate::wm::hit_region::{self, Hit};
use crate::wm::window;
use model::Action;

/// Title of the Settings window.
pub const TITLE: &str = "Settings";

const SC_ESC: u8 = 0x01;
const SC_BACKSPACE: u8 = 0x0E;
const SC_TAB: u8 = 0x0F;
const SC_ENTER: u8 = 0x1C;
const SC_SPACE: u8 = 0x39;
const SC_HOME: u8 = 0x47;
const SC_UP: u8 = 0x48;
const SC_PAGE_UP: u8 = 0x49;
const SC_LEFT: u8 = 0x4B;
const SC_RIGHT: u8 = 0x4D;
const SC_END: u8 = 0x4F;
const SC_DOWN: u8 = 0x50;
const SC_PAGE_DOWN: u8 = 0x51;
/// Volume change of one arrow key press on the slider.
const VOLUME_KEY_STEP: i32 = 5;
/// Longest URL the Network page accepts.
const MAX_URL_LEN: usize = 200;

/// Initialise the Settings application.
pub fn init() {
    crate::println!("[settings] Settings application initialized");
}

/// Open (or focus) the Settings window; returns its id.
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

/// Open the window on `page` (deep link from menus and the shell).
pub fn open_page(page: Page) -> u32 {
    actions::show(page);
    crate::wm::dirty::mark();
    open_window()
}

/// Shell: show the next page.
pub fn cycle_page() {
    let st = state::get();
    actions::show(st.page.cycle());
}

/// Render hook used by `wm::tick`.
pub fn render_settings_window(fb: &crate::display::fb::FramebufferInfo) {
    actions::background_tick();
    let Some(win_id) = window::find_by_title(TITLE) else {
        revert_if_orphaned();
        return;
    };
    let Some(win) = window::get(win_id) else {
        return;
    };
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
}

/// A closed window must not leave the display in an unconfirmed mode.
fn revert_if_orphaned() {
    let st = state::get();
    if let Some((w, h)) = st.confirm.revert() {
        let _ = display::apply(w, h);
    }
}

fn on_hit(hit: &Hit) -> bool {
    window::focus(hit.win_id);
    state::controls().focus = hit.region.tag;
    state::controls().focus_visible = false;
    let Some(action) = Action::from_key(hit.region.tag) else {
        return false;
    };
    match action {
        Action::Volume | Action::PageScroll => begin_drag(action, hit),
        other => actions::perform(other),
    }
    crate::wm::dirty::mark();
    true
}

fn begin_drag(action: Action, hit: &Hit) {
    let key = action.key();
    let Some(track) = state::controls().track(key) else {
        return;
    };
    let grab = grab_offset(&track, hit.screen_y as i32);
    let value = track.value_at(hit.screen_x as i32, hit.screen_y as i32, grab);
    actions::on_drag(key, value);
    drag::begin(key, track, grab, actions::on_drag);
}

/// Where inside the scrollbar thumb a press landed; clicks outside the
/// thumb grab its middle so it jumps under the pointer.
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
    let st = state::get();
    let (offset, len) = scroll::thumb(h, view_h, content_h, st.scroll);
    let top = y + offset as i32;
    if press_y >= top && press_y < top + len as i32 {
        press_y - top
    } else {
        (len / 2) as i32
    }
}

/// Keyboard input while the Settings window is focused. True when consumed.
pub fn handle_key(scancode: u8, ch: Option<char>) -> bool {
    let st = state::get();
    let consumed = key_in_url_field(st, scancode, ch) || key_navigation(st, scancode);
    if consumed {
        crate::wm::dirty::mark();
    }
    consumed
}

fn key_in_url_field(st: &mut state::State, scancode: u8, ch: Option<char>) -> bool {
    if state::controls().focus != Action::UrlField.key() {
        return false;
    }
    match (scancode, ch) {
        (SC_BACKSPACE, _) => {
            st.url.pop();
        }
        (SC_ENTER, _) => actions::perform(Action::TestConnection),
        (SC_TAB | SC_ESC, _) => return false,
        (_, Some(c)) if (c.is_ascii_graphic() || c == ' ') && st.url.len() < MAX_URL_LEN => {
            st.url.push(c);
        }
        _ => return false,
    }
    true
}

fn key_navigation(st: &mut state::State, scancode: u8) -> bool {
    match scancode {
        SC_TAB => {
            state::controls().move_focus(crate::usb::hid::shift_held());
        }
        SC_UP | SC_DOWN => step_up_down(st, scancode == SC_DOWN),
        SC_LEFT | SC_RIGHT => return step_slider(scancode == SC_RIGHT),
        SC_PAGE_UP | SC_PAGE_DOWN | SC_HOME | SC_END => scroll_by_key(st, scancode),
        SC_ENTER | SC_SPACE => return activate_focus(),
        _ => return false,
    }
    true
}

fn step_up_down(st: &mut state::State, down: bool) {
    let focus = state::controls().focus;
    let in_sidebar =
        focus == KEY_NONE || matches!(Action::from_key(focus), Some(Action::SelectPage(_)));
    if in_sidebar {
        let next = st.page.neighbour(if down { 1 } else { -1 });
        actions::perform(Action::SelectPage(next));
        state::controls().focus = Action::SelectPage(next).key();
        state::controls().focus_visible = true;
    } else {
        let delta = if down {
            actions::SCROLL_STEP
        } else {
            -actions::SCROLL_STEP
        };
        st.scroll += delta;
    }
}

fn step_slider(right: bool) -> bool {
    if Action::from_key(state::controls().focus) != Some(Action::Volume) {
        return false;
    }
    let delta = if right {
        VOLUME_KEY_STEP
    } else {
        -VOLUME_KEY_STEP
    };
    let level = scroll_free_step(crate::wm::tray_popups::volume_level(), delta);
    actions::on_drag(Action::Volume.key(), level);
    true
}

fn scroll_free_step(level: u32, delta: i32) -> u32 {
    crate::wm::controls::slider::step(level, 0, 100, delta)
}

fn scroll_by_key(st: &mut state::State, scancode: u8) {
    let page = actions::SCROLL_STEP * 6;
    match scancode {
        SC_PAGE_UP => st.scroll -= page,
        SC_PAGE_DOWN => st.scroll += page,
        SC_HOME => st.scroll = 0,
        _ => st.scroll = i32::MAX,
    }
}

fn activate_focus() -> bool {
    match Action::from_key(state::controls().focus) {
        Some(Action::Volume | Action::PageScroll | Action::UrlField) | None => false,
        Some(action) => {
            actions::perform(action);
            true
        }
    }
}
