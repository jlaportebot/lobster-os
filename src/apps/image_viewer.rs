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
