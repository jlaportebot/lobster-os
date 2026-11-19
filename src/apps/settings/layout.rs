//! Geometry of the Settings window: sidebar, page title and scrolling
//! content viewport. Pure maths, so it is tested for every window size.

use crate::wm::controls::Rect;

/// Preferred window size.
pub const PREFERRED_W: u32 = 880;
pub const PREFERRED_H: u32 = 600;
/// Width of the page list on the left.
pub const SIDEBAR_W: u32 = 200;
/// Height of the page title strip above the content.
pub const TITLE_H: u32 = 56;
/// Width of the scrollbar column at the right edge.
pub const SCROLLBAR_W: u32 = 12;
/// Padding between the viewport edges and the page content.
pub const CONTENT_PAD: u32 = 24;
/// Pages stay at a readable line length however wide the window is.
pub const MAX_CONTENT_W: u32 = 640;

/// Where everything sits, in client-area coordinates.
#[derive(Clone, Copy, Debug)]
pub struct Layout {
    pub sidebar: Rect,
    pub title: Rect,
    /// Scrolling content area (clips page content).
    pub viewport: Rect,
    pub scrollbar: Rect,
    /// Left edge and width of the page content inside the viewport.
    pub content_x: i32,
    pub content_w: u32,
}

/// Lay out a client area of `client_w` x `client_h` px.
#[must_use]
pub fn compute(client_w: u32, client_h: u32) -> Layout {
    let sidebar_w = SIDEBAR_W.min(client_w / 3);
    let main_x = sidebar_w as i32;
    let main_w = client_w - sidebar_w;
    let viewport_h = client_h.saturating_sub(TITLE_H);
    let bar_w = SCROLLBAR_W.min(main_w);
    let viewport_w = main_w - bar_w;
    let content_w = viewport_w
        .saturating_sub(2 * CONTENT_PAD)
        .min(MAX_CONTENT_W);
    Layout {
        sidebar: Rect::new(0, 0, sidebar_w, client_h),
        title: Rect::new(main_x, 0, main_w, TITLE_H.min(client_h)),
        viewport: Rect::new(main_x, TITLE_H as i32, viewport_w, viewport_h),
        scrollbar: Rect::new(
            main_x + viewport_w as i32,
            TITLE_H as i32,
            bar_w,
            viewport_h,
        ),
        content_x: main_x + CONTENT_PAD as i32,
        content_w,
    }
}

/// Position and size of a newly opened window: the preferred size clamped
/// to the screen minus the taskbar, centred in what is left.
#[must_use]
pub fn initial_window(screen_w: u32, screen_h: u32, panel_h: u32) -> (u32, u32, u32, u32) {
    let usable_h = screen_h.saturating_sub(panel_h);
    let w = PREFERRED_W.min(screen_w);
    let h = PREFERRED_H.min(usable_h);
    ((screen_w - w) / 2, (usable_h - h) / 2, w, h)
}
#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    fn inside(inner: Rect, w: u32, h: u32) -> bool {
        inner.x >= 0 && inner.y >= 0 && inner.right() <= w as i32 && inner.bottom() <= h as i32
    }

    fn disjoint(a: Rect, b: Rect) -> bool {
        a.right() <= b.x || b.right() <= a.x || a.bottom() <= b.y || b.bottom() <= a.y
    }

    fn check(client_w: u32, client_h: u32) {
        let l = compute(client_w, client_h);
        for rect in [l.sidebar, l.title, l.viewport, l.scrollbar] {
            assert!(
                inside(rect, client_w, client_h),
                "{rect:?} escapes {client_w}x{client_h}"
            );
        }
        assert!(disjoint(l.sidebar, l.viewport));
        assert!(disjoint(l.sidebar, l.title));
        assert!(disjoint(l.title, l.viewport));
        assert!(disjoint(l.scrollbar, l.viewport));
    }

    #[test]
    fn layout_fits_at_the_preferred_size() {
        check(PREFERRED_W - 4, PREFERRED_H - 28);
    }

    #[test]
    fn layout_fits_at_800x600() {
        check(796, 548);
    }

    #[test]
    fn layout_fits_a_small_window() {
        check(520, 360);
    }

    #[test]
    fn sidebar_has_a_fixed_width_and_full_height() {
        let l = compute(876, 572);
        assert_eq!(l.sidebar.w, SIDEBAR_W);
        assert_eq!(l.sidebar.h, 572);
    }

    #[test]
    fn content_width_never_exceeds_the_readable_maximum() {
        assert_eq!(compute(2000, 900).content_w, MAX_CONTENT_W);
        assert!(compute(520, 360).content_w < MAX_CONTENT_W);
    }

    #[test]
    fn content_is_inset_from_the_viewport_edges() {
        let l = compute(876, 572);
        assert!(l.content_x >= l.viewport.x + CONTENT_PAD as i32);
        assert!(l.content_x + l.content_w as i32 <= l.scrollbar.x);
    }

    #[test]
    fn initial_window_is_centred_at_the_preferred_size() {
        let (x, y, w, h) = initial_window(1280, 720, 48);
        assert_eq!((w, h), (PREFERRED_W, PREFERRED_H));
        assert_eq!(x, (1280 - PREFERRED_W) / 2);
        assert_eq!(y, (720 - 48 - PREFERRED_H) / 2);
    }

    #[test]
    fn initial_window_is_clamped_on_an_800x600_screen() {
        let (x, y, w, h) = initial_window(800, 600, 48);
        assert!(w <= 800 && h <= 600 - 48);
        assert!(x + w <= 800 && y + h <= 600 - 48);
    }
}
