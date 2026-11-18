//! Notification system — toasts + history
//!
//! Apps post notifications; toasts slide in at the top-right and fade after
//! a timeout, stacking downward. A history (log) keeps the last N for the
//! notification-center tray popup. Drawn above windows, below context menu.
//!
//! Public API:
//!   - notify(title, body, severity) — main entry point
//!   - info/success/warning/error — convenience wrappers
//!   - history_count() / history(i) — for notification center
//!   - clear_history() — clear all notifications
//!   - tick() — called from wm tick to advance animations/expire
//!   - draw(fb) — draw toasts

use crate::display::graphics::{draw_rect, draw_rect_outline, fill_rounded_rect};
use crate::display::text::{self, Face};
use crate::wm::theme;

/// Maximum live toasts
const MAX_TOASTS: usize = 4;
/// Maximum history entries
const MAX_HISTORY: usize = 24;
/// Toast display duration (ms)
const TOAST_MS: u64 = 4000;
/// Slide-in duration (ms)
const SLIDE_MS: u64 = 180;

/// Severity levels tint the accent bar
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Level {
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Clone, Copy)]
pub struct Notification {
    pub title: [u8; 48],
    pub title_len: usize,
    pub body: [u8; 96],
    pub body_len: usize,
    pub level: Level,
    /// uptime_ms when shown (0 = in history only)
    pub shown_at: u64,
}

#[derive(Clone, Copy)]
struct Toast {
    notif: Notification,
    /// 0..=100 animation progress (100 = fully in)
    progress: u32,
}

static mut TOASTS: [Option<Toast>; MAX_TOASTS] = [None; MAX_TOASTS];
static mut HISTORY: [Option<Notification>; MAX_HISTORY] = [None; MAX_HISTORY];
static mut HISTORY_COUNT: usize = 0;

fn now_ms() -> u64 {
    crate::arch::ArchTimer::uptime_ms()
}

fn set_str(dst: &mut [u8], len: &mut usize, s: &str) {
    let bytes = s.as_bytes();
    let n = bytes.len().min(dst.len());
    dst[..n].copy_from_slice(&bytes[..n]);
    *len = n;
}

impl Notification {
    pub fn title_str(&self) -> &str {
        core::str::from_utf8(&self.title[..self.title_len]).unwrap_or("")
    }
    pub fn body_str(&self) -> &str {
        core::str::from_utf8(&self.body[..self.body_len]).unwrap_or("")
    }
}

/// Post a notification. Shows a toast and records it in history.
pub fn post(title: &str, body: &str, level: Level) {
    unsafe {
        let mut notif = Notification {
            title: [0; 48],
            title_len: 0,
            body: [0; 96],
            body_len: 0,
            level,
            shown_at: now_ms(),
        };
        set_str(&mut notif.title, &mut notif.title_len, title);
        set_str(&mut notif.body, &mut notif.body_len, body);

        // Add to history (ring)
        HISTORY[HISTORY_COUNT % MAX_HISTORY] = Some(notif);
        HISTORY_COUNT += 1;

        // Find a free toast slot (or replace the oldest)
        let mut slot = None;
        for i in 0..MAX_TOASTS {
            if TOASTS[i].is_none() {
                slot = Some(i);
                break;
            }
        }
        if slot.is_none() {
            // Recycle slot 0 (simplest)
            slot = Some(0);
        }
        TOASTS[slot.unwrap()] = Some(Toast { notif, progress: 0 });
    }
}

/// Public API: notify(title, body, severity)
pub fn notify(title: &str, body: &str, severity: Level) {
    post(title, body, severity);
}

/// Convenience wrappers
pub fn info(title: &str, body: &str) {
    post(title, body, Level::Info);
}
pub fn success(title: &str, body: &str) {
    post(title, body, Level::Success);
}
pub fn warning(title: &str, body: &str) {
    post(title, body, Level::Warning);
}
pub fn error(title: &str, body: &str) {
    post(title, body, Level::Error);
}

/// Clear all notification history
pub fn clear_history() {
    unsafe {
        HISTORY = [None; MAX_HISTORY];
        HISTORY_COUNT = 0;
    }
}

/// Number of notifications in history
pub fn history_count() -> usize {
    unsafe { HISTORY_COUNT.min(MAX_HISTORY) }
}

/// Notifications posted since boot or the last history clear (unlike
/// [`history_count`] this keeps growing past the ring size, so callers can
/// compute an unread count).
pub fn total_posted() -> usize {
    // SAFETY: notification state is only touched from the single UI thread.
    unsafe { HISTORY_COUNT }
}

/// Get a history entry (most recent first)
pub fn history(i: usize) -> Option<&'static Notification> {
    unsafe {
        let total = HISTORY_COUNT;
        if total == 0 || i >= total.min(MAX_HISTORY) {
            return None;
        }
        // Most recent first: index back from the ring head
        let head = total % MAX_HISTORY;
        let idx = if head == 0 {
            MAX_HISTORY - 1 - i
        } else if i < head {
            head - 1 - i
        } else {
            MAX_HISTORY - (i - head + 1)
        };
        HISTORY[idx].as_ref()
    }
}

/// True while the toast stack is changing on screen: a toast is sliding in,
/// or the number of visible toasts differs from the last call.
pub fn needs_redraw() -> bool {
    static LAST_VISIBLE: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);
    // SAFETY: TOASTS is only touched from the single render thread.
    let toasts = unsafe { &*core::ptr::addr_of!(TOASTS) };
    let visible = toasts.iter().flatten().count();
    let is_sliding = toasts.iter().flatten().any(|toast| toast.progress < 100);
    let previous = LAST_VISIBLE.swap(visible, core::sync::atomic::Ordering::Relaxed);
    is_sliding || previous != visible
}

/// Advance toast animations + expire old toasts. Called from wm::tick.
pub fn tick() {
    let now = now_ms();
    unsafe {
        for t in TOASTS.iter_mut() {
            if let Some(toast) = t.as_mut() {
                // Animate in
                if toast.progress < 100 {
                    let elapsed = now.saturating_sub(toast.notif.shown_at);
                    if elapsed >= SLIDE_MS {
                        toast.progress = 100;
                    } else {
                        toast.progress = (elapsed * 100 / SLIDE_MS) as u32;
                    }
                }
                // Expire
                if now.saturating_sub(toast.notif.shown_at) >= TOAST_MS {
                    *t = None;
                }
            }
        }
    }
}

/// Dismiss the top-most toast (click on it)
pub fn dismiss_top() {
    unsafe {
        for t in TOASTS.iter_mut() {
            if t.is_some() {
                *t = None;
                break;
            }
        }
    }
}

// ─── Rendering ────────────────────────────────────────────────────────

const TOAST_W: u32 = 300;
const TOAST_H: u32 = 64;
/// Title and glyph size at the Medium font setting.
const TOAST_TITLE_PX: u32 = 14;
/// Body size at the Medium font setting.
const TOAST_BODY_PX: u32 = 13;
/// Tallest line box a toast text row may use.
const TOAST_LINE_H: u32 = 22;
/// Top of the title row inside a toast.
const TOAST_TITLE_Y: u32 = 9;
/// Top of the body row inside a toast.
const TOAST_BODY_Y: u32 = 33;
/// Left edge of the title text (after the level glyph).
const TOAST_TEXT_X: u32 = 36;
/// Right padding inside a toast.
const TOAST_PAD_RIGHT: u32 = 12;
const TOAST_MARGIN: u32 = 12;
const TOAST_GAP: u32 = 8;

fn level_color(level: Level) -> u32 {
    match level {
        Level::Info => 0xFF88C0D0,
        Level::Success => 0xFFA3BE8C,
        Level::Warning => 0xFFEBCB8B,
        Level::Error => 0xFFBF616A,
    }
}

fn level_glyph(level: Level) -> &'static str {
    match level {
        Level::Info => "i",
        Level::Success => "\u{2713}",
        Level::Warning => "!",
        Level::Error => "\u{2715}",
    }
}

/// Draw active toasts (called from wm::tick after windows, before panel)
pub fn draw(fb: &crate::display::fb::FramebufferInfo) {
    let theme = theme::current();
    let mut y = TOAST_MARGIN;

    unsafe {
        for t in TOASTS.iter() {
            let Some(toast) = t else { continue };

            // Slide-in from the right
            let slide = (100 - toast.progress) as u32;
            let x_offset = (slide * TOAST_W) / 100;
            let x = fb.width - TOAST_W - TOAST_MARGIN + x_offset;
            let ty = y;

            // Shadow
            draw_rect(fb, x + 4, ty + 6, TOAST_W, TOAST_H, 0x55000000);
            // Card
            fill_rounded_rect(fb, x, ty, TOAST_W, TOAST_H, 10, theme.menu_bg);
            draw_rect_outline(fb, x, ty, TOAST_W, TOAST_H, theme.menu_border);
            // Accent bar (left edge)
            fill_rounded_rect(fb, x, ty, 6, TOAST_H, 3, level_color(toast.notif.level));
            let accent = level_color(toast.notif.level);
            let glyph = theme::chrome_text(Face::UiBold, TOAST_TITLE_PX, accent, TOAST_LINE_H);
            text::draw(
                fb,
                x + 16,
                ty + TOAST_TITLE_Y,
                level_glyph(toast.notif.level),
                &glyph,
            );
            let title =
                theme::chrome_text(Face::UiBold, TOAST_TITLE_PX, theme.label_text, TOAST_LINE_H);
            let text_w = TOAST_W - TOAST_TEXT_X - TOAST_PAD_RIGHT;
            text::draw_fitted(
                fb,
                x + TOAST_TEXT_X,
                ty + TOAST_TITLE_Y,
                text_w,
                toast.notif.title_str(),
                &title,
            );
            let body =
                theme::chrome_text(Face::Ui, TOAST_BODY_PX, theme.label_text_dim, TOAST_LINE_H);
            let body_w = TOAST_W - 16 - TOAST_PAD_RIGHT;
            text::draw_fitted(
                fb,
                x + 16,
                ty + TOAST_BODY_Y,
                body_w,
                toast.notif.body_str(),
                &body,
            );

            y += TOAST_H + TOAST_GAP;
            if y + TOAST_H > fb.height {
                break;
            }
        }
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    #[test]
    fn notification_queue_fifo_expiry() {
        let _guard = crate::test_support::serial_guard();
        // Test that notifications are added to history in FIFO order
        clear_history();
        assert_eq!(history_count(), 0);

        // Post several notifications
        post("Title 1", "Body 1", Level::Info);
        post("Title 2", "Body 2", Level::Success);
        post("Title 3", "Body 3", Level::Warning);

        assert_eq!(history_count(), 3);

        // Most recent first
        let h0 = history(0).unwrap();
        assert_eq!(h0.title_str(), "Title 3");
        assert_eq!(h0.body_str(), "Body 3");

        let h1 = history(1).unwrap();
        assert_eq!(h1.title_str(), "Title 2");

        let h2 = history(2).unwrap();
        assert_eq!(h2.title_str(), "Title 1");

        // Test clear
        clear_history();
        assert_eq!(history_count(), 0);
    }

    #[test]
    fn toast_geometry() {
        // Test toast positioning constants
        assert_eq!(TOAST_W, 300);
        assert_eq!(TOAST_H, 64);
        assert_eq!(MAX_TOASTS, 4);
    }

    #[test]
    fn notify_public_api() {
        let _guard = crate::test_support::serial_guard();
        clear_history();
        notify("Test Title", "Test Body", Level::Error);
        assert_eq!(history_count(), 1);
        let h = history(0).unwrap();
        assert_eq!(h.title_str(), "Test Title");
        assert_eq!(h.body_str(), "Test Body");
        assert_eq!(h.level, Level::Error);
    }
}
