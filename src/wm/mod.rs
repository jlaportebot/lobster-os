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
pub mod compositor;
pub mod config;
pub mod context_menu;
pub mod desktop_icons;
pub mod dirty;
pub mod event_router;
pub mod greeter;
pub mod gui_files;
pub mod gui_shell;
pub mod input;
pub mod launcher;
pub mod notifications;
pub mod panel;
#[cfg(feature = "qemu_raspi5b")]
pub mod scene;
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
        render_app(crate::apps::file_manager::render_to_windows);
        render_app(crate::apps::browser::render_to_windows);
        render_app(crate::apps::editor::render_to_windows);
        render_app(crate::apps::notes::render_to_windows);
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

    let content_dirty = dirty::should_render_static();
    render_apps(content_dirty);
    if !user_only {
        window_frame::tick_animations();
        notifications::tick();
    }
    damage_live_regions(content_dirty);
    scene::compose();
    compositor::draw_software_cursor(&device);
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
    context_menu::draw();
}

/// Handle keyboard shortcuts (called from input module)
pub fn handle_shortcut(scancode: u8) {
    use crate::usb::hid;
    use crate::wm::window::SnapPosition;

    // Mod + key shortcuts
    let mod_key = hid::gui_held();

    if mod_key {
        match scancode {
            // Super+D: minimize all / show desktop
            0x07 => {
                let count = window::count();
                for id in 1..=count as u32 {
                    if window::exists(id) {
                        window::minimize(id);
                    }
                }
            }
            // Super+Left: snap focused window left
            0x50 => {
                let fid = window::focused_id();
                if fid != 0 {
                    window::snap(fid, SnapPosition::Left);
                }
            }
            // Super+Right: snap focused window right
            0x4F => {
                let fid = window::focused_id();
                if fid != 0 {
                    window::snap(fid, SnapPosition::Right);
                }
            }
            // Super+Up: snap focused window top half
            0x52 => {
                let fid = window::focused_id();
                if fid != 0 {
                    window::snap(fid, SnapPosition::Full);
                }
            }
            // Super+Down: minimize focused window or restore from snapped
            0x51 => {
                let fid = window::focused_id();
                if fid != 0 {
                    window::minimize(fid);
                }
            }
            // Ctrl+Alt+1..4: switch workspace
            _ if hid::alt_held() => match scancode {
                0x1E => workspace::switch_to(0), // 1
                0x1F => workspace::switch_to(1), // 2
                0x20 => workspace::switch_to(2), // 3
                0x21 => workspace::switch_to(3), // 4
                _ => {}
            },
            _ => {}
        }
    }

    // Alt+F4: close focused window
    if hid::alt_held() && scancode == 0x29 {
        let fid = window::focused_id();
        if fid != 0 {
            window::destroy(fid);
        }
    }
}
