//! Appearance: theme, accent colour, wallpaper gallery, large text.

use super::{heading, setting, Flow};
use crate::apps::settings::model::Action;
use crate::display::text::Face;
use crate::wm::controls::{style, Rect, Ui, LABEL_PX};
use crate::wm::hit_region::HitKind;
use crate::wm::theme::{self, ThemeId};
use crate::wm::wallpaper;

/// Width of the theme segmented control.
const THEME_W: u32 = 200;
/// Accent swatch diameter and spacing.
const SWATCH_D: u32 = 36;
const SWATCH_GAP: u32 = 12;
/// Wallpaper thumbnail size and spacing.
const THUMB_W: u32 = 132;
const THUMB_H: u32 = 78;
const THUMB_GAP: u32 = 14;
/// Height of the caption under a thumbnail.
const CAPTION_H: u32 = 24;
/// Width of the large-text switch cell.
const SWITCH_CELL_W: u32 = 44;
/// Corner inset of the first rows of a rounded gradient.
const CORNER_INSETS: [u32; 3] = [3, 1, 1];

pub fn draw(ui: &mut Ui, flow: &mut Flow) {
    heading(ui, flow, "THEME");
    let seg = setting(
        ui,
        flow,
        "Colour scheme",
        "Applies to every window, the taskbar and menus",
        THEME_W,
    );
    let keys = [Action::Theme(0).key(), Action::Theme(1).key()];
    let chosen = usize::from(theme::active_id() == ThemeId::Light);
    ui.segmented(&keys, &["Dark", "Light"], seg, chosen);
    flow.space(8);
    heading(ui, flow, "ACCENT COLOUR");
    accent_row(ui, flow);
    flow.space(8);
    heading(ui, flow, "WALLPAPER");
    wallpaper_gallery(ui, flow);
    flow.space(8);
    heading(ui, flow, "TEXT");
    let cell = setting(
        ui,
        flow,
        "Large text",
        "Bigger text in the taskbar, menus and window frames",
        SWITCH_CELL_W,
    );
    let on = theme::font_size() == theme::FontSize::Large;
    ui.switch(Action::LargeText.key(), cell.x, cell.y + 4, on, true);
}

fn accent_row(ui: &mut Ui, flow: &mut Flow) {
    let row = flow.take(SWATCH_D + 28);
    ui.card(row);
    let current = theme::accent_color();
    for (i, (_, argb)) in theme::ACCENTS.iter().enumerate() {
        let x = row.x + 16 + (i as u32 * (SWATCH_D + SWATCH_GAP + 24)) as i32;
        let swatch = Rect::new(x, row.y + 14, SWATCH_D, SWATCH_D);
        ui.swatch(
            Action::Accent(i as u8).key(),
            swatch,
            *argb,
            *argb == current,
        );
    }
    let caption = style(
        Face::Ui,
        LABEL_PX - 2,
        crate::wm::theme::current().label_text_dim,
    );
    let active = theme::ACCENTS
        .iter()
        .find(|(_, c)| *c == current)
        .map_or("Custom", |(n, _)| n);
    let label_w = crate::display::text::width(active, &caption);
    ui.text(
        row.right() - 16 - label_w as i32,
        row.y + 20,
        active,
        &caption,
    );
}

fn wallpaper_gallery(ui: &mut Ui, flow: &mut Flow) {
    let per_row = ((flow.w + THUMB_GAP) / (THUMB_W + THUMB_GAP)).max(1) as usize;
    let rows = wallpaper::PRESETS.len().div_ceil(per_row) as u32;
    let block = flow.take(rows * (THUMB_H + CAPTION_H + THUMB_GAP));
    let active = wallpaper::active_index();
    for (i, preset) in wallpaper::PRESETS.iter().enumerate() {
        let col = (i % per_row) as u32;
        let line = (i / per_row) as u32;
        let x = block.x + (col * (THUMB_W + THUMB_GAP)) as i32;
        let y = block.y + (line * (THUMB_H + CAPTION_H + THUMB_GAP)) as i32;
        let thumb = Rect::new(x, y, THUMB_W, THUMB_H);
        thumbnail(
            ui,
            Action::Wallpaper(i as u8).key(),
            thumb,
            preset,
            i == active,
        );
    }
}

fn thumbnail(ui: &mut Ui, key: u32, r: Rect, preset: &wallpaper::Preset, selected: bool) {
    let th = theme::current();
    if selected {
        ui.rounded(
            Rect::new(r.x - 3, r.y - 3, r.w + 6, r.h + 6),
            8,
            theme::accent_color(),
        );
    }
    gradient(ui, r, preset.top, preset.bottom);
    ui.outline(r, th.panel_border);
    let color = if selected {
        th.label_text
    } else {
        th.label_text_dim
    };
    let s = style(Face::Ui, LABEL_PX - 1, color);
    ui.text_fitted(r.x, r.bottom() + 5, r.w, preset.name, &s);
    ui.register(
        key,
        Rect::new(r.x, r.y, r.w, r.h + CAPTION_H),
        HitKind::Button,
        true,
    );
}

/// Vertical gradient between two ARGB colours with softened corners.
fn gradient(ui: &Ui, r: Rect, top: u32, bottom: u32) {
    for row in 0..r.h {
        let t = row * 255 / r.h.max(1);
        let color = blend(top, bottom, t);
        let from_edge = row.min(r.h - 1 - row) as usize;
        let inset = CORNER_INSETS.get(from_edge).copied().unwrap_or(0);
        ui.fill(
            Rect::new(r.x + inset as i32, r.y + row as i32, r.w - 2 * inset, 1),
            color,
        );
    }
}

fn blend(a: u32, b: u32, t: u32) -> u32 {
    let mix = |shift: u32| {
        let (x, y) = ((a >> shift) & 0xFF, (b >> shift) & 0xFF);
        ((x * (255 - t) + y * t) / 255) << shift
    };
    0xFF00_0000 | mix(16) | mix(8) | mix(0)
}
