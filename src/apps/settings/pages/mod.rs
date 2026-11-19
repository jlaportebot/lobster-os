//! The content of each Settings page, built from shared row helpers.

mod about;
mod appearance;
mod apps;
mod datetime;
mod display;
mod network;
mod sound;
mod storage;
mod users;

use super::model::Page;
use super::state::State;
use crate::display::text::{self, Face};
use crate::wm::controls::{style, Rect, Ui, CONTROL_H, LABEL_PX};
use crate::wm::theme;
use alloc::string::String;

/// Height of a setting row without a description line.
const ROW_H: u32 = 56;
/// Height of a setting row with a description line.
const ROW_TALL_H: u32 = 64;
/// Vertical gap between rows.
const ROW_GAP: u32 = 8;
/// Height of a key/value line inside an info card.
const INFO_LINE_H: u32 = 36;
/// Inner horizontal padding of cards.
const CARD_PAD: i32 = 16;
/// Space below the last row so the end of a page is not flush with the edge.
const BOTTOM_PAD: i32 = 24;
/// Section heading height.
const HEADING_H: u32 = 28;

/// Vertical cursor that hands out rows of one content column.
pub struct Flow {
    pub x: i32,
    pub y: i32,
    pub w: u32,
}

impl Flow {
    /// The next `h` px tall row; the cursor moves below it plus the gap.
    pub fn take(&mut self, h: u32) -> Rect {
        let r = Rect::new(self.x, self.y, self.w, h);
        self.y += (h + ROW_GAP) as i32;
        r
    }

    /// Extra space before the next row.
    pub fn space(&mut self, n: u32) {
        self.y += n as i32;
    }

    /// Height used so far relative to `top`, with bottom padding.
    #[must_use]
    pub fn height_from(&self, top: i32) -> u32 {
        (self.y - top + BOTTOM_PAD).max(0) as u32
    }
}

/// Draw `page` into `flow` and return nothing; the caller measures `flow.y`.
pub fn draw(page: Page, ui: &mut Ui, flow: &mut Flow, st: &mut State) {
    match page {
        Page::Appearance => appearance::draw(ui, flow),
        Page::Display => display::draw(ui, flow, st),
        Page::Sound => sound::draw(ui, flow, st),
        Page::Network => network::draw(ui, flow, st),
        Page::Users => users::draw(ui, flow, st),
        Page::DateTime => datetime::draw(ui, flow),
        Page::Storage => storage::draw(ui, flow),
        Page::Apps => apps::draw(ui, flow, st),
        Page::About => about::draw(ui, flow),
    }
}

/// Small section label above a group of rows.
fn heading(ui: &Ui, flow: &mut Flow, label: &str) {
    let th = theme::current();
    let s = style(Face::UiBold, LABEL_PX - 1, th.label_text_dim);
    let r = flow.take(HEADING_H);
    flow.y -= ROW_GAP as i32;
    let top = r.y + (r.h.saturating_sub(text::line_height(&s)) / 2) as i32;
    ui.text(r.x + 4, top, label, &s);
}

/// A card with a title (and optional description) on the left; returns the
/// rectangle on the right where the control goes, `ctrl_w` px wide.
fn setting(ui: &Ui, flow: &mut Flow, title: &str, desc: &str, ctrl_w: u32) -> Rect {
    let th = theme::current();
    let h = if desc.is_empty() { ROW_H } else { ROW_TALL_H };
    let card = flow.take(h);
    ui.card(card);
    let title_style = style(Face::UiBold, LABEL_PX, th.label_text);
    let desc_style = style(Face::Ui, LABEL_PX - 2, th.label_text_dim);
    let lines = text::line_height(&title_style)
        + if desc.is_empty() {
            0
        } else {
            text::line_height(&desc_style)
        };
    let top = card.y + (card.h.saturating_sub(lines) / 2) as i32;
    let text_w = card.w.saturating_sub(ctrl_w + 3 * CARD_PAD as u32);
    ui.text_fitted(card.x + CARD_PAD, top, text_w, title, &title_style);
    if !desc.is_empty() {
        let y = top + text::line_height(&title_style) as i32;
        ui.text_fitted(card.x + CARD_PAD, y, text_w, desc, &desc_style);
    }
    let cy = card.y + (card.h.saturating_sub(CONTROL_H) / 2) as i32;
    Rect::new(
        card.right() - CARD_PAD - ctrl_w as i32,
        cy,
        ctrl_w,
        CONTROL_H,
    )
}

/// One card holding aligned `label  value` lines.
fn info_card(ui: &Ui, flow: &mut Flow, lines: &[(&str, String)]) {
    let th = theme::current();
    let card = flow.take(lines.len() as u32 * INFO_LINE_H);
    ui.card(card);
    let key_style = style(Face::Ui, LABEL_PX, th.label_text_dim);
    let value_style = style(Face::Ui, LABEL_PX, th.label_text);
    let value_x = card.x + card.w as i32 / 3;
    let value_w = (card.right() - CARD_PAD - value_x).max(0) as u32;
    for (i, (label, value)) in lines.iter().enumerate() {
        let line_y = card.y + (i as u32 * INFO_LINE_H) as i32;
        let top = line_y + (INFO_LINE_H.saturating_sub(text::line_height(&key_style)) / 2) as i32;
        if i > 0 {
            ui.fill(
                Rect::new(card.x + CARD_PAD, line_y, card.w - 2 * CARD_PAD as u32, 1),
                th.panel_border,
            );
        }
        ui.text_fitted(
            card.x + CARD_PAD,
            top,
            card.w / 3 - CARD_PAD as u32,
            label,
            &key_style,
        );
        ui.text_fitted(value_x, top, value_w, value, &value_style);
    }
}

/// A dimmed explanation line under a row.
fn note(ui: &Ui, flow: &mut Flow, message: &str) {
    if message.is_empty() {
        return;
    }
    let th = theme::current();
    let s = style(Face::Ui, LABEL_PX - 1, th.label_text_dim);
    let r = flow.take(text::line_height(&s) + 4);
    ui.text_fitted(r.x + 4, r.y, r.w, message, &s);
}
