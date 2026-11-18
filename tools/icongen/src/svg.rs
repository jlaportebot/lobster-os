//! Builds the SVG documents that are rasterised into sprites.

use crate::manifest::{FileKind, FileIcon};
use std::fs;
use std::path::Path;

/// Glyph share of a tile's edge.
const GLYPH_FRACTION: f32 = 0.58;
/// Corner radius share of a tile's edge.
const CORNER_FRACTION: f32 = 0.22;
/// Tabler's design grid.
const TABLER_GRID: f32 = 24.0;
/// Minimum on-screen stroke width (px) so thin glyphs stay readable.
const MIN_STROKE_PX: f32 = 1.8;
/// Stroke width (px) for symbolic masks at their smallest size.
const MASK_STROKE_PX: f32 = 1.5;
/// File icons are authored on this grid and scaled to the sprite size.
const FILE_GRID: f32 = 48.0;
/// Glyph edge (file-grid units) for badges on pages and folders.
const BADGE_EDGE: f32 = 20.0;
/// Sprites at or below this edge get a proportionally larger badge.
const SMALL_SPRITE_PX: u32 = 24;
/// Badge enlargement for small sprites so the glyph survives downscaling.
const SMALL_BADGE_BOOST: f32 = 1.25;
const FOLDER_GLYPH_EDGE: f32 = 18.0;

/// Loads a Tabler SVG and returns its inner markup (children of `<svg>`).
pub fn tabler_inner(dir: &Path, style: &str, name: &str) -> Result<String, String> {
    let path = dir.join(style).join(format!("{name}.svg"));
    let text = fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let start = text
        .find('>')
        .ok_or_else(|| format!("{}: no <svg> tag", path.display()))?;
    let end = text
        .rfind("</svg>")
        .ok_or_else(|| format!("{}: no </svg>", path.display()))?;
    Ok(text[start + 1..end].to_string())
}

fn channel(hex: &str, index: usize) -> f32 {
    let digits = hex.get(index * 2..index * 2 + 2).unwrap_or("00");
    f32::from(u8::from_str_radix(digits, 16).unwrap_or(0))
}

/// Mixes `hex` toward `target` (0 or 255) by `amount` and returns `#rrggbb`.
pub fn shade(hex: &str, target: f32, amount: f32) -> String {
    let mix = |i: usize| (channel(hex, i) * (1.0 - amount) + target * amount).round() as u8;
    format!("#{:02x}{:02x}{:02x}", mix(0), mix(1), mix(2))
}

fn document(size: u32, grid: f32, body: &str) -> String {
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{size}\" height=\"{size}\" \
         viewBox=\"0 0 {grid} {grid}\">{body}</svg>"
    )
}

fn gradient(id: &str, top: &str, bottom: &str) -> String {
    format!(
        "<linearGradient id=\"{id}\" x1=\"0\" y1=\"0\" x2=\"0\" y2=\"1\">\
         <stop offset=\"0\" stop-color=\"{top}\"/><stop offset=\"1\" stop-color=\"{bottom}\"/>\
         </linearGradient>"
    )
}

/// Full-colour app tile at `size` px.
pub fn tile(size: u32, color: &str, glyph: &str) -> String {
    let edge = size as f32;
    let radius = edge * CORNER_FRACTION;
    let scale = edge * GLYPH_FRACTION / TABLER_GRID;
    let offset = (edge - TABLER_GRID * scale) / 2.0;
    let stroke = (MIN_STROKE_PX / scale).max(2.0);
    let defs = format!(
        "{}<linearGradient id=\"h\" x1=\"0\" y1=\"0\" x2=\"0\" y2=\"1\">\
         <stop offset=\"0\" stop-color=\"#fff\" stop-opacity=\"0.6\"/>\
         <stop offset=\"0.4\" stop-color=\"#fff\" stop-opacity=\"0\"/></linearGradient>",
        gradient("g", &shade(color, 255.0, 0.22), &shade(color, 0.0, 0.22))
    );
    let body = format!(
        "<defs>{defs}</defs>\
         <rect width=\"{edge}\" height=\"{edge}\" rx=\"{radius}\" fill=\"url(#g)\"/>\
         <rect x=\"0.5\" y=\"0.5\" width=\"{w}\" height=\"{w}\" rx=\"{r2}\" fill=\"none\" \
         stroke=\"url(#h)\" stroke-width=\"1\"/>\
         <g transform=\"translate({offset} {offset}) scale({scale})\" fill=\"none\" \
         stroke=\"#fff\" stroke-width=\"{stroke}\" stroke-linecap=\"round\" \
         stroke-linejoin=\"round\">{glyph}</g>",
        w = edge - 1.0,
        r2 = radius - 0.5,
    );
    document(size, edge, &body)
}

/// White-on-transparent glyph whose alpha becomes the mask.
pub fn mask(size: u32, glyph: &str) -> String {
    let stroke = (MASK_STROKE_PX * TABLER_GRID / size as f32).max(2.0);
    let body = format!(
        "<g fill=\"none\" stroke=\"#fff\" stroke-width=\"{stroke}\" stroke-linecap=\"round\" \
         stroke-linejoin=\"round\">{glyph}</g>"
    );
    document(size, TABLER_GRID, &body)
}

fn badge(glyph: &str, color: &str, edge: f32, cx: f32, cy: f32, size: u32) -> String {
    let edge = if size <= SMALL_SPRITE_PX { edge * SMALL_BADGE_BOOST } else { edge };
    let scale = edge / TABLER_GRID;
    let on_screen = size as f32 / FILE_GRID * scale;
    let stroke = (1.6 / on_screen).max(2.2);
    let (x, y) = (cx - edge / 2.0, cy - edge / 2.0);
    format!(
        "<g transform=\"translate({x} {y}) scale({scale})\" fill=\"none\" stroke=\"{color}\" \
         stroke-width=\"{stroke}\" stroke-linecap=\"round\" stroke-linejoin=\"round\">{glyph}</g>"
    )
}

const FOLDER_BACK: &str = "M4 12a4 4 0 0 1 4-4h10.5a4 4 0 0 1 3 1.4l2.2 2.6H40a4 4 0 0 1 4 4V38a4 4 0 0 1-4 4H8a4 4 0 0 1-4-4Z";
const FOLDER_DEFS: &str = "<linearGradient id=\"fb\" x1=\"0\" y1=\"0\" x2=\"0\" y2=\"1\">\
    <stop offset=\"0\" stop-color=\"#d58a0c\"/><stop offset=\"1\" stop-color=\"#b9720a\"/></linearGradient>\
    <linearGradient id=\"ff\" x1=\"0\" y1=\"0\" x2=\"0\" y2=\"1\">\
    <stop offset=\"0\" stop-color=\"#ffd25e\"/><stop offset=\"1\" stop-color=\"#f6a823\"/></linearGradient>";

fn folder_front(open: bool) -> String {
    if open {
        return "<path d=\"M10 22H45L40.5 40.5H5Z\" fill=\"url(#ff)\" stroke=\"url(#ff)\" \
                stroke-width=\"4\" stroke-linejoin=\"round\"/>"
            .to_string();
    }
    "<rect x=\"4\" y=\"16\" width=\"40\" height=\"26\" rx=\"4\" fill=\"url(#ff)\"/>\
     <path d=\"M5 20V19a3 3 0 0 1 3-3h32a3 3 0 0 1 3 3v1\" fill=\"none\" stroke=\"#fff\" \
     stroke-opacity=\"0.45\"/>"
        .to_string()
}

fn folder(dir: &Path, size: u32, open: bool, glyph: Option<&str>) -> Result<String, String> {
    let mut body = format!(
        "<defs>{FOLDER_DEFS}</defs><path d=\"{FOLDER_BACK}\" fill=\"url(#fb)\"/>{}",
        folder_front(open)
    );
    if let Some(name) = glyph {
        let inner = tabler_inner(dir, "outline", name)?;
        body += &badge(&inner, "#fff", FOLDER_GLYPH_EDGE, 24.0, 29.5, size);
    }
    Ok(document(size, FILE_GRID, &body))
}

const PAGE: &str = "M9 7a4 4 0 0 1 4-4h14l12 12v26a4 4 0 0 1-4 4H13a4 4 0 0 1-4-4Z";
const FOLD: &str = "M27 3v8a4 4 0 0 0 4 4h8Z";

fn page(dir: &Path, size: u32, glyph: &str, color: &str, tinted: bool) -> Result<String, String> {
    let (top, bottom, fold, ink) = if tinted {
        (shade(color, 255.0, 0.2), shade(color, 0.0, 0.2), shade(color, 0.0, 0.4), "#fff".to_string())
    } else {
        ("#f8fafc".into(), "#cbd5e1".into(), "#9aa8ba".into(), format!("#{color}"))
    };
    let inner = tabler_inner(dir, "outline", glyph)?;
    let body = format!(
        "<defs>{}</defs><path d=\"{PAGE}\" fill=\"url(#p)\" stroke=\"#64748b\" stroke-opacity=\"0.55\"/>\
         <path d=\"{FOLD}\" fill=\"{fold}\"/>{}",
        gradient("p", &top, &bottom),
        badge(&inner, &ink, BADGE_EDGE, 24.0, 31.0, size)
    );
    Ok(document(size, FILE_GRID, &body))
}

fn trash(dir: &Path, size: u32, full: bool) -> Result<String, String> {
    let inner = tabler_inner(dir, "filled", "trash")?;
    let papers = if full {
        "<rect x=\"13\" y=\"4\" width=\"14\" height=\"16\" rx=\"1.5\" fill=\"#f1f5f9\" \
         transform=\"rotate(-12 20 12)\"/><rect x=\"21\" y=\"3\" width=\"14\" height=\"16\" \
         rx=\"1.5\" fill=\"#e2e8f0\" transform=\"rotate(10 28 11)\"/>"
    } else {
        ""
    };
    let body = format!(
        "<defs>{}</defs>{papers}<g transform=\"translate(4 4) scale(1.6667)\" fill=\"url(#b)\">{inner}</g>",
        gradient("b", "#aab4c3", "#6b7686")
    );
    Ok(document(size, FILE_GRID, &body))
}

/// SVG for one file-type icon at `size` px.
pub fn file_icon(dir: &Path, size: u32, icon: &FileIcon) -> Result<String, String> {
    match &icon.kind {
        FileKind::Folder => folder(dir, size, false, None),
        FileKind::FolderOpen => folder(dir, size, true, None),
        FileKind::FolderGlyph(name) => folder(dir, size, false, Some(name)),
        FileKind::Page { glyph, color, tinted } => page(dir, size, glyph, color, *tinted),
        FileKind::Trash { full } => trash(dir, size, *full),
    }
}
