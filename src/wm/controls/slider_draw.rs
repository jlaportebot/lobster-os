//! Slider, dropdown, progress bar, scrollbar and text field drawing.

use super::drag::Track;
use super::scroll;
use super::ui::{on_color, style, Look, Ui};
use super::widgets::{CONTROL_H, LABEL_PX, RADIUS};
use super::Rect;
use crate::display::icons::Glyph;
use crate::display::text::{self, Face};
use crate::wm::hit_region::HitKind;
use crate::wm::{popup_list, theme};

/// Slider thumb diameter.
const THUMB_D: u32 = 20;
/// Slider track thickness.
const TRACK_H: u32 = 6;
/// Height of one row in a dropdown popup.
pub const POPUP_ROW_H: u32 = 32;
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

    /// Open the popup list for a dropdown whose field is `anchor`. The
    /// list is drawn by [`crate::wm::popup_list`] in screen space so it can
    /// extend past this window; picking a row delivers `keys[i]` to this
    /// window's handler and dismissing delivers `dismiss_key`. Call every
    /// frame while the dropdown is open: repeats are ignored, and a frame
    /// that does not call it closes the list.
    pub fn dropdown_popup(
        &mut self,
        anchor: Rect,
        items: &[&str],
        keys: &[u32],
        selected: usize,
        dismiss_key: u32,
    ) {
        self.popup_requested = true;
        let screen = self.to_screen(anchor);
        popup_list::open(self.win_id, screen, items, keys, selected, dismiss_key);
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
