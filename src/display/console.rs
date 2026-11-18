//! Framebuffer text console — draw characters on an 8x16 cell grid using the
//! anti-aliased mono face from [`super::text`]

use super::fb::FramebufferInfo;
use crate::println;

/// Color constants (ARGB format)
pub const WHITE: u32 = 0xFFFFFFFF;
pub const BLACK: u32 = 0xFF000000;
pub const RED: u32 = 0xFFFF0000;
pub const GREEN: u32 = 0xFF00FF00;
pub const BLUE: u32 = 0xFF0000FF;
pub const CYAN: u32 = 0xFF00FFFF;
pub const YELLOW: u32 = 0xFFFFFF00;
pub const MAGENTA: u32 = 0xFFFF00FF;
pub const GRAY: u32 = 0xFF808080;
pub const LIGHT_GRAY: u32 = 0xFFC0C0C0;
pub const DARK_GRAY: u32 = 0xFF404040;

/// Character dimensions
pub const CHAR_WIDTH: u32 = 8;
pub const CHAR_HEIGHT: u32 = 16;

/// Framebuffer console state
pub struct FbConsole {
    pub fb_info: FramebufferInfo,
    pub cursor_x: u32,
    pub cursor_y: u32,
    pub fg_color: u32,
    pub bg_color: u32,
    pub initialized: bool,
}

impl FbConsole {
    pub const fn empty() -> Self {
        Self {
            fb_info: FramebufferInfo::empty(),
            cursor_x: 0,
            cursor_y: 0,
            fg_color: WHITE,
            bg_color: BLACK,
            initialized: false,
        }
    }

    /// Initialize the console with framebuffer info
    pub fn init(&mut self, info: FramebufferInfo) {
        self.fb_info = info;
        self.cursor_x = 0;
        self.cursor_y = 0;
        self.fg_color = WHITE;
        self.bg_color = BLACK;
        self.initialized = true;
        self.clear();
    }

    /// Clear the screen
    pub fn clear(&mut self) {
        if !self.initialized {
            return;
        }
        let info = self.fb_info;
        if info.base_addr == 0 {
            return;
        }
        // Fill entire framebuffer with background color
        unsafe {
            let ptr = info.virt_addr as *mut u32;
            let total_pixels = (info.size / 4) as usize;
            for i in 0..total_pixels.min((info.width * info.height) as usize) {
                core::ptr::write_volatile(ptr.add(i), crate::display::graphics::fx(self.bg_color));
            }
        }
        self.cursor_x = 0;
        self.cursor_y = 0;
    }

    /// Draw a single character at a pixel position
    pub fn draw_char(&mut self, x: u32, y: u32, c: char, fg: u32, bg: u32) {
        if !self.initialized || self.fb_info.base_addr == 0 {
            return;
        }
        super::text::draw_cell(&self.fb_info, x, y, c, fg, Some(bg));
    }

    /// Write a character at the cursor position and advance
    pub fn write_char(&mut self, c: char) {
        if !self.initialized {
            return;
        }

        match c {
            '\n' => {
                self.cursor_x = 0;
                self.cursor_y += CHAR_HEIGHT;
            }
            '\r' => {
                self.cursor_x = 0;
            }
            '\t' => {
                // Tab to next 8-character boundary
                let tab_stop = ((self.cursor_x / CHAR_WIDTH + 8) & !7) * CHAR_WIDTH;
                self.cursor_x = tab_stop;
            }
            '\x08' => {
                // Backspace
                if self.cursor_x >= CHAR_WIDTH {
                    self.cursor_x -= CHAR_WIDTH;
                    self.draw_char(
                        self.cursor_x,
                        self.cursor_y,
                        ' ',
                        self.fg_color,
                        self.bg_color,
                    );
                }
            }
            _ => {
                self.draw_char(
                    self.cursor_x,
                    self.cursor_y,
                    c,
                    self.fg_color,
                    self.bg_color,
                );
                self.cursor_x += CHAR_WIDTH;

                // Wrap line
                if self.cursor_x + CHAR_WIDTH > self.fb_info.width {
                    self.cursor_x = 0;
                    self.cursor_y += CHAR_HEIGHT;
                }
            }
        }

        // Scroll if we've gone past the bottom
        if self.cursor_y + CHAR_HEIGHT > self.fb_info.height {
            self.scroll_up(1);
        }
    }

    /// Write a string
    pub fn write_str(&mut self, s: &str) {
        for c in s.chars() {
            self.write_char(c);
        }
    }

    /// Scroll the console up by N text lines
    pub fn scroll_up(&mut self, lines: u32) {
        if !self.initialized || self.fb_info.base_addr == 0 {
            return;
        }

        let info = self.fb_info;
        let pixels_per_line = CHAR_HEIGHT * lines;
        let pitch_words = info.pitch / 4;

        unsafe {
            let ptr = info.virt_addr as *mut u32;

            // Copy rows up
            for y in 0..(info.height - pixels_per_line) {
                let src_offset = ((y + pixels_per_line) * pitch_words) as usize;
                let dst_offset = (y * pitch_words) as usize;
                for x in 0..info.width as usize {
                    let pixel = core::ptr::read_volatile(ptr.add(src_offset + x));
                    core::ptr::write_volatile(ptr.add(dst_offset + x), pixel);
                }
            }

            // Clear the bottom
            for y in (info.height - pixels_per_line)..info.height {
                let offset = (y * pitch_words) as usize;
                for x in 0..info.width as usize {
                    core::ptr::write_volatile(
                        ptr.add(offset + x),
                        crate::display::graphics::fx(self.bg_color),
                    );
                }
            }
        }

        self.cursor_y = self.cursor_y.saturating_sub(pixels_per_line);
    }

    /// Set foreground color
    pub fn set_fg_color(&mut self, color: u32) {
        self.fg_color = color;
    }

    /// Set background color
    pub fn set_bg_color(&mut self, color: u32) {
        self.bg_color = color;
    }

    /// Get console dimensions in characters
    pub fn cols(&self) -> u32 {
        self.fb_info.width / CHAR_WIDTH
    }

    /// Get console rows
    pub fn rows(&self) -> u32 {
        self.fb_info.height / CHAR_HEIGHT
    }
}

/// Implement core::fmt::Write for FbConsole so println! works
impl core::fmt::Write for FbConsole {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        self.write_str(s);
        Ok(())
    }
}

/// Global framebuffer console
static mut FB_CONSOLE: Option<FbConsole> = None;

/// Initialize the framebuffer console
pub fn init(info: FramebufferInfo) {
    unsafe {
        let mut console = FbConsole::empty();
        console.init(info);
        FB_CONSOLE = Some(console);
    }
    println!(
        "[console] Framebuffer console initialized ({}x{} chars)",
        get_cols(),
        get_rows()
    );
}

/// Get number of columns
pub fn get_cols() -> u32 {
    unsafe {
        if let Some(ref c) = FB_CONSOLE {
            c.cols()
        } else {
            80
        }
    }
}

/// Get number of rows
pub fn get_rows() -> u32 {
    unsafe {
        if let Some(ref c) = FB_CONSOLE {
            c.rows()
        } else {
            25
        }
    }
}

/// Print a string to the framebuffer console
pub fn early_print(s: &str) {
    unsafe {
        if let Some(ref mut c) = FB_CONSOLE {
            c.write_str(s);
        }
    }
}

/// Write a character to the framebuffer console
pub fn early_putchar(c: char) {
    unsafe {
        if let Some(ref mut c_) = FB_CONSOLE {
            c_.write_char(c);
        }
    }
}

/// Clear the framebuffer console
pub fn clear() {
    unsafe {
        if let Some(ref mut c) = FB_CONSOLE {
            c.clear();
        }
    }
}

/// Set console foreground color
pub fn set_fg_color(color: u32) {
    unsafe {
        if let Some(ref mut c) = FB_CONSOLE {
            c.set_fg_color(color);
        }
    }
}

/// Set console background color
pub fn set_bg_color(color: u32) {
    unsafe {
        if let Some(ref mut c) = FB_CONSOLE {
            c.set_bg_color(color);
        }
    }
}
