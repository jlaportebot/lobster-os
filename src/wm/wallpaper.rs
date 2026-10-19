//! Wallpaper engine — gradient presets + selection, persisted to /sd
//!
//! The desktop background is a vertical (or diagonal) gradient sampled per
//! scanline. A set of named presets cycle via the desktop context menu;
//! the active preset persists to /sd/etc/wallpaper.conf so it survives
//! reboots.

use crate::display::fb::get_info;
use crate::display::graphics::draw_rect;

/// Pack a color for the framebuffer. The guest display surface is BGRx, so
/// the human-authored ARGB preset values are swizzled here once at paint time.
#[inline]
fn pack(argb: u32) -> u32 {
    let r = (argb >> 16) & 0xFF;
    let g = (argb >> 8) & 0xFF;
    let b = argb & 0xFF;
    0xFF000000 | (b << 16) | (g << 8) | r
}

/// A gradient preset: top and bottom colors (ARGB)
#[derive(Clone, Copy)]
pub struct Preset {
    pub name: &'static str,
    pub top: u32,
    pub bottom: u32,
}

/// Built-in wallpaper presets (Nord + friends)
pub static PRESETS: &[Preset] = &[
    Preset { name: "Nord Dusk",    top: 0xFF2E3440, bottom: 0xFF3B4252 },
    Preset { name: "Nord Aurora",  top: 0xFF5E81AC, bottom: 0xFF2E3440 },
    Preset { name: "Deep Ocean",   top: 0xFF0F2027, bottom: 0xFF203A43 },
    Preset { name: "Sunset",       top: 0xFF355C7D, bottom: 0xFFC06C84 },
    Preset { name: "Forest",       top: 0xFF134E5E, bottom: 0xFF71B280 },
    Preset { name: "Royal",        top: 0xFF141E30, bottom: 0xFF243B55 },
    Preset { name: "Midnight",     top: 0xFF0B1021, bottom: 0xFF1B2735 },
];

static mut ACTIVE: usize = 0;

/// Get the active preset index
pub fn active_index() -> usize {
    unsafe { ACTIVE }
}

/// Get the active preset
pub fn active() -> &'static Preset {
    &PRESETS[active_index() % PRESETS.len()]
}

/// Cycle to the next preset and persist the choice.
pub fn next() {
    unsafe {
        ACTIVE = (ACTIVE + 1) % PRESETS.len();
    }
    persist();
    let p = active();
    crate::wm::notifications::info("Wallpaper", p.name);
}

/// Set a preset by index (clamped) and persist.
pub fn set_index(i: usize) {
    unsafe {
        ACTIVE = i % PRESETS.len();
    }
    persist();
    paint_to_screen();
}

/// Set a preset by name (case-insensitive). Returns true when found.
pub fn set_by_name(name: &str) -> bool {
    for (i, p) in PRESETS.iter().enumerate() {
        if p.name.eq_ignore_ascii_case(name) {
            set_index(i);
            return true;
        }
    }
    false
}

/// Load the persisted choice from /sd/etc/wallpaper.conf (best effort).
pub fn load_persisted() {
    let data = match crate::fs::vfs::read_file("/sd/etc/wallpaper.conf") {
        Ok(d) => d,
        Err(_) => return,
    };
    let text = core::str::from_utf8(&data).unwrap_or("");
    // Format: "wallpaper=Nord Dusk"
    for line in text.lines() {
        if let Some(val) = line.strip_prefix("wallpaper=") {
            if set_by_name(val.trim()) {
                crate::println!("[wallpaper] Restored '{}' from /sd", val.trim());
            }
            return;
        }
    }
}

fn persist() {
    let p = active();
    let conf = alloc::format!("wallpaper={}\n", p.name);
    // Best effort — the /sd mount may not be up yet.
    if crate::fs::vfs::write_file("/sd/etc/wallpaper.conf", conf.as_bytes()).is_ok() {
        crate::println!("[wallpaper] Saved '{}' to /sd", p.name);
    }
}

/// Paint the wallpaper into the back buffer (row-fill gradient, QEMU-optimized).
pub fn paint(fb: &crate::display::fb::FramebufferInfo, back_buf: *mut u32) {
    let p = active();

    let (tr, tg, tb) = ((p.top >> 16) & 0xFF, (p.top >> 8) & 0xFF, p.top & 0xFF);
    let (br, bg_, bb) = ((p.bottom >> 16) & 0xFF, (p.bottom >> 8) & 0xFF, p.bottom & 0xFF);

    let row_pixels = fb.width as usize;
    let mut row_buf: [u32; 1920] = [0; 1920];

    for y in 0..fb.height as usize {
        let t = y as f32 / fb.height.max(1) as f32;
        let r = tr as f32 + (br as f32 - tr as f32) * t;
        let g = tg as f32 + (bg_ as f32 - tg as f32) * t;
        let b = tb as f32 + (bb as f32 - tb as f32) * t;
        let argb = 0xFF000000 | ((r as u32 & 0xFF) << 16) | ((g as u32 & 0xFF) << 8) | (b as u32 & 0xFF);
        let color = pack(argb);

        let count = row_pixels.min(1920);
        for x in 0..count {
            row_buf[x] = color;
        }
        unsafe {
            core::ptr::copy_nonoverlapping(row_buf.as_ptr(), back_buf.add(y * row_pixels), count);
        }
    }
}

/// Paint the wallpaper DIRECTLY to the device framebuffer (QEMU path).
/// In QEMU builds the compositor is skipped, so the wallpaper engine writes
/// straight to the FB (row-bulk for speed).
pub fn paint_to_screen() {
    let fb = get_info();
    if fb.width == 0 || fb.virt_addr == 0 {
        return;
    }
    let p = active();
    let (tr, tg, tb) = ((p.top >> 16) & 0xFF, (p.top >> 8) & 0xFF, p.top & 0xFF);
    let (br, bg_, bb) = ((p.bottom >> 16) & 0xFF, (p.bottom >> 8) & 0xFF, p.bottom & 0xFF);

    let row_pixels = fb.width as usize;
    let mut row_buf: [u32; 1920] = [0; 1920];
    let dst = fb.virt_addr as *mut u32;

    for y in 0..fb.height as usize {
        let t = y as f32 / fb.height.max(1) as f32;
        let r = tr as f32 + (br as f32 - tr as f32) * t;
        let g = tg as f32 + (bg_ as f32 - tg as f32) * t;
        let b = tb as f32 + (bb as f32 - tb as f32) * t;
        let argb = 0xFF000000 | ((r as u32 & 0xFF) << 16) | ((g as u32 & 0xFF) << 8) | (b as u32 & 0xFF);
        let color = pack(argb);
        let count = row_pixels.min(1920);
        for x in 0..count {
            row_buf[x] = color;
        }
        unsafe {
            core::ptr::copy_nonoverlapping(row_buf.as_ptr(), dst.add(y * row_pixels), count);
        }
    }
}

/// Paint a single wallpaper row into a destination slice (for dirty-rect repair).
pub fn paint_row(fb: &crate::display::fb::FramebufferInfo, y: u32, dst: &mut [u32]) {
    let p = active();
    let (tr, tg, tb) = ((p.top >> 16) & 0xFF, (p.top >> 8) & 0xFF, p.top & 0xFF);
    let (br, bg_, bb) = ((p.bottom >> 16) & 0xFF, (p.bottom >> 8) & 0xFF, p.bottom & 0xFF);
    let t = y as f32 / fb.height.max(1) as f32;
    let r = tr as f32 + (br as f32 - tr as f32) * t;
    let g = tg as f32 + (bg_ as f32 - tg as f32) * t;
    let b = tb as f32 + (bb as f32 - tb as f32) * t;
    let argb = 0xFF000000 | ((r as u32 & 0xFF) << 16) | ((g as u32 & 0xFF) << 8) | (b as u32 & 0xFF);
    let color = pack(argb);
    for px in dst.iter_mut().take(fb.width as usize) {
        *px = color;
    }
}
