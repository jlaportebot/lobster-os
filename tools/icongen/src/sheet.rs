//! Contact sheet: every icon at every size on light and dark backgrounds.

use crate::{Entry, Kind};
use resvg::tiny_skia::Pixmap;
use std::path::Path;

const COLUMNS: u32 = 6;
const GAP: u32 = 8;
const BAND_H: u32 = 56;
const CELL_W: u32 = 3 * 48 + 4 * GAP;
const LIGHT: [u8; 3] = [0xee, 0xf0, 0xf4];
const DARK: [u8; 3] = [0x1e, 0x22, 0x2b];

fn fill(canvas: &mut [u8], width: u32, x0: u32, y0: u32, w: u32, h: u32, rgb: [u8; 3]) {
    for y in y0..y0 + h {
        for x in x0..x0 + w {
            let at = ((y * width + x) * 4) as usize;
            canvas[at..at + 4].copy_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
        }
    }
}

fn put(canvas: &mut [u8], width: u32, at: (u32, u32), size: u32, kind: Kind, data: &[u8], ink: [u8; 3]) {
    for row in 0..size {
        for col in 0..size {
            let i = (row * size + col) as usize;
            let (src, a) = match kind {
                Kind::Color => ([data[i * 4], data[i * 4 + 1], data[i * 4 + 2]], u32::from(data[i * 4 + 3])),
                Kind::Mask => ([0, 0, 0], u32::from(data[i])),
            };
            let o = (((at.1 + row) * width + at.0 + col) * 4) as usize;
            for c in 0..3 {
                let s = match kind {
                    Kind::Color => u32::from(src[c]),
                    Kind::Mask => u32::from(ink[c]) * a / 255,
                };
                canvas[o + c] = (s + u32::from(canvas[o + c]) * (255 - a) / 255) as u8;
            }
        }
    }
}

fn scaled(canvas: &[u8], w: u32, h: u32, k: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity((w * h * k * k * 4) as usize);
    for y in 0..h * k {
        for x in 0..w * k {
            let at = (((y / k) * w + x / k) * 4) as usize;
            out.extend_from_slice(&canvas[at..at + 4]);
        }
    }
    out
}

/// Renders all entries into a PNG at `scale`x nearest-neighbour zoom.
pub fn write(path: &Path, entries: &[Entry], scale: u32) -> Result<(), String> {
    let rows = (entries.len() as u32).div_ceil(COLUMNS);
    let (w, h) = (COLUMNS * CELL_W, rows * BAND_H * 2);
    let mut canvas = vec![255u8; (w * h * 4) as usize];
    for (n, entry) in entries.iter().enumerate() {
        let (cx, cy) = ((n as u32 % COLUMNS) * CELL_W, (n as u32 / COLUMNS) * BAND_H * 2);
        fill(&mut canvas, w, cx, cy, CELL_W, BAND_H, LIGHT);
        fill(&mut canvas, w, cx, cy + BAND_H, CELL_W, BAND_H, DARK);
        let mut x = cx + GAP;
        for sprite in &entry.sprites {
            let y = cy + (BAND_H - sprite.size) / 2;
            put(&mut canvas, w, (x, y), sprite.size, entry.kind, &sprite.data, [0x20, 0x20, 0x28]);
            put(&mut canvas, w, (x, y + BAND_H), sprite.size, entry.kind, &sprite.data, [0xf0, 0xf0, 0xf0]);
            x += sprite.size + GAP;
        }
    }
    let data = scaled(&canvas, w, h, scale);
    let mut pixmap = Pixmap::new(w * scale, h * scale).ok_or("sheet too large")?;
    pixmap.data_mut().copy_from_slice(&data);
    pixmap.save_png(path).map_err(|e| format!("save {}: {e}", path.display()))
}
