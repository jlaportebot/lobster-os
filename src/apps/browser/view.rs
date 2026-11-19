//! Drawing of the browser window with the shared controls.

use super::layout::{Run, NO_LINK};
use super::state::{Browser, Page};
use super::url;
use crate::display::icons::Glyph;
use crate::display::text::{self, Face, TextStyle};
use crate::wm::controls::{style, ButtonKind, Rect, Ui, CONTROL_H, LABEL_PX};
use crate::wm::hit_region::HitKind;
use crate::wm::theme;
use alloc::string::String;

/// Hit key of the Back button.
pub const KEY_BACK: u32 = 1;
/// Hit key of the Forward button.
pub const KEY_FORWARD: u32 = 2;
/// Hit key of the Reload button.
pub const KEY_RELOAD: u32 = 3;
/// Hit key of the address field.
pub const KEY_ADDRESS: u32 = 4;
/// Hit key of the bookmark star.
pub const KEY_STAR: u32 = 5;
/// Hit key of the page scrollbar.
pub const KEY_SCROLL: u32 = 6;
/// Hit keys of links are `KEY_LINK_BASE + link id`.
pub const KEY_LINK_BASE: u32 = 0x1000;

/// Height of the toolbar band.
pub const TOOLBAR_H: u32 = 44;
/// Height of the status line.
pub const STATUS_H: u32 = 24;
/// Width of the page scrollbar.
pub const SCROLLBAR_W: u32 = 14;
/// Page background.
const PAGE_BG: u32 = 0xFFFF_FFFF;
/// Outline of image placeholders.
const BOX_COLOR: u32 = 0xFF99_9999;
/// Gold of a set bookmark star.
const STAR_ON: u32 = 0xFFE0_A800;
/// Toolbar padding and gaps.
const PAD: i32 = 8;
const GAP: i32 = 4;
/// Glyph size inside toolbar buttons.
const GLYPH_PX: u32 = 20;
/// Link hit regions kept free for the toolbar and scrollbar (window limit is 128).
const MAX_LINK_REGIONS: usize = 100;
/// Status line text size.
const STATUS_PX: u32 = 12;
/// Gap between the page bottom edge and the underline of a run.
const UNDERLINE_OFFSET: i32 = 2;

/// Rectangle of the page viewport in client coordinates.
#[must_use]
pub fn page_rect(cw: u32, ch: u32) -> Rect {
    let h = ch.saturating_sub(TOOLBAR_H + STATUS_H);
    Rect::new(0, TOOLBAR_H as i32, cw.saturating_sub(SCROLLBAR_W), h)
}

fn icon_button(ui: &mut Ui, key: u32, r: Rect, glyph: Glyph, enabled: bool) {
    ui.button(key, r, "", ButtonKind::Secondary, enabled);
    let th = theme::current();
    let color = if enabled {
        th.btn_text
    } else {
        th.label_text_dim
    };
    ui.glyph(glyph, GLYPH_PX, r, color);
}

fn toolbar(ui: &mut Ui, b: &Browser, cw: u32) {
    let th = theme::current();
    ui.fill(Rect::new(0, 0, cw, TOOLBAR_H), th.win_bg);
    let y = (TOOLBAR_H - CONTROL_H) as i32 / 2;
    let mut x = PAD;
    let step = (CONTROL_H as i32) + GAP;
    let side = Rect::new(x, y, CONTROL_H, CONTROL_H);
    icon_button(ui, KEY_BACK, side, Glyph::ArrowLeft, b.can_back());
    x += step;
    icon_button(
        ui,
        KEY_FORWARD,
        Rect::new(x, y, CONTROL_H, CONTROL_H),
        Glyph::ArrowRight,
        b.can_forward(),
    );
    x += step;
    icon_button(
        ui,
        KEY_RELOAD,
        Rect::new(x, y, CONTROL_H, CONTROL_H),
        Glyph::Refresh,
        true,
    );
    x += step;
    let star_x = cw as i32 - PAD - CONTROL_H as i32;
    let field_w = (star_x - GAP - x).max(40) as u32;
    ui.text_field(
        KEY_ADDRESS,
        Rect::new(x, y, field_w, CONTROL_H),
        &b.address,
        "Address",
    );
    star(ui, b, Rect::new(star_x, y, CONTROL_H, CONTROL_H));
}

fn star(ui: &mut Ui, b: &Browser, r: Rect) {
    ui.button(KEY_STAR, r, "", ButtonKind::Secondary, true);
    let on = b.is_bookmarked();
    let th = theme::current();
    let s = style(
        Face::Ui,
        LABEL_PX + 4,
        if on { STAR_ON } else { th.label_text_dim },
    );
    ui.text_centered(r, "\u{2605}", &s);
}

fn run_style(r: &Run) -> TextStyle {
    TextStyle {
        face: r.face,
        size: r.size,
        color: r.color,
    }
}

fn draw_run(ui: &Ui, p: &Page, r: &Run) {
    let st = run_style(r);
    ui.text((r.x + r.pad) as i32, r.y as i32, p.layout.run_text(r), &st);
    if r.underline {
        let y = r.y as i32 + text::ascent(&st) as i32 + UNDERLINE_OFFSET;
        ui.fill(Rect::new(r.x as i32, y, r.w, 1), r.color);
    }
    if r.boxed {
        let h = text::line_height(&st) + 2;
        ui.outline(Rect::new(r.x as i32, r.y as i32 - 1, r.w, h), BOX_COLOR);
    }
}

/// Draw the page and register its link regions. `scroll` is in px.
fn page(ui: &mut Ui, b: &Browser, view: Rect) {
    ui.set_view(view, (0, view.y - b.scroll));
    ui.fill(ui.view_extent(), PAGE_BG);
    let Some(p) = b.page.as_ref() else { return };
    let top = b.scroll as u32;
    let bottom = top + view.h;
    for rect in &p.layout.rects {
        if rect.y < bottom && rect.y + rect.h > top {
            ui.fill(
                Rect::new(rect.x as i32, rect.y as i32, rect.w, rect.h),
                rect.color,
            );
        }
    }
    let first = p.layout.runs.partition_point(|r| r.bottom <= top);
    let mut regions = 0;
    for r in p.layout.runs[first..].iter().take_while(|r| r.top < bottom) {
        draw_run(ui, p, r);
        if r.link != NO_LINK && regions < MAX_LINK_REGIONS {
            let area = Rect::new(r.x as i32, r.top as i32, r.w, r.bottom - r.top);
            ui.register(KEY_LINK_BASE + r.link, area, HitKind::Custom, false);
            regions += 1;
        }
    }
}

fn status_bar(ui: &mut Ui, b: &Browser, size: (u32, u32), hover: Option<String>) {
    let th = theme::current();
    let (cw, ch) = size;
    let y = ch.saturating_sub(STATUS_H) as i32;
    ui.fill(Rect::new(0, y, cw, STATUS_H), th.win_bg);
    ui.fill(Rect::new(0, y, cw, 1), th.menu_border);
    let s = style(Face::Ui, STATUS_PX, th.label_text_dim);
    let line = hover.unwrap_or_else(|| b.status_line());
    let ty = y + (STATUS_H.saturating_sub(text::line_height(&s)) / 2) as i32;
    ui.text_fitted(PAD, ty, cw.saturating_sub(2 * PAD as u32), &line, &s);
}

/// URL under the pointer, resolved against the current page.
fn hovered_url(b: &Browser, client: (u32, u32), view: Rect) -> Option<String> {
    let p = b.page.as_ref()?;
    let (cx, cy) = crate::wm::input::cursor_pos();
    let (lx, ly) = (cx as i32 - client.0 as i32, cy as i32 - client.1 as i32);
    if !view.contains(lx, ly) {
        return None;
    }
    let px = lx as u32;
    let py = (ly - view.y + b.scroll) as u32;
    let href = p.layout.href(p.layout.link_at(px, py)?);
    if href == super::pages::RETRY_HREF {
        return Some(String::from("Try loading this page again"));
    }
    Some(url::resolve(&b.url, href))
}

/// Draw one frame of the browser into `ui`; `client` is the client origin on screen.
pub fn frame(ui: &mut Ui, b: &mut Browser, client: (u32, u32), size: (u32, u32)) {
    let (cw, ch) = size;
    let view = page_rect(cw, ch);
    b.content_w = view.w;
    b.view_h = view.h;
    b.relayout_if_needed();
    toolbar(ui, b, cw);
    let hover = hovered_url(b, client, view);
    page(ui, b, view);
    ui.set_view(Rect::new(0, 0, cw, ch), (0, 0));
    let height = b.page.as_ref().map_or(0, |p| p.layout.height);
    let track = Rect::new(view.right(), view.y, SCROLLBAR_W, view.h);
    ui.scrollbar(KEY_SCROLL, track, height, view.h, b.scroll);
    status_bar(ui, b, size, hover);
}
