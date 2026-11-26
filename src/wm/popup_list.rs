//! Screen-space list popup (dropdown lists).
//!
//! A dropdown's list must be able to extend past its own window, so it is
//! not drawn into the window surface. Instead the owning window hands the
//! anchor (in screen coordinates) and the rows to this module, which draws
//! the list as an overlay above all windows and routes pointer and keyboard
//! input to it while open. A pick is delivered to the owner exactly like a
//! click on a registered row would be: through the window's hit handler,
//! with the row's key as the region tag.
//!
//! Only one list is open at a time. The list is placed by the pure
//! function [`place`]: below the anchor when it fits, above it when it fits
//! there, otherwise on the roomier side with the height clamped (the rows
//! then scroll).

use crate::display::fb::FramebufferInfo;
use crate::display::graphics;
use crate::display::icons::{self, Glyph};
use crate::display::text::{self, Face};
use crate::sync::spinlock::Spinlock;
use crate::wm::controls::{style, Rect, LABEL_PX, POPUP_ROW_H, RADIUS, SHADOW_ALPHA};
use crate::wm::hit_region::{self, Hit, HitKind, HitRegion};
use crate::wm::{scene, theme};

/// Most rows one list can hold.
const MAX_ITEMS: usize = 16;
/// Longest row label kept, in bytes; longer labels are cut.
const MAX_LABEL_LEN: usize = 40;
/// Inner padding between the list border and its rows.
const PAD: u32 = 4;
/// Gap between the anchor control and the list.
const GAP: u32 = 4;
/// Vertical offset of the drop shadow.
const SHADOW_DY: u32 = 3;
/// Extra pixels around the list that the shadow covers (damage margin).
const SHADOW_MARGIN: u32 = 6;
/// Radius of the list corners.
const LIST_RADIUS: u32 = RADIUS + 2;
/// Alpha of the highlighted row tint.
const HIGHLIGHT_ALPHA: u8 = 90;
/// Left inset of a row label.
const LABEL_INSET: u32 = 10;
/// Width reserved at the right of a row for the check mark.
const CHECK_RESERVE: u32 = 40;
/// Size of the check mark glyph.
const CHECK_PX: u32 = 16;
/// Width of the scroll position bar drawn when rows are hidden.
const SCROLL_BAR_W: u32 = 3;
/// Rows moved by Page Up / Page Down.
const PAGE_ROWS: i32 = 4;
/// Marks "no row" for the selected and hovered indices.
const NONE: usize = usize::MAX;

const SC_ESC: u8 = 0x01;
const SC_ENTER: u8 = 0x1C;
const SC_SPACE: u8 = 0x39;
const SC_HOME: u8 = 0x47;
const SC_UP: u8 = 0x48;
const SC_PAGE_UP: u8 = 0x49;
const SC_END: u8 = 0x4F;
const SC_DOWN: u8 = 0x50;
const SC_PAGE_DOWN: u8 = 0x51;

/// What an input event did to the popup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PopupClick {
    /// A row was picked; deliver `key` to window `owner`. The popup closed.
    Chosen {
        /// Window that opened the list.
        owner: u32,
        /// Key the owner registered for the row.
        key: u32,
    },
    /// The list was dismissed (outside click or Escape); the owner gets
    /// `key`, the dismiss key it registered at open time. The popup closed.
    Dismissed {
        /// Window that opened the list.
        owner: u32,
        /// Dismiss key the owner passed to `open`.
        key: u32,
    },
    /// The event landed on the list but changed nothing the owner cares
    /// about (border, hover, scrolling); it is consumed.
    Inert,
    /// No list is open, or the event is not for the list.
    Closed,
}

struct Popup {
    open: bool,
    owner: u32,
    dismiss_key: u32,
    anchor: Rect,
    count: usize,
    labels: [[u8; MAX_LABEL_LEN]; MAX_ITEMS],
    label_lens: [u8; MAX_ITEMS],
    keys: [u32; MAX_ITEMS],
    selected: usize,
    highlight: usize,
    first_row: usize,
    visible_rows: usize,
    rect: Rect,
}

impl Popup {
    const fn new() -> Self {
        Self {
            open: false,
            owner: 0,
            dismiss_key: 0,
            anchor: Rect::new(0, 0, 0, 0),
            count: 0,
            labels: [[0; MAX_LABEL_LEN]; MAX_ITEMS],
            label_lens: [0; MAX_ITEMS],
            keys: [0; MAX_ITEMS],
            selected: NONE,
            highlight: NONE,
            first_row: 0,
            visible_rows: 0,
            rect: Rect::new(0, 0, 0, 0),
        }
    }

    fn label(&self, row: usize) -> &str {
        let len = usize::from(self.label_lens[row]);
        core::str::from_utf8(&self.labels[row][..len]).unwrap_or("")
    }

    fn set_row(&mut self, row: usize, label: &str, key: u32) {
        let mut len = label.len().min(MAX_LABEL_LEN);
        while !label.is_char_boundary(len) {
            len -= 1;
        }
        self.labels[row][..len].copy_from_slice(&label.as_bytes()[..len]);
        self.label_lens[row] = len as u8;
        self.keys[row] = key;
    }

    /// Screen rectangle of the list including its shadow, for damage.
    fn damage_rect(&self) -> scene::Rect {
        let r = self.rect;
        scene::Rect::from_xywh(
            (r.x.max(0) as u32).saturating_sub(SHADOW_MARGIN),
            (r.y.max(0) as u32).saturating_sub(SHADOW_MARGIN),
            r.w + 2 * SHADOW_MARGIN,
            r.h + 2 * SHADOW_MARGIN + SHADOW_DY,
        )
    }

    /// Row under the screen point, if the point is on a visible row.
    fn row_at(&self, x: i32, y: i32) -> Option<usize> {
        let inner = Rect::new(
            self.rect.x,
            self.rect.y + PAD as i32,
            self.rect.w,
            self.visible_rows as u32 * POPUP_ROW_H,
        );
        if !inner.contains(x, y) {
            return None;
        }
        let offset = ((y - inner.y) as u32 / POPUP_ROW_H) as usize;
        Some(self.first_row + offset).filter(|row| *row < self.count)
    }

    fn scroll_to_highlight(&mut self) {
        if self.highlight == NONE || self.visible_rows == 0 {
            return;
        }
        if self.highlight < self.first_row {
            self.first_row = self.highlight;
        }
        let past = self.highlight + 1;
        if past > self.first_row + self.visible_rows {
            self.first_row = past - self.visible_rows;
        }
    }

    fn move_highlight(&mut self, delta: i32) {
        let last = self.count as i64 - 1;
        let from = if self.highlight == NONE {
            if delta > 0 {
                -1
            } else {
                last + 1
            }
        } else {
            self.highlight as i64
        };
        self.highlight = (from + i64::from(delta)).clamp(0, last) as usize;
        self.scroll_to_highlight();
    }

    fn pick(&mut self, row: usize) -> PopupClick {
        let click = PopupClick::Chosen {
            owner: self.owner,
            key: self.keys[row],
        };
        self.close_in_place();
        click
    }

    fn dismiss(&mut self) -> PopupClick {
        let click = PopupClick::Dismissed {
            owner: self.owner,
            key: self.dismiss_key,
        };
        self.close_in_place();
        click
    }

    fn close_in_place(&mut self) {
        if self.open {
            scene::damage(self.damage_rect());
        }
        self.open = false;
    }
}

static POPUP: Spinlock<Popup> = Spinlock::new(Popup::new());

/// Where a list of `list_w` x `list_h` px goes for an `anchor` control, on a
/// screen `screen_w` wide whose usable height (above the taskbar) is
/// `work_h`. Below the anchor if it fits, else above, else the roomier side
/// with the height cut to the room. Never leaves the screen horizontally
/// and never reaches into the taskbar.
#[must_use]
pub fn place(anchor: Rect, list_w: u32, list_h: u32, screen_w: u32, work_h: u32) -> Rect {
    let w = list_w.min(screen_w);
    let max_x = screen_w.saturating_sub(w) as i32;
    let x = anchor.x.clamp(0, max_x);
    let below_top = anchor.bottom() + GAP as i32;
    let below_room = (work_h as i32 - below_top).max(0) as u32;
    let above_room = (anchor.y - GAP as i32).max(0) as u32;
    if list_h <= below_room {
        return Rect::new(x, below_top, w, list_h);
    }
    if list_h <= above_room || above_room > below_room {
        let h = list_h.min(above_room);
        return Rect::new(x, anchor.y - GAP as i32 - h as i32, w, h);
    }
    Rect::new(x, below_top, w, below_room)
}

/// Fit the list on screen and decide how many rows show at once.
fn layout(p: &mut Popup, screen_w: u32, work_h: u32) {
    let full_h = p.count as u32 * POPUP_ROW_H + 2 * PAD;
    let placed = place(p.anchor, p.anchor.w, full_h, screen_w, work_h);
    let rows_fit = (placed.h.saturating_sub(2 * PAD) / POPUP_ROW_H) as usize;
    p.visible_rows = rows_fit.clamp(1, p.count.max(1));
    let h = p.visible_rows as u32 * POPUP_ROW_H + 2 * PAD;
    let above = placed.y < p.anchor.y;
    let y = if above {
        p.anchor.y - GAP as i32 - h as i32
    } else {
        placed.y
    };
    p.rect = Rect::new(placed.x, y, placed.w, h);
    p.first_row = p.first_row.min(p.count.saturating_sub(p.visible_rows));
    p.scroll_to_highlight();
}

/// Open the list for window `owner` below/above `anchor_screen` (screen
/// coordinates). `keys[i]` is delivered to the owner when row `i` is
/// picked, `dismiss_key` when the list is dismissed. `selected` marks the
/// current value (out of range for none). Opening an already open list for
/// the same owner is a no-op so a window may call this every frame.
pub fn open(
    owner: u32,
    anchor_screen: Rect,
    items: &[&str],
    keys: &[u32],
    selected: usize,
    dismiss_key: u32,
) {
    let screen = crate::display::fb::device_info();
    let work_h = screen.height.saturating_sub(crate::wm::panel::PANEL_HEIGHT);
    open_on(
        owner,
        anchor_screen,
        items,
        keys,
        (selected, dismiss_key),
        (screen.width, work_h),
    );
}

/// [`open`] with the screen size given explicitly (host tests).
fn open_on(
    owner: u32,
    anchor: Rect,
    items: &[&str],
    keys: &[u32],
    (selected, dismiss_key): (usize, u32),
    (screen_w, work_h): (u32, u32),
) {
    let mut p = POPUP.lock();
    if p.open && p.owner == owner {
        return;
    }
    p.close_in_place();
    p.owner = owner;
    p.dismiss_key = dismiss_key;
    p.anchor = anchor;
    p.count = items.len().min(keys.len()).min(MAX_ITEMS);
    for row in 0..p.count {
        p.set_row(row, items[row], keys[row]);
    }
    p.selected = if selected < p.count { selected } else { NONE };
    p.highlight = p.selected;
    p.first_row = 0;
    p.open = p.count > 0;
    layout(&mut p, screen_w, work_h);
    scene::damage(p.damage_rect());
}

/// Close the list without telling the owner.
pub fn close() {
    POPUP.lock().close_in_place();
}

/// Close the list if window `window_id` owns it (destroyed or minimised).
pub fn on_window_destroyed(window_id: u32) {
    let mut p = POPUP.lock();
    if p.open && p.owner == window_id {
        p.close_in_place();
    }
}

/// Whether a list is showing.
#[must_use]
pub fn is_open() -> bool {
    POPUP.lock().open
}

/// Whether window `window_id` owns the open list.
#[must_use]
pub fn is_open_for(window_id: u32) -> bool {
    let p = POPUP.lock();
    p.open && p.owner == window_id
}

/// A left press at screen (x, y). Picks the row under the pointer, ignores
/// presses on the list border, and dismisses on any press outside.
pub fn handle_click(x: u32, y: u32) -> PopupClick {
    let mut p = POPUP.lock();
    if !p.open {
        return PopupClick::Closed;
    }
    let (px, py) = (x as i32, y as i32);
    if let Some(row) = p.row_at(px, py) {
        return p.pick(row);
    }
    if p.rect.contains(px, py) {
        return PopupClick::Inert;
    }
    p.dismiss()
}

/// Pointer moved to screen (x, y): move the highlight to the row under it.
pub fn handle_hover(x: u32, y: u32) {
    let mut p = POPUP.lock();
    if !p.open {
        return;
    }
    let Some(row) = p.row_at(x as i32, y as i32) else {
        return;
    };
    if p.highlight != row {
        p.highlight = row;
        scene::damage(p.damage_rect());
    }
}

/// Mouse wheel over the list: scroll by `rows` (positive = down).
pub fn handle_wheel(rows: i32) -> bool {
    let mut p = POPUP.lock();
    if !p.open {
        return false;
    }
    let last_first = p.count.saturating_sub(p.visible_rows) as i64;
    p.first_row = (p.first_row as i64 + i64::from(rows)).clamp(0, last_first) as usize;
    scene::damage(p.damage_rect());
    true
}

/// A key press while the list may be open. Arrows, Home/End and Page keys
/// move the highlight, Enter/Space pick it, Escape dismisses.
pub fn handle_key(scancode: u8) -> PopupClick {
    let mut p = POPUP.lock();
    if !p.open {
        return PopupClick::Closed;
    }
    match scancode {
        SC_ESC => return p.dismiss(),
        SC_ENTER | SC_SPACE if p.highlight != NONE => {
            let row = p.highlight;
            return p.pick(row);
        }
        SC_UP => p.move_highlight(-1),
        SC_DOWN => p.move_highlight(1),
        SC_PAGE_UP => p.move_highlight(-PAGE_ROWS),
        SC_PAGE_DOWN => p.move_highlight(PAGE_ROWS),
        SC_HOME => p.move_highlight(i32::MIN / 2),
        SC_END => p.move_highlight(i32::MAX / 2),
        _ => return PopupClick::Closed,
    }
    scene::damage(p.damage_rect());
    PopupClick::Inert
}

/// Hand a result to the owning window. Returns true when the event was
/// consumed by the popup (anything but `Closed`).
pub fn apply(click: PopupClick) -> bool {
    match click {
        PopupClick::Chosen { owner, key } | PopupClick::Dismissed { owner, key } => {
            deliver(owner, key);
            true
        }
        PopupClick::Inert => true,
        PopupClick::Closed => false,
    }
}

/// Call the owner's hit handler as if the row registered under `key` was
/// clicked.
fn deliver(owner: u32, key: u32) {
    let region = HitRegion::new(0, 0, 0, 0, key, HitKind::MenuItem);
    let hit = Hit {
        win_id: owner,
        region,
        local_x: 0,
        local_y: 0,
        screen_x: 0,
        screen_y: 0,
    };
    hit_region::dispatch(&hit);
}

/// Draw the list into `target` (the scene back buffer).
pub fn draw(target: &FramebufferInfo) {
    let p = POPUP.lock();
    if !p.open {
        return;
    }
    let th = theme::current();
    let r = p.rect;
    let (x, y) = (r.x.max(0) as u32, r.y.max(0) as u32);
    let shadow = (x, y + SHADOW_DY, r.w, r.h);
    graphics::blend_rounded_rect(target, shadow, LIST_RADIUS, 0x000000, SHADOW_ALPHA);
    graphics::fill_rounded_rect(target, x, y, r.w, r.h, LIST_RADIUS, th.panel_border);
    let inner = (x + 1, y + 1, r.w - 2, r.h - 2);
    graphics::fill_rounded_rect(
        target,
        inner.0,
        inner.1,
        inner.2,
        inner.3,
        LIST_RADIUS - 1,
        th.menu_bg,
    );
    for slot in 0..p.visible_rows {
        draw_row(target, &p, slot);
    }
    draw_scroll_bar(target, &p);
}

fn draw_row(target: &FramebufferInfo, p: &Popup, slot: usize) {
    let th = theme::current();
    let row = p.first_row + slot;
    let top = p.rect.y + PAD as i32 + (slot as u32 * POPUP_ROW_H) as i32;
    let (rx, ry) = ((p.rect.x + PAD as i32) as u32, top as u32);
    let rw = p.rect.w - 2 * PAD;
    if p.highlight == row {
        let tint = (rx, ry, rw, POPUP_ROW_H);
        graphics::blend_rounded_rect(target, tint, RADIUS, theme::accent_color(), HIGHLIGHT_ALPHA);
    }
    let s = style(Face::Ui, LABEL_PX, th.menu_item_text);
    let ty = ry + POPUP_ROW_H.saturating_sub(text::line_height(&s)) / 2;
    let label_w = rw.saturating_sub(CHECK_RESERVE);
    text::draw_fitted(target, rx + LABEL_INSET, ty, label_w, p.label(row), &s);
    if p.selected == row {
        let cx = rx + rw - CHECK_PX - LABEL_INSET / 2;
        let cy = ry + (POPUP_ROW_H - CHECK_PX) / 2;
        icons::draw_glyph(target, Glyph::Check, CHECK_PX, cx, cy, th.label_text);
    }
}

fn draw_scroll_bar(target: &FramebufferInfo, p: &Popup) {
    if p.visible_rows >= p.count {
        return;
    }
    let th = theme::current();
    let track_h = p.visible_rows as u32 * POPUP_ROW_H;
    let thumb_h = (track_h * p.visible_rows as u32 / p.count as u32).max(POPUP_ROW_H / 2);
    let travel = track_h - thumb_h;
    let steps = (p.count - p.visible_rows) as u32;
    let offset = travel * p.first_row as u32 / steps;
    let x = (p.rect.right() as u32).saturating_sub(SCROLL_BAR_W + 2);
    let y = p.rect.y as u32 + PAD + offset;
    graphics::fill_rounded_rect(
        target,
        x,
        y,
        SCROLL_BAR_W,
        thumb_h,
        SCROLL_BAR_W / 2,
        th.scroll_thumb,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::serial_guard;

    const SCREEN_W: u32 = 1024;
    const WORK_H: u32 = 728;
    const OWNER: u32 = 7;
    const DISMISS: u32 = 99;
    const LIST_W: u32 = 240;
    const ROWS: usize = 4;
    const ANCHOR_H: u32 = 32;
    const KEYS: [u32; ROWS] = [100, 101, 102, 103];
    const ITEMS: [&str; ROWS] = ["1920 x 1080", "1280 x 720", "1024 x 768", "800 x 600"];

    fn anchor_at(y: i32) -> Rect {
        Rect::new(100, y, LIST_W, ANCHOR_H)
    }

    fn open_at(y: i32, selected: usize) {
        close();
        open_on(
            OWNER,
            anchor_at(y),
            &ITEMS,
            &KEYS,
            (selected, DISMISS),
            (SCREEN_W, WORK_H),
        );
    }

    fn row_center(row: usize) -> (u32, u32) {
        let p = POPUP.lock();
        let slot = (row - p.first_row) as i32;
        let y = p.rect.y + PAD as i32 + slot * POPUP_ROW_H as i32 + POPUP_ROW_H as i32 / 2;
        ((p.rect.x + 20) as u32, y as u32)
    }

    fn list_h(rows: usize) -> u32 {
        rows as u32 * POPUP_ROW_H + 2 * PAD
    }

    #[test]
    fn places_below_when_room() {
        let anchor = anchor_at(100);
        let r = place(anchor, LIST_W, list_h(ROWS), SCREEN_W, WORK_H);
        assert_eq!(r.y, anchor.bottom() + GAP as i32);
        assert_eq!(r.h, list_h(ROWS));
    }

    #[test]
    fn flips_above_near_bottom() {
        let anchor = anchor_at(600);
        let r = place(anchor, LIST_W, list_h(ROWS), SCREEN_W, WORK_H);
        assert_eq!(r.bottom(), anchor.y - GAP as i32);
        assert_eq!(r.h, list_h(ROWS));
    }

    #[test]
    fn clamps_to_larger_side_when_neither_fits() {
        let work_h = 200;
        let tall = list_h(12);
        let low = place(anchor_at(120), LIST_W, tall, SCREEN_W, work_h);
        assert_eq!(low.bottom(), 120 - GAP as i32, "roomier side is above");
        assert_eq!(low.h, 120 - GAP);
        let high = place(anchor_at(10), LIST_W, tall, SCREEN_W, work_h);
        assert_eq!(
            high.y,
            10 + ANCHOR_H as i32 + GAP as i32,
            "roomier side is below"
        );
        assert!(high.bottom() <= work_h as i32);
    }

    #[test]
    fn never_overlaps_taskbar() {
        for y in (0..WORK_H as i32).step_by(37) {
            for rows in [1, ROWS, 12] {
                let r = place(anchor_at(y), LIST_W, list_h(rows), SCREEN_W, WORK_H);
                assert!(r.bottom() <= WORK_H as i32, "y={y} rows={rows} -> {r:?}");
                assert!(r.y >= 0, "y={y} rows={rows} -> {r:?}");
            }
        }
    }

    #[test]
    fn never_leaves_screen_horizontally() {
        for x in [-50, 0, 700, 900, 1010] {
            let anchor = Rect::new(x, 100, LIST_W, ANCHOR_H);
            let r = place(anchor, LIST_W, list_h(ROWS), SCREEN_W, WORK_H);
            assert!(r.x >= 0 && r.right() <= SCREEN_W as i32, "x={x} -> {r:?}");
        }
        let wide = place(
            anchor_at(100),
            SCREEN_W + 50,
            list_h(ROWS),
            SCREEN_W,
            WORK_H,
        );
        assert_eq!((wide.x, wide.w), (0, SCREEN_W));
    }

    #[test]
    fn chosen_returns_owner_and_key() {
        let _guard = serial_guard();
        open_at(100, 0);
        let (x, y) = row_center(3);
        assert_eq!(
            handle_click(x, y),
            PopupClick::Chosen {
                owner: OWNER,
                key: KEYS[3]
            }
        );
        assert!(!is_open());
    }

    #[test]
    fn outside_click_dismisses() {
        let _guard = serial_guard();
        open_at(100, 0);
        assert_eq!(
            handle_click(5, 5),
            PopupClick::Dismissed {
                owner: OWNER,
                key: DISMISS
            }
        );
        assert!(!is_open());
        assert_eq!(handle_click(5, 5), PopupClick::Closed);
    }

    #[test]
    fn esc_and_enter_keys() {
        let _guard = serial_guard();
        open_at(100, 1);
        assert_eq!(handle_key(SC_DOWN), PopupClick::Inert);
        assert_eq!(
            handle_key(SC_ENTER),
            PopupClick::Chosen {
                owner: OWNER,
                key: KEYS[2]
            }
        );
        open_at(100, 1);
        assert_eq!(
            handle_key(SC_ESC),
            PopupClick::Dismissed {
                owner: OWNER,
                key: DISMISS
            }
        );
        assert!(!is_open());
        assert_eq!(handle_key(SC_ESC), PopupClick::Closed);
    }

    #[test]
    fn selected_row_stays_visible_when_scrolled() {
        let _guard = serial_guard();
        let many: [&str; MAX_ITEMS] = ["row"; MAX_ITEMS];
        let keys: [u32; MAX_ITEMS] = core::array::from_fn(|i| i as u32);
        close();
        let anchor = Rect::new(100, 150, LIST_W, ANCHOR_H);
        open_on(OWNER, anchor, &many, &keys, (0, DISMISS), (SCREEN_W, 300));
        let visible = POPUP.lock().visible_rows;
        assert!(visible >= 1 && visible < MAX_ITEMS, "visible = {visible}");
        for _ in 0..MAX_ITEMS {
            handle_key(SC_DOWN);
        }
        let p = POPUP.lock();
        assert_eq!(p.highlight, MAX_ITEMS - 1);
        assert!(p.highlight >= p.first_row && p.highlight < p.first_row + p.visible_rows);
        assert!(p.rect.bottom() <= 300 && p.rect.y >= 0);
    }

    #[test]
    fn close_on_owner_window_destroyed() {
        let _guard = serial_guard();
        open_at(100, 0);
        on_window_destroyed(OWNER + 1);
        assert!(is_open_for(OWNER));
        on_window_destroyed(OWNER);
        assert!(!is_open());
    }
}
