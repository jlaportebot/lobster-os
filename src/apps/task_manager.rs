//! Task Manager — SysMon + Process Manager merged (B6.3)
//!
//! One window, two tabs: Performance (CPU/memory/network) and
//! Processes (list + kill). Reuses the two apps' `render_pane`s as
//! tab content so both stay single-sourced.

use crate::display::graphics;
use crate::wm::theme;
use crate::wm::window;

pub const APP_NAME: &str = "Task Manager";
const TAB_BAR_H: u32 = 24;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Performance,
    Processes,
}

struct TaskManager {
    tab: Tab,
    perf: crate::apps::sysmon::SystemMonitor,
    procs: crate::apps::process_manager::ProcessManager,
}

static mut TASK_MANAGER: Option<TaskManager> = None;

fn get() -> &'static mut TaskManager {
    unsafe {
        if TASK_MANAGER.is_none() {
            TASK_MANAGER = Some(TaskManager {
                tab: Tab::Performance,
                perf: crate::apps::sysmon::SystemMonitor::new(),
                procs: crate::apps::process_manager::ProcessManager::new(),
            });
        }
        TASK_MANAGER.as_mut().expect("task manager")
    }
}

/// Open the Task Manager window (replaces the two separate apps).
pub fn open() {
    if window::find_by_title(APP_NAME).is_none() {
        window::create(800, 50, 460, 440, APP_NAME);
    }
    window::focus(window::find_by_title(APP_NAME).unwrap_or(0));
    crate::wm::dirty::mark();
}

/// Switch tabs.
pub fn set_tab(tab: Tab) {
    get().tab = tab;
    crate::wm::dirty::mark();
}

fn switch_tab(delta: i32) {
    let tm = get();
    tm.tab = match tm.tab {
        Tab::Performance => Tab::Processes,
        Tab::Processes => Tab::Performance,
    };
    let _ = delta;
    crate::wm::dirty::mark();
}

/// Keyboard: Tab key switches panes; arrows/page keys + K drive the
/// process list when the Processes tab is active.
pub fn handle_key(scancode: u8) -> bool {
    let Some(win_id) = window::find_by_title(APP_NAME) else {
        return false;
    };
    if window::focused_id() != win_id {
        return false;
    }
    match scancode {
        0x0F => {
            // Tab key — switch panes
            switch_tab(1);
            true
        }
        0x48 | 0x50 => {
            // Up/Down — process selection (Processes tab)
            let tm = get();
            if tm.tab == Tab::Processes {
                if scancode == 0x48 {
                    tm.procs.select_up();
                } else {
                    tm.procs.select_down();
                }
                crate::wm::dirty::mark();
            }
            false
        }
        0x2E => {
            // K — kill the selected process (Processes tab)
            let tm = get();
            if tm.tab == Tab::Processes {
                tm.procs.kill_selected();
                crate::wm::dirty::mark();
            }
            false
        }
        _ => false,
    }
}

/// Click handling: tab bar hits switch panes. Returns true if consumed.
pub fn handle_click(sx: u32, sy: u32) -> bool {
    let Some(win_id) = window::find_by_title(APP_NAME) else {
        return false;
    };
    let Some((x, y, w, h)) = window::get_info(win_id) else {
        return false;
    };
    let focused = window::focused_id() == win_id;
    let _ = focused;
    // client region (frame chrome: 2px border, 24px title)
    let cx = x + 2;
    let cy = y + 26;
    let cw = w.saturating_sub(4);
    if sx < cx || sy < cy || sx >= cx + cw || sy >= cy + h.saturating_sub(28) {
        return false;
    }
    if sy < cy + TAB_BAR_H {
        // inside the tab bar: find which tab by x
        let mut tab_x = cx + 8;
        let tabs = [
            ("Performance", Tab::Performance),
            ("Processes", Tab::Processes),
        ];
        for (label, tab) in tabs {
            let tw = (label.len() as u32) * 8 + 16;
            if sx >= tab_x && sx < tab_x + tw {
                set_tab(tab);
                return true;
            }
            tab_x += tw + 4;
        }
    }
    false
}

/// Render the Task Manager window.
pub fn render_to_windows(fbuf: &crate::display::fb::FramebufferInfo) {
    let Some(win_id) = window::find_by_title(APP_NAME) else {
        return;
    };
    let Some((x, y, w, h)) = window::get_info(win_id) else {
        return;
    };
    let focused = window::focused_id() == win_id;
    let (cx, cy, cw, ch) =
        crate::wm::window_frame::draw_frame(fbuf, win_id, x, y, w, h, APP_NAME, focused);
    let t = theme::current();

    // Tab bar
    graphics::draw_rect(fbuf, cx, cy, cw, TAB_BAR_H, t.menu_bg);
    let tabs = [
        ("Performance", Tab::Performance),
        ("Processes", Tab::Processes),
    ];
    let mut tab_x = cx + 8;
    let tm = get();
    let accent = crate::wm::theme::accent_color();
    for (label, tab) in tabs {
        let is_active = tm.tab == tab;
        let bg = if is_active { accent } else { t.menu_bg };
        let fg = if is_active {
            0xFFFFFFFF
        } else {
            t.label_text_dim
        };
        let tw = (label.len() as u32) * 8 + 16;
        graphics::draw_rect(fbuf, tab_x, cy + 4, tw, TAB_BAR_H - 8, bg);
        graphics::draw_text(fbuf, tab_x + 8, cy + 8, label, fg, bg);
        tab_x += tw + 4;
    }

    // Pane region below the tab bar
    let py = cy + TAB_BAR_H + 4;
    let ph = ch.saturating_sub(TAB_BAR_H + 8);
    match tm.tab {
        Tab::Performance => tm
            .perf
            .render_pane(fbuf, cx + 2, py, cw.saturating_sub(4), ph),
        Tab::Processes => tm
            .procs
            .render_pane(fbuf, cx + 2, py, cw.saturating_sub(4), ph),
    }
}
