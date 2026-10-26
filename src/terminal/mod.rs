//! Terminal emulator for LobsterOS
//!
//! VT100/ANSI terminal implementation with:
//! - Parser state machine for escape sequences
//! - 2D screen buffer (cell grid with attributes)
//! - Framebuffer rendering

pub mod parser;
pub mod screen;

use crate::usb::hid;

/// Terminal dimensions (columns x rows)
pub const COLS: usize = 80;
pub const ROWS: usize = 25;

/// Terminal state
pub struct Terminal {
    pub parser: parser::Parser,
    pub screen: screen::ScreenBuffer,
    pub scrollback: screen::Scrollback,
    input_buf: [u8; 1024], // Raw input buffer
    input_len: usize,
}

impl Terminal {
    pub fn new() -> Self {
        Self {
            parser: parser::Parser::new(),
            screen: screen::ScreenBuffer::new(),
            scrollback: screen::Scrollback::new(),
            input_buf: [0u8; 1024],
            input_len: 0,
        }
    }

    /// Heap-allocate a Terminal (host tests: the 10k-line scrollback is ~10 MB,
    /// far too large for a test thread's stack). Constructs field-by-field
    /// through a raw pointer — no large stack temporary is ever created.
    #[cfg(all(test, feature = "std"))]
    pub fn new_boxed() -> Box<Self> {
        use std::alloc::{alloc, handle_alloc_error, Layout};
        let layout = Layout::new::<Self>();
        let ptr = unsafe {
            let raw = alloc(layout);
            if raw.is_null() {
                handle_alloc_error(layout);
            }
            raw as *mut Self
        };
        unsafe {
            core::ptr::addr_of_mut!((*ptr).parser)
                .write(parser::Parser::new());
            core::ptr::addr_of_mut!((*ptr).screen)
                .write(screen::ScreenBuffer::new());
            // Scrollback is the ~10 MB field — build it in place, no stack temp.
            screen::Scrollback::new_in_place(core::ptr::addr_of_mut!((*ptr).scrollback));
            core::ptr::addr_of_mut!((*ptr).input_buf).write([0u8; 1024]);
            core::ptr::addr_of_mut!((*ptr).input_len).write(0);
            Box::from_raw(ptr)
        }
    }

    /// Feed a byte to the VT100 parser
    pub fn feed_byte(&mut self, byte: u8) {
        self.parser.feed(byte, &mut self.screen);
    }

    /// Handle a keyboard key press
    pub fn on_keypress(&mut self, scancode: u8, shift: bool, ctrl: bool, alt: bool) {
        // Handle special keys
        if ctrl && shift {
            // Ctrl+Shift+C = Copy, Ctrl+Shift+V = Paste
            match hid::scancode_to_ascii(scancode, false) {
                Some('c') => {
                    // Copy current screen to clipboard
                    let row = self.screen.get_row_string(self.screen.cursor_row);
                    screen::copy_to_clipboard(row.as_bytes());
                    return;
                }
                Some('v') => {
                    // Paste from clipboard
                    let paste_data = screen::paste_from_clipboard();
                    for &b in paste_data {
                        self.send_byte(b);
                    }
                    return;
                }
                _ => {}
            }
        }

        // Shift+PageUp/PageDown = scrollback navigation
        if shift {
            match scancode {
                0x49 => {
                    // PageUp scancode
                    self.scrollback.scroll_up(10);
                    self.mark_dirty();
                    return;
                }
                0x51 => {
                    // PageDown scancode
                    self.scrollback.scroll_down(10);
                    self.mark_dirty();
                    return;
                }
                _ => {}
            }
        }

        // Convert scancode to ASCII
        let ch = hid::scancode_to_ascii(scancode, shift);

        if ctrl {
            match ch {
                Some('a') => self.send_key(0x01), // Ctrl+A
                Some('c') => self.send_key(0x03), // Ctrl+C (SIGINT)
                Some('d') => self.send_key(0x04), // Ctrl+D (EOF)
                Some('l') => {
                    self.screen.clear();
                    self.screen.cursor_home();
                } // Ctrl+L (clear)
                _ => {}
            }
        } else if alt {
            // Alt+key combinations
            match scancode {
                0x2B => self.cycle_focus(), // Alt+Tab
                _ => {}
            }
        }

        // Regular character input
        if let Some(c) = ch {
            match c {
                '\n' | '\r' => self.send_key(0x0A),     // Enter (LF)
                '\x7F' | '\x08' => self.send_key(0x7F), // Backspace (DEL)
                '\t' => self.send_key(0x09),            // Tab
                c if c >= ' ' && c <= '~' => {
                    self.send_byte(c as u8);
                }
                _ => {}
            }
        }
    }

    /// Send a single byte to terminal input
    fn send_byte(&mut self, byte: u8) {
        if self.input_len < self.input_buf.len() {
            self.input_buf[self.input_len] = byte;
            self.input_len += 1;
        }
    }

    /// Send a key code (control character)
    fn send_key(&mut self, code: u8) {
        self.send_byte(code);
    }

    /// Cycle window focus (Alt+Tab)
    fn cycle_focus(&mut self) {
        // This would integrate with WM to cycle focus
        #[cfg(target_arch = "aarch64")]
        crate::wm::window::cycle_focus();
    }

    /// Get input bytes and reset buffer
    pub fn take_input(&mut self) -> &[u8] {
        let slice = &self.input_buf[..self.input_len];
        self.input_len = 0;
        slice
    }

    /// Get screen buffer for rendering
    pub fn get_screen(&self) -> &screen::ScreenBuffer {
        &self.screen
    }

    /// Mark screen as dirty (needs redraw)
    pub fn mark_dirty(&mut self) {
        self.screen.dirty = true;
    }
}

impl Default for Terminal {
    fn default() -> Self {
        Self::new()
    }
}

impl core::fmt::Write for Terminal {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for byte in s.bytes() {
            self.feed_byte(byte);
        }
        Ok(())
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    #[test]
    fn test_terminal_creation() {
        let term = Box::leak(Terminal::new_boxed());
        assert_eq!(term.screen.cols, COLS);
        assert_eq!(term.screen.rows, ROWS);
    }

    #[test]
    fn test_feed_byte() {
        // Terminal embeds a 10k-line scrollback (~10 MB) — construct it on the
        // HEAP, not the stack, or the test thread's stack overflows.
        // Box::leak avoids a drop at 10 MB; process exit reclaims it.
        let mut term = Box::leak(Terminal::new_boxed());
        term.feed_byte(b'H');
        term.feed_byte(b'i');
        term.feed_byte(b'\n');
        // Should have written "Hi\n" to screen
    }
}
