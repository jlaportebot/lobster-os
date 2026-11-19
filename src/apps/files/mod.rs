//! Files application (O4.2): a mouse-and-keyboard file manager on the shared
//! controls.
//!
//! * [`paths`], [`history`], [`selection`], [`model`], [`fsops`], [`layout`]
//!   and [`targets`] are pure and host-tested.
//! * [`render`] draws with the controls in `crate::wm::controls`;
//!   [`actions`], [`keys`], [`menus`], [`ops`] and [`nav`] change state.
//! * This module wires the window, input and the public entry points.

pub mod actions;
pub mod diag;
pub mod fsops;
pub mod history;
pub mod keys;
pub mod layout;
pub mod menus;
pub mod model;
pub mod nav;
pub mod ops;
pub mod paths;
mod render;
pub mod selection;
pub mod state;
pub mod targets;

use crate::wm::controls::Ui;
use crate::wm::hit_region::{self, Hit};
use crate::wm::window;

/// Title of the Files window.
pub const TITLE: &str = "Files";

/// Initialise the Files application.
pub fn init() {
    crate::println!("[files] Files application initialized");
}

/// The window state, loading the start folder the first time.
pub fn ensure_state() -> &'static mut state::State {
    let first = state::get().is_none();
    let st = state::get_or_init(&nav::start_folder());
    if first {
        nav::load(st);
    }
    st
}

/// Open (or focus) the Files window; returns its id.
pub fn open_window() -> u32 {
    if let Some(id) = window::find_by_title(TITLE) {
        window::focus(id);
        return id;
    }
    ensure_state();
    let screen = crate::display::fb::get_info();
    let panel_h = crate::wm::panel::panel_height();
    let (x, y, w, h) = layout::initial_window(screen.width, screen.height, panel_h);
    window::create(x, y, w, h, TITLE).unwrap_or(0)
}

/// Open the window showing folder `path` (desktop folders, Settings).
pub fn open_at(path: &str) -> u32 {
    let id = open_window();
    if let Some(st) = state::get() {
        nav::navigate(st, path);
    }
    crate::wm::dirty::mark();
    id
}

/// Render hook used by `wm::tick`.
pub fn render_to_windows(fb: &crate::display::fb::FramebufferInfo) {
    let Some(win_id) = window::find_by_title(TITLE) else {
        return;
    };
    let (Some(win), Some(st)) = (window::get(win_id), state::get()) else {
        return;
    };
    let focused = window::focused_id() == win_id;
    let target = crate::wm::scene::window_target(win_id, fb);
    let (cx, cy, cw, ch) = crate::wm::window_frame::draw_frame(
        &target, win_id, win.x, win.y, win.w, win.h, TITLE, focused,
    );
    hit_region::set_handler(win_id, on_hit);
    ops::tick(st);
    let mut ui = Ui::new(&target, win_id, (cx, cy), (cw, ch), state::controls());
    render::frame(&mut ui, st, (cw, ch));
    ui.finish();
}

fn on_hit(hit: &Hit) -> bool {
    window::focus(hit.win_id);
    let Some(st) = state::get() else {
        return false;
    };
    actions::press(st, hit)
}

/// Keyboard input while the Files window is focused. True when consumed.
pub fn handle_key(scancode: u8) -> bool {
    if window::get_title(window::focused_id()) != TITLE {
        return false;
    }
    state::get().is_some_and(|st| keys::handle(st, scancode))
}

/// Right press at screen (`x`, `y`): shows the item or background menu when
/// the pointer is over the item area of the Files window.
pub fn right_click(x: u32, y: u32) -> bool {
    let Some(hit) = hit_region::route_hit(x, y) else {
        return false;
    };
    let is_files = window::get_title(hit.win_id) == TITLE;
    let on_body = targets::Target::from_key(hit.region.tag) == Some(targets::Target::Body);
    let (true, true, Some(st)) = (is_files, on_body, state::get()) else {
        return false;
    };
    window::focus(hit.win_id);
    menus::show(st, &hit, x, y);
    true
}

/// Run a Files context-menu action; false when the id is not ours.
pub fn handle_context_action(id: u32) -> bool {
    menus::dispatch(id)
}
