//! Start-menu geometry. Pure functions of the screen size so the menu can
//! be proven never to touch the taskbar on the host.

/// Gap between the menu and the screen edges and the taskbar.
pub const EDGE_MARGIN: u32 = 8;
/// Preferred menu width.
pub const MENU_WIDTH: u32 = 440;
/// Tallest the menu gets on large screens.
pub const MENU_MAX_HEIGHT: u32 = 580;
/// Inner padding of the menu surface.
pub const PADDING: u32 = 16;
/// Corner radius of the menu surface.
pub const MENU_RADIUS: u32 = 12;
/// Height of the search field.
pub const SEARCH_HEIGHT: u32 = 36;
/// Height of the section title row ("Pinned" / "All apps").
pub const SECTION_HEIGHT: u32 = 28;
/// Vertical gap between stacked blocks.
pub const BLOCK_GAP: u32 = 10;
/// Height of the user / power bar at the bottom.
pub const BAR_HEIGHT: u32 = 56;
/// Pinned grid columns.
pub const GRID_COLUMNS: u32 = 5;
/// Height of one pinned tile cell (icon, gap, name).
pub const GRID_CELL_HEIGHT: u32 = 80;
/// Pinned tile icon size (an icon-set size).
pub const GRID_ICON: u32 = 32;
/// Height of one All-apps / result row.
pub const LIST_ROW_HEIGHT: u32 = 40;
/// All-apps row icon size (an icon-set size).
pub const LIST_ICON: u32 = 24;
/// Width of the scrollbar column in list modes.
pub const SCROLLBAR_WIDTH: u32 = 8;
/// Smallest scrollbar thumb.
pub const MIN_THUMB: u32 = 24;
/// Avatar circle diameter.
pub const AVATAR_SIZE: u32 = 32;
/// Square power button edge.
pub const POWER_BUTTON: u32 = 36;
/// Width of the power popup.
pub const POWER_MENU_WIDTH: u32 = 168;
/// Height of one power popup entry.
pub const POWER_ROW_HEIGHT: u32 = 36;
/// Padding around the power popup entries.
pub const POWER_MENU_PAD: u32 = 4;
/// Width of the "All apps" / "Back" button in the section row.
pub const SECTION_BUTTON_WIDTH: u32 = 96;

/// Axis-aligned rectangle in screen pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

impl Rect {
    pub const fn new(x: u32, y: u32, w: u32, h: u32) -> Self {
        Self { x, y, w, h }
    }

    /// Whether the point lies inside the rectangle.
    pub fn contains(&self, px: u32, py: u32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }

    /// Whether the two rectangles share at least one pixel.
    pub fn intersects(&self, other: &Rect) -> bool {
        self.x < other.x + other.w
            && other.x < self.x + self.w
            && self.y < other.y + other.h
            && other.y < self.y + self.h
    }
}

/// Every fixed rectangle of the menu for one screen size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub menu: Rect,
    pub search: Rect,
    pub section_title: Rect,
    pub section_button: Rect,
    pub content: Rect,
    pub bar: Rect,
    pub power_button: Rect,
    pub power_menu: Rect,
}

/// Compute the layout above a taskbar of `panel_h` pixels.
pub fn compute(screen_w: u32, screen_h: u32, panel_h: u32) -> Layout {
    let w = MENU_WIDTH.min(screen_w.saturating_sub(2 * EDGE_MARGIN));
    let room = screen_h.saturating_sub(panel_h + 2 * EDGE_MARGIN);
    let h = MENU_MAX_HEIGHT.min(room);
    let menu = Rect::new(
        EDGE_MARGIN,
        screen_h.saturating_sub(panel_h + EDGE_MARGIN + h),
        w,
        h,
    );
    let inner_x = menu.x + PADDING;
    let inner_w = w.saturating_sub(2 * PADDING);
    let search = Rect::new(inner_x, menu.y + PADDING, inner_w, SEARCH_HEIGHT);
    let section_y = search.y + SEARCH_HEIGHT + BLOCK_GAP;
    let bar = Rect::new(menu.x, menu.y + h.saturating_sub(BAR_HEIGHT), w, BAR_HEIGHT);
    let content_y = section_y + SECTION_HEIGHT + BLOCK_GAP / 2;
    let content_h = bar
        .y
        .saturating_sub(BLOCK_GAP / 2)
        .saturating_sub(content_y);
    let button_x = inner_x + inner_w.saturating_sub(SECTION_BUTTON_WIDTH);
    let power_button = Rect::new(
        (menu.x + w).saturating_sub(PADDING + POWER_BUTTON),
        bar.y + (BAR_HEIGHT - POWER_BUTTON) / 2,
        POWER_BUTTON,
        POWER_BUTTON,
    );
    let power_h = POWER_ROW_HEIGHT * 4 + 2 * POWER_MENU_PAD;
    let power_menu = Rect::new(
        (power_button.x + POWER_BUTTON).saturating_sub(POWER_MENU_WIDTH),
        bar.y.saturating_sub(power_h + POWER_MENU_PAD),
        POWER_MENU_WIDTH,
        power_h,
    );
    Layout {
        menu,
        search,
        section_title: Rect::new(inner_x, section_y, inner_w, SECTION_HEIGHT),
        section_button: Rect::new(button_x, section_y, SECTION_BUTTON_WIDTH, SECTION_HEIGHT),
        content: Rect::new(inner_x, content_y, inner_w, content_h),
        bar,
        power_button,
        power_menu,
    }
}

impl Layout {
    /// Pinned tiles that fit: whole rows only, at least one row.
    pub fn grid_capacity(&self) -> usize {
        let rows = (self.content.h / GRID_CELL_HEIGHT).max(1);
        (rows * GRID_COLUMNS) as usize
    }

    /// Cell of the `index`-th pinned tile.
    pub fn grid_cell(&self, index: usize) -> Rect {
        let cell_w = self.content.w / GRID_COLUMNS;
        let col = index as u32 % GRID_COLUMNS;
        let row = index as u32 / GRID_COLUMNS;
        Rect::new(
            self.content.x + col * cell_w,
            self.content.y + row * GRID_CELL_HEIGHT,
            cell_w,
            GRID_CELL_HEIGHT,
        )
    }

    /// Rows of a list that fit the content area.
    pub fn list_page(&self) -> usize {
        ((self.content.h / LIST_ROW_HEIGHT) as usize).max(1)
    }

    /// Whether `count` rows need a scrollbar.
    pub fn needs_scrollbar(&self, count: usize) -> bool {
        count > self.list_page()
    }

    /// Row rectangle for the `slot`-th visible row (0 = first on screen).
    /// Leaves room for the scrollbar column when `scrollbar` is set.
    pub fn list_row(&self, slot: usize, scrollbar: bool) -> Rect {
        let gutter = if scrollbar { SCROLLBAR_WIDTH + 4 } else { 0 };
        Rect::new(
            self.content.x,
            self.content.y + slot as u32 * LIST_ROW_HEIGHT,
            self.content.w - gutter,
            LIST_ROW_HEIGHT,
        )
    }

    /// Scrollbar track on the right edge of the content area.
    pub fn scroll_track(&self) -> Rect {
        Rect::new(
            self.content.x + self.content.w - SCROLLBAR_WIDTH,
            self.content.y,
            SCROLLBAR_WIDTH,
            self.list_page() as u32 * LIST_ROW_HEIGHT,
        )
    }

    /// Thumb for a list of `count` rows scrolled to `scroll`.
    pub fn scroll_thumb(&self, count: usize, scroll: usize) -> Rect {
        let track = self.scroll_track();
        let page = self.list_page();
        let max_scroll = count.saturating_sub(page).max(1) as u32;
        let thumb_h = (track.h * page as u32 / count.max(1) as u32).clamp(MIN_THUMB, track.h);
        let travel = track.h - thumb_h;
        let offset = travel * (scroll as u32).min(max_scroll) / max_scroll;
        Rect::new(track.x, track.y + offset, track.w, thumb_h)
    }

    /// Scroll position for a thumb whose top edge sits at `thumb_top`.
    pub fn scroll_for_thumb(&self, count: usize, thumb_top: u32) -> usize {
        let track = self.scroll_track();
        let thumb = self.scroll_thumb(count, 0);
        let travel = track.h.saturating_sub(thumb.h).max(1);
        let offset = thumb_top.saturating_sub(track.y).min(travel);
        let max_scroll = count.saturating_sub(self.list_page()) as u32;
        ((offset * max_scroll + travel / 2) / travel) as usize
    }

    /// Power popup entry rectangle.
    pub fn power_row(&self, index: usize) -> Rect {
        Rect::new(
            self.power_menu.x + POWER_MENU_PAD,
            self.power_menu.y + POWER_MENU_PAD + index as u32 * POWER_ROW_HEIGHT,
            POWER_MENU_WIDTH - 2 * POWER_MENU_PAD,
            POWER_ROW_HEIGHT,
        )
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    const PANEL_HEIGHT: u32 = 36;
    const SCREENS: [(u32, u32); 4] = [(1920, 1080), (1280, 720), (1024, 768), (800, 600)];

    fn panel_rect(w: u32, h: u32) -> Rect {
        Rect::new(0, h - PANEL_HEIGHT, w, PANEL_HEIGHT)
    }

    #[test]
    fn menu_never_touches_the_panel_or_leaves_the_screen() {
        for (w, h) in SCREENS {
            let layout = compute(w, h, PANEL_HEIGHT);
            assert!(!layout.menu.intersects(&panel_rect(w, h)), "{w}x{h}");
            assert!(layout.menu.x + layout.menu.w <= w, "{w}x{h}");
            assert_eq!(
                layout.menu.y + layout.menu.h + EDGE_MARGIN,
                h - PANEL_HEIGHT
            );
        }
    }

    #[test]
    fn popups_and_content_stay_inside_the_menu() {
        for (w, h) in SCREENS {
            let l = compute(w, h, PANEL_HEIGHT);
            for r in [l.search, l.content, l.bar, l.power_button, l.power_menu] {
                assert!(
                    r.x >= l.menu.x && r.x + r.w <= l.menu.x + l.menu.w,
                    "{w}x{h}"
                );
                assert!(
                    r.y >= l.menu.y && r.y + r.h <= l.menu.y + l.menu.h,
                    "{w}x{h}"
                );
            }
            assert!(!l.content.intersects(&l.bar));
            assert!(!l.power_menu.intersects(&l.power_button));
        }
    }

    #[test]
    fn tiny_screens_do_not_underflow() {
        let l = compute(100, 50, PANEL_HEIGHT);
        assert!(l.menu.h <= 50);
        assert!(!l.menu.intersects(&panel_rect(100, 50)));
    }

    #[test]
    fn grid_cells_tile_the_content_without_overlap() {
        let l = compute(1280, 720, PANEL_HEIGHT);
        let a = l.grid_cell(0);
        let b = l.grid_cell(1);
        let below = l.grid_cell(GRID_COLUMNS as usize);
        assert_eq!((a.x, a.y), (l.content.x, l.content.y));
        assert_eq!(b.x, a.x + a.w);
        assert_eq!(below.y, a.y + GRID_CELL_HEIGHT);
        assert!(!a.intersects(&b) && !a.intersects(&below));
    }

    #[test]
    fn grid_capacity_fits_whole_rows_in_the_content() {
        for (w, h) in SCREENS {
            let l = compute(w, h, PANEL_HEIGHT);
            let cap = l.grid_capacity();
            let last = l.grid_cell(cap - 1);
            assert_eq!(cap % GRID_COLUMNS as usize, 0);
            assert!(last.y + last.h <= l.content.y + l.content.h || cap == GRID_COLUMNS as usize);
        }
    }

    #[test]
    fn list_rows_stack_inside_the_content() {
        let l = compute(1024, 768, PANEL_HEIGHT);
        let last = l.list_row(l.list_page() - 1, true);
        assert!(last.y + last.h <= l.content.y + l.content.h);
        assert_eq!(
            l.list_row(1, false).y,
            l.list_row(0, false).y + LIST_ROW_HEIGHT
        );
        assert!(l.list_row(0, true).w < l.list_row(0, false).w);
    }

    #[test]
    fn thumb_spans_the_track_at_both_ends() {
        let l = compute(800, 600, PANEL_HEIGHT);
        let count = l.list_page() * 3;
        let track = l.scroll_track();
        let top = l.scroll_thumb(count, 0);
        let bottom = l.scroll_thumb(count, count - l.list_page());
        assert_eq!(top.y, track.y);
        assert_eq!(bottom.y + bottom.h, track.y + track.h);
    }

    #[test]
    fn dragging_the_thumb_round_trips_to_a_scroll_position() {
        let l = compute(800, 600, PANEL_HEIGHT);
        let count = l.list_page() * 3;
        for scroll in [0, 1, l.list_page(), count - l.list_page()] {
            let thumb = l.scroll_thumb(count, scroll);
            assert_eq!(l.scroll_for_thumb(count, thumb.y), scroll);
        }
    }

    #[test]
    fn power_rows_fill_the_popup() {
        let l = compute(1280, 720, PANEL_HEIGHT);
        let last = l.power_row(3);
        assert!(last.y + last.h <= l.power_menu.y + l.power_menu.h);
        assert!(l.power_menu.contains(last.x, last.y));
    }
}
