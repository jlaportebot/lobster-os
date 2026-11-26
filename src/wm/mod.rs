//! Window Manager for LobsterOS
//!
//! Provides a tiling+floating hybrid window manager with:
//! - Compositor: double-buffered scene composition
//! - Window management: create, destroy, move, resize, focus
//! - Theme engine: colors, fonts, spacing, rounded corners
//! - Widget toolkit: buttons, labels, text inputs, scrollbars, panels
//! - Desktop Panel: taskbar, app launcher, system tray, clock
//! - Application Launcher: .desktop file parsing and app launching
//! - Input bridge: routes USB HID keyboard/mouse events to windows
//!
//! All rendering goes through `display::graphics` primitives.

pub mod alt_tab;
pub mod calendar;
pub mod compositor;
pub mod config;
pub mod context_menu;
pub mod controls;
pub mod desktop_files;
pub mod desktop_icons;
pub mod desktop_layout;
pub mod dialog;
pub mod dirty;
pub mod event_router;
pub mod greeter;
pub mod gui_shell;
pub mod hit_region;
pub mod input;
pub mod launcher;
pub mod lock;
pub mod notifications;
pub mod panel;
pub mod panel_layout;
pub mod scene;
pub mod shortcuts;
pub mod start_menu;
pub mod theme;
pub mod tray_popups;
pub mod user_windows;
pub mod wallpaper;
pub mod widget;
pub mod window;
pub mod window_frame;
pub mod workspace;

use crate::display::fb::get_info;
use crate::println;

/// Initialize the window manager
pub fn init() {
    let fb = get_info();
    if fb.width == 0 {
        println!("[wm] No framebuffer — window manager disabled");
        return;
    }
    println!(
        "[wm] FB info: {}x{} @ {}bpp, virt={:#018x}",
        fb.width, fb.height, fb.bpp, fb.virt_addr
    );
    println!("[wm] Step 1: compositor::init...");
    compositor::init(fb);
    println!("[wm] Step 2: window::init...");
    window::init();
    println!("[wm] Step 3: theme::init...");
    theme::init();
    println!("[wm] Step 4: workspace::init...");
    workspace::init();
    println!("[wm] Step 5: input::init...");
    input::init();
    println!("[wm] Step 6: panel::init...");
    panel::init(&fb);
    println!("[wm] Step 6b: desktop_icons::init...");
    desktop_icons::init();
    println!("[wm] Step 6c: dialog::init...");
    dialog::init();
    println!(
        "[wm] Window manager initialized ({}x{})",
        fb.width, fb.height
    );
}

/// Frames between terminal repaints when idle (cursor blink cadence).
#[cfg(feature = "qemu_raspi5b")]
const TERMINAL_IDLE_FRAMES: u32 = 8;
/// Frames between monitor repaints when idle (graphs update at ~2 Hz).
#[cfg(feature = "qemu_raspi5b")]
const MONITOR_IDLE_FRAMES: u32 = 15;
/// The panel (clock, task list) is recomposed at least this often.
#[cfg(feature = "qemu_raspi5b")]
const PANEL_REFRESH_FRAMES: u32 = 30;

/// Run one app's window renderer, then stop redirecting drawing to its
/// surface.
#[cfg(feature = "qemu_raspi5b")]
fn render_app(render: fn(&crate::display::fb::FramebufferInfo)) {
    let screen = crate::display::fb::device_info();
    render(&screen);
    scene::end_window_render();
}

/// Redraw the built-in apps whose content may have changed into their
/// window surfaces. `content_dirty` forces every app to redraw.
#[cfg(feature = "qemu_raspi5b")]
fn render_apps(content_dirty: bool) {
    let frame = dirty::frame();
    if content_dirty || frame % TERMINAL_IDLE_FRAMES == 0 {
        render_app(crate::apps::terminal::render_to_windows);
    }
    if content_dirty || frame % MONITOR_IDLE_FRAMES == 0 {
        render_app(crate::apps::sysmon::render_to_windows);
        render_app(crate::apps::process_manager::render_to_windows);
        render_app(crate::apps::task_manager::render_to_windows);
    }
    if content_dirty {
        render_app(crate::apps::calculator::render_to_windows);
        render_app(crate::apps::files::render_to_windows);
        render_app(crate::apps::browser::render_to_windows);
        render_app(crate::apps::editor::render_to_windows);
        render_app(crate::apps::notes::render_to_windows);
        render_app(crate::apps::welcome::render_to_windows);
        render_app(crate::apps::settings::render_settings_window);
        render_app(crate::apps::image_viewer::bmp_view::render_to_windows);
        render_app(crate::wm::user_windows::render_to_windows);
    }
}

/// Damage the screen areas whose content changes without an input event.
#[cfg(feature = "qemu_raspi5b")]
fn damage_live_regions(content_dirty: bool) {
    let screen = crate::display::fb::device_info();
    let panel_h = panel::PANEL_HEIGHT;
    if content_dirty || dirty::frame() % PANEL_REFRESH_FRAMES == 0 {
        scene::damage(scene::Rect::from_xywh(
            0,
            screen.height.saturating_sub(panel_h),
            screen.width,
            panel_h,
        ));
    }
    let overlay_open = start_menu::is_open()
        || tray_popups::is_open()
        || alt_tab::is_visible()
        || context_menu::is_visible();
    let pointer_moved = input::poll_count() > 0;
    // Popups track hover and selection, and toasts animate on their own.
    if (overlay_open && (content_dirty || pointer_moved)) || notifications::needs_redraw() {
        scene::damage_all();
    }
}

/// One frame on the scene-compositor path (QEMU build).
#[cfg(feature = "qemu_raspi5b")]
fn tick_scene(user_only: bool) {
    let device = crate::display::fb::device_info();
    if device.width == 0 {
        return;
    }
    if !scene::is_active() {
        scene::init();
    }
    compositor::restore_software_cursor(&device);

    if user_only {
        input::pump_events_user_only();
    } else {
        input::pump_events();
    }
    let had_events = input::poll_count();
    // Only keys and buttons change app content; pointer motion alone just
    // moves the cursor (drags damage the scene through move/resize).
    if had_events > 0 && input::action_count() > 0 {
        dirty::mark();
    }
    if window_frame::animating() {
        dirty::mark();
    }

    if lock::take_redraw() {
        scene::damage_all();
    }
    let content_dirty = dirty::should_render_static();
    render_apps(content_dirty);
    if !user_only {
        window_frame::tick_animations();
        notifications::tick();
        desktop_icons::tick();
    }
    damage_live_regions(content_dirty);
    let presented = scene::compose();
    compositor::draw_software_cursor(&device);
    if presented && crate::display::splash::is_done() {
        crate::display::splash::announce_desktop_ready_if_pending();
    }
}

/// Run one frame of the window manager: poll input, update, composite
pub fn tick() {
    // Reentrancy guard: sys_sleep pumps ticks from inside a syscall that
    // was itself called from a tick-driven shell path. A nested tick only
    // routes input to user windows and recomposes; it must not re-run
    // shell/panel input handling.
    static IN_TICK: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
    let is_nested = IN_TICK.swap(true, core::sync::atomic::Ordering::Relaxed);

    #[cfg(feature = "qemu_raspi5b")]
    tick_scene(is_nested);

    #[cfg(not(feature = "qemu_raspi5b"))]
    if !is_nested {
        tick_hardware();
    }

    if !is_nested {
        IN_TICK.store(false, core::sync::atomic::Ordering::Relaxed);
    }
}

/// One frame on the hardware path (layer compositor in compositor.rs).
#[cfg(not(feature = "qemu_raspi5b"))]
fn tick_hardware() {
    let had_events = input::poll_count();
    input::pump_events();
    if had_events > 0 {
        dirty::mark();
    }
    compositor::composite();
    let fb = get_info();
    if fb.width > 0 {
        desktop_icons::draw(&fb);
        panel::draw(&fb);
    }
    window_frame::tick_animations();
    notifications::tick();
    start_menu::draw(&fb);
    tray_popups::draw(&fb);
    alt_tab::draw(&fb);
    notifications::draw(&fb);
    context_menu::draw(&fb);
    dialog::draw(&fb);
    if lock::is_active() {
        lock::draw(&fb);
    }
    if crate::display::splash::is_done() {
        crate::display::splash::announce_desktop_ready_if_pending();
    }
}

/// Handle keyboard shortcuts (called from input module)
pub fn handle_shortcut(scancode: u8) {
    use crate::usb::hid;
    use crate::wm::shortcuts::{lookup, Shortcut};
    use crate::wm::window::SnapPosition;

    let Some(action) = lookup(scancode, hid::gui_held(), hid::alt_held()) else {
        return;
    };
    let focused = window::focused_id();
    match action {
        Shortcut::ShowDesktop => {
            for id in 1..=window::max_id() {
                if window::exists(id) {
                    window::minimize(id);
                }
            }
        }
        Shortcut::SnapLeft if focused != 0 => window::snap(focused, SnapPosition::Left),
        Shortcut::SnapRight if focused != 0 => window::snap(focused, SnapPosition::Right),
        Shortcut::Maximize if focused != 0 => window::snap(focused, SnapPosition::Full),
        Shortcut::Minimize if focused != 0 => window::minimize(focused),
        Shortcut::CloseWindow if focused != 0 => window::destroy(focused),
        Shortcut::Workspace(index) => workspace::switch_to(index),
        _ => {}
    }
}
