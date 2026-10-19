//! 2D graphics primitives for the framebuffer

use super::console::FONT;
use super::console::{CHAR_HEIGHT, CHAR_WIDTH};
use super::fb::FramebufferInfo;

/// Draw a single pixel
pub fn draw_pixel(fb: &FramebufferInfo, x: u32, y: u32, color: u32) {
    if x >= fb.width || y >= fb.height {
        return;
    }
    let offset = (y * (fb.pitch / 4) + x) as usize;
    unsafe {
        let ptr = (fb.virt_addr as *mut u32).add(offset);
        core::ptr::write_volatile(ptr, color);
    }
}

/// Draw a filled rectangle
pub fn draw_rect(fb: &FramebufferInfo, x: u32, y: u32, w: u32, h: u32, color: u32) {
    for dy in 0..h {
        let py = y + dy;
        if py >= fb.height {
            break;
        }
        for dx in 0..w {
            draw_pixel(fb, x + dx, py, color);
        }
    }
}

/// Draw a rectangle outline
/// Fill a rounded rectangle (corner radius clamped to half of w/h).
pub fn fill_rounded_rect(
    fb: &FramebufferInfo,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    radius: u32,
    color: u32,
) {
    let r = radius.min(w / 2).min(h / 2);
    // Body: everything between the corner bands
    draw_rect(fb, x, y + r, w, h - 2 * r, color);
    // Top band: two corner cutouts — draw full rows then carve by drawing
    // nothing; instead draw the full band and let corners overwrite later.
    // Simpler correct approach: scanline the rounded region.
    for dy in 0..r {
        // For each row in the top band, horizontal inset follows the circle
        let inset = corner_inset(r, dy);
        draw_rect(fb, x + inset, y + dy, w - 2 * inset, 1, color);
        draw_rect(fb, x + inset, y + h - 1 - dy, w - 2 * inset, 1, color);
    }
}

/// Horizontal inset for a rounded-corner scanline: distance from the corner
/// box edge to the circle at row `dy` (0..r). Uses integer circle math.
fn corner_inset(r: u32, dy: u32) -> u32 {
    // (r-dy)^2 + inset^2 = r^2  =>  inset = sqrt(r^2 - (r-dy)^2)
    let a = r - dy;
    let val = (r * r).saturating_sub(a * a);
    // integer sqrt (r <= 1024 keeps this cheap)
    let mut s = 0u32;
    while s * s < val {
        s += 1;
    }
    s
}

pub fn draw_rect_outline(fb: &FramebufferInfo, x: u32, y: u32, w: u32, h: u32, color: u32) {
    // Top edge
    for dx in 0..w {
        draw_pixel(fb, x + dx, y, color);
    }
    // Bottom edge
    for dx in 0..w {
        draw_pixel(fb, x + dx, y + h - 1, color);
    }
    // Left edge
    for dy in 0..h {
        draw_pixel(fb, x, y + dy, color);
    }
    // Right edge
    for dy in 0..h {
        draw_pixel(fb, x + w - 1, y + dy, color);
    }
}

/// Draw a line using Bresenham's algorithm
pub fn draw_line(fb: &FramebufferInfo, x0: u32, y0: u32, x1: u32, y1: u32, color: u32) {
    let mut dx = (x1 as i64 - x0 as i64).abs();
    let mut dy = (y1 as i64 - y0 as i64).abs();
    let sx: i64 = if x0 < x1 { 1 } else { -1 };
    let sy: i64 = if y0 < y1 { 1 } else { -1 };
    let mut err = dx - dy;
    let mut cx = x0 as i64;
    let mut cy = y0 as i64;

    loop {
        if cx >= 0 && cy >= 0 {
            draw_pixel(fb, cx as u32, cy as u32, color);
        }
        if cx == x1 as i64 && cy == y1 as i64 {
            break;
        }
        let e2 = 2 * err;
        if e2 > -dy {
            err -= dy;
            dx -= 1; // consume one step in x
            cx += sx;
        }
        if e2 < dx {
            err += dx;
            dy -= 1; // consume one step in y
            cy += sy;
        }
    }
}

/// Draw a circle using midpoint circle algorithm
pub fn draw_circle(fb: &FramebufferInfo, cx: u32, cy: u32, r: u32, color: u32) {
    let mut x = r as i64;
    let mut y = 0i64;
    let mut err = 1i64 - r as i64;

    while x >= y {
        draw_pixel(fb, (cx as i64 + x) as u32, (cy as i64 + y) as u32, color);
        draw_pixel(fb, (cx as i64 + y) as u32, (cx as i64 + x) as u32, color);
        draw_pixel(fb, (cx as i64 - x) as u32, (cy as i64 + y) as u32, color);
        draw_pixel(fb, (cx as i64 - y) as u32, (cy as i64 + x) as u32, color);
        draw_pixel(fb, (cx as i64 + x) as u32, (cy as i64 - y) as u32, color);
        draw_pixel(fb, (cx as i64 + y) as u32, (cy as i64 - x) as u32, color);
        draw_pixel(fb, (cx as i64 - x) as u32, (cy as i64 - y) as u32, color);
        draw_pixel(fb, (cx as i64 - y) as u32, (cy as i64 - x) as u32, color);

        y += 1;
        if err < 0 {
            err += 2 * y + 1;
        } else {
            x -= 1;
            err += 2 * (y - x) + 1;
        }
    }
}

/// Fill a rectangular region (fast memset-like fill)
pub fn fill_region(fb: &FramebufferInfo, x: u32, y: u32, w: u32, h: u32, color: u32) {
    let pitch_words = fb.pitch / 4;
    unsafe {
        let base = fb.virt_addr as *mut u32;
        for dy in 0..h {
            let py = y + dy;
            if py >= fb.height {
                break;
            }
            let row_offset = (py * pitch_words) as usize;
            for dx in 0..w {
                let px = x + dx;
                if px >= fb.width {
                    break;
                }
                core::ptr::write_volatile(base.add(row_offset + px as usize), color);
            }
        }
    }
}

/// Draw a 1-bit bitmap with foreground/background colors
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
    let bytes_per_row = (bmp_w + 7) / 8;
    for row in 0..bmp_h {
        if (y + row) >= fb.height {
            break;
        }
        for col in 0..bmp_w {
            let byte_idx = (row * bytes_per_row + col / 8) as usize;
            if byte_idx >= bitmap.len() {
                break;
            }
            let bit = (bitmap[byte_idx] >> (7 - (col % 8))) & 1;
            let color = if bit != 0 { fg } else { bg };
            draw_pixel(fb, x + col, y + row, color);
        }
    }
}

/// Blit a pixel buffer to the framebuffer
pub fn blit(fb: &FramebufferInfo, x: u32, y: u32, src: &[u32], src_w: u32, src_h: u32) {
    for row in 0..src_h {
        let py = y + row;
        if py >= fb.height {
            break;
        }
        for col in 0..src_w {
            let px = x + col;
            if px >= fb.width {
                break;
            }
            let src_idx = (row * src_w + col) as usize;
            if src_idx >= src.len() {
                break;
            }
            draw_pixel(fb, px, py, src[src_idx]);
        }
    }
}

/// Draw a text string using the baked-in font
pub fn draw_text(fb: &FramebufferInfo, x: u32, y: u32, text: &str, fg: u32, bg: u32) {
    let mut cx = x;
    for c in text.chars() {
        let code = c as usize;
        if code < 128 {
            let glyph = &FONT[code];
            for row in 0..CHAR_HEIGHT {
                let row_bits = glyph[row as usize];
                for col in 0..CHAR_WIDTH {
                    let color = if (row_bits >> (7 - col)) & 1 != 0 {
                        fg
                    } else {
                        bg
                    };
                    draw_pixel(fb, cx + col, y + row, color);
                }
            }
        }
        cx += CHAR_WIDTH;
    }
}

/// Draw text centered at a given x position
pub fn draw_text_centered(
    fb: &FramebufferInfo,
    center_x: u32,
    y: u32,
    text: &str,
    fg: u32,
    bg: u32,
) {
    let text_width = (text.len() as u32) * CHAR_WIDTH;
    let x = center_x.saturating_sub(text_width / 2);
    draw_text(fb, x, y, text, fg, bg);
}

/// Fill a rectangular region with a solid color
pub fn fill_rect(fb: &FramebufferInfo, x: u32, y: u32, w: u32, h: u32, color: u32) {
    fill_region(fb, x, y, w, h, color);
}
