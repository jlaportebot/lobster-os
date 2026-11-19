//! Pointer capture for sliders and scrollbars.
//!
//! The hit-region router only delivers presses, so a control that needs to
//! follow the pointer while the button is held starts a capture here. The
//! input layer feeds pointer motion to [`on_pointer_move`] and the release
//! to [`end`].

use crate::wm::controls::scroll;
use crate::wm::controls::slider;
use core::sync::atomic::{AtomicU32, Ordering};

/// Geometry of a draggable track, in screen coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Track {
    /// Slider: thumb centre travels along `x..x+w`.
    Horizontal { x: i32, w: u32, min: u32, max: u32 },
    /// Scrollbar: the value is the scroll offset of `content_h` in `view_h`.
    Vertical {
        y: i32,
        h: u32,
        view_h: u32,
        content_h: u32,
    },
}

impl Track {
    /// Value the pointer at (`px`, `py`) selects; `grab` is how far below
    /// the thumb's top edge a vertical drag started.
    #[must_use]
    pub fn value_at(&self, px: i32, py: i32, grab: i32) -> u32 {
        match *self {
            Track::Horizontal { x, w, min, max } => slider::value_at(px, x, w, min, max),
            Track::Vertical {
                y,
                h,
                view_h,
                content_h,
            } => scroll::scroll_for_thumb(py - y - grab, h, view_h, content_h) as u32,
        }
    }
}

/// Called with `(key, value)` whenever a captured drag changes the value.
pub type OnChange = fn(u32, u32);

#[derive(Clone, Copy)]
struct Capture {
    key: u32,
    track: Track,
    grab: i32,
    on_change: OnChange,
}

static mut ACTIVE: Option<Capture> = None;
/// Window whose hover state the pointer hook should watch (0 = none).
static HOVER_WIN: AtomicU32 = AtomicU32::new(0);
/// Hit-region key last seen under the pointer in that window.
static HOVER_KEY: AtomicU32 = AtomicU32::new(0);

/// Start following the pointer for `key` along `track`.
pub fn begin(key: u32, track: Track, grab: i32, on_change: OnChange) {
    // SAFETY: input and rendering run on the single UI thread.
    unsafe {
        ACTIVE = Some(Capture {
            key,
            track,
            grab,
            on_change,
        });
    }
}

/// Whether a drag is in progress.
#[must_use]
pub fn is_active() -> bool {
    // SAFETY: single UI thread.
    unsafe { ACTIVE.is_some() }
}

/// Stop following the pointer (button released).
pub fn end() {
    // SAFETY: single UI thread.
    unsafe {
        ACTIVE = None;
    }
}

/// Ask the pointer hook to redraw `win_id` when the control under the
/// pointer changes, so hover looks update without a click.
pub fn watch_hover(win_id: u32) {
    HOVER_WIN.store(win_id, Ordering::Relaxed);
}

/// Pointer moved to screen position (`x`, `y`). Returns true when a drag
/// consumed the motion.
pub fn on_pointer_move(x: u32, y: u32) -> bool {
    // SAFETY: single UI thread.
    if let Some(capture) = unsafe { ACTIVE } {
        let value = capture.track.value_at(x as i32, y as i32, capture.grab);
        (capture.on_change)(capture.key, value);
        crate::wm::dirty::mark();
        return true;
    }
    refresh_on_hover_change(x, y);
    false
}

fn refresh_on_hover_change(x: u32, y: u32) {
    if HOVER_WIN.load(Ordering::Relaxed) == 0 {
        return;
    }
    let key = crate::wm::hit_region::route_hit(x, y).map_or(0, |hit| {
        if hit.win_id == HOVER_WIN.load(Ordering::Relaxed) {
            hit.region.tag
        } else {
            0
        }
    });
    if HOVER_KEY.swap(key, Ordering::Relaxed) != key {
        crate::wm::dirty::mark();
    }
}
