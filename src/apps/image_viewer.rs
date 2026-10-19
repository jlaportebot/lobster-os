//! Image Viewer app — display BMP images and generated patterns
//!
//! Supports basic BMP file rendering with zoom and pan.

use crate::display::fb;
use crate::display::graphics;
use alloc::string::String;

const APP_NAME: &str = "Image Viewer";

/// Image viewer state
pub struct ImageViewer {
    image_w: u32,
    image_h: u32,
    zoom: u32,
    offset_x: i32,
    offset_y: i32,
    running: bool,
    file_path: String,
}

impl ImageViewer {
    pub fn new() -> Self {
        Self {
            image_w: 0,
            image_h: 0,
            zoom: 100,
            offset_x: 0,
            offset_y: 0,
            running: true,
            file_path: String::new(),
        }
    }

    /// Load a BMP image from VFS
    pub fn load_bmp(&mut self, path: &str) -> Result<(), &'static str> {
        self.file_path = String::from(path);
        // BMP loading would use VFS read
        // For now, set a placeholder size
        self.image_w = 320;
        self.image_h = 240;
        Ok(())
    }

    /// Generate a test pattern (gradient) for display
    pub fn show_test_pattern(&mut self) {
        self.image_w = 320;
        self.image_h = 240;
        self.file_path = String::from("test-pattern");
    }

    pub fn zoom_in(&mut self) {
        if self.zoom < 400 {
            self.zoom += 25;
        }
    }

    pub fn zoom_out(&mut self) {
        if self.zoom > 25 {
            self.zoom -= 25;
        }
    }

    pub fn pan(&mut self, dx: i32, dy: i32) {
        self.offset_x += dx;
        self.offset_y += dy;
    }

    pub fn reset_view(&mut self) {
        self.zoom = 100;
        self.offset_x = 0;
        self.offset_y = 0;
    }

    /// Render the image viewer
    pub fn render(&self, x: u32, y: u32, width: u32, height: u32) {
        let info = fb::get_info();

        // Title bar
        graphics::draw_rect(&info, x, y, width, 28, 0x2D2D30);
        graphics::draw_text(&info, x + 8, y + 6, APP_NAME, 0xFFFFFF, 0x2D2D30);

        let zoom_str = alloc::format!("Zoom: {}%", self.zoom);
        graphics::draw_text(&info, x + width - 80, y + 6, &zoom_str, 0xAAAACC, 0x2D2D30);

        // Image area background
        let img_y = y + 28;
        let img_h = height.saturating_sub(28);
        graphics::draw_rect(&info, x, img_y, width, img_h, 0x0A0A0A);

        // Draw a checkerboard pattern to indicate transparency
        let checker_size = 16u32;
        let mut cy = img_y;
        while cy < img_y + img_h {
            let mut cx = x;
            let row = (cy - img_y) / checker_size;
            while cx < x + width {
                let col = (cx - x) / checker_size;
                let color = if (row + col) % 2 == 0 {
                    0x1A1A1A
                } else {
                    0x2A2A2A
                };
                graphics::draw_rect(&info, cx, cy, checker_size, checker_size, color);
                cx += checker_size;
            }
            cy += checker_size;
        }

        // Draw image or placeholder
        if self.image_w > 0 {
            // Draw a gradient as placeholder for the image
            let draw_w = self.image_w * self.zoom / 100;
            let draw_h = self.image_h * self.zoom / 100;
            let draw_x = x as i32 + (width as i32 - draw_w as i32) / 2 + self.offset_x;
            let draw_y = img_y as i32 + (img_h as i32 - draw_h as i32) / 2 + self.offset_y;

            if draw_x >= 0 && draw_y >= 0 {
                let dx = draw_x as u32;
                let dy = draw_y as u32;
                let dw = draw_w.min(width);
                let dh = draw_h.min(img_h);

                // Draw gradient placeholder
                for row in 0..dh.min(draw_h) {
                    let r = (row * 255) / dh.max(1);
                    let color = 0xFF000000 | (r << 16) | (100 << 8) | 200;
                    graphics::draw_rect(&info, dx, dy + row, dw, 1, color);
                }
            }
        } else {
            graphics::draw_text(
                &info,
                x + 20,
                img_y + 20,
                "No image loaded",
                0x666688,
                0x0A0A0A,
            );
            graphics::draw_text(
                &info,
                x + 20,
                img_y + 40,
                "Right-click to open file",
                0x444466,
                0x0A0A0A,
            );
        }

        // Status bar
        let status_y = y + height - 20;
        let status = if self.image_w > 0 {
            alloc::format!("{}x{} | {}", self.image_w, self.image_h, self.file_path)
        } else {
            String::from("No file loaded")
        };
        graphics::draw_rect(&info, x, status_y, width, 20, 0x1A1A1A);
        graphics::draw_text(&info, x + 8, status_y + 4, &status, 0x666688, 0x1A1A1A);
    }

    pub fn is_running(&self) -> bool {
        self.running
    }

    pub fn close(&mut self) {
        self.running = false;
    }
}

impl Default for ImageViewer {
    fn default() -> Self {
        Self::new()
    }
}

pub fn init() {
    crate::println!("[apps] Image Viewer initialized");
}

// ═══════════════════════════════════════════════════════════════════════
// Real BMP image rendering (B6.1) — QEMU direct-FB path
// ═══════════════════════════════════════════════════════════════════════

#[cfg(feature = "qemu_raspi5b")]
pub mod bmp_view {
    use crate::wm::window;

    /// Decoded BMP (24/32-bit, bottom-up or top-down)
    pub struct BmpImage {
        pub width: u32,
        pub height: u32,
        /// ARGB pixels, row-major, top-down
        pub pixels: alloc::vec::Vec<u32>,
    }

    /// Parse a BMP file body. Returns None on unsupported formats.
    pub fn parse_bmp(data: &[u8]) -> Option<BmpImage> {
        if data.len() < 54 || &data[0..2] != b"BM" {
            return None;
        }
        let pixel_offset = u32::from_le_bytes([data[10], data[11], data[12], data[13]]) as usize;
        let width = i32::from_le_bytes([data[18], data[19], data[20], data[21]]);
        let height_raw = i32::from_le_bytes([data[22], data[23], data[24], data[25]]);
        let bpp = u16::from_le_bytes([data[28], data[29]]);
        if width <= 0 || height_raw == 0 || bpp < 24 || bpp > 32 {
            return None;
        }
        let top_down = height_raw < 0;
        let height = height_raw.unsigned_abs();
        let w = width as u32;
        let h = height as u32;
        let bytes_pp = (bpp / 8) as usize;
        let row_size = ((w as usize * bytes_pp) + 3) & !3; // 4-byte aligned

        let mut pixels = alloc::vec::Vec::with_capacity((w * h) as usize);
        for row in 0..h {
            let src_row = if top_down { row } else { h - 1 - row };
            let base = pixel_offset + src_row as usize * row_size;
            if base + row_size > data.len() {
                return None;
            }
            for col in 0..w {
                let o = base + col as usize * bytes_pp;
                let b = data[o] as u32;
                let g = data[o + 1] as u32;
                let r = data[o + 2] as u32;
                let a = if bytes_pp == 4 { data[o + 3] as u32 } else { 0xFF };
                // Store as ARGB; the fb swizzle happens at draw time via fx().
                pixels.push((a << 24) | (r << 16) | (g << 8) | b);
            }
        }
        Some(BmpImage { width: w, height: h, pixels })
    }

    static mut IMAGE: Option<BmpImage> = None;
    static mut SOURCE: alloc::string::String = alloc::string::String::new();

    /// Load a BMP from disk into the viewer (creates the window if needed).
    pub fn open(path: &str) -> Result<(), &'static str> {
        let data = crate::fs::vfs::read_file(path).map_err(|_| "cannot read file")?;
        let img = parse_bmp(&data).ok_or("unsupported format (need 24/32-bit BMP)")?;
        unsafe {
            IMAGE = Some(img);
            SOURCE = alloc::string::String::from(path);
        }
        match window::find_by_title("Image Viewer") {
            Some(id) => {
                window::focus(id);
                Ok(())
            }
            None => {
                window::create(400, 200, 640, 520, "Image Viewer")
                    .map(|_| ())
                    .ok_or("window create failed")
            }
        }
    }

    /// Render into the Image Viewer window (called from wm::tick).
    pub fn render_to_windows(fb: &crate::display::fb::FramebufferInfo) {
        let Some(win_id) = window::find_by_title("Image Viewer") else { return };
        let Some((x, y, w, h)) = window::get_info(win_id) else { return };
        let focused = window::focused_id() == win_id;
        let (cx, cy, cw, ch) = crate::wm::window_frame::draw_frame(
            fb, win_id, x, y, w, h, "Image Viewer", focused,
        );

        let theme = crate::wm::theme::current();
        unsafe {
            match IMAGE.as_ref() {
                Some(img) => {
                    // Center + fit-scale the image into the client area
                    let scale_x = cw as u32 * 100 / img.width.max(1);
                    let scale_y = ch as u32 * 100 / img.height.max(1);
                    let scale = scale_x.min(scale_y).min(100); // never upscale > 1:1
                    let dw = img.width * scale / 100;
                    let dh = img.height * scale / 100;
                    let dx = cx + (cw.saturating_sub(dw)) / 2;
                    let dy = cy + (ch.saturating_sub(dh)) / 2;

                    for py in 0..dh {
                        for px in 0..dw {
                            let sx = px * img.width / dw.max(1);
                            let sy = py * img.height / dh.max(1);
                            let argb = img.pixels[(sy * img.width + sx) as usize];
                            crate::display::graphics::draw_pixel(
                                fb, dx + px, dy + py,
                                crate::display::graphics::fx(argb),
                            );
                        }
                    }
                }
                None => {
                    crate::display::graphics::draw_rect(fb, cx, cy, cw, ch, theme.win_bg);
                    crate::display::graphics::draw_text(
                        fb, cx + 12, cy + 14,
                        "No image loaded — 'image <path-to-bmp>'",
                        theme.label_text_dim, theme.win_bg,
                    );
                }
            }
        }
    }
}

/// Shell: load a BMP into the viewer.
#[cfg(feature = "qemu_raspi5b")]
pub fn open_image(path: &str) -> Result<(), &'static str> {
    bmp_view::open(path)
}
