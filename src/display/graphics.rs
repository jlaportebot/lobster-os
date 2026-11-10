//! 2D drawing primitives.
//!
//! Every primitive takes a render target ([`FramebufferInfo`]) and screen
//! coordinates. A target may be the device framebuffer, the compositor's
//! back buffer, or a window surface; it carries an origin and a clip
//! rectangle, and all writes go through [`span`], which is the only place
//! that turns coordinates into a pointer. Nothing can write outside the
//! target's clip.
//!
//! Colours are `0xAARRGGBB`. [`fx`] converts to the byte order the
//! BCM2835-style framebuffer scans out; surfaces store converted pixels so
//! compositing is a plain copy.

use super::console::FONT;
use super::console::{CHAR_HEIGHT, CHAR_WIDTH};
use super::fb::FramebufferInfo;

const FULL_ALPHA: u32 = 255;
const OPAQUE: u32 = 0xFF00_0000;

/// A clipped horizontal run of pixels inside a render target.
struct Span {
    ptr: *mut u32,
    len: usize,
    /// Pixels cut off on the left by clipping.
    skipped: usize,
}

impl Span {
    fn pixels(&mut self) -> &mut [u32] {
        // SAFETY: `span` only builds a Span whose `ptr..ptr+len` lies inside
        // the target buffer (the clip rectangle is contained in the buffer
        // by `FramebufferInfo`'s constructors), and the kernel renders from
        // a single thread, so no other reference to these pixels is live.
        unsafe { core::slice::from_raw_parts_mut(self.ptr, self.len) }
    }
}

/// Clip the run `[x, x + w)` on row `y` to the target.
fn span(fb: &FramebufferInfo, x: u32, y: u32, w: u32) -> Option<Span> {
    if y < fb.clip_y0 || y >= fb.clip_y1 || fb.virt_addr == 0 {
        return None;
    }
    let x0 = x.max(fb.clip_x0);
    let x1 = x.saturating_add(w).min(fb.clip_x1);
    if x0 >= x1 {
        return None;
    }
    let stride = (fb.pitch / 4) as usize;
    let offset = (y - fb.origin_y) as usize * stride + (x0 - fb.origin_x) as usize;
    // SAFETY: (x0, y) is inside the clip rectangle, which lies inside the
    // buffer starting at `virt_addr`, so the offset stays in bounds.
    let ptr = unsafe { (fb.virt_addr as *mut u32).add(offset) };
    Some(Span {
        ptr,
        len: (x1 - x0) as usize,
        skipped: (x0 - x) as usize,
    })
}

/// Convert `0xAARRGGBB` to the framebuffer's stored pixel format
/// (`0xAABBGGRR`, alpha forced opaque).
#[inline]
#[must_use]
pub fn fx(argb: u32) -> u32 {
    let r = (argb >> 16) & 0xFF;
    let g = (argb >> 8) & 0xFF;
    let b = argb & 0xFF;
    OPAQUE | (b << 16) | (g << 8) | r
}

/// Blend stored-format `src` over stored-format `dst` with coverage 0..=255.
#[inline]
fn blend(dst: u32, src: u32, alpha: u32) -> u32 {
    let inverse = FULL_ALPHA - alpha;
    let mix = |shift: u32| {
        let s = (src >> shift) & 0xFF;
        let d = (dst >> shift) & 0xFF;
        ((s * alpha + d * inverse + 127) / FULL_ALPHA) << shift
    };
    OPAQUE | mix(16) | mix(8) | mix(0)
}

fn fill_span(fb: &FramebufferInfo, x: u32, y: u32, w: u32, pixel: u32) {
    if let Some(mut run) = span(fb, x, y, w) {
        run.pixels().fill(pixel);
    }
}

pub fn draw_pixel(fb: &FramebufferInfo, x: u32, y: u32, color: u32) {
    fill_span(fb, x, y, 1, fx(color));
}

/// Fill a solid rectangle.
pub fn draw_rect(fb: &FramebufferInfo, x: u32, y: u32, w: u32, h: u32, color: u32) {
    let pixel = fx(color);
    let y_start = y.max(fb.clip_y0);
    let y_end = y.saturating_add(h).min(fb.clip_y1);
    for row in y_start..y_end {
        fill_span(fb, x, row, w, pixel);
    }
}

/// Fill a rectangle, blending `color` over what is there (alpha 0..=255).
pub fn blend_rect(fb: &FramebufferInfo, x: u32, y: u32, w: u32, h: u32, color: u32, alpha: u8) {
    let pixel = fx(color);
    let y_start = y.max(fb.clip_y0);
    let y_end = y.saturating_add(h).min(fb.clip_y1);
    for row in y_start..y_end {
        if let Some(mut run) = span(fb, x, row, w) {
            for dst in run.pixels() {
                *dst = blend(*dst, pixel, u32::from(alpha));
            }
        }
    }
}

/// Filled rounded rectangle (solid).
pub fn fill_rounded_rect(
    fb: &FramebufferInfo,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    radius: u32,
    color: u32,
) {
    if w == 0 || h == 0 {
        return;
    }
    let r = radius.min(w / 2).min(h / 2);
    draw_rect(fb, x, y + r, w, h - 2 * r, color);
    for dy in 0..r {
        let inset = corner_inset(r, dy);
        draw_rect(fb, x + inset, y + dy, w - 2 * inset, 1, color);
        draw_rect(fb, x + inset, y + h - 1 - dy, w - 2 * inset, 1, color);
    }
}

/// Horizontal inset of a rounded corner at `dy` rows from the edge.
fn corner_inset(r: u32, dy: u32) -> u32 {
    let a = r - dy;
    let val = (r * r).saturating_sub(a * a);
    let mut s = 0u32;
    while s * s < val {
        s += 1;
    }
    r.saturating_sub(s)
}

/// One-pixel rectangle outline.
pub fn draw_rect_outline(fb: &FramebufferInfo, x: u32, y: u32, w: u32, h: u32, color: u32) {
    if w == 0 || h == 0 {
        return;
    }
    draw_rect(fb, x, y, w, 1, color);
    draw_rect(fb, x, y + h - 1, w, 1, color);
    draw_rect(fb, x, y, 1, h, color);
    draw_rect(fb, x + w - 1, y, 1, h, color);
}

/// Bresenham line.
pub fn draw_line(fb: &FramebufferInfo, x0: u32, y0: u32, x1: u32, y1: u32, color: u32) {
    let (x0, y0, x1, y1) = (i64::from(x0), i64::from(y0), i64::from(x1), i64::from(y1));
    let dx = (x1 - x0).abs();
    let dy = -(y1 - y0).abs();
    let step_x = if x0 < x1 { 1 } else { -1 };
    let step_y = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    let (mut cx, mut cy) = (x0, y0);
    loop {
        draw_pixel(fb, cx as u32, cy as u32, color);
        if cx == x1 && cy == y1 {
            break;
        }
        let doubled = 2 * err;
        if doubled >= dy {
            err += dy;
            cx += step_x;
        }
        if doubled <= dx {
            err += dx;
            cy += step_y;
        }
    }
}

/// Circle outline (midpoint algorithm).
pub fn draw_circle(fb: &FramebufferInfo, cx: u32, cy: u32, r: u32, color: u32) {
    let (cx, cy) = (i64::from(cx), i64::from(cy));
    let mut x = i64::from(r);
    let mut y = 0i64;
    let mut err = 1 - x;
    let plot = |px: i64, py: i64| {
        if px >= 0 && py >= 0 {
            draw_pixel(fb, px as u32, py as u32, color);
        }
    };
    while x >= y {
        for (dx, dy) in [(x, y), (y, x)] {
            plot(cx + dx, cy + dy);
            plot(cx - dx, cy + dy);
            plot(cx + dx, cy - dy);
            plot(cx - dx, cy - dy);
        }
        y += 1;
        if err < 0 {
            err += 2 * y + 1;
        } else {
            x -= 1;
            err += 2 * (y - x) + 1;
        }
    }
}

/// Filled circle.
pub fn fill_circle(fb: &FramebufferInfo, cx: u32, cy: u32, r: u32, color: u32) {
    let r = i64::from(r);
    for dy in -r..=r {
        let row = i64::from(cy) + dy;
        if row < 0 {
            continue;
        }
        let mut half = 0i64;
        while (half + 1) * (half + 1) + dy * dy <= r * r {
            half += 1;
        }
        let left = (i64::from(cx) - half).max(0);
        let width = i64::from(cx) + half - left + 1;
        draw_rect(fb, left as u32, row as u32, width as u32, 1, color);
    }
}

/// Fill a region with a solid colour (alias of [`draw_rect`]).
pub fn fill_region(fb: &FramebufferInfo, x: u32, y: u32, w: u32, h: u32, color: u32) {
    draw_rect(fb, x, y, w, h, color);
}

/// Fill a rectangle (alias of [`draw_rect`]).
pub fn fill_rect(fb: &FramebufferInfo, x: u32, y: u32, w: u32, h: u32, color: u32) {
    draw_rect(fb, x, y, w, h, color);
}

/// Draw a 1-bit-per-pixel bitmap (MSB first) in `fg` on `bg`.
#[allow(clippy::too_many_arguments)]
pub fn draw_bitmap(
    fb: &FramebufferInfo,
    x: u32,
    y: u32,
    bitmap: &[u8],
    bmp_w: u32,
    bmp_h: u32,
    fg: u32,
    bg: u32,
) {
    let bytes_per_row = bmp_w.div_ceil(8);
    for row in 0..bmp_h {
        for col in 0..bmp_w {
            let Some(byte) = bitmap.get((row * bytes_per_row + col / 8) as usize) else {
                return;
            };
            let is_set = (byte >> (7 - (col % 8))) & 1 != 0;
            draw_pixel(fb, x + col, y + row, if is_set { fg } else { bg });
        }
    }
}

/// Copy `0xAARRGGBB` pixels to the target (opaque).
pub fn blit(fb: &FramebufferInfo, x: u32, y: u32, src: &[u32], src_w: u32, src_h: u32) {
    for row in 0..src_h {
        let Some(mut run) = span(fb, x, y + row, src_w) else {
            continue;
        };
        let start = (row * src_w) as usize + run.skipped;
        let Some(line) = src.get(start..start + run.len) else {
            return;
        };
        for (dst, &color) in run.pixels().iter_mut().zip(line) {
            *dst = fx(color);
        }
    }
}

/// Copy stored-format pixels (another surface's rows) to the target.
pub fn blit_stored(fb: &FramebufferInfo, x: u32, y: u32, src: &[u32], src_w: u32, src_h: u32) {
    for row in 0..src_h {
        let Some(mut run) = span(fb, x, y + row, src_w) else {
            continue;
        };
        let start = (row * src_w) as usize + run.skipped;
        let Some(line) = src.get(start..start + run.len) else {
            return;
        };
        run.pixels().copy_from_slice(line);
    }
}

/// Draw an 8-bit coverage mask in `color`, blending over the target.
/// Used for anti-aliased glyphs and icons.
pub fn draw_alpha_mask(
    fb: &FramebufferInfo,
    x: u32,
    y: u32,
    mask: &[u8],
    mask_w: u32,
    mask_h: u32,
    color: u32,
) {
    let pixel = fx(color);
    for row in 0..mask_h {
        let Some(mut run) = span(fb, x, y + row, mask_w) else {
            continue;
        };
        let start = (row * mask_w) as usize + run.skipped;
        let Some(line) = mask.get(start..start + run.len) else {
            return;
        };
        for (dst, &coverage) in run.pixels().iter_mut().zip(line) {
            match coverage {
                0 => {}
                255 => *dst = pixel,
                partial => *dst = blend(*dst, pixel, u32::from(partial)),
            }
        }
    }
}

/// Draw one 8x16 bitmap-font glyph row-by-row.
fn draw_glyph(fb: &FramebufferInfo, x: u32, y: u32, glyph: &[u8], fg: u32, bg: Option<u32>) {
    for row in 0..CHAR_HEIGHT {
        let Some(mut run) = span(fb, x, y + row, CHAR_WIDTH) else {
            continue;
        };
        let bits = glyph[row as usize];
        let first_col = run.skipped as u32;
        for (index, dst) in run.pixels().iter_mut().enumerate() {
            let col = first_col + index as u32;
            if (bits >> (7 - col)) & 1 != 0 {
                *dst = fg;
            } else if let Some(background) = bg {
                *dst = background;
            }
        }
    }
}

/// Draw text with the built-in 8x16 monospace bitmap font.
/// A `bg` of 0 means transparent background.
pub fn draw_text(fb: &FramebufferInfo, x: u32, y: u32, text: &str, fg: u32, bg: u32) {
    let fg_px = fx(fg);
    let bg_px = (bg != 0).then(|| fx(bg));
    let mut cx = x;
    for c in text.chars() {
        if let Some(glyph) = FONT.get(c as usize) {
            draw_glyph(fb, cx, y, glyph, fg_px, bg_px);
        }
        cx = cx.saturating_add(CHAR_WIDTH);
    }
}

/// Draw 8x16 text horizontally centred on `center_x`.
pub fn draw_text_centered(
    fb: &FramebufferInfo,
    center_x: u32,
    y: u32,
    text: &str,
    fg: u32,
    bg: u32,
) {
    let text_width = (text.chars().count() as u32) * CHAR_WIDTH;
    let x = center_x.saturating_sub(text_width / 2);
    draw_text(fb, x, y, text, fg, bg);
}

/// Copy one row of stored-format pixels, treating 0 as "nothing here".
pub fn blit_stored_keyed(fb: &FramebufferInfo, x: u32, y: u32, src: &[u32]) {
    let Some(mut run) = span(fb, x, y, src.len() as u32) else {
        return;
    };
    let Some(line) = src.get(run.skipped..run.skipped + run.len) else {
        return;
    };
    for (dst, &pixel) in run.pixels().iter_mut().zip(line) {
        if pixel != 0 {
            *dst = pixel;
        }
    }
}
