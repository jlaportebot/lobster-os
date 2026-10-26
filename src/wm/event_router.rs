//! Compositor event router (C2): the priority-ordered routing table for
//! mouse-button events.
//!
//! `input::handle_mouse_button` updates the button state, delivers the
//! release of a captured user-window press, handles the right-click
//! gesture, then asks [`route_button`] to walk [`STAGES`] top to bottom.
//! For each stage, if `hit(ev)` is true the stage's `handle(ev)` runs; if it
//! returns `true` the event is *consumed* and the walk stops. A stage whose
//! `hit` is true but whose handler returns `false` lets the event fall
//! through to the next stage.
//!
//! # Order (do not reorder without re-running the click-routing suite)
//!
//! | # | stage            | sees                  | why it sits here |
//! |---|------------------|-----------------------|------------------|
//! | 0 | `context_menu`   | every button edge     | Modal popup drawn above everything. While visible it swallows ALL button edges (even outside clicks and releases) so a click can never leak to what is drawn beneath it. |
//! | 1 | `task_manager`   | left press            | Tab bar lives inside a window's client area; must beat generic window focus. |
//! | 2 | `terminal_tabs`  | left press            | Same: tab strip inside the Terminal client area. |
//! | 3 | `editor_tabs`    | left press            | Same: tab strip inside the Editor client area. |
//! | 4 | `user_windows`   | left press, content   | EL0 app content. Beats the panel so a window overlapping the panel area gets its own clicks; chrome hits are NOT claimed and fall through to the chrome stage. |
//! | 5 | `panel`          | left press            | Launcher, task buttons, tray. Above desktop icons and plain windows. |
//! | 6 | `tray_popups`    | left press            | Popups inside click is consumed; outside click closes (and is consumed). |
//! | 7 | `start_menu`     | left press            | Same inside-consume / outside-close contract. |
//! | 8 | `desktop_icons`  | left press on an icon | Icons are the bottom-most clickable surface. |
//! | 9 | `window_chrome`  | left press + release  | Catch-all: focus, drag, resize, close/min/max (window.rs). Also owns the left-release that ends drag/resize. |
//!
//! # Invariants
//!
//! * Stage 0 is first because it is modal; stage 9 is last because it is
//!   the fallback that always runs on a left press that nothing claimed.
//! * Only stage 0 and stage 9 observe releases. The press→release capture
//!   for user windows is done *before* the table (in `input.rs`), so a
//!   release is never swallowed by an early return (e.g. context menu).
//! * Right-press is a gesture handled before the table
//!   ([`route_right_press`]) and never reaches the stages.
//! * The restricted nested pump (`input::pump_events_user_only`, used from
//!   inside syscalls while an EL0 GUI app runs) deliberately does not use
//!   this table: panel/desktop/shell routing must only run from the main
//!   loop. It calls stage 4's primitives only.
//! * Hover observers ([`route_move`]) consume nothing.

use crate::apps;
use crate::usb::hid::HidEvent;
use crate::wm::{
    config, context_menu, desktop_icons, input, panel, start_menu, tray_popups, user_windows,
    window,
};

/// A mouse-button edge, as seen by the router.
#[derive(Clone, Copy)]
pub struct ButtonEv {
    pub x: u32,
    pub y: u32,
    pub left: bool,
    pub right: bool,
    pub middle: bool,
    /// Left-button state before this event.
    pub prev_left: bool,
}

impl ButtonEv {
    fn left_press(&self) -> bool {
        self.left && !self.prev_left
    }
    fn left_release(&self) -> bool {
        !self.left && self.prev_left
    }
}

/// One routing stage.
pub struct Stage {
    pub name: &'static str,
    /// Cheap applicability / hit test. The handler runs only if this is true.
    pub hit: fn(&ButtonEv) -> bool,
    /// Returns true if the event was consumed.
    pub handle: fn(&ButtonEv) -> bool,
}

/// The routing table, in priority order. See module docs.
pub static STAGES: [Stage; 10] = [
    Stage {
        name: "context_menu",
        hit: hit_context_menu,
        handle: handle_context_menu,
    },
    Stage {
        name: "task_manager",
        hit: hit_left_press,
        handle: handle_task_manager,
    },
    Stage {
        name: "terminal_tabs",
        hit: hit_left_press,
        handle: handle_terminal_tabs,
    },
    Stage {
        name: "editor_tabs",
        hit: hit_left_press,
        handle: handle_editor_tabs,
    },
    Stage {
        name: "user_windows",
        hit: hit_user_window_content,
        handle: handle_user_windows,
    },
    Stage {
        name: "panel",
        hit: hit_left_press,
        handle: handle_panel,
    },
    Stage {
        name: "tray_popups",
        hit: hit_left_press,
        handle: handle_tray_popups,
    },
    Stage {
        name: "start_menu",
        hit: hit_left_press,
        handle: handle_start_menu,
    },
    Stage {
        name: "desktop_icons",
        hit: hit_desktop_icon,
        handle: handle_desktop_icons,
    },
    Stage {
        name: "window_chrome",
        hit: hit_press_or_release,
        handle: handle_window_chrome,
    },
];

/// Walk the table; returns the name of the consuming stage, if any.
pub fn route_button(ev: &ButtonEv) -> Option<&'static str> {
    for stage in STAGES.iter() {
        if (stage.hit)(ev) && (stage.handle)(ev) {
            return Some(stage.name);
        }
    }
    None
}

// ─── Gestures handled outside the table ──────────────────────────────

/// Right-button press: toggle/show the desktop context menu. Always consumes.
pub fn route_right_press(x: u32, y: u32) {
    // If context menu is already visible, hide it
    if context_menu::is_visible() {
        context_menu::hide();
        return;
    }
    // Only offer the desktop menu when no window is focused
    let focused = window::focused_id();
    let on_window = window::get_info(focused).is_some();
    if !on_window {
        context_menu::show_desktop_menu(x, y);
    }
}

/// Pointer moved (cursor already updated). Hover observers consume nothing;
/// focus-follows-z is opt-in (`wmconfig ffz on`).
pub fn route_move(x: u32, y: u32, dx: i16, dy: i16) {
    // Hover tracking
    start_menu::handle_move(x, y);
    desktop_icons::update_hover(x, y);

    // Drag / resize bookkeeping
    window::handle_event(HidEvent::MouseMove { dx, dy });

    // Focus-follows-z: chrome hover raises focus. Skipped while a button
    // is held (drag/resize/press-capture in progress) or a modal is up.
    if config::focus_follows_z() {
        let (l, r, m) = input::mouse_buttons();
        if !(context_menu::is_visible() || l || r || m) {
            if let Some(id) = window::chrome_window_at(x, y) {
                if id != window::focused_id() {
                    window::focus(id);
                }
            }
        }
    }
}

// ─── Hit tests ───────────────────────────────────────────────────────

fn hit_context_menu(_: &ButtonEv) -> bool {
    context_menu::is_visible()
}

fn hit_left_press(ev: &ButtonEv) -> bool {
    ev.left_press()
}

fn hit_press_or_release(ev: &ButtonEv) -> bool {
    ev.left_press() || ev.left_release()
}

fn hit_user_window_content(ev: &ButtonEv) -> bool {
    ev.left_press() && matches!(user_windows::hit_test(ev.x, ev.y), Some((_, _, _, true)))
}

fn hit_desktop_icon(ev: &ButtonEv) -> bool {
    ev.left_press() && desktop_icons::hit_test(ev.x, ev.y).is_some()
}

// ─── Stage handlers ──────────────────────────────────────────────────

/// Modal: updates hover, runs a clicked action, and consumes everything.
fn handle_context_menu(ev: &ButtonEv) -> bool {
    context_menu::handle_hover(ev.x, ev.y);
    if ev.left_press() {
        if let Some(action) = context_menu::handle_click(ev.x, ev.y) {
            input::handle_context_action(action);
        }
    }
    true
}

fn handle_task_manager(ev: &ButtonEv) -> bool {
    apps::task_manager::handle_click(ev.x, ev.y)
}

fn handle_terminal_tabs(ev: &ButtonEv) -> bool {
    apps::terminal::handle_click(ev.x, ev.y)
}

fn handle_editor_tabs(ev: &ButtonEv) -> bool {
    apps::editor::handle_click(ev.x, ev.y)
}

fn handle_user_windows(ev: &ButtonEv) -> bool {
    if let Some((win_id, ux, uy, true)) = user_windows::hit_test(ev.x, ev.y) {
        input::begin_user_click(win_id, ux, uy);
        return true;
    }
    false
}

fn handle_panel(ev: &ButtonEv) -> bool {
    if let Some(action) = panel::handle_click(ev.x, ev.y) {
        input::handle_panel_action(action);
        return true;
    }
    false
}

fn handle_tray_popups(ev: &ButtonEv) -> bool {
    tray_popups::handle_click(ev.x, ev.y)
}

fn handle_start_menu(ev: &ButtonEv) -> bool {
    start_menu::handle_click(ev.x, ev.y)
}

fn handle_desktop_icons(ev: &ButtonEv) -> bool {
    if let Some(icon_idx) = desktop_icons::hit_test(ev.x, ev.y) {
        desktop_icons::handle_click(icon_idx);
        return true;
    }
    false
}

/// Fallback: clear icon selection on press, then hand the edge to window.rs
/// (focus / drag / resize / buttons on press; ends drag/resize on release).
fn handle_window_chrome(ev: &ButtonEv) -> bool {
    if ev.left_press() {
        desktop_icons::select(None);
    }
    window::handle_event(HidEvent::MouseButton {
        left: ev.left,
        right: ev.right,
        middle: ev.middle,
    });
    true
}
