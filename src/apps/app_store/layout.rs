//! Geometry of the App Store window. Pure maths, tested at the preferred
//! 960x640 window and the 800x600 minimum.

use crate::wm::controls::Rect;

/// Preferred window size (the frame included).
pub const PREFERRED_W: u32 = 960;
pub const PREFERRED_H: u32 = 640;
/// Width of the section list on the left.
pub const SIDEBAR_W: u32 = 200;
/// Height of the title / search strip.
pub const HEADER_H: u32 = 60;
/// Height of the status line under the header.
pub const STATUS_H: u32 = 28;
/// Height of the progress bar shown while a job runs.
pub const JOB_BAR_H: u32 = 64;
/// Height of one catalogue row.
pub const ROW_H: u32 = 68;
/// Width of the scrollbar column.
pub const SCROLLBAR_W: u32 = 12;
/// Width of the details pane beside the list.
pub const DETAILS_W: u32 = 320;
/// Narrower content areas show the details instead of the list.
pub const SPLIT_MIN_W: u32 = 700;
/// Space around the header controls.
pub const PAD: i32 = 20;
/// Width of the search field and of the refresh button.
pub const SEARCH_W: u32 = 230;
pub const REFRESH_W: u32 = 104;
/// Control height in the header.
pub const CONTROL_H: u32 = 34;
/// Width of a row's state button.
pub const ROW_BUTTON_W: u32 = 92;
/// Height of a row's state button.
pub const ROW_BUTTON_H: u32 = 32;

/// Where everything sits, in client-area coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub sidebar: Rect,
    pub title: Rect,
    pub search: Rect,
    pub refresh: Rect,
    pub status: Rect,
    /// Scrolling list viewport (empty when the details replace the list).
    pub list: Rect,
    pub scrollbar: Rect,
    pub details: Option<Rect>,
    pub job_bar: Option<Rect>,
}

/// Lay out a client area. `show_details` opens the details pane,
/// `job_bar` reserves the progress strip.
#[must_use]
pub fn compute(client_w: u32, client_h: u32, show_details: bool, job_bar: bool) -> Layout {
    let sidebar_w = SIDEBAR_W.min(client_w / 3);
    let main_x = sidebar_w as i32;
    let main_w = client_w - sidebar_w;
    let bar_h = if job_bar { JOB_BAR_H.min(client_h) } else { 0 };
    let body_y = (HEADER_H + STATUS_H).min(client_h);
    let body_h = client_h.saturating_sub(body_y + bar_h);
    let split = main_w >= SPLIT_MIN_W;
    let pane_w = if show_details && split { DETAILS_W } else { 0 };
    let covers = show_details && !split;
    let list_w = if covers { 0 } else { main_w - pane_w };
    let bar_w = if list_w == 0 { 0 } else { SCROLLBAR_W };
    let header_y = (HEADER_H.saturating_sub(CONTROL_H) / 2) as i32;
    let refresh_x = client_w as i32 - PAD - REFRESH_W as i32;
    Layout {
        sidebar: Rect::new(0, 0, sidebar_w, client_h),
        title: Rect::new(main_x + PAD, 0, main_w / 3, HEADER_H),
        search: Rect::new(
            refresh_x - 12 - SEARCH_W as i32,
            header_y,
            SEARCH_W,
            CONTROL_H,
        ),
        refresh: Rect::new(refresh_x, header_y, REFRESH_W, CONTROL_H),
        status: Rect::new(main_x, HEADER_H as i32, main_w, STATUS_H),
        list: Rect::new(main_x, body_y as i32, list_w - bar_w.min(list_w), body_h),
        scrollbar: Rect::new(
            main_x + (list_w - bar_w) as i32,
            body_y as i32,
            bar_w,
            body_h,
        ),
        details: (show_details).then(|| {
            let x = if covers {
                main_x
            } else {
                client_w as i32 - pane_w as i32
            };
            let w = if covers { main_w } else { pane_w };
            Rect::new(x, body_y as i32, w, body_h)
        }),
        job_bar: (job_bar).then(|| Rect::new(main_x, (client_h - bar_h) as i32, main_w, bar_h)),
    }
}

/// Row `index` of the list at scroll offset `scroll`, in list-viewport
/// coordinates added to the viewport origin.
#[must_use]
pub fn row_rect(list: Rect, index: usize, scroll: i32) -> Rect {
    Rect::new(
        list.x,
        list.y + (index as u32 * ROW_H) as i32 - scroll,
        list.w,
        ROW_H,
    )
}

/// Height of `count` rows.
#[must_use]
pub fn content_height(count: usize) -> u32 {
    count as u32 * ROW_H
}

/// The state button inside a row.
#[must_use]
pub fn row_button(row: Rect) -> Rect {
    let y = row.y + (row.h.saturating_sub(ROW_BUTTON_H) / 2) as i32;
    Rect::new(
        row.right() - PAD - ROW_BUTTON_W as i32,
        y,
        ROW_BUTTON_W,
        ROW_BUTTON_H,
    )
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

    fn inside(r: Rect, w: u32, h: u32) -> bool {
        r.x >= 0 && r.y >= 0 && r.right() <= w as i32 && r.bottom() <= h as i32
    }

    fn disjoint(a: Rect, b: Rect) -> bool {
        a.w == 0
            || b.w == 0
            || a.right() <= b.x
            || b.right() <= a.x
            || a.bottom() <= b.y
            || b.bottom() <= a.y
    }

    fn rects(l: &Layout) -> alloc::vec::Vec<Rect> {
        let mut v = alloc::vec![
            l.sidebar,
            l.title,
            l.search,
            l.refresh,
            l.status,
            l.list,
            l.scrollbar
        ];
        v.extend(l.details);
        v.extend(l.job_bar);
        v
    }

    fn check(w: u32, h: u32, details: bool, job: bool) {
        let l = compute(w, h, details, job);
        for r in rects(&l) {
            assert!(
                inside(r, w, h),
                "{r:?} escapes {w}x{h} (details={details}, job={job})"
            );
        }
        assert!(disjoint(l.sidebar, l.list));
        assert!(disjoint(l.list, l.scrollbar));
        assert!(disjoint(l.search, l.refresh));
        assert!(disjoint(l.title, l.search), "title overlaps search at {w}");
        if let Some(d) = l.details {
            assert!(disjoint(d, l.list) && disjoint(d, l.scrollbar));
            assert!(disjoint(d, l.sidebar));
        }
        if let Some(bar) = l.job_bar {
            assert!(disjoint(bar, l.list) && disjoint(bar, l.details.unwrap_or(l.list)));
        }
    }

    #[test]
    fn layout_fits_at_preferred_and_minimum_sizes() {
        for (w, h) in [(960, 608), (950, 600), (800, 568), (790, 560)] {
            for details in [false, true] {
                for job in [false, true] {
                    check(w, h, details, job);
                }
            }
        }
    }

    #[test]
    fn details_sit_beside_the_list_when_wide_and_cover_it_when_narrow() {
        let wide = compute(950, 608, true, false);
        assert_eq!(wide.details.map(|d| d.w), Some(DETAILS_W));
        assert!(wide.list.w > 300);
        let narrow = compute(790, 560, true, false);
        assert_eq!(narrow.list.w, 0);
        assert_eq!(narrow.details.map(|d| d.w), Some(790 - SIDEBAR_W));
    }

    #[test]
    fn row_geometry_scrolls_and_keeps_the_button_inside() {
        let l = compute(960, 608, false, false);
        let row = row_rect(l.list, 3, 50);
        assert_eq!(row.y, l.list.y + 3 * ROW_H as i32 - 50);
        let b = row_button(row);
        assert!(b.x >= row.x && b.right() <= row.right());
        assert!(b.y >= row.y && b.bottom() <= row.bottom());
        assert_eq!(content_height(5), 5 * ROW_H);
    }

    #[test]
    fn window_is_clamped_to_the_screen() {
        assert_eq!(initial_window(1280, 720, 48), (160, 16, 960, 640));
        let (x, y, w, h) = initial_window(800, 600, 48);
        assert_eq!((x, w), (0, 800));
        assert!(y + h <= 600 - 48);
    }
}
