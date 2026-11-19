//! Vertical scrolling maths shared by every scrollable view.

/// Shortest scrollbar thumb, so it stays grabbable for very long content.
pub const MIN_THUMB: u32 = 24;

/// Largest valid scroll offset for `content_h` shown in `view_h` px.
#[must_use]
pub fn max_scroll(content_h: u32, view_h: u32) -> i32 {
    content_h.saturating_sub(view_h) as i32
}

/// `scroll` limited to the valid range.
#[must_use]
pub fn clamp_scroll(scroll: i32, content_h: u32, view_h: u32) -> i32 {
    scroll.clamp(0, max_scroll(content_h, view_h))
}

/// Thumb `(offset, length)` inside a track of `track_h` px.
#[must_use]
pub fn thumb(track_h: u32, view_h: u32, content_h: u32, scroll: i32) -> (u32, u32) {
    if content_h <= view_h || content_h == 0 {
        return (0, track_h);
    }
    let len = (u64::from(track_h) * u64::from(view_h) / u64::from(content_h)) as u32;
    let len = len.max(MIN_THUMB).min(track_h);
    let travel = u64::from(track_h - len);
    let range = max_scroll(content_h, view_h) as u64;
    let scroll = clamp_scroll(scroll, content_h, view_h) as u64;
    let offset = (travel * scroll + range / 2) / range;
    (offset as u32, len)
}

/// Scroll offset that puts the thumb's top edge at `thumb_top` px inside the
/// track (used while dragging the thumb).
#[must_use]
pub fn scroll_for_thumb(thumb_top: i32, track_h: u32, view_h: u32, content_h: u32) -> i32 {
    let (_, len) = thumb(track_h, view_h, content_h, 0);
    let travel = i64::from(track_h.saturating_sub(len));
    if travel == 0 {
        return 0;
    }
    let top = i64::from(thumb_top).clamp(0, travel);
    let range = i64::from(max_scroll(content_h, view_h));
    ((top * range + travel / 2) / travel) as i32
}
#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    #[test]
    fn max_scroll_is_zero_when_content_fits() {
        assert_eq!(max_scroll(300, 400), 0);
        assert_eq!(max_scroll(500, 400), 100);
    }

    #[test]
    fn clamp_never_exceeds_the_range() {
        assert_eq!(clamp_scroll(-20, 500, 400), 0);
        assert_eq!(clamp_scroll(250, 500, 400), 100);
        assert_eq!(clamp_scroll(40, 500, 400), 40);
    }

    #[test]
    fn thumb_fills_the_track_when_nothing_scrolls() {
        assert_eq!(thumb(400, 400, 300, 0), (0, 400));
    }

    #[test]
    fn thumb_length_is_proportional_with_a_floor() {
        let (_, len) = thumb(400, 400, 800, 0);
        assert_eq!(len, 200);
        let (_, tiny) = thumb(400, 400, 400_000, 0);
        assert_eq!(tiny, MIN_THUMB);
    }

    #[test]
    fn thumb_reaches_the_bottom_at_max_scroll() {
        let (offset, len) = thumb(400, 400, 800, 400);
        assert_eq!(offset + len, 400);
    }

    #[test]
    fn scroll_for_thumb_inverts_thumb_offset() {
        let (offset, _) = thumb(400, 400, 1000, 300);
        let back = scroll_for_thumb(offset as i32, 400, 400, 1000);
        assert!((back - 300).abs() <= 3, "got {back}");
    }

    #[test]
    fn scroll_for_thumb_clamps_outside_the_track() {
        assert_eq!(scroll_for_thumb(-40, 400, 400, 1000), 0);
        assert_eq!(scroll_for_thumb(9999, 400, 400, 1000), 600);
    }
}
