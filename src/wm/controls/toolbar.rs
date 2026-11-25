//! Toolbar pieces: glyph buttons and the clickable breadcrumb bar.

use super::ui::{style, Look, Ui};
use super::{Rect, LABEL_PX, RADIUS};
use crate::display::icons::Glyph;
use crate::display::text::{self, Face};
use crate::wm::hit_region::HitKind;
use crate::wm::theme;

/// Glyph edge inside toolbar buttons.
const GLYPH_PX: u32 = 20;
/// Alpha of the hover wash.
const HOVER_ALPHA: u8 = 30;
/// Alpha of the pressed wash.
const PRESSED_ALPHA: u8 = 60;
/// Alpha of a disabled glyph.
const DISABLED_ALPHA: u8 = 90;
/// Horizontal padding inside a breadcrumb segment.
const CRUMB_PAD: u32 = 8;
/// Width of the chevron between breadcrumb segments.
pub const SEP_W: u32 = 16;
/// Corner radius of a breadcrumb segment wash.
const CRUMB_RADIUS: u32 = 5;

/// One breadcrumb segment to draw: its label and hit-region key.
#[derive(Clone, Copy, Debug)]
pub struct CrumbItem<'a> {
    pub key: u32,
    pub label: &'a str,
}

impl Ui<'_> {
    /// Square button showing `glyph`; draws a dimmed glyph and registers
    /// nothing when disabled.
    pub fn icon_button(&mut self, key: u32, r: Rect, glyph: Glyph, enabled: bool) {
        let th = theme::current();
        match self.look(r, enabled) {
            Look::Hover => self.blend(r, RADIUS, th.label_text, HOVER_ALPHA),
            Look::Pressed => self.blend(r, RADIUS, th.label_text, PRESSED_ALPHA),
            _ => {}
        }
        let color = if enabled {
            th.label_text
        } else {
            theme::blend_colors(th.label_text, th.panel_bg, u32::from(DISABLED_ALPHA))
        };
        self.glyph(glyph, GLYPH_PX, r, color);
        if enabled {
            self.register(key, r, HitKind::Button, false);
        }
    }

    /// Width a breadcrumb label needs, including its padding.
    #[must_use]
    pub fn crumb_width(label: &str) -> u32 {
        text::width(label, &style(Face::UiBold, LABEL_PX, 0)) + 2 * CRUMB_PAD
    }

    /// Breadcrumb bar over `r`: a field holding clickable segments
    /// separated by chevrons. `items` are laid out left to right.
    pub fn breadcrumbs(&mut self, r: Rect, items: &[CrumbItem]) -> alloc::vec::Vec<Rect> {
        let th = theme::current();
        self.ring(r, RADIUS, 1, th.input_border, th.input_bg);
        let mut x = r.x + 4;
        let last = items.len().saturating_sub(1);
        let mut rects = alloc::vec::Vec::new();
        for (i, item) in items.iter().enumerate() {
            let w = Self::crumb_width(item.label).min((r.right() - x).max(0) as u32);
            let seg = Rect::new(x, r.y + 3, w, r.h.saturating_sub(6));
            self.crumb(seg, item, i == last);
            rects.push(seg);
            x += w as i32;
            if i != last {
                let sep = Rect::new(x, r.y, SEP_W, r.h);
                self.glyph(Glyph::ChevronRight, 16, sep, th.label_text_dim);
                x += SEP_W as i32;
            }
        }
        rects
    }

    fn crumb(&mut self, seg: Rect, item: &CrumbItem, current: bool) {
        let th = theme::current();
        if self.look(seg, true) != Look::Normal {
            self.blend(seg, CRUMB_RADIUS, th.label_text, HOVER_ALPHA);
        }
        let color = if current {
            th.label_text
        } else {
            th.label_text_dim
        };
        let s = style(Face::UiBold, LABEL_PX, color);
        self.text_centered(seg, item.label, &s);
        self.register(item.key, seg, HitKind::Button, false);
    }
}
