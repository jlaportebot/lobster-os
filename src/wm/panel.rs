//! Desktop taskbar: start button, pinned apps, running windows, tray, clock
//! and the show-desktop strip.
//!
//! The geometry and click semantics live in `panel_layout` (pure, tested on
//! the host). This module snapshots the window table, paints the laid-out
//! items and carries out their actions. Drawing and hit-testing share one
//! item list, so a pixel is clickable exactly when it is painted as a
//! button.

use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::display::fb::FramebufferInfo;
use crate::display::graphics::{blend_rect, blend_rounded_rect, fill_circle, fill_rounded_rect};
use crate::display::icons::{self, Glyph};
use crate::display::text::{self, Face};
use crate::println;
use crate::sync::spinlock::Spinlock;
use crate::wm::panel_layout::{
    self as layout, Action, DesktopToggle, Item, ShowDesktop, TaskAction, WinView,
};
use crate::wm::theme::{self, Theme};
use crate::wm::tray_popups::{self, PopupKind};
use crate::wm::{input, notifications, start_menu, window, workspace};

/// Taskbar height in px (re-exported for the compositor's damage logic).
pub const PANEL_HEIGHT: u32 = layout::PANEL_HEIGHT;

/// Opacity of the bar over the wallpaper.
const BAR_ALPHA: u8 = 235;
/// Opacity of the 1 px highlight along the top edge.
const TOP_HIGHLIGHT_ALPHA: u8 = 44;
/// Overlay opacity for a hovered, pressed and active (focused) button.
const HOVER_ALPHA: u8 = 30;
const PRESSED_ALPHA: u8 = 62;
const ACTIVE_ALPHA: u8 = 46;
/// Corner radius of buttons and the start mark.
const BUTTON_RADIUS: u32 = 8;
/// Edge of the start mark tile and the glyph painted on it.
const START_MARK: u32 = 28;
const START_GLYPH: u32 = 20;
const START_MARK_COLOR: u32 = 0xFFCC_3300;
/// App tile edge on pinned and task buttons.
const APP_ICON: u32 = 24;
/// Tray glyph edge.
const TRAY_GLYPH: u32 = 20;
/// Padding between a button's left edge and its icon.
const ICON_PAD: u32 = 10;
/// Space between a task icon and its title.
const LABEL_GAP: u32 = 8;
/// Running-indicator pill: widths for a focused and a background window.
const PILL_H: u32 = 3;
const PILL_FOCUSED_W: u32 = 14;
const PILL_BACKGROUND_W: u32 = 6;
const PILL_BOTTOM_INSET: u32 = 3;
/// Workspace dot radius for the active and the idle dot.
const DOT_RADIUS: u32 = 4;
const IDLE_DOT_ALPHA: u32 = 110;
/// Text sizes at the Medium font setting.
const TITLE_PX: u32 = 13;
const TIME_PX: u32 = 14;
const DATE_PX: u32 = 11;
const BADGE_PX: u32 = 11;
/// Line-box caps so text always fits the 40 px buttons.
const TIME_LINE_H: u32 = 18;
const DATE_LINE_H: u32 = 14;
const BADGE_LINE_H: u32 = 14;
const TITLE_LINE_H: u32 = 20;
/// Bell badge circle radius and its offset from the glyph's top-right.
const BADGE_RADIUS: u32 = 8;
const BADGE_COLOR: u32 = 0xFFD0_3A3A;
/// Dim alpha for the title of a minimised window.
const MINIMIZED_ALPHA: u32 = 150;
const MS_PER_MINUTE: u64 = 60_000;
const MINUTES_PER_HOUR: u64 = 60;
const HOURS_PER_DAY: u64 = 24;

static VISIBLE: AtomicBool = AtomicBool::new(true);
/// Notifications already seen by opening the bell.
static SEEN_NOTIFICATIONS: AtomicUsize = AtomicUsize::new(0);
static SHOW_DESKTOP: Spinlock<ShowDesktop> = Spinlock::new(ShowDesktop::new());

/// Initialize the taskbar (it keeps no framebuffer-sized state: geometry is
/// derived from the live screen size on every draw and click).
pub fn init(_fb: &FramebufferInfo) {
    VISIBLE.store(true, Ordering::Relaxed);
    println!("[panel] Taskbar initialized ({} px)", PANEL_HEIGHT);
}

/// Taskbar height, the single source of truth for the work area.
pub fn panel_height() -> u32 {
    PANEL_HEIGHT
}

// ─── Window snapshot ──────────────────────────────────────────────────

/// Pinned-app index a window title belongs to, matched through the icon
/// table so aliases ("File Manager" for Files) resolve.
fn pinned_index(title: &str) -> Option<usize> {
    let tile = icons::for_app(title);
    if tile == icons::for_app("") {
        return None;
    }
    layout::PINNED_APPS
        .iter()
        .position(|name| icons::for_app(name) == tile)
}

fn view_of(win: &window::Window, focused_id: u32) -> WinView {
    let minimized = win.state == window::WinState::Minimized;
    WinView {
        id: win.id,
        pinned: pinned_index(win.title_str()),
        focused: win.id == focused_id && !minimized,
        minimized,
    }
}

/// The windows on the active workspace, in creation order.
fn snapshot() -> Vec<WinView> {
    let focused_id = window::focused_id();
    let mut views = Vec::new();
    for id in 1..=window::max_id() {
        let Some(win) = window::get(id) else {
            continue;
        };
        if workspace::is_visible_here(id) {
            views.push(view_of(&win, focused_id));
        }
    }
    views
}

fn bar_y(fb: &FramebufferInfo) -> u32 {
    fb.height.saturating_sub(PANEL_HEIGHT)
}

fn bar_items(fb: &FramebufferInfo) -> Vec<Item> {
    layout::layout(fb.width, bar_y(fb), workspace::count(), &snapshot())
}

// ─── Painting ─────────────────────────────────────────────────────────

/// Pointer-derived state of one item.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Interaction {
    Idle,
    Hovered,
    Pressed,
}

fn interaction_of(item: &Item) -> Interaction {
    let (px, py) = input::cursor_pos();
    if !item.rect.contains(px, py) {
        return Interaction::Idle;
    }
    if input::mouse_buttons().0 {
        Interaction::Pressed
    } else {
        Interaction::Hovered
    }
}

/// Draw the taskbar (called from the compositor after windows).
pub fn draw(fb: &FramebufferInfo) {
    if !VISIBLE.load(Ordering::Relaxed) {
        return;
    }
    let th = theme::current();
    let y = bar_y(fb);
    blend_rect(fb, 0, y, fb.width, PANEL_HEIGHT, th.taskbar_bg, BAR_ALPHA);
    blend_rect(fb, 0, y, fb.width, 1, 0xFFFF_FFFF, TOP_HIGHLIGHT_ALPHA);
    for item in &bar_items(fb) {
        draw_item(fb, th, item, interaction_of(item));
    }
}

fn draw_item(fb: &FramebufferInfo, th: &Theme, item: &Item, state: Interaction) {
    match item.action {
        Action::Start => draw_start(fb, th, item, state),
        Action::Pinned(index) => draw_pinned(fb, th, item, state, index),
        Action::Window(_) => draw_task(fb, th, item, state),
        Action::Overflow => draw_overflow(fb, th, item, state),
        Action::Workspace(index) => draw_workspace(fb, th, item, index),
        Action::Network | Action::Volume => draw_glyph_button(fb, th, item, state),
        Action::Bell => draw_bell(fb, th, item, state),
        Action::Clock => draw_clock(fb, th, item, state),
        Action::ShowDesktop => draw_show_desktop(fb, th, item, state),
    }
}

/// Overlay behind a button for its pointer or focus state.
fn paint_button_bg(
    fb: &FramebufferInfo,
    th: &Theme,
    item: &Item,
    state: Interaction,
    active: bool,
) {
    let alpha = match state {
        Interaction::Pressed => PRESSED_ALPHA,
        Interaction::Hovered => HOVER_ALPHA,
        Interaction::Idle if active => ACTIVE_ALPHA,
        Interaction::Idle => return,
    };
    let r = item.rect;
    blend_rounded_rect(
        fb,
        (r.x, r.y, r.w, r.h),
        BUTTON_RADIUS,
        th.taskbar_text,
        alpha,
    );
}

fn center_in(item: &Item, size: u32) -> (u32, u32) {
    let r = item.rect;
    (
        r.x + r.w.saturating_sub(size) / 2,
        r.y + r.h.saturating_sub(size) / 2,
    )
}

fn draw_start(fb: &FramebufferInfo, th: &Theme, item: &Item, state: Interaction) {
    paint_button_bg(fb, th, item, state, start_menu::is_open());
    let (x, y) = center_in(item, START_MARK);
    fill_rounded_rect(
        fb,
        x,
        y,
        START_MARK,
        START_MARK,
        BUTTON_RADIUS,
        START_MARK_COLOR,
    );
    let (gx, gy) = center_in(item, START_GLYPH);
    icons::draw_glyph(fb, Glyph::Grid, START_GLYPH, gx, gy, 0xFFFF_FFFF);
}

fn draw_running_pill(fb: &FramebufferInfo, item: &Item, view: WinView) {
    let width = if view.focused {
        PILL_FOCUSED_W
    } else {
        PILL_BACKGROUND_W
    };
    let r = item.rect;
    let x = r.x + r.w.saturating_sub(width) / 2;
    let y = r.y + r.h - PILL_H - PILL_BOTTOM_INSET + 1;
    blend_rounded_rect(fb, (x, y, width, PILL_H), 1, theme::accent_color(), u8::MAX);
}

fn draw_pinned(fb: &FramebufferInfo, th: &Theme, item: &Item, state: Interaction, index: usize) {
    let active = item.window.is_some_and(|v| v.focused);
    paint_button_bg(fb, th, item, state, active);
    let (x, y) = center_in(item, APP_ICON);
    let name = layout::PINNED_APPS.get(index).copied().unwrap_or("");
    icons::draw(fb, icons::for_app(name), APP_ICON, x, y - 1);
    if let Some(view) = item.window {
        draw_running_pill(fb, item, view);
    }
}

fn draw_task(fb: &FramebufferInfo, th: &Theme, item: &Item, state: Interaction) {
    let Some(view) = item.window else {
        return;
    };
    paint_button_bg(fb, th, item, state, view.focused);
    let Some(win) = window::get(view.id) else {
        return;
    };
    let r = item.rect;
    let icon_x = if item.labelled {
        r.x + ICON_PAD
    } else {
        center_in(item, APP_ICON).0
    };
    let icon_y = r.y + (r.h - APP_ICON) / 2 - 1;
    icons::draw(
        fb,
        icons::for_app(win.title_str()),
        APP_ICON,
        icon_x,
        icon_y,
    );
    draw_running_pill(fb, item, view);
    if item.labelled {
        draw_task_title(fb, th, item, view, win.title_str());
    }
}

fn draw_task_title(fb: &FramebufferInfo, th: &Theme, item: &Item, view: WinView, title: &str) {
    let r = item.rect;
    let left = r.x + ICON_PAD + APP_ICON + LABEL_GAP;
    let max_w = (r.x + r.w).saturating_sub(left + ICON_PAD);
    let color = if view.minimized {
        theme::blend_colors(th.taskbar_text, th.taskbar_bg, MINIMIZED_ALPHA)
    } else {
        th.taskbar_text
    };
    let style = theme::chrome_text(Face::Ui, TITLE_PX, color, TITLE_LINE_H);
    let top = theme::centered_line_y(r.y, r.h, &style);
    text::draw_fitted(fb, left, top, max_w, title, &style);
}

fn draw_overflow(fb: &FramebufferInfo, th: &Theme, item: &Item, state: Interaction) {
    paint_button_bg(fb, th, item, state, false);
    let label = alloc::format!("+{}", item.hidden);
    let style = theme::chrome_text(Face::UiBold, TITLE_PX, th.taskbar_text, TITLE_LINE_H);
    let top = theme::centered_line_y(item.rect.y, item.rect.h, &style);
    text::draw_centered(fb, item.rect.x + item.rect.w / 2, top, &label, &style);
}

fn draw_workspace(fb: &FramebufferInfo, th: &Theme, item: &Item, index: usize) {
    let r = item.rect;
    let (cx, cy) = (r.x + r.w / 2, r.y + r.h / 2);
    if index == workspace::active() {
        fill_circle(fb, cx, cy, DOT_RADIUS, theme::accent_color());
        return;
    }
    let idle = theme::blend_colors(th.taskbar_text, th.taskbar_bg, IDLE_DOT_ALPHA);
    fill_circle(fb, cx, cy, DOT_RADIUS - 1, idle);
}

/// Network state for the tray glyph: wired and addressed, else offline.
fn network_glyph() -> Glyph {
    #[cfg(target_arch = "aarch64")]
    let addressed = crate::net::config().ip_addr != [0, 0, 0, 0];
    #[cfg(not(target_arch = "aarch64"))]
    let addressed = true;
    if addressed && tray_popups::network_enabled() {
        Glyph::Ethernet
    } else {
        Glyph::WifiOff
    }
}

fn volume_glyph() -> Glyph {
    match tray_popups::volume_level() {
        0 => Glyph::VolumeMuted,
        1..=50 => Glyph::Volume,
        _ => Glyph::VolumeHigh,
    }
}

fn draw_tray_glyph(fb: &FramebufferInfo, th: &Theme, item: &Item, glyph: Glyph) {
    let (x, y) = center_in(item, TRAY_GLYPH);
    icons::draw_glyph(fb, glyph, TRAY_GLYPH, x, y, th.taskbar_text);
}

fn draw_glyph_button(fb: &FramebufferInfo, th: &Theme, item: &Item, state: Interaction) {
    let open = tray_popups::open_kind() == PopupKind::QuickSettings;
    paint_button_bg(fb, th, item, state, open);
    let glyph = match item.action {
        Action::Network => network_glyph(),
        _ => volume_glyph(),
    };
    draw_tray_glyph(fb, th, item, glyph);
}

/// Notifications posted since the bell was last opened.
fn unread_count() -> usize {
    let total = notifications::total_posted();
    if total < SEEN_NOTIFICATIONS.load(Ordering::Relaxed) {
        SEEN_NOTIFICATIONS.store(total, Ordering::Relaxed);
    }
    total - SEEN_NOTIFICATIONS.load(Ordering::Relaxed)
}

fn draw_bell(fb: &FramebufferInfo, th: &Theme, item: &Item, state: Interaction) {
    let open = tray_popups::open_kind() == PopupKind::Clock;
    paint_button_bg(fb, th, item, state, open);
    draw_tray_glyph(fb, th, item, Glyph::Bell);
    let badge = layout::badge_text(unread_count());
    if badge.is_empty() {
        return;
    }
    let (gx, gy) = center_in(item, TRAY_GLYPH);
    let (cx, cy) = (gx + TRAY_GLYPH, gy + BADGE_RADIUS / 2);
    fill_circle(fb, cx, cy, BADGE_RADIUS, BADGE_COLOR);
    let style = theme::chrome_text(Face::UiBold, BADGE_PX, 0xFFFF_FFFF, BADGE_LINE_H);
    let top = theme::centered_line_y(cy.saturating_sub(BADGE_RADIUS), 2 * BADGE_RADIUS, &style);
    text::draw_centered(fb, cx, top, &badge, &style);
}

/// Time and date strings; uptime stands in until a wall clock is set.
fn clock_lines(uptime_ms: u64) -> (String, String) {
    if let Some(now) = crate::clock::local_at(uptime_ms) {
        let time = layout::time_text(now.hour, now.minute, crate::clock::use_24_hour());
        let date = layout::date_text(now.weekday_name(), now.day, now.month_name());
        return (time, date);
    }
    let minutes = uptime_ms / MS_PER_MINUTE;
    let time = layout::time_text(
        (minutes / MINUTES_PER_HOUR % HOURS_PER_DAY) as u8,
        (minutes % MINUTES_PER_HOUR) as u8,
        true,
    );
    (time, String::from("uptime"))
}

fn draw_clock(fb: &FramebufferInfo, th: &Theme, item: &Item, state: Interaction) {
    let open = tray_popups::open_kind() == PopupKind::Calendar;
    paint_button_bg(fb, th, item, state, open);
    let (time, date) = clock_lines(crate::arch::ArchTimer::uptime_ms());
    let time_style = theme::chrome_text(Face::UiBold, TIME_PX, th.taskbar_text, TIME_LINE_H);
    let dim = theme::blend_colors(th.taskbar_text, th.taskbar_bg, MINIMIZED_ALPHA + 40);
    let date_style = theme::chrome_text(Face::Ui, DATE_PX, dim, DATE_LINE_H);
    let block = text::line_height(&time_style) + text::line_height(&date_style);
    let top = item.rect.y + item.rect.h.saturating_sub(block) / 2;
    let cx = item.rect.x + item.rect.w / 2;
    text::draw_centered(fb, cx, top, &time, &time_style);
    let date_top = top + text::line_height(&time_style);
    text::draw_centered(fb, cx, date_top, &date, &date_style);
}

fn draw_show_desktop(fb: &FramebufferInfo, th: &Theme, item: &Item, state: Interaction) {
    let r = item.rect;
    if state != Interaction::Idle {
        blend_rect(
            fb,
            r.x,
            r.y + 1,
            r.w,
            r.h - 1,
            th.taskbar_text,
            PRESSED_ALPHA,
        );
    }
    blend_rect(fb, r.x, r.y + 1, 1, r.h - 1, th.taskbar_text, HOVER_ALPHA);
}

// ─── Clicks ───────────────────────────────────────────────────────────

/// Handle a left press at (`px`, `py`). Returns true when the taskbar used
/// it; empty bar space falls through so outside-click handlers still run.
pub fn handle_press(px: u32, py: u32) -> bool {
    if !VISIBLE.load(Ordering::Relaxed) {
        return false;
    }
    let Some(fb) = crate::wm::compositor::fb_info() else {
        return false;
    };
    if py < bar_y(&fb) {
        return false;
    }
    let items = bar_items(&fb);
    let Some(item) = layout::item_at(&items, px, py) else {
        return false;
    };
    run_action(item);
    crate::wm::dirty::mark();
    true
}

fn run_action(item: &Item) {
    match item.action {
        Action::Start => {
            tray_popups::close();
            start_menu::toggle();
        }
        Action::Pinned(index) => {
            close_overlays();
            task_action(item.window, layout::PINNED_APPS.get(index).copied());
        }
        Action::Window(_) | Action::Overflow => {
            close_overlays();
            task_action(item.window, None);
        }
        Action::Workspace(index) => workspace::switch_to(index),
        Action::ShowDesktop => {
            close_overlays();
            toggle_show_desktop();
        }
        Action::Network | Action::Volume => toggle_popup(PopupKind::QuickSettings, item),
        Action::Bell => {
            SEEN_NOTIFICATIONS.store(notifications::total_posted(), Ordering::Relaxed);
            toggle_popup(PopupKind::Clock, item);
        }
        Action::Clock => toggle_popup(PopupKind::Calendar, item),
    }
}

fn close_overlays() {
    start_menu::close();
    tray_popups::close();
}

fn toggle_popup(kind: PopupKind, item: &Item) {
    start_menu::close();
    if tray_popups::open_kind() == kind {
        tray_popups::close();
        return;
    }
    tray_popups::open_anchored(kind, item.rect.x + item.rect.w / 2);
}

fn task_action(target: Option<WinView>, launch: Option<&str>) {
    match layout::task_click(target) {
        TaskAction::Launch => {
            if let Some(name) = launch {
                start_menu::launch_by_name(name);
            }
        }
        TaskAction::Restore(id) => window::focus(id),
        TaskAction::Minimize(id) => window::minimize(id),
    }
}

fn toggle_show_desktop() {
    let visible: Vec<u32> = snapshot()
        .iter()
        .filter(|v| !v.minimized)
        .map(|v| v.id)
        .collect();
    let outcome = SHOW_DESKTOP.lock().toggle(&visible, window::exists);
    match outcome {
        DesktopToggle::Minimize(ids) => ids.into_iter().for_each(window::minimize),
        DesktopToggle::Restore(ids) => ids.into_iter().for_each(window::restore_minimized),
    }
}

// ─── Visibility and diagnostics ───────────────────────────────────────

/// Toggle panel visibility.
pub fn toggle() {
    VISIBLE.fetch_xor(true, Ordering::Relaxed);
}

/// Check if the panel is visible.
pub fn is_visible() -> bool {
    VISIBLE.load(Ordering::Relaxed)
}

/// Set panel visibility.
pub fn set_visible(visible: bool) {
    VISIBLE.store(visible, Ordering::Relaxed);
}

/// Print every taskbar item with its screen rectangle (serial diagnostics).
pub fn describe() {
    let Some(fb) = crate::wm::compositor::fb_info() else {
        println!("taskbar: no framebuffer");
        return;
    };
    for item in bar_items(&fb) {
        let r = item.rect;
        println!(
            "taskbar: {:?} x={} y={} w={} h={} win={:?}",
            item.action,
            r.x,
            r.y,
            r.w,
            r.h,
            item.window.map(|v| (v.id, v.focused, v.minimized))
        );
    }
}

/// Toggle panel visibility from shell.
pub fn cmd_panel_toggle() {
    toggle();
    let state = if is_visible() { "shown" } else { "hidden" };
    println!("[panel] Panel {}", state);
}

/// Set panel visibility from shell.
pub fn cmd_panel_visible(visible: bool) {
    set_visible(visible);
    let state = if visible { "shown" } else { "hidden" };
    println!("[panel] Panel {}", state);
}
