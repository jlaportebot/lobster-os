//! Build-time generated icon set.
//!
//! The sprites come from `tools/icongen` (Tabler Icons, MIT) and are linked in
//! as one read-only blob. Looking up and drawing an icon is O(1) and never
//! allocates.
//!
//! Pixel formats:
//! * [`Icon`] sprites are premultiplied RGBA, one `[r, g, b, a]` quad per
//!   pixel, row-major. That is the framebuffer's stored order, so opaque
//!   pixels are copied and only edge pixels are blended.
//! * [`Glyph`] sprites are 8-bit coverage masks tinted at draw time.

use super::fb::FramebufferInfo;
use super::graphics;

/// Bytes per pixel of a colour sprite.
const COLOR_BYTES: usize = 4;

/// Location of one pre-rendered size inside [`SPRITE_DATA`].
#[derive(Clone, Copy)]
pub(crate) struct Sprite {
    /// Edge length in pixels (sprites are square).
    pub(crate) size: u32,
    /// Byte offset of the first pixel.
    pub(crate) offset: u32,
}

/// All sprite pixels, laid out by the generator.
static SPRITE_DATA: &[u8] = include_bytes!("../../assets/icons/icons.bin");

include!("../../assets/icons/index.rs");

/// Index of the sprite to use for `requested`: the largest size not above it,
/// or the smallest one when every size is larger.
fn nearest_index(sprites: &[Sprite; 3], requested: u32) -> usize {
    let mut best: Option<usize> = None;
    let mut smallest = 0;
    for (i, sprite) in sprites.iter().enumerate() {
        if sprite.size < sprites[smallest].size {
            smallest = i;
        }
        let fits = sprite.size <= requested;
        if fits && best.is_none_or(|b| sprite.size > sprites[b].size) {
            best = Some(i);
        }
    }
    best.unwrap_or(smallest)
}

fn sprite_bytes(sprite: Sprite, bytes_per_pixel: usize) -> &'static [u8] {
    let start = sprite.offset as usize;
    let len = (sprite.size * sprite.size) as usize * bytes_per_pixel;
    SPRITE_DATA.get(start..start + len).unwrap_or(&[])
}

/// Pixel size that [`draw`] would use for `icon` when asked for `size`.
#[must_use]
pub fn icon_size_for(icon: Icon, size: u32) -> u32 {
    let sprites = &ICON_SPRITES[icon as usize];
    sprites[nearest_index(sprites, size)].size
}

/// Draw `icon` with its top-left corner at (`x`, `y`), alpha-blended and
/// clipped to the target. Uses the largest sprite not above `size` (or the
/// smallest available when `size` is below all of them).
pub fn draw(fb: &FramebufferInfo, icon: Icon, size: u32, x: u32, y: u32) {
    let sprites = &ICON_SPRITES[icon as usize];
    let sprite = sprites[nearest_index(sprites, size)];
    let data = sprite_bytes(sprite, COLOR_BYTES);
    graphics::draw_premul_bytes(fb, x, y, data, sprite.size, sprite.size);
}

/// Draw `glyph` in `color` (`0xAARRGGBB`) at (`x`, `y`), choosing the sprite
/// like [`draw`].
pub fn draw_glyph(fb: &FramebufferInfo, glyph: Glyph, size: u32, x: u32, y: u32, color: u32) {
    let sprites = &GLYPH_SPRITES[glyph as usize];
    let sprite = sprites[nearest_index(sprites, size)];
    let mask = sprite_bytes(sprite, 1);
    graphics::draw_alpha_mask(fb, x, y, mask, sprite.size, sprite.size, color);
}

fn extension(name: &str) -> Option<&str> {
    let dot = name.rfind('.')?;
    (dot > 0 && dot + 1 < name.len()).then(|| &name[dot + 1..])
}

fn has_ext(ext: &str, list: &[&str]) -> bool {
    list.iter()
        .any(|candidate| ext.eq_ignore_ascii_case(candidate))
}

/// File-type icons per extension group, checked in order.
const EXTENSION_GROUPS: &[(&[&str], Icon)] = &[
    (
        &[
            "txt", "md", "log", "cfg", "conf", "ini", "csv", "json", "toml",
        ],
        Icon::FileText,
    ),
    (
        &[
            "rs", "py", "c", "h", "cpp", "js", "ts", "sh", "lua", "html", "css",
        ],
        Icon::FileCode,
    ),
    (
        &["png", "jpg", "jpeg", "bmp", "gif", "svg", "webp"],
        Icon::FileImage,
    ),
    (&["wav", "mp3", "ogg", "flac", "aac"], Icon::FileAudio),
    (&["mp4", "mkv", "avi", "webm", "mov"], Icon::FileVideo),
    (
        &["zip", "tar", "gz", "tgz", "xz", "bz2", "7z", "lobpkg"],
        Icon::FileArchive,
    ),
    (&["bin", "elf", "exe", "img"], Icon::FileExecutable),
    (&["pdf"], Icon::FilePdf),
    (&["lnk", "url"], Icon::FileLink),
];

/// Icon for a directory entry, chosen from `is_dir` and the
/// (case-insensitive) file extension. Unknown names get the generic file.
#[must_use]
pub fn for_file_name(name: &str, is_dir: bool) -> Icon {
    if is_dir {
        return Icon::FileFolder;
    }
    let Some(ext) = extension(name) else {
        return Icon::FileGeneric;
    };
    EXTENSION_GROUPS
        .iter()
        .find(|(list, _)| has_ext(ext, list))
        .map_or(Icon::FileGeneric, |&(_, icon)| icon)
}

/// App names (and common aliases) mapped to their tile.
const APP_NAMES: &[(&[&str], Icon)] = &[
    (&["Terminal"], Icon::AppTerminal),
    (&["Files", "File Manager"], Icon::AppFiles),
    (&["Editor", "Text Editor"], Icon::AppEditor),
    (&["Calculator", "Calc"], Icon::AppCalculator),
    (
        &["Task Manager", "Process Manager", "Procs"],
        Icon::AppTaskManager,
    ),
    (&["System Monitor", "System"], Icon::AppSystemMonitor),
    (&["Settings"], Icon::AppSettings),
    (&["Network"], Icon::AppNetwork),
    (&["App Store"], Icon::AppStore),
    (&["Images", "Image Viewer"], Icon::AppImages),
    (&["Browser"], Icon::AppBrowser),
    (&["Notes"], Icon::AppNotes),
    (&["About", "Help"], Icon::AppAbout),
    (&["Welcome"], Icon::AppWelcome),
    (&["Python"], Icon::AppPython),
];

/// Tile for a start-menu or launcher name (case-insensitive). Unknown names,
/// such as installed packages, get the generic app tile.
#[must_use]
pub fn for_app(name: &str) -> Icon {
    APP_NAMES
        .iter()
        .find(|(names, _)| has_ext(name, names))
        .map_or(Icon::AppGeneric, |&(_, icon)| icon)
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    extern crate std;
    use std::vec;
    use std::vec::Vec;

    const CANVAS: u32 = 100;
    const BACKGROUND: u32 = 0xFF10_2030;
    const CANARY: u32 = 0xFFAB_CDEF;

    fn target(pixels: &mut Vec<u32>) -> FramebufferInfo {
        let screen = FramebufferInfo::empty();
        // SAFETY: `pixels` holds CANVAS * CANVAS values and outlives every
        // use of the returned target in the calling test.
        unsafe { screen.surface(pixels.as_mut_ptr(), 0, 0, CANVAS, CANVAS) }
    }

    #[test]
    fn every_icon_has_sized_nonempty_sprites() {
        for icon in Icon::ALL {
            for sprite in ICON_SPRITES[icon as usize] {
                let bytes = sprite_bytes(sprite, COLOR_BYTES);
                assert_eq!(bytes.len(), (sprite.size * sprite.size * 4) as usize);
                assert!(bytes.chunks_exact(4).any(|px| px[3] != 0), "{icon:?}");
            }
        }
    }

    #[test]
    fn every_glyph_has_sized_nonempty_sprites() {
        for glyph in Glyph::ALL {
            let sizes = GLYPH_SPRITES[glyph as usize].map(|s| s.size);
            assert_eq!(sizes, [16, 20, 24]);
            for sprite in GLYPH_SPRITES[glyph as usize] {
                let bytes = sprite_bytes(sprite, 1);
                assert_eq!(bytes.len(), (sprite.size * sprite.size) as usize);
                assert!(bytes.iter().any(|&a| a != 0), "{glyph:?}");
            }
        }
    }

    #[test]
    fn icon_sizes_are_the_documented_sets() {
        assert_eq!(
            ICON_SPRITES[Icon::AppTerminal as usize].map(|s| s.size),
            [48, 32, 24]
        );
        assert_eq!(
            ICON_SPRITES[Icon::FileFolder as usize].map(|s| s.size),
            [48, 24, 16]
        );
    }

    #[test]
    fn nearest_size_prefers_largest_not_above_request() {
        assert_eq!(icon_size_for(Icon::AppFiles, 48), 48);
        assert_eq!(icon_size_for(Icon::AppFiles, 40), 32);
        assert_eq!(icon_size_for(Icon::AppFiles, 31), 24);
        assert_eq!(icon_size_for(Icon::AppFiles, 200), 48);
        assert_eq!(icon_size_for(Icon::AppFiles, 8), 24);
        assert_eq!(icon_size_for(Icon::FileText, 20), 16);
    }

    #[test]
    fn file_name_mapping() {
        let cases = [
            ("docs", true, Icon::FileFolder),
            ("a.txt", false, Icon::FileText),
            ("main.rs", false, Icon::FileCode),
            ("x.py", false, Icon::FileCode),
            ("x.c", false, Icon::FileCode),
            ("run.sh", false, Icon::FileCode),
            ("p.png", false, Icon::FileImage),
            ("p.jpg", false, Icon::FileImage),
            ("p.bmp", false, Icon::FileImage),
            ("s.wav", false, Icon::FileAudio),
            ("s.mp3", false, Icon::FileAudio),
            ("a.zip", false, Icon::FileArchive),
            ("a.tar", false, Icon::FileArchive),
            ("a.gz", false, Icon::FileArchive),
            ("pkg.lobpkg", false, Icon::FileArchive),
            ("doc.pdf", false, Icon::FilePdf),
            ("README", false, Icon::FileGeneric),
            (".hidden", false, Icon::FileGeneric),
            ("trailing.", false, Icon::FileGeneric),
            ("PHOTO.JPG", false, Icon::FileImage),
            ("Notes.TxT", false, Icon::FileText),
            ("weird.xyz", false, Icon::FileGeneric),
        ];
        for (name, is_dir, expected) in cases {
            assert_eq!(for_file_name(name, is_dir), expected, "{name}");
        }
    }

    #[test]
    fn app_names_from_start_menu_all_resolve() {
        let expected = [
            ("Terminal", Icon::AppTerminal),
            ("Files", Icon::AppFiles),
            ("Editor", Icon::AppEditor),
            ("Calculator", Icon::AppCalculator),
            ("Task Manager", Icon::AppTaskManager),
            ("Settings", Icon::AppSettings),
            ("Network", Icon::AppNetwork),
            ("App Store", Icon::AppStore),
            ("Images", Icon::AppImages),
            ("Browser", Icon::AppBrowser),
            ("Notes", Icon::AppNotes),
            ("About", Icon::AppAbout),
        ];
        for entry in crate::wm::start_menu::APPS {
            let found = expected.iter().find(|(n, _)| *n == entry.name);
            let (_, icon) = found.expect("start-menu app missing from test table");
            assert_eq!(for_app(entry.name), *icon);
        }
        assert_eq!(for_app("terminal"), Icon::AppTerminal);
        assert_eq!(for_app("my-package"), Icon::AppGeneric);
    }

    #[test]
    fn draw_never_writes_outside_clip() {
        let mut pixels = vec![CANARY; (CANVAS * CANVAS) as usize];
        let clip = target(&mut pixels).clipped(20, 20, 60, 60);
        draw(&clip, Icon::AppTerminal, 48, 10, 10);
        draw_glyph(&clip, Glyph::Close, 24, 50, 50, 0xFFFF_FFFF);
        for y in 0..CANVAS {
            for x in 0..CANVAS {
                let inside = (20..60).contains(&x) && (20..60).contains(&y);
                if !inside {
                    assert_eq!(pixels[(y * CANVAS + x) as usize], CANARY, "({x},{y})");
                }
            }
        }
        assert!(pixels.iter().any(|&p| p != CANARY));
    }

    #[test]
    fn transparent_corner_keeps_background() {
        let background = graphics::fx(BACKGROUND);
        let mut pixels = vec![background; (CANVAS * CANVAS) as usize];
        let fb = target(&mut pixels);
        draw(&fb, Icon::AppFiles, 48, 20, 20);
        assert_eq!(pixels[(20 * CANVAS + 20) as usize], background);
        let centre = pixels[(44 * CANVAS + 30) as usize];
        assert_ne!(centre, background);
    }
}
