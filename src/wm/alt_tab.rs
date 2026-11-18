//! Alt-Tab window switcher overlay
//!
//! While Alt is held and Tab is pressed repeatedly, an overlay appears in the
//! screen center listing all windows with the selection advancing each press.
//! Releasing Alt focuses the selected window and closes the overlay.
//! Tab also steps BACKWARD when Shift is held.

use crate::display::graphics::{draw_rect, draw_rect_outline, fill_rounded_rect};
use crate::display::text::{self, Face};
use crate::wm::theme;
use crate::wm::window;

const TILE_W: u32 = 150;
const TILE_H: u32 = 170;
/// Header text size at the Medium font setting.
const HEADER_PX: u32 = 13;
/// Tile caption size at the Medium font setting.
const CAPTION_PX: u32 = 13;
/// Size of the initial shown on a tile thumbnail.
const INITIAL_PX: u32 = 24;
/// Height available to the header text line.
const HEADER_LINE_H: u32 = 20;
/// Height available to a tile caption line.
const CAPTION_LINE_H: u32 = 20;
/// Horizontal padding inside a tile caption.
const CAPTION_PAD: u32 = 8;
const MAX_TILES: usize = 8;

static mut VISIBLE: bool = false;
static mut SELECTED: usize = 0;

pub fn is_visible() -> bool {
    unsafe { VISIBLE }
}

/// Open the switcher (first Tab press). Selection starts at the window
/// after the currently-focused one (classic behavior).
pub fn open() {
    unsafe {
        let count = window::count();
        if count == 0 {
            return;
        }
        // Start at the window after focused
        let focused = window::focused_id();
        let mut sel = 0;
        let mut seen_focused = false;
        for i in 1..=window::max_id() {
            if !window::exists(i) {
                continue;
            }
            if seen_focused {
                sel = i;
                break;
            }
            if i == focused {
                seen_focused = true;
            }
        }
        // Wraps to 1 if focused is the last
        if sel == 0 {
            for i in 1..=window::max_id() {
                if window::exists(i) {
                    sel = i;
                    break;
                }
            }
        }
        SELECTED = sel as usize;
        VISIBLE = true;
    }
}

/// Advance the selection by one (with wrap). Called on each extra Tab press.
pub fn advance(backwards: bool) {
    unsafe {
        if !VISIBLE {
            open();
            return;
        }
        let count = window::count();
        if count == 0 {
            return;
        }
        let max_id = window::max_id();
        let mut sel = SELECTED as u32;
        for _ in 0..max_id {
            if backwards {
                sel = if sel <= 1 { max_id } else { sel - 1 };
            } else {
                sel = if sel >= max_id { 1 } else { sel + 1 };
            }
            if window::exists(sel) {
                break;
            }
        }
        SELECTED = sel as usize;
    }
}

/// Close the overlay, focusing the selected window.
pub fn close(activate: bool) {
    unsafe {
        let sel = SELECTED as u32;
        VISIBLE = false;
        if activate && sel != 0 && window::exists(sel) {
            window::focus(sel);
        }
    }
}

/// Draw the switcher overlay (centered card with window tiles).
pub fn draw(fb: &crate::display::fb::FramebufferInfo) {
    if !is_visible() {
        return;
    }
    let theme = theme::current();

    let count = window::count();
    if count == 0 {
        return;
    }
    let tiles = count.min(MAX_TILES) as u32;
    let card_w = tiles * TILE_W + (tiles + 1) * 10;
    let card_h = TILE_H + 60;
    let x0 = (fb.width.saturating_sub(card_w)) / 2;
    let y0 = (fb.height.saturating_sub(card_h)) / 2 - 40;

    // Backdrop dim
    draw_rect(fb, 0, 0, fb.width, fb.height, 0x33000000);

    // Card
    draw_rect(fb, x0 + 6, y0 + 10, card_w, card_h, 0x66000000);
    fill_rounded_rect(fb, x0, y0, card_w, card_h, 14, theme.menu_bg);
    draw_rect_outline(fb, x0, y0, card_w, card_h, theme.menu_border);

    // Header
    let header = theme::chrome_text(Face::Ui, HEADER_PX, theme.label_text_dim, HEADER_LINE_H);
    text::draw_fitted(
        fb,
        x0 + 14,
        y0 + 12,
        card_w.saturating_sub(28),
        "Switch Window  (hold Alt, Tab to cycle)",
        &header,
    );

    // Tiles
    let selected = unsafe { SELECTED } as u32;
    let mut tx = x0 + 10;
    let mut drawn = 0u32;
    for id in 1..=window::max_id() {
        if drawn >= tiles {
            break;
        }
        if !window::exists(id) {
            continue;
        }
        let ty = y0 + 34;
        let is_sel = id == selected;

        // Tile background
        let tile_bg = if is_sel {
            theme.menu_item_hover
        } else {
            theme.panel_bg
        };
        fill_rounded_rect(fb, tx, ty, TILE_W - 10, TILE_H, 10, tile_bg);
        if is_sel {
            draw_rect_outline(fb, tx, ty, TILE_W - 10, TILE_H, theme.input_border_focused);
        }

        // Thumbnail placeholder: window-colored block w/ title-bar stripe
        let title = window::get_title(id);
        let thumb_x = tx + 10;
        let thumb_y = ty + 10;
        let thumb_w = TILE_W - 30;
        let thumb_h = TILE_H - 40;
        draw_rect(fb, thumb_x, thumb_y, thumb_w, thumb_h, theme.win_bg);
        draw_rect(fb, thumb_x, thumb_y, thumb_w, 14, theme.title_bar);

        let initial_style = theme::chrome_text(Face::UiBold, INITIAL_PX, theme.label_text, thumb_h);
        text::draw_centered(
            fb,
            tx + (TILE_W - 10) / 2,
            thumb_y + theme::centered_line_y(14, thumb_h - 14, &initial_style),
            title_initial(title),
            &initial_style,
        );

        let caption_color = if is_sel {
            theme.label_text
        } else {
            theme.label_text_dim
        };
        let face = if is_sel { Face::UiBold } else { Face::Ui };
        let caption = theme::chrome_text(face, CAPTION_PX, caption_color, CAPTION_LINE_H);
        text::draw_fitted(
            fb,
            tx + CAPTION_PAD,
            ty + TILE_H - 26,
            TILE_W - 10 - 2 * CAPTION_PAD,
            title,
            &caption,
        );

        tx += TILE_W;
        drawn += 1;
    }
}

/// First character of the title, shown as the tile's icon stand-in.
fn title_initial(title: &str) -> &str {
    title.chars().next().map_or("", |c| &title[..c.len_utf8()])
}
