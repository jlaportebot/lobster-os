//! Notification system — toasts + history
//!
//! Apps post notifications; toasts slide in at the top-right and fade after
//! a timeout, stacking downward. A history (log) keeps the last N for the
//! notification-center tray popup. Drawn above windows, below context menu.

use crate::display::fb::get_info;
use crate::display::graphics::{draw_rect, draw_rect_outline, draw_text, fill_rounded_rect};
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
#[derive(Clone, Copy, PartialEq)]
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

/// Number of notifications in history
pub fn history_count() -> usize {
    unsafe { HISTORY_COUNT.min(MAX_HISTORY) }
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
        Level::Error => "\u{2717}",
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
            // Level glyph
            draw_text(
                fb,
                x + 16,
                ty + 10,
                level_glyph(toast.notif.level),
                level_color(toast.notif.level),
                theme.menu_bg,
            );
            // Title
            draw_text(
                fb,
                x + 34,
                ty + 10,
                toast.notif.title_str(),
                theme.label_text,
                theme.menu_bg,
            );
            // Body (single line, clipped by construction: 96 bytes max)
            draw_text(
                fb,
                x + 16,
                ty + 34,
                toast.notif.body_str(),
                theme.label_text_dim,
                theme.menu_bg,
            );

            y += TOAST_H + TOAST_GAP;
            if y + TOAST_H > fb.height {
                break;
            }
        }
    }
}
