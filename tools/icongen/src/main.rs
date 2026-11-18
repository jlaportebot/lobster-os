//! Generates the LobsterOS icon assets from pinned Tabler SVGs.
//!
//! Usage: `icongen [--sheet <png>] [--scale <n>]`, run from `tools/icongen`.
//! Output is deterministic: identical SVGs produce byte-identical files.

mod emit;
mod manifest;
mod render;
mod sheet;
mod svg;

use manifest::{FILE_SIZES, GLYPH_SIZES, TILE_SIZES};
use std::path::{Path, PathBuf};

/// Whether sprite bytes are premultiplied RGBA or an alpha mask.
#[derive(Clone, Copy)]
pub enum Kind {
    Color,
    Mask,
}

/// One rendered size of an icon.
pub struct Sprite {
    pub size: u32,
    pub data: Vec<u8>,
}

/// An icon with all of its sizes.
pub struct Entry {
    pub variant: &'static str,
    pub kind: Kind,
    pub sprites: Vec<Sprite>,
}

fn color_entries(svg_dir: &Path) -> Result<Vec<Entry>, String> {
    let mut entries = Vec::new();
    for app in manifest::APPS {
        let inner = svg::tabler_inner(svg_dir, "outline", app.glyph)?;
        let mut sprites = Vec::new();
        for size in TILE_SIZES {
            let data = render::rgba(&svg::tile(size, app.color, &inner), size)?;
            sprites.push(Sprite { size, data });
        }
        entries.push(Entry { variant: app.variant, kind: Kind::Color, sprites });
    }
    for icon in manifest::files() {
        let mut sprites = Vec::new();
        for size in FILE_SIZES {
            let data = render::rgba(&svg::file_icon(svg_dir, size, &icon)?, size)?;
            sprites.push(Sprite { size, data });
        }
        entries.push(Entry { variant: icon.variant, kind: Kind::Color, sprites });
    }
    Ok(entries)
}

fn glyph_entries(svg_dir: &Path) -> Result<Vec<Entry>, String> {
    let mut entries = Vec::new();
    for sym in manifest::GLYPHS {
        let inner = svg::tabler_inner(svg_dir, "outline", sym.glyph)?;
        let mut sprites = Vec::new();
        for size in GLYPH_SIZES {
            sprites.push(Sprite { size, data: render::alpha(&svg::mask(size, &inner), size)? });
        }
        entries.push(Entry { variant: sym.variant, kind: Kind::Mask, sprites });
    }
    Ok(entries)
}

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    let svg_dir = PathBuf::from("svg");
    let out_dir = PathBuf::from("../../assets/icons");
    let icons = color_entries(&svg_dir)?;
    let glyphs = glyph_entries(&svg_dir)?;
    std::fs::create_dir_all(&out_dir).map_err(|e| format!("create {}: {e}", out_dir.display()))?;
    let bytes = emit::write(&out_dir, &icons, &glyphs)?;
    println!("wrote {} icons, {} glyphs, {bytes} bytes", icons.len(), glyphs.len());
    if let Some(path) = flag(&args, "--sheet") {
        let scale = flag(&args, "--scale").and_then(|s| s.parse().ok()).unwrap_or(2);
        let all: Vec<Entry> = icons.into_iter().chain(glyphs).collect();
        sheet::write(Path::new(&path), &all, scale)?;
    }
    Ok(())
}

fn main() {
    if let Err(message) = run() {
        eprintln!("icongen: {message}");
        std::process::exit(1);
    }
}
