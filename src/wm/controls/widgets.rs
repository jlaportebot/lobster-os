//! Buttons, switches, segmented controls, swatches, list rows and cards.

use super::ui::{on_color, style, Look, Ui};
use super::Rect;
use crate::display::icons::Glyph;
use crate::display::text::Face;
use crate::wm::hit_region::HitKind;
use crate::wm::theme;

/// Width and height of a switch.
pub const SWITCH_W: u32 = 44;
pub const SWITCH_H: u32 = 24;
/// Standard control height for buttons, dropdowns and segmented controls.
pub const CONTROL_H: u32 = 32;
/// Corner radius shared by buttons and fields.
pub const RADIUS: u32 = 6;
/// Text size of control labels at the Medium text setting.
pub const LABEL_PX: u32 = 14;
/// Thickness of the keyboard focus ring.
const FOCUS_RING: u32 = 2;
/// Inset between a switch track and its knob.
const KNOB_INSET: u32 = 3;
/// Alpha of the hover tint laid over a control.
const HOVER_ALPHA: u8 = 40;
/// Alpha of the pressed tint laid over a control.
const PRESSED_ALPHA: u8 = 80;
/// Alpha of a disabled control's text and fills.
const DISABLED_ALPHA: u8 = 110;

/// Visual weight of a button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonKind {
    /// Accent-filled: the main action.
    Primary,
    /// Neutral fill.
    Secondary,
    /// Red fill for destructive actions.
    Danger,
}

/// Destructive action colour.
const DANGER: u32 = 0xFFD64545;

impl Ui<'_> {
    /// Rounded surface used to group related rows.
    pub fn card(&self, r: Rect) {
        let th = theme::current();
        self.ring(r, RADIUS + 2, 1, th.panel_border, th.panel_bg);
    }

    fn tint(&self, r: Rect, radius: u32, look: Look) {
        let th = theme::current();
        match look {
            Look::Hover => self.blend(r, radius, th.label_text, HOVER_ALPHA / 2),
            Look::Pressed => self.blend(r, radius, th.label_text, PRESSED_ALPHA / 2),
            Look::Disabled => self.blend(r, radius, th.panel_bg, DISABLED_ALPHA * 2),
            Look::Normal => {}
        }
    }

    fn focus_ring(&self, key: u32, r: Rect, radius: u32) {
        if self.has_focus(key) {
            let _ = radius;
            let accent = theme::accent_color();
            for step in 0..FOCUS_RING {
                self.outline(r.inset(step), accent);
            }
        }
    }

    /// Push button; returns its look for callers that need it.
    pub fn button(&mut self, key: u32, r: Rect, label: &str, kind: ButtonKind, enabled: bool) {
        let th = theme::current();
        let look = self.look(r, enabled);
        let (fill, text) = match kind {
            ButtonKind::Primary => (theme::accent_color(), on_color(theme::accent_color())),
            ButtonKind::Secondary => (th.btn_bg, th.btn_text),
            ButtonKind::Danger => (DANGER, 0xFFFFFFFF),
        };
        self.rounded(r, RADIUS, fill);
        if kind == ButtonKind::Secondary {
            self.ring(r, RADIUS, 1, th.panel_border, fill);
        }
        self.tint(r, RADIUS, look);
        let dim = look == Look::Disabled;
        let color = if dim { th.label_text_dim } else { text };
        self.text_centered(r, label, &style(Face::UiBold, LABEL_PX, color));
        self.focus_ring(key, r, RADIUS);
        if enabled {
            self.register(key, r, HitKind::Button, true);
        }
    }

    /// On/off switch with its top-left corner at (`x`, `y`).
    pub fn switch(&mut self, key: u32, x: i32, y: i32, on: bool, enabled: bool) {
        let th = theme::current();
        let r = Rect::new(x, y, SWITCH_W, SWITCH_H);
        let look = self.look(r, enabled);
        let accent = theme::accent_color();
        let track = if on { accent } else { th.scroll_thumb };
        self.rounded(r, SWITCH_H / 2, track);
        let knob_d = SWITCH_H - 2 * KNOB_INSET;
        let knob_x = if on {
            r.right() - (KNOB_INSET + knob_d) as i32
        } else {
            r.x + KNOB_INSET as i32
        };
        let knob = Rect::new(knob_x, y + KNOB_INSET as i32, knob_d, knob_d);
        self.disc(knob, if on { on_color(accent) } else { 0xFFFFFFFF });
        self.tint(r, SWITCH_H / 2, look);
        self.focus_ring(key, r, SWITCH_H / 2);
        if enabled {
            self.register(key, r, HitKind::Checkbox, true);
        }
    }

    /// Row of mutually exclusive options. `keys[i]` is segment `i`'s key.
    pub fn segmented(&mut self, keys: &[u32], labels: &[&str], r: Rect, selected: usize) {
        let th = theme::current();
        self.ring(r, RADIUS, 1, th.panel_border, th.input_bg);
        let count = labels.len().max(1) as u32;
        let seg_w = r.w / count;
        for (i, label) in labels.iter().enumerate() {
            let seg = Rect::new(r.x + (i as u32 * seg_w) as i32, r.y, seg_w, r.h).inset(3);
            let look = self.look(seg, true);
            let chosen = i == selected;
            let accent = theme::accent_color();
            if chosen {
                self.rounded(seg, RADIUS - 2, accent);
            }
            self.tint(seg, RADIUS - 2, if chosen { Look::Normal } else { look });
            let color = if chosen {
                on_color(accent)
            } else {
                th.label_text
            };
            self.text_centered(seg, label, &style(Face::UiBold, LABEL_PX, color));
            self.focus_ring(keys[i], seg, RADIUS - 2);
            self.register(keys[i], seg, HitKind::Tab, true);
        }
    }

    /// Round colour swatch with a ring and check mark when selected.
    pub fn swatch(&mut self, key: u32, r: Rect, color: u32, selected: bool) {
        let th = theme::current();
        let look = self.look(r, true);
        if selected {
            self.disc(r, th.label_text);
            self.disc(r.inset(2), th.panel_bg);
        } else if look != Look::Normal {
            self.disc(r, th.scroll_thumb);
        }
        let inner = r.inset(if selected { 5 } else { 3 });
        self.disc(inner, color);
        if selected {
            self.glyph(Glyph::Check, 16, inner, on_color(color));
        }
        self.focus_ring(key, r, r.w / 2);
        self.register(key, r, HitKind::Button, true);
    }

    /// List row: leading glyph, title with an optional second line, and an
    /// optional trailing button (`key`, label, kind, enabled).
    pub fn list_row(
        &mut self,
        r: Rect,
        glyph: Option<Glyph>,
        title: &str,
        subtitle: &str,
        trailing: Option<(u32, &str, ButtonKind, bool)>,
    ) {
        let th = theme::current();
        let mut text_x = r.x + 14;
        if let Some(glyph) = glyph {
            let icon = Rect::new(r.x + 12, r.y, 24, r.h);
            self.glyph(glyph, 20, icon, th.label_text_dim);
            text_x += 30;
        }
        let button_w = trailing.map_or(0, |(_, label, ..)| button_width(label));
        let text_w = r.w.saturating_sub((text_x - r.x) as u32 + button_w + 20);
        let title_style = style(Face::UiBold, LABEL_PX, th.label_text);
        let sub_style = style(Face::Ui, LABEL_PX - 2, th.label_text_dim);
        let two_lines = !subtitle.is_empty();
        let block = crate::display::text::line_height(&title_style)
            + if two_lines {
                crate::display::text::line_height(&sub_style)
            } else {
                0
            };
        let top = r.y + (r.h.saturating_sub(block) / 2) as i32;
        self.text_fitted(text_x, top, text_w, title, &title_style);
        if two_lines {
            let y = top + crate::display::text::line_height(&title_style) as i32;
            self.text_fitted(text_x, y, text_w, subtitle, &sub_style);
        }
        if let Some((key, label, kind, enabled)) = trailing {
            let b = Rect::new(
                r.right() - button_w as i32 - 12,
                r.y + (r.h.saturating_sub(CONTROL_H) / 2) as i32,
                button_w,
                CONTROL_H,
            );
            self.button(key, b, label, kind, enabled);
        }
    }
}

/// Horizontal padding inside a button around its label.
const BUTTON_PAD_X: u32 = 18;

/// Width a button needs for `label`.
#[must_use]
pub fn button_width(label: &str) -> u32 {
    let s = style(Face::UiBold, LABEL_PX, 0);
    crate::display::text::width(label, &s) + 2 * BUTTON_PAD_X
}
