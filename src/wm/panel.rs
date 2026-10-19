//! Desktop Panel/Taskbar — persistent bottom panel with app launcher, task list, and system tray

use crate::display::fb::FramebufferInfo;
use crate::display::graphics::{draw_rect, draw_rect_outline, draw_text};
use crate::println;
use crate::wm::compositor;
use crate::wm::theme;
use crate::wm::widget::{Button, Label};

// ─── Panel Dimensions ──────────────────────────────────────────────────
const PANEL_HEIGHT: u32 = 36;
const LAUNCHER_WIDTH: u32 = 48;
const CLOCK_WIDTH: u32 = 120;
const TRAY_ITEM_WIDTH: u32 = 40;
const TASK_BUTTON_MIN_W: u32 = 120;
const TASK_BUTTON_MAX_W: u32 = 200;

// ─── Panel State ───────────────────────────────────────────────────────

struct PanelState {
    visible: bool,
    launcher_btn: Button,
    task_buttons: [Option<TaskButton>; 16],
    task_count: usize,
    clock_label: Label,
    tray_network: TrayItem,
    tray_volume: TrayItem,
    tray_battery: TrayItem,
    workspace_label: Label,
    last_clock_update: u64,
}

#[derive(Clone, Copy, Debug)]
struct TaskButton {
    widget: Button,
    window_id: u32,
    active: bool,
}

#[derive(Clone, Copy, Debug)]
struct TrayItem {
    widget: Button,
    kind: TrayKind,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum TrayKind {
    Network,
    Volume,
    Battery,
}

static mut PANEL_STATE: PanelState = PanelState {
    visible: true,
    launcher_btn: Button::ZERO,
    task_buttons: [None; 16],
    task_count: 0,
    clock_label: Label::ZERO,
    tray_network: TrayItem {
        widget: Button::ZERO,
        kind: TrayKind::Network,
    },
    tray_volume: TrayItem {
        widget: Button::ZERO,
        kind: TrayKind::Volume,
    },
    tray_battery: TrayItem {
        widget: Button::ZERO,
        kind: TrayKind::Battery,
    },
    workspace_label: Label::ZERO,
    last_clock_update: 0,
};

// ─── Initialization ────────────────────────────────────────────────────

/// Initialize the desktop panel
pub fn init(fb: &FramebufferInfo) {
    unsafe {
        let panel_y = fb.height.saturating_sub(PANEL_HEIGHT);
        let _center_x = fb.width / 2;

        // Launcher button (left side)
        let mut launcher = Button::ZERO;
        launcher.x = 4;
        launcher.y = panel_y + 4;
        launcher.w = LAUNCHER_WIDTH;
        launcher.h = PANEL_HEIGHT - 8;
        launcher.set_text("🏠");
        launcher.state = crate::wm::widget::WidgetState::Normal;
        PANEL_STATE.launcher_btn = launcher;

        // Clock label (right side)
        let mut clock = Label::ZERO;
        clock.x = fb.width - CLOCK_WIDTH - 8;
        clock.y = panel_y + 8;
        clock.set_text("--:--");
        PANEL_STATE.clock_label = clock;

        // Workspace label (center-left)
        let mut workspace = Label::ZERO;
        workspace.x = LAUNCHER_WIDTH + 16;
        workspace.y = panel_y + 8;
        workspace.set_text("Workspace 1");
        PANEL_STATE.workspace_label = workspace;

        // System tray items (right side, left of clock)
        let tray_x_start = fb.width - CLOCK_WIDTH - 8 - 3 * (TRAY_ITEM_WIDTH + 4);

        let mut net = TrayItem {
            widget: Button::ZERO,
            kind: TrayKind::Network,
        };
        net.widget.x = tray_x_start;
        net.widget.y = panel_y + 4;
        net.widget.w = TRAY_ITEM_WIDTH;
        net.widget.h = PANEL_HEIGHT - 8;
        net.widget.set_text("📶");
        PANEL_STATE.tray_network = net;

        let mut vol = TrayItem {
            widget: Button::ZERO,
            kind: TrayKind::Volume,
        };
        vol.widget.x = tray_x_start + TRAY_ITEM_WIDTH + 4;
        vol.widget.y = panel_y + 4;
        vol.widget.w = TRAY_ITEM_WIDTH;
        vol.widget.h = PANEL_HEIGHT - 8;
        vol.widget.set_text("🔊");
        PANEL_STATE.tray_volume = vol;

        let mut bat = TrayItem {
            widget: Button::ZERO,
            kind: TrayKind::Battery,
        };
        bat.widget.x = tray_x_start + 2 * (TRAY_ITEM_WIDTH + 4);
        bat.widget.y = panel_y + 4;
        bat.widget.w = TRAY_ITEM_WIDTH;
        bat.widget.h = PANEL_HEIGHT - 8;
        bat.widget.set_text("🔋");
        PANEL_STATE.tray_battery = bat;

        PANEL_STATE.task_count = 0;
        PANEL_STATE.last_clock_update = 0;
        PANEL_STATE.visible = true;
    }
    println!("[panel] Desktop panel initialized");
}

// ─── Panel Rendering ──────────────────────────────────────────────────

/// Draw the panel (called from compositor after windows)
pub fn draw(fb: &FramebufferInfo) {
    unsafe {
        if !PANEL_STATE.visible {
            return;
        }

        let panel_y = fb.height.saturating_sub(PANEL_HEIGHT);
        let theme = theme::current();

        // Panel background
        draw_rect(fb, 0, panel_y, fb.width, PANEL_HEIGHT, theme.taskbar_bg);

        // Top border
        draw_rect(fb, 0, panel_y, fb.width, 1, theme.taskbar_border);

        // Draw launcher button
        draw_button(fb, &PANEL_STATE.launcher_btn, 0, panel_y);

        // Draw task buttons
        let mut task_x = LAUNCHER_WIDTH + 16;
        for i in 0..PANEL_STATE.task_count {
            if let Some(task) = &PANEL_STATE.task_buttons[i] {
                let mut btn = task.widget;
                btn.x = task_x;
                btn.y = panel_y + 4;
                draw_button(fb, &btn, 0, panel_y);

                let btn_w = btn.w.clamp(TASK_BUTTON_MIN_W, TASK_BUTTON_MAX_W);
                task_x += btn_w + 4;
            }
        }

        // Draw workspace label
        draw_text(
            fb,
            PANEL_STATE.workspace_label.x,
            PANEL_STATE.workspace_label.y,
            &PANEL_STATE.workspace_label.text_str(),
            theme.taskbar_text,
            theme.taskbar_bg,
        );

        // Draw system tray items
        draw_button(fb, &PANEL_STATE.tray_network.widget, 0, panel_y);
        draw_button(fb, &PANEL_STATE.tray_volume.widget, 0, panel_y);
        draw_button(fb, &PANEL_STATE.tray_battery.widget, 0, panel_y);

        // Draw clock
        update_clock(fb);
        draw_text(
            fb,
            PANEL_STATE.clock_label.x,
            PANEL_STATE.clock_label.y,
            &PANEL_STATE.clock_label.text_str(),
            theme.taskbar_text,
            theme.taskbar_bg,
        );
    }
}

fn draw_button(fb: &FramebufferInfo, btn: &Button, offset_x: u32, offset_y: u32) {
    let theme = theme::current();
    let bg = match btn.state {
        crate::wm::widget::WidgetState::Normal => theme.btn_bg,
        crate::wm::widget::WidgetState::Hovered => theme.btn_bg_hover,
        crate::wm::widget::WidgetState::Pressed => theme.btn_bg_pressed,
        _ => theme.btn_bg,
    };

    let px = offset_x + btn.x;
    let py = offset_y + btn.y;

    draw_rect(fb, px, py, btn.w, btn.h, bg);
    draw_rect_outline(fb, px, py, btn.w, btn.h, theme.btn_border);

    if btn.text_len > 0 {
        let text = btn.text_str();
        let char_w = 8u32;
        let text_w = text.chars().count() as u32 * char_w;
        let text_x = px + (btn.w.saturating_sub(text_w)) / 2;
        let text_y = py + (btn.h.saturating_sub(theme.font_size)) / 2;
        draw_text(fb, text_x, text_y, text, theme.btn_text, bg);
    }
}

fn update_clock(_fb: &FramebufferInfo) {
    unsafe {
        let now = crate::arch::ArchTimer::uptime_ms();
        if now - PANEL_STATE.last_clock_update >= 1000 {
            let secs = now / 1000;
            let mins = secs / 60;
            let hours = mins / 60;
            let time_str = alloc::format!("{:02}:{:02}", hours % 24, mins % 60);
            PANEL_STATE.clock_label.set_text(&time_str);
            PANEL_STATE.last_clock_update = now;
        }

        // Update battery (placeholder)
        PANEL_STATE.tray_battery.widget.set_text("🔋100%");

        // Update network (placeholder)
        #[cfg(target_arch = "aarch64")]
        if crate::net::config().ip_addr != [0, 0, 0, 0] {
            PANEL_STATE.tray_network.widget.set_text("📶");
        } else {
            PANEL_STATE.tray_network.widget.set_text("❌");
        }
        #[cfg(target_arch = "x86_64")]
        PANEL_STATE.tray_network.widget.set_text("📶");
    }
}

// ─── Task Button Management ───────────────────────────────────────────

/// Add a task button for a window
pub fn add_task(window_id: u32, title: &str) {
    unsafe {
        if PANEL_STATE.task_count >= 16 {
            return;
        }

        let mut btn = Button::ZERO;
        btn.w = TASK_BUTTON_MIN_W;
        btn.h = PANEL_HEIGHT - 8;
        btn.set_text(title);
        btn.state = crate::wm::widget::WidgetState::Normal;

        PANEL_STATE.task_buttons[PANEL_STATE.task_count] = Some(TaskButton {
            widget: btn,
            window_id,
            active: true,
        });
        PANEL_STATE.task_count += 1;
    }
}

/// Remove a task button for a window
pub fn remove_task(window_id: u32) {
    unsafe {
        for i in 0..PANEL_STATE.task_count {
            if let Some(task) = &PANEL_STATE.task_buttons[i] {
                if task.window_id == window_id {
                    // Shift remaining tasks left
                    for j in i..PANEL_STATE.task_count - 1 {
                        PANEL_STATE.task_buttons[j] = PANEL_STATE.task_buttons[j + 1];
                    }
                    PANEL_STATE.task_buttons[PANEL_STATE.task_count - 1] = None;
                    PANEL_STATE.task_count -= 1;
                    break;
                }
            }
        }
    }
}

/// Update task button active state
pub fn set_task_active(window_id: u32, active: bool) {
    unsafe {
        for i in 0..PANEL_STATE.task_count {
            if let Some(task) = &mut PANEL_STATE.task_buttons[i] {
                if task.window_id == window_id {
                    task.active = active;
                    task.widget.state = if active {
                        crate::wm::widget::WidgetState::Pressed
                    } else {
                        crate::wm::widget::WidgetState::Normal
                    };
                    break;
                }
            }
        }
    }
}

/// Update task button title
pub fn set_task_title(window_id: u32, title: &str) {
    unsafe {
        for i in 0..PANEL_STATE.task_count {
            if let Some(task) = &mut PANEL_STATE.task_buttons[i] {
                if task.window_id == window_id {
                    task.widget.set_text(title);
                    break;
                }
            }
        }
    }
}

// ─── Panel Visibility ─────────────────────────────────────────────────

/// Panel height (public for other WM components)
pub fn panel_height() -> u32 {
    PANEL_HEIGHT
}

/// Toggle panel visibility
pub fn toggle() {
    unsafe {
        PANEL_STATE.visible = !PANEL_STATE.visible;
    }
}

/// Check if panel is visible
pub fn is_visible() -> bool {
    unsafe { PANEL_STATE.visible }
}

/// Set panel visibility
pub fn set_visible(visible: bool) {
    unsafe {
        PANEL_STATE.visible = visible;
    }
}

// ─── Input Handling ───────────────────────────────────────────────────

/// Handle mouse click on panel
/// Returns the window ID if a task was clicked, or special values:
/// - 0xFFFFFFFF = launcher clicked
/// - 0xFFFFFFFE = network tray clicked
/// - 0xFFFFFFFD = volume tray clicked
/// - 0xFFFFFFFC = battery tray clicked
pub fn handle_click(px: u32, py: u32) -> Option<u32> {
    unsafe {
        let panel_y = get_panel_y();

        if py < panel_y || py >= panel_y + PANEL_HEIGHT {
            return None; // Click not on panel
        }

        // Check launcher button
        if PANEL_STATE.launcher_btn.hit_test(px, py) {
            return Some(0xFFFFFFFF);
        }

        // Check task buttons
        let mut task_x = LAUNCHER_WIDTH + 16;
        for i in 0..PANEL_STATE.task_count {
            if let Some(task) = &PANEL_STATE.task_buttons[i] {
                let btn_w = task.widget.w.clamp(TASK_BUTTON_MIN_W, TASK_BUTTON_MAX_W);
                if px >= task_x
                    && px < task_x + btn_w
                    && py >= panel_y + 4
                    && py < panel_y + PANEL_HEIGHT - 4
                {
                    return Some(task.window_id);
                }
                task_x += btn_w + 4;
            }
        }

        // Check tray items
        if PANEL_STATE.tray_network.widget.hit_test(px, py) {
            return Some(0xFFFFFFFE);
        }
        if PANEL_STATE.tray_volume.widget.hit_test(px, py) {
            return Some(0xFFFFFFFD);
        }
        if PANEL_STATE.tray_battery.widget.hit_test(px, py) {
            return Some(0xFFFFFFFC);
        }

        None
    }
}

fn get_panel_y() -> u32 {
    let fb = compositor::fb_info().expect("fb_info not set");
    fb.height.saturating_sub(PANEL_HEIGHT)
}

// ─── Shell Commands ───────────────────────────────────────────────────

/// Toggle panel visibility from shell
pub fn cmd_panel_toggle() {
    toggle();
    let state = if is_visible() { "shown" } else { "hidden" };
    println!("[panel] Panel {}", state);
}

/// Set panel visibility from shell
pub fn cmd_panel_visible(visible: bool) {
    set_visible(visible);
    let state = if visible { "shown" } else { "hidden" };
    println!("[panel] Panel {}", state);
}
