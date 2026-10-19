//! Global UI dirty tracking
//!
//! Static desktop content (window bodies, file lists, editors, …) is expensive
//! to repaint — a full pass costs ~100ms under TCG. Instead of redrawing
//! everything every frame, apps render only when something actually changed:
//! any HID event, window operation, or explicit app state change sets the
//! global dirty flag; the render pass consumes it. Heartbeat re-renders
//! (1/second) act as a correctness net.

use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

static DIRTY: AtomicBool = AtomicBool::new(true);
static FRAME: AtomicU32 = AtomicU32::new(0);

/// Mark the UI dirty — the next tick re-renders static content.
pub fn mark() {
    DIRTY.store(true, Ordering::Release);
}

/// Whether static apps should render this tick (dirty or heartbeat).
pub fn should_render_static() -> bool {
    let frame = FRAME.fetch_add(1, Ordering::Relaxed);
    if frame % 30 == 0 {
        return true; // 1s heartbeat correctness net
    }
    DIRTY.swap(false, Ordering::AcqRel)
}

/// Frame counter for per-app cadence (e.g. sysmon at 2Hz).
pub fn frame() -> u32 {
    FRAME.load(Ordering::Relaxed)
}
