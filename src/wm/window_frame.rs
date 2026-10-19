//! Uniform window chrome + open/close animations
//!
//! Every app's render_to_windows path calls `draw_frame` first, then renders
//! its client area — giving the whole desktop one consistent themed window
//! frame (title bar, gradient accent, buttons, border) instead of per-app
//! ad-hoc chrome. `tick_animations` drives open/close/minimize effects.

use crate::display::fb::get_info;
use crate::display::graphics::{
    draw_rect, draw_rect_outline, draw_text, fill_rounded_rect,
};
use crate::wm::theme;

pub const TITLE_H: u32 = 24;
pub const BORDER_W: u32 = 2;
pub const CLOSE_BTN_W: u32 = 36;

/// Animation kinds
#[derive(Clone, Copy, PartialEq)]
pub enum Anim {
    /// Window appearing: scale up from 92% + fade (alpha stepped)
    Open,
    /// Window closing: scale down toward its taskbar button
    Close,
    /// Minimize: slide toward the taskbar
    Minimize,
}

struct AnimState {
    active: bool,
    kind: Anim,
    /// 0..=100 progress
    progress: u32,
    window_id: u32,
}

static mut ANIM: Option<AnimState> = None;

/// Duration of open/close/minimize animations in ms
const ANIM_MS: u64 = 160;

/// Begin a window animation (replaces any running one).
pub fn start(kind: Anim, window_id: u32) {
    unsafe {
        ANIM = Some(AnimState {
            active: true,
            kind,
            progress: 0,
            window_id,
        });
    }
}

pub fn animating() -> bool {
    unsafe { ANIM.as_ref().map(|a| a.active).unwrap_or(false) }
}

/// Advance the animation. Returns the window id when it just finished.
pub fn tick_animations() -> Option<(u32, Anim)> {
    unsafe {
        let anim = ANIM.as_mut()?;
        if !anim.active {
            return None;
        }
        let start = anim_start_ms();
        let elapsed = crate::arch::ArchTimer::uptime_ms().saturating_sub(start);
        if elapsed >= ANIM_MS {
            anim.active = false;
            let (id, kind) = (anim.window_id, anim.kind);
            ANIM = None;
            return Some((id, kind));
        }
        anim.progress = (elapsed * 100 / ANIM_MS) as u32;
        None
    }
}

// Animation start timestamp rides in the state via a companion static.
static mut ANIM_START_MS: u64 = 0;

fn anim_start_ms() -> u64 {
    unsafe { ANIM_START_MS }
}

pub fn mark_start() {
    unsafe {
        ANIM_START_MS = crate::arch::ArchTimer::uptime_ms();
    }
}

/// Draw the uniform frame for a window (themed chrome). Returns the client
/// area (x, y, w, h) the app should render into.
pub fn draw_frame(
    fb: &crate::display::fb::FramebufferInfo,
    win_id: u32,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    title: &str,
    focused: bool,
) -> (u32, u32, u32, u32) {
    let theme = theme::current();

    // Apply open/close scale animation
    let (mut sx, mut sy, mut sw, mut sh) = (x, y, w, h);
    if animating_for(win_id) {
        let (prog, kind) = anim_params();
        let pct = prog as u32;
        match kind {
            Anim::Open => {
                // Scale from 94% to 100%
                let s = 94 + (pct * 6) / 100;
                sw = w * s / 100;
                sh = h * s / 100;
                sx = x + (w - sw) / 2;
                sy = y + (h - sh) / 2;
            }
            Anim::Close | Anim::Minimize => {
                // Scale down toward 80%
                let s = 100 - (pct * 20) / 100;
                sw = w * s / 100;
                sh = h * s / 100;
                sx = x + (w - sw) / 2;
                sy = y + (h - sh) / 2;
            }
        }
    }

    // Shadow
    draw_rect(fb, sx + 4, sy + 6, sw, sh, 0x40000000);
    // Body
    fill_rounded_rect(fb, sx, sy, sw, sh, 8, theme.win_bg);
    // Border (focused = brighter)
    let border = if focused {
        theme.win_border
    } else {
        theme.win_border_unfocused
    };
    draw_rect_outline(fb, sx, sy, sw, sh, border);

    // Title bar
    let tb_color = if focused {
        theme.title_bar
    } else {
        theme.title_bar_unfocused
    };
    draw_rect(fb, sx + BORDER_W, sy + BORDER_W, sw.saturating_sub(2 * BORDER_W), TITLE_H, tb_color);

    // Title text
    let tt_color = if focused {
        theme.title_text
    } else {
        theme.title_text_unfocused
    };
    draw_text(fb, sx + 10, sy + BORDER_W + (TITLE_H - theme.font_size) / 2, title, tt_color, tb_color);

    // Close button (top-right)
    let btn_x = sx + sw.saturating_sub(CLOSE_BTN_W + BORDER_W + 4);
    let btn_y = sy + BORDER_W + 4;
    let btn_h = TITLE_H.saturating_sub(8);
    fill_rounded_rect(fb, btn_x, btn_y, CLOSE_BTN_W, btn_h, 6, theme.close_btn);
    draw_text(fb, btn_x + (CLOSE_BTN_W - 8) / 2, btn_y + (btn_h - theme.font_size) / 2, "x", 0xFFFFFFFF, theme.close_btn);

    // Client area
    let cx = sx + BORDER_W;
    let cy = sy + BORDER_W + TITLE_H;
    let cw = sw.saturating_sub(2 * BORDER_W);
    let ch = sh.saturating_sub(TITLE_H + 2 * BORDER_W);
    (cx, cy, cw, ch)
}

fn animating_for(win_id: u32) -> bool {
    unsafe {
        match ANIM.as_ref() {
            Some(a) => a.active && a.window_id == win_id,
            None => false,
        }
    }
}

fn anim_params() -> (u32, Anim) {
    unsafe {
        match ANIM.as_ref() {
            Some(a) => (a.progress, a.kind),
            None => (100, Anim::Open),
        }
    }
}
