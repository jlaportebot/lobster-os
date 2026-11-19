//! File-manager style item views: a table with sortable column headers and
//! rows, and a grid of large icon tiles with wrapped names.

use super::ui::{style, Look, Ui};
use super::{Rect, LABEL_PX};
use crate::display::icons::{self, Icon};
use crate::display::text::{self, Face};
use crate::wm::hit_region::HitKind;
use crate::wm::theme;
use alloc::string::String;
use alloc::vec::Vec;

/// Side of the small icon in a list row.
pub const ROW_ICON: u32 = 16;
/// Side of the large icon in a grid tile.
pub const TILE_ICON: u32 = 48;
/// Horizontal text inset inside a row or header cell.
const CELL_PAD: i32 = 10;
/// Gap between a row icon and its name.
const ICON_GAP: i32 = 8;
/// Alpha of the accent wash behind selected items.
const SELECTED_ALPHA: u8 = 70;
/// Alpha of the hover tint.
const HOVER_ALPHA: u8 = 22;
/// Corner radius of selection and hover washes.
const WASH_RADIUS: u32 = 6;
/// Text size of item names at the Medium text setting.
const NAME_PX: u32 = LABEL_PX;
/// Text size of the secondary columns.
const CELL_PX: u32 = LABEL_PX - 1;
/// Text size of the tile labels.
const TILE_PX: u32 = LABEL_PX - 1;
/// Name lines shown under a tile icon.
const TILE_LINES: usize = 2;
/// Gap between the tile icon and its label.
const TILE_GAP: i32 = 6;
/// Space above the tile icon.
const TILE_TOP: i32 = 8;
/// Horizontal padding of a tile label.
const TILE_PAD: u32 = 6;

/// One column of a table: where it sits and what it shows.
#[derive(Clone, Copy, Debug)]
pub struct Column<'a> {
    /// Hit-region key of the header cell.
    pub key: u32,
    pub x: i32,
    pub w: u32,
    pub label: &'a str,
    /// `Some(descending)` when the table is sorted by this column.
    pub sorted: Option<bool>,
    /// Right-align the cell text (numbers).
    pub right: bool,
}

/// Greedy wrap of `name` into at most `max_lines` lines no wider than
/// `max_w` according to `width_of`; the last line ends in "…" when text
/// was cut. Breaks at spaces when possible, else between characters.
#[must_use]
pub fn wrap_lines(
    name: &str,
    max_w: u32,
    width_of: &dyn Fn(&str) -> u32,
    max_lines: usize,
) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut rest = name;
    while !rest.is_empty() && lines.len() < max_lines {
        let last = lines.len() + 1 == max_lines;
        let (line, tail) = split_line(rest, max_w, width_of, last);
        lines.push(line);
        rest = tail.trim_start_matches(' ');
    }
    lines
}

fn split_line<'a>(
    rest: &'a str,
    max_w: u32,
    width_of: &dyn Fn(&str) -> u32,
    last: bool,
) -> (String, &'a str) {
    if width_of(rest) <= max_w {
        return (String::from(rest), "");
    }
    let mut cut = 0;
    for (at, c) in rest.char_indices() {
        let end = at + c.len_utf8();
        let probe_w = if last {
            width_of(&alloc::format!("{}\u{2026}", &rest[..end]))
        } else {
            width_of(&rest[..end])
        };
        if probe_w > max_w {
            break;
        }
        cut = end;
    }
    let cut = cut.max(rest.chars().next().map_or(0, char::len_utf8));
    if last {
        return (alloc::format!("{}\u{2026}", &rest[..cut]), "");
    }
    let at_space = rest[..cut].rfind(' ').filter(|s| *s > 0);
    let cut = at_space.unwrap_or(cut);
    (String::from(&rest[..cut]), &rest[cut..])
}

impl Ui<'_> {
    /// Selection / hover wash behind an item.
    fn item_wash(&self, r: Rect, selected: bool) {
        let th = theme::current();
        if selected {
            self.blend(r, WASH_RADIUS, theme::accent_color(), SELECTED_ALPHA);
        } else if self.look(r, true) != Look::Normal {
            self.blend(r, WASH_RADIUS, th.label_text, HOVER_ALPHA);
        }
    }

    /// Draw a colour sprite at `size` px centred in `r`.
    pub fn icon(&self, icon: Icon, size: u32, r: Rect) {
        let x = r.x + (r.w.saturating_sub(size) / 2) as i32;
        let y = r.y + (r.h.saturating_sub(size) / 2) as i32;
        let s = self.to_screen(Rect::new(x, y, size, size));
        icons::draw(&self.clip, icon, size, s.x.max(0) as u32, s.y.max(0) as u32);
    }

    /// Column header row over `r`: labels, a sort arrow on the sorted
    /// column and one hit region per column.
    pub fn column_header(&mut self, r: Rect, cols: &[Column]) {
        let th = theme::current();
        self.fill(r, th.panel_bg);
        self.fill(Rect::new(r.x, r.bottom() - 1, r.w, 1), th.panel_border);
        for col in cols {
            let cell = Rect::new(r.x + col.x, r.y, col.w, r.h - 1);
            if self.look(cell, true) != Look::Normal {
                self.blend(cell, 0, th.label_text, HOVER_ALPHA);
            }
            let s = style(Face::UiBold, CELL_PX, th.label_text_dim);
            let top = cell.y + (cell.h.saturating_sub(text::line_height(&s)) / 2) as i32;
            let arrow = if col.sorted.is_some() { 20 } else { 0 };
            let label_w = col.w.saturating_sub(2 * CELL_PAD as u32 + arrow);
            self.text_fitted(cell.x + CELL_PAD, top, label_w, col.label, &s);
            if let Some(descending) = col.sorted {
                self.sort_arrow(cell, descending, text::width(col.label, &s).min(label_w));
            }
            self.register(col.key, cell, HitKind::Button, false);
        }
    }

    fn sort_arrow(&self, cell: Rect, descending: bool, label_w: u32) {
        let glyph = if descending {
            icons::Glyph::ChevronDown
        } else {
            icons::Glyph::ChevronUp
        };
        let at = Rect::new(cell.x + CELL_PAD + label_w as i32 + 2, cell.y, 16, cell.h);
        self.glyph(glyph, 16, at, theme::current().label_text_dim);
    }

    /// One table row: small icon, name (fitted to `name_w`) and the other
    /// cells as `(x, w, text, right_aligned)`.
    pub fn item_row(
        &self,
        r: Rect,
        icon: Icon,
        name: (&str, u32),
        cells: &[(i32, u32, &str, bool)],
        selected: bool,
    ) {
        let th = theme::current();
        self.item_wash(r, selected);
        let icon_box = Rect::new(r.x + CELL_PAD, r.y, ROW_ICON, r.h);
        self.icon(icon, ROW_ICON, icon_box);
        let name_s = style(Face::Ui, NAME_PX, th.label_text);
        let top = r.y + (r.h.saturating_sub(text::line_height(&name_s)) / 2) as i32;
        let name_x = r.x + CELL_PAD + ROW_ICON as i32 + ICON_GAP;
        let avail = name
            .1
            .saturating_sub((name_x - r.x) as u32 + CELL_PAD as u32);
        self.text_fitted(name_x, top, avail, name.0, &name_s);
        let cell_s = style(Face::Ui, CELL_PX, th.label_text_dim);
        for &(x, w, s, right) in cells {
            self.cell_text(r, (x, w), s, right, &cell_s);
        }
    }

    fn cell_text(&self, row: Rect, col: (i32, u32), s: &str, right: bool, st: &text::TextStyle) {
        let top = row.y + (row.h.saturating_sub(text::line_height(st)) / 2) as i32;
        let inner = col.1.saturating_sub(2 * CELL_PAD as u32);
        let shown = text::width(s, st).min(inner);
        let x = if right {
            row.x + col.0 + CELL_PAD + (inner - shown) as i32
        } else {
            row.x + col.0 + CELL_PAD
        };
        self.text_fitted(x, top, inner, s, st);
    }

    /// One grid tile: large icon over a name wrapped to two lines.
    pub fn item_tile(&self, r: Rect, icon: Icon, name: &str, selected: bool) {
        let th = theme::current();
        self.item_wash(r.inset(2), selected);
        let icon_box = Rect::new(r.x, r.y + TILE_TOP, r.w, TILE_ICON);
        self.icon(icon, TILE_ICON, icon_box);
        let s = style(Face::Ui, TILE_PX, th.label_text);
        let max_w = r.w.saturating_sub(2 * TILE_PAD);
        let lines = wrap_lines(name, max_w, &|t| text::width(t, &s), TILE_LINES);
        let line_h = text::line_height(&s) as i32;
        let mut y = r.y + TILE_TOP + TILE_ICON as i32 + TILE_GAP;
        for line in &lines {
            let x = r.x + (r.w.saturating_sub(text::width(line, &s).min(max_w)) / 2) as i32;
            self.text_fitted(x, y, max_w, line, &s);
            y += line_h;
        }
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    fn eight(s: &str) -> u32 {
        s.chars().count() as u32 * 8
    }

    #[test]
    fn short_names_stay_on_one_line() {
        assert_eq!(wrap_lines("notes.txt", 100, &eight, 2), ["notes.txt"]);
    }

    #[test]
    fn long_names_break_at_spaces() {
        let lines = wrap_lines("quarterly report final", 100, &eight, 2);
        assert_eq!(lines, ["quarterly", "report final"]);
    }

    #[test]
    fn unbroken_names_split_between_characters() {
        let lines = wrap_lines("abcdefghijklmnopqrstu", 80, &eight, 3);
        assert_eq!(lines, ["abcdefghij", "klmnopqrst", "u"]);
    }

    #[test]
    fn overflow_ends_the_last_line_with_an_ellipsis() {
        let lines = wrap_lines("abcdefghijklmnopqrstuvwxyz0123456789", 80, &eight, 2);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], "abcdefghij");
        assert!(lines[1].ends_with('\u{2026}'));
        assert!(eight(&lines[1]) <= 80);
    }

    #[test]
    fn every_line_fits_the_width() {
        for name in ["a", "hello world again and again", "x".repeat(60).as_str()] {
            for line in wrap_lines(name, 64, &eight, 2) {
                assert!(eight(&line) <= 64, "{line:?}");
            }
        }
    }

    #[test]
    fn empty_names_produce_no_lines() {
        assert!(wrap_lines("", 80, &eight, 2).is_empty());
    }
}
