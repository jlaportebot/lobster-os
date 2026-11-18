//! Anti-aliased text engine.
//!
//! Glyphs come from atlases generated at build time by `tools/fontgen.py`
//! (see `assets/fonts/README.md`). Each atlas is a sorted record table plus
//! 8-bit coverage rows, read in place from the kernel image: lookup is a
//! binary search and drawing allocates nothing. All blending goes through
//! the clipped span path in [`super::graphics`].

use super::fb::FramebufferInfo;
use super::graphics;

/// Which typeface a [`TextStyle`] selects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Face {
    /// Proportional UI face (Inter Regular).
    Ui,
    /// Proportional UI face, semi-bold (Inter SemiBold).
    UiBold,
    /// Fixed-cell monospace face (DejaVu Sans Mono).
    Mono,
}

/// Font, pixel size and colour for one run of text.
#[derive(Clone, Copy, Debug)]
pub struct TextStyle {
    /// Typeface.
    pub face: Face,
    /// Requested size in px; the nearest generated atlas size is used.
    pub size: u32,
    /// Foreground colour, `0xAARRGGBB`.
    pub color: u32,
}

/// One generated atlas embedded in the kernel image.
struct AtlasData {
    face: Face,
    size: u32,
    bytes: &'static [u8],
}

static ATLASES: &[AtlasData] = &include!("../../assets/fonts/index.rs");

/// Ellipsis used by [`draw_fitted`].
const ELLIPSIS: char = '\u{2026}';
/// Header length of an atlas blob.
const HEADER_LEN: usize = 12;
/// Length of one glyph record.
const RECORD_LEN: usize = 12;
/// Magic bytes at the start of every atlas.
const ATLAS_MAGIC: &[u8; 4] = b"LFA1";
/// Colour of the box drawn for code points no atlas covers.
const MISSING_GLYPH_INSET: u32 = 1;
/// Mono cell metrics of the legacy 8x16 console cell.
const MONO_CELL_SIZE_PX: u32 = 13;

/// A parsed view of one atlas blob.
#[derive(Clone, Copy)]
struct Atlas {
    bytes: &'static [u8],
}

/// One glyph's metrics and coverage rows.
struct Glyph {
    advance: u32,
    bearing_x: i32,
    top: i32,
    w: u32,
    h: u32,
    rows: &'static [u8],
}

impl Atlas {
    fn byte(&self, at: usize) -> u32 {
        u32::from(self.bytes.get(at).copied().unwrap_or(0))
    }

    fn size(&self) -> u32 {
        self.byte(4)
    }

    fn cell_w(&self) -> u32 {
        self.byte(5)
    }

    fn ascent(&self) -> u32 {
        self.byte(6)
    }

    fn descent(&self) -> u32 {
        self.byte(7)
    }

    fn line_height(&self) -> u32 {
        self.byte(8)
    }

    fn count(&self) -> usize {
        (self.byte(10) | (self.byte(11) << 8)) as usize
    }

    fn record(&self, index: usize) -> Option<&'static [u8]> {
        let start = HEADER_LEN + index * RECORD_LEN;
        self.bytes.get(start..start + RECORD_LEN)
    }

    fn code_point_at(&self, index: usize) -> Option<u32> {
        let rec = self.record(index)?;
        Some(u32::from(rec[0]) | (u32::from(rec[1]) << 8))
    }

    fn pixel_base(&self) -> usize {
        HEADER_LEN + self.count() * RECORD_LEN
    }

    fn glyph(&self, cp: u32) -> Option<Glyph> {
        let (mut lo, mut hi) = (0usize, self.count());
        while lo < hi {
            let mid = (lo + hi) / 2;
            let found = self.code_point_at(mid)?;
            if found == cp {
                return self.decode(mid);
            }
            if found < cp {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        None
    }

    fn decode(&self, index: usize) -> Option<Glyph> {
        let rec = self.record(index)?;
        let (w, h) = (u32::from(rec[5]), u32::from(rec[6]));
        let offset = u32::from_le_bytes([rec[8], rec[9], rec[10], rec[11]]) as usize;
        let start = self.pixel_base() + offset;
        let rows = self.bytes.get(start..start + (w * h) as usize)?;
        Some(Glyph {
            advance: u32::from(rec[2]),
            bearing_x: i32::from(rec[3] as i8),
            top: i32::from(rec[4] as i8),
            w,
            h,
            rows,
        })
    }

    /// Advance for code points without a glyph: that of '0', else the cell.
    fn fallback_advance(&self) -> u32 {
        match self.glyph(u32::from('0')) {
            Some(zero) => zero.advance,
            None => self.cell_w().max(1),
        }
    }

    fn advance_of(&self, c: char) -> u32 {
        match self.glyph(c as u32) {
            Some(glyph) => glyph.advance,
            None => self.fallback_advance(),
        }
    }
}

fn atlas_for(face: Face, size: u32) -> Atlas {
    let mut best: Option<&AtlasData> = None;
    for candidate in ATLASES.iter().filter(|a| a.face == face) {
        let closer = match best {
            None => true,
            Some(current) => {
                let (new, old) = (candidate.size.abs_diff(size), current.size.abs_diff(size));
                new < old || (new == old && candidate.size > current.size)
            }
        };
        if closer {
            best = Some(candidate);
        }
    }
    Atlas {
        bytes: best.map_or(&[][..], |a| a.bytes),
    }
}

fn atlas(style: &TextStyle) -> Atlas {
    atlas_for(style.face, style.size)
}

/// Pixel size of the atlas that [`TextStyle`] would select; useful to
/// callers that want to know what they will actually get.
#[must_use]
pub fn resolved_size(style: &TextStyle) -> u32 {
    atlas(style).size()
}

/// Height of one line box in px (ascent plus descent of the chosen atlas).
#[must_use]
pub fn line_height(style: &TextStyle) -> u32 {
    atlas(style).line_height()
}

/// Distance from the top of the line box to the baseline in px.
#[must_use]
pub fn ascent(style: &TextStyle) -> u32 {
    atlas(style).ascent()
}

/// Depth of the line box below the baseline in px.
#[must_use]
pub fn descent(style: &TextStyle) -> u32 {
    atlas(style).descent()
}

/// Advance width of `text` in px: the sum of its glyph advances.
#[must_use]
pub fn width(text: &str, style: &TextStyle) -> u32 {
    let atlas = atlas(style);
    text.chars().map(|c| atlas.advance_of(c)).sum()
}

fn draw_glyph(fb: &FramebufferInfo, x: i64, y: i64, glyph: &Glyph, color: u32) {
    let left = x + i64::from(glyph.bearing_x);
    let skip_cols = (-left).clamp(0, i64::from(glyph.w)) as usize;
    for row in 0..glyph.h {
        let py = y + i64::from(glyph.top) + i64::from(row);
        if py < 0 {
            continue;
        }
        let start = (row * glyph.w) as usize;
        let Some(coverage) = glyph.rows.get(start + skip_cols..start + glyph.w as usize) else {
            continue;
        };
        graphics::draw_alpha_row(
            fb,
            (left + skip_cols as i64) as u32,
            py as u32,
            coverage,
            color,
        );
    }
}

fn draw_missing(fb: &FramebufferInfo, x: i64, y: i64, atlas: &Atlas, color: u32) {
    let advance = atlas.fallback_advance();
    let box_w = advance.saturating_sub(2 * MISSING_GLYPH_INSET + 1).max(2);
    let box_h = (atlas.ascent() * 7 / 10).max(2);
    let top = y + i64::from(atlas.ascent()) - i64::from(box_h);
    if x < 0 || top < 0 {
        return;
    }
    graphics::draw_rect_outline(
        fb,
        x as u32 + MISSING_GLYPH_INSET,
        top as u32,
        box_w,
        box_h,
        color,
    );
}

fn draw_char(fb: &FramebufferInfo, x: i64, y: i64, c: char, atlas: &Atlas, color: u32) -> u32 {
    match atlas.glyph(c as u32) {
        Some(glyph) => {
            draw_glyph(fb, x, y, &glyph, color);
            glyph.advance
        }
        None => {
            draw_missing(fb, x, y, atlas, color);
            atlas.fallback_advance()
        }
    }
}

/// Draw `text` with its line box starting at (`x`, `y`) (`y` is the top of
/// the line). Returns the advance in px. Nothing is written outside the
/// target's clip.
pub fn draw(fb: &FramebufferInfo, x: u32, y: u32, text: &str, style: &TextStyle) -> u32 {
    let atlas = atlas(style);
    let mut pen = 0u32;
    for c in text.chars() {
        pen += draw_char(
            fb,
            i64::from(x) + i64::from(pen),
            i64::from(y),
            c,
            &atlas,
            style.color,
        );
    }
    pen
}

/// Draw `text` horizontally centred on `center_x`.
pub fn draw_centered(fb: &FramebufferInfo, center_x: u32, y: u32, text: &str, style: &TextStyle) {
    let x = center_x.saturating_sub(width(text, style) / 2);
    draw(fb, x, y, text, style);
}

/// Number of leading chars of `text` that fit in `max_w` together with an
/// ellipsis, and whether truncation is needed at all. Pure, so it can be
/// tested without a render target.
#[must_use]
pub fn fit_prefix(text: &str, max_w: u32, style: &TextStyle) -> (usize, bool) {
    let atlas = atlas(style);
    let total: u32 = text.chars().map(|c| atlas.advance_of(c)).sum();
    if total <= max_w {
        return (text.chars().count(), false);
    }
    let budget = max_w.saturating_sub(atlas.advance_of(ELLIPSIS));
    let mut used = 0u32;
    let mut kept = 0usize;
    for c in text.chars() {
        used += atlas.advance_of(c);
        if used > budget {
            break;
        }
        kept += 1;
    }
    (kept, true)
}

/// Draw `text` into at most `max_w` px, ending with "…" when it had to be
/// shortened. Returns the advance actually used (never above `max_w`).
pub fn draw_fitted(
    fb: &FramebufferInfo,
    x: u32,
    y: u32,
    max_w: u32,
    text: &str,
    style: &TextStyle,
) -> u32 {
    let (kept, truncated) = fit_prefix(text, max_w, style);
    if !truncated {
        return draw(fb, x, y, text, style);
    }
    let atlas = atlas(style);
    if atlas.advance_of(ELLIPSIS) > max_w {
        return 0;
    }
    let end = text.char_indices().nth(kept).map_or(text.len(), |(i, _)| i);
    let used = draw(fb, x, y, &text[..end], style);
    let mut buf = [0u8; 4];
    used + draw(fb, x + used, y, ELLIPSIS.encode_utf8(&mut buf), style)
}

/// Style of the mono face matching the legacy 8x16 cell.
#[must_use]
pub fn mono_cell_style(color: u32) -> TextStyle {
    TextStyle {
        face: Face::Mono,
        size: MONO_CELL_SIZE_PX,
        color,
    }
}

/// Draw one character into the mono cell at (`x`, `y`). When `bg` is
/// `Some`, the whole cell is filled with it first. Returns the cell width.
pub fn draw_cell(fb: &FramebufferInfo, x: u32, y: u32, c: char, fg: u32, bg: Option<u32>) -> u32 {
    let style = mono_cell_style(fg);
    let atlas = atlas(&style);
    if let Some(bg) = bg {
        graphics::draw_rect(fb, x, y, atlas.cell_w(), atlas.line_height(), bg);
    }
    draw_char(fb, i64::from(x), i64::from(y), c, &atlas, fg);
    atlas.cell_w()
}

/// Visit every pixel of the 8x16 mono cell for `c` as `(col, row, coverage)`,
/// including uncovered ones, for callers that composite on their own.
pub fn for_each_cell_pixel(c: char, mut visit: impl FnMut(u32, u32, u8)) {
    let atlas = atlas(&mono_cell_style(0));
    let glyph = atlas.glyph(c as u32);
    for row in 0..atlas.line_height() {
        for col in 0..atlas.cell_w() {
            let coverage = glyph.as_ref().map_or(0, |g| glyph_coverage(g, col, row));
            visit(col, row, coverage);
        }
    }
}

fn glyph_coverage(glyph: &Glyph, col: u32, row: u32) -> u8 {
    let gx = col as i32 - glyph.bearing_x;
    let gy = row as i32 - glyph.top;
    if gx < 0 || gy < 0 || gx >= glyph.w as i32 || gy >= glyph.h as i32 {
        return 0;
    }
    glyph
        .rows
        .get((gy as u32 * glyph.w + gx as u32) as usize)
        .copied()
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const REQUIRED_SYMBOLS: &str = "•…–—‘’“”←↑→↓✓✕×÷−±°⌫▲▼◀▶☰⏎⇧⌘★●";
    const UI_SIZES: [u32; 7] = [11, 12, 13, 14, 16, 20, 24];
    const BOLD_SIZES: [u32; 6] = [12, 13, 14, 16, 20, 24];
    const BG: u32 = 0xFF102030;
    const FG: u32 = 0xFFF0E0D0;

    fn style(face: Face, size: u32) -> TextStyle {
        TextStyle {
            face,
            size,
            color: FG,
        }
    }

    fn required_chars() -> impl Iterator<Item = char> {
        let ascii = (0x20u32..0x7F).filter_map(char::from_u32);
        let latin = (0xA0u32..0x100).filter_map(char::from_u32);
        ascii.chain(latin).chain(REQUIRED_SYMBOLS.chars())
    }

    /// An in-memory target with a canary border around the clip.
    struct Canvas {
        pixels: Vec<u32>,
        w: u32,
        h: u32,
    }

    impl Canvas {
        fn new(w: u32, h: u32) -> Self {
            Self {
                pixels: vec![graphics::fx(BG); (w * h) as usize],
                w,
                h,
            }
        }

        fn target(&mut self) -> FramebufferInfo {
            let screen = FramebufferInfo::empty();
            // SAFETY: the buffer holds w*h pixels and outlives the target
            // within each test.
            unsafe { screen.surface(self.pixels.as_mut_ptr(), 0, 0, self.w, self.h) }
        }

        fn at(&self, x: u32, y: u32) -> u32 {
            self.pixels[(y * self.w + x) as usize]
        }
    }

    #[test]
    fn every_required_code_point_resolves_in_each_face_and_size() {
        for (face, sizes) in [(Face::Ui, &UI_SIZES[..]), (Face::UiBold, &BOLD_SIZES[..])] {
            for &size in sizes {
                let atlas = atlas(&style(face, size));
                assert_eq!(atlas.size(), size);
                for c in required_chars().filter(|&c| c != '\u{23FB}') {
                    assert!(
                        atlas.glyph(c as u32).is_some(),
                        "{face:?} {size} U+{:04X}",
                        c as u32
                    );
                }
            }
        }
        let mono = atlas(&style(Face::Mono, 13));
        assert!(required_chars().all(|c| mono.glyph(c as u32).is_some()));
    }

    #[test]
    fn unknown_code_point_advances_like_zero() {
        let s = style(Face::Ui, 13);
        assert_eq!(width("\u{4E2D}", &s), width("0", &s));
    }

    #[test]
    fn width_is_sum_of_advances_and_stable() {
        let s = style(Face::Ui, 14);
        let sum: u32 = "Hello".chars().map(|c| width(&c.to_string(), &s)).sum();
        assert_eq!(width("Hello", &s), sum);
        assert_eq!(width("Hello", &s), width("Hello", &s));
        assert!(sum > 0);
    }

    #[test]
    fn nearest_atlas_size_is_used() {
        assert_eq!(resolved_size(&style(Face::Ui, 15)), 16);
        assert_eq!(resolved_size(&style(Face::Ui, 100)), 24);
        assert_eq!(resolved_size(&style(Face::UiBold, 11)), 12);
        assert_eq!(resolved_size(&style(Face::Mono, 14)), 13);
    }

    #[test]
    fn atlas_metrics_are_consistent() {
        for data in ATLASES {
            let atlas = Atlas { bytes: data.bytes };
            assert_eq!(&data.bytes[..4], ATLAS_MAGIC);
            assert_eq!(atlas.size(), data.size);
            assert_eq!(atlas.ascent() + atlas.descent(), atlas.line_height());
            assert!(
                atlas.line_height() >= atlas.size(),
                "{:?} {}",
                data.face,
                data.size
            );
            assert!(atlas.count() > 200);
        }
    }

    #[test]
    fn mono_cell_is_exactly_8x16() {
        let s = mono_cell_style(FG);
        assert_eq!((width("ab", &s), line_height(&s)), (16, 16));
        let atlas = atlas(&s);
        assert!(required_chars().all(|c| atlas.advance_of(c) == 8));
    }

    #[test]
    fn fitted_text_never_exceeds_max_width() {
        let s = style(Face::Ui, 13);
        let text = "A rather long window title that cannot possibly fit";
        for max_w in [0, 5, 12, 40, 100, 200] {
            let mut canvas = Canvas::new(300, 30);
            let used = draw_fitted(&canvas.target(), 2, 2, max_w, text, &s);
            assert!(used <= max_w, "max_w {max_w} used {used}");
        }
    }

    #[test]
    fn fit_prefix_truncates_with_room_for_ellipsis() {
        let s = style(Face::Ui, 13);
        let text = "A rather long window title";
        let (kept, truncated) = fit_prefix(text, 80, &s);
        assert!(truncated);
        let head: String = text.chars().take(kept).collect();
        assert!(width(&head, &s) + width("…", &s) <= 80);
        assert_eq!(fit_prefix("Hi", 80, &s), (2, false));
    }

    #[test]
    fn drawing_never_touches_pixels_outside_the_clip() {
        let mut canvas = Canvas::new(120, 40);
        let clip = canvas.target().clipped(30, 10, 70, 30);
        draw(&clip, 20, 8, "Clipped text WWW", &style(Face::UiBold, 20));
        let mut touched = 0;
        for y in 0..canvas.h {
            for x in 0..canvas.w {
                let inside = (30..70).contains(&x) && (10..30).contains(&y);
                if !inside {
                    assert_eq!(canvas.at(x, y), graphics::fx(BG), "({x},{y})");
                } else if canvas.at(x, y) != graphics::fx(BG) {
                    touched += 1;
                }
            }
        }
        assert!(touched > 0);
    }

    #[test]
    fn solid_glyph_pixels_equal_foreground_and_empty_keep_background() {
        let mut canvas = Canvas::new(40, 30);
        draw(&canvas.target(), 4, 4, "I", &style(Face::UiBold, 24));
        let solid = canvas
            .pixels
            .iter()
            .filter(|&&p| p == graphics::fx(FG))
            .count();
        let untouched = canvas
            .pixels
            .iter()
            .filter(|&&p| p == graphics::fx(BG))
            .count();
        assert!(solid > 10);
        assert_eq!(canvas.at(0, 0), graphics::fx(BG));
        assert!(untouched > solid);
    }

    #[test]
    fn partial_coverage_blends_between_background_and_foreground() {
        let mut canvas = Canvas::new(40, 30);
        draw(&canvas.target(), 4, 4, "o", &style(Face::Ui, 20));
        let mixed = canvas
            .pixels
            .iter()
            .any(|&p| p != graphics::fx(BG) && p != graphics::fx(FG));
        assert!(mixed);
    }

    #[test]
    fn cell_fill_covers_exactly_the_cell() {
        let mut canvas = Canvas::new(32, 32);
        let advance = draw_cell(&canvas.target(), 8, 8, ' ', FG, Some(0xFFFF0000));
        assert_eq!(advance, 8);
        assert_eq!(canvas.at(8, 8), graphics::fx(0xFFFF0000));
        assert_eq!(canvas.at(15, 23), graphics::fx(0xFFFF0000));
        assert_eq!(canvas.at(16, 8), graphics::fx(BG));
        assert_eq!(canvas.at(8, 24), graphics::fx(BG));
    }

    #[test]
    fn cell_pixel_visitor_matches_cell_dimensions() {
        let mut count = 0;
        let mut lit = 0;
        for_each_cell_pixel('H', |_, _, cov| {
            count += 1;
            lit += usize::from(cov == 255);
        });
        assert_eq!(count, 8 * 16);
        assert!(lit > 4);
    }

    #[test]
    fn centered_text_is_symmetric_about_the_centre() {
        let s = style(Face::Ui, 13);
        let mut canvas = Canvas::new(200, 30);
        draw_centered(&canvas.target(), 100, 4, "Title", &s);
        let first = (0..200)
            .find(|&x| (0..30).any(|y| canvas.at(x, y) != graphics::fx(BG)))
            .unwrap_or(0);
        let last = (0..200)
            .rev()
            .find(|&x| (0..30).any(|y| canvas.at(x, y) != graphics::fx(BG)))
            .unwrap_or(0);
        assert!((100 - first as i32 - (last as i32 - 100)).abs() <= 3);
    }
}
