//! Slider, dropdown, progress bar, scrollbar and text field drawing.

use super::drag::Track;
use super::dropdown::Dropdown;
use super::scroll;
use super::ui::{on_color, style, Look, Ui};
use super::widgets::{CONTROL_H, LABEL_PX, RADIUS};
use super::Rect;
use crate::display::icons::Glyph;
use crate::display::text::{self, Face};
use crate::wm::hit_region::HitKind;
use crate::wm::theme;

/// Slider thumb diameter.
const THUMB_D: u32 = 20;
/// Slider track thickness.
const TRACK_H: u32 = 6;
/// Height of one row in a dropdown popup.
pub const POPUP_ROW_H: u32 = 32;
/// Vertical gap between a dropdown field and its popup.
const POPUP_GAP: u32 = 4;
/// Opacity of popup drop shadows.
pub const SHADOW_ALPHA: u8 = 90;
/// Progress bar thickness.
pub const PROGRESS_H: u32 = 8;

impl Ui<'_> {
    /// Horizontal slider over `r`; registers its track for dragging.
    pub fn slider(&mut self, key: u32, r: Rect, value: u32, range: (u32, u32), enabled: bool) {
        let th = theme::current();
        let (min, max) = range;
        let look = self.look(r, enabled);
        let track = Rect::new(
            r.x + (THUMB_D / 2) as i32,
            r.y + (r.h.saturating_sub(TRACK_H) / 2) as i32,
            r.w.saturating_sub(THUMB_D),
            TRACK_H,
        );
        let accent = if enabled {
            theme::accent_color()
        } else {
            th.scroll_thumb
        };
        self.rounded(track, TRACK_H / 2, th.scroll_thumb);
        let thumb_cx = super::slider::thumb_x(value, track.x, track.w, min, max);
        let filled = Rect::new(track.x, track.y, (thumb_cx - track.x) as u32, TRACK_H);
        self.rounded(filled, TRACK_H / 2, accent);
        let ty = r.y + (r.h.saturating_sub(THUMB_D) / 2) as i32;
        let thumb = Rect::new(thumb_cx - (THUMB_D / 2) as i32, ty, THUMB_D, THUMB_D);
        let edge = if self.has_focus(key) {
            theme::accent_color()
        } else {
            th.panel_border
        };
        self.disc(thumb, edge);
        let face = if look == Look::Pressed {
            th.btn_bg_pressed
        } else {
            0xFFFFFFFF
        };
        self.disc(thumb.inset(2), face);
        if enabled {
            let screen = self.to_screen(track);
            self.add_track(
                key,
                Track::Horizontal {
                    x: screen.x,
                    w: screen.w,
                    min,
                    max,
                },
            );
            self.register(key, r, HitKind::Slider, true);
        }
    }

    /// Closed dropdown field showing `label`.
    pub fn dropdown(&mut self, key: u32, r: Rect, label: &str, enabled: bool) {
        let th = theme::current();
        let look = self.look(r, enabled);
        let border = if self.has_focus(key) {
            theme::accent_color()
        } else {
            th.input_border
        };
        self.ring(r, RADIUS, 1, border, th.input_bg);
        if look == Look::Hover || look == Look::Pressed {
            self.blend(r, RADIUS, th.label_text, 20);
        }
        let color = if enabled {
            th.label_text
        } else {
            th.label_text_dim
        };
        let s = style(Face::Ui, LABEL_PX, color);
        let ty = r.y + (r.h.saturating_sub(text::line_height(&s)) / 2) as i32;
        self.text_fitted(r.x + 12, ty, r.w.saturating_sub(44), label, &s);
        let chevron = Rect::new(r.right() - 32, r.y, 24, r.h);
        self.glyph(Glyph::ChevronDown, 16, chevron, th.label_text_dim);
        if enabled {
            self.register(key, r, HitKind::Button, true);
        }
    }

    /// Popup list for an open dropdown, drawn over the content below
    /// `anchor` (or above it when there is no room). A scrim covering the
    /// whole view is registered first so outside clicks reach `dismiss_key`.
    #[allow(clippy::too_many_arguments)]
    pub fn dropdown_popup(
        &mut self,
        anchor: Rect,
        items: &[&str],
        keys: &[u32],
        menu: &Dropdown,
        selected: usize,
        dismiss_key: u32,
        view: Rect,
    ) {
        let th = theme::current();
        self.set_modal(false);
        self.register(dismiss_key, view, HitKind::Custom, false);
        let height = items.len() as u32 * POPUP_ROW_H + 8;
        let below = anchor.bottom() + POPUP_GAP as i32;
        let fits =
            self.to_client(Rect::new(0, below, 1, height)).bottom() <= self.viewport_bottom();
        let top = if fits {
            below
        } else {
            anchor.y - POPUP_GAP as i32 - height as i32
        };
        let popup = Rect::new(anchor.x, top, anchor.w, height);
        self.blend(popup.offset(0, 3), RADIUS + 2, 0x000000, SHADOW_ALPHA);
        self.ring(popup, RADIUS + 2, 1, th.panel_border, th.menu_bg);
        for (i, item) in items.iter().enumerate() {
            let row = Rect::new(
                popup.x + 4,
                popup.y + 4 + (i as u32 * POPUP_ROW_H) as i32,
                popup.w - 8,
                POPUP_ROW_H,
            );
            let hot = menu.highlight() == i || self.look(row, true) != Look::Normal;
            if hot {
                self.blend(row, RADIUS, theme::accent_color(), 90);
            }
            let color = th.menu_item_text;
            let s = style(Face::Ui, LABEL_PX, color);
            let ty = row.y + (row.h.saturating_sub(text::line_height(&s)) / 2) as i32;
            self.text_fitted(row.x + 10, ty, row.w - 40, item, &s);
            if i == selected {
                let tick = Rect::new(row.right() - 28, row.y, 20, row.h);
                self.glyph(Glyph::Check, 16, tick, th.label_text);
            }
            self.register(keys[i], row, HitKind::MenuItem, false);
        }
    }

    /// Thin progress bar filled to `percent` (0..=100).
    pub fn progress(&self, r: Rect, percent: u32) {
        let th = theme::current();
        self.rounded(r, r.h / 2, th.progress_track);
        let fill = r.w * percent.min(100) / 100;
        if fill > 0 {
            let bar = Rect::new(r.x, r.y, fill.max(r.h), r.h);
            self.rounded(bar, r.h / 2, theme::accent_color());
        }
    }

    /// Vertical scrollbar over `r` for content of `content_h` px shown in
    /// the view `view_h` px tall, scrolled by `scroll`. Hidden when
    /// everything fits.
    pub fn scrollbar(&mut self, key: u32, r: Rect, content_h: u32, view_h: u32, scroll: i32) {
        if content_h <= view_h {
            return;
        }
        let th = theme::current();
        let (offset, len) = scroll::thumb(r.h, view_h, content_h, scroll);
        let thumb = Rect::new(r.x + 3, r.y + offset as i32, r.w.saturating_sub(6), len);
        let look = self.look(thumb, true);
        let color = if look == Look::Normal {
            th.scroll_thumb
        } else {
            th.scroll_thumb_hover
        };
        self.rounded(thumb, (thumb.w / 2).max(1), color);
        let screen = self.to_screen(r);
        self.add_track(
            key,
            Track::Vertical {
                y: screen.y,
                h: r.h,
                view_h,
                content_h,
            },
        );
        self.register(key, r, HitKind::ScrollBar, false);
    }

    /// Single-line text field showing `value`, with a caret when focused.
    pub fn text_field(&mut self, key: u32, r: Rect, value: &str, placeholder: &str) {
        let th = theme::current();
        let focused = self.state.focus == key;
        let border = if focused {
            th.input_border_focused
        } else {
            th.input_border
        };
        self.ring(r, RADIUS, 1, border, th.input_bg);
        let shown = !value.is_empty();
        let color = if shown {
            th.input_text
        } else {
            th.label_text_dim
        };
        let s = style(Face::Ui, LABEL_PX, color);
        let ty = r.y + (r.h.saturating_sub(text::line_height(&s)) / 2) as i32;
        let inner_w = r.w.saturating_sub(24);
        let content = if shown { value } else { placeholder };
        self.text_fitted(r.x + 12, ty, inner_w, content, &s);
        if focused {
            let caret_x = r.x + 12 + (text::width(value, &s).min(inner_w)) as i32;
            self.fill(
                Rect::new(caret_x, ty, 2, text::line_height(&s)),
                th.input_cursor,
            );
        }
        self.register(key, r, HitKind::TextInput, true);
    }

    fn viewport_bottom(&self) -> i32 {
        self.viewport_rect().bottom()
    }
}

/// Height of a standard control row.
pub const ROW_H: u32 = CONTROL_H;

/// Colour of text drawn on the accent colour (re-exported for pages).
#[must_use]
pub fn accent_text() -> u32 {
    on_color(theme::accent_color())
}
