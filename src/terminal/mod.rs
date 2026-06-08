//! Terminal emulator for LobsterOS
//!
//! VT100/ANSI terminal implementation with:
//! - Parser state machine for escape sequences
//! - 2D screen buffer (cell grid with attributes)
//! - Framebuffer rendering

pub mod parser;
pub mod screen;

use crate::usb::hid;
use crate::println;
use core::fmt::Write;

/// Terminal dimensions (columns x rows)
pub const COLS: usize = 80;
pub const ROWS: usize = 25;

/// Terminal state
pub struct Terminal {
    pub parser: parser::Parser,
    pub screen: screen::ScreenBuffer,
    input_buf: [u8; 1024],  // Raw input buffer
    input_len: usize,
}

impl Terminal {
    pub fn new() -> Self {
        Self {
            parser: parser::Parser::new(),
            screen: screen::ScreenBuffer::new(),
            input_buf: [0u8; 1024],
            input_len: 0,
        }
    }

    /// Feed a byte to the VT100 parser
    pub fn feed_byte(&mut self, byte: u8) {
        self.parser.feed(byte, &mut self.screen);
    }

    /// Handle a keyboard key press
    pub fn on_keypress(&mut self, scancode: u8, shift: bool, ctrl: bool, alt: bool) {
        // Convert scancode to ASCII or control sequence
        let ch = hid::scancode_to_ascii(scancode, shift);
        
        // Handle special keys
        if ctrl {
            match ch {
                Some('a') => self.send_key(0x01),  // Ctrl+A
                Some('c') => self.send_key(0x03),  // Ctrl+C (SIGINT)
                Some('d') => self.send_key(0x04),  // Ctrl+D (EOF)
                Some('l') => {
                    self.screen.clear();
                    self.screen.cursor_home();
                },  // Ctrl+L (clear)
                _ => {}
            }
        } else if alt {
            // Alt+key combinations
            match scancode {
                0x2B => self.cycle_focus(),  // Alt+Tab
                _ => {}
            }
        } else {
            // Regular character input
            if let Some(c) = ch {
                match c {
                    '\n' | '\r' => self.send_key(0x0A),  // Enter (LF)
                    '\x7F' | '\x08' => self.send_key(0x7F),  // Backspace (DEL)
                    '\t' => self.send_key(0x09),  // Tab
                    c if c >= ' ' && c <= '~' => {
                        self.send_byte(c as u8);
                    }
                    _ => {}
                }
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

impl core::fmt::Write for Terminal {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for byte in s.bytes() {
            self.feed_byte(byte);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_terminal_creation() {
        let term = Terminal::new();
        assert_eq!(term.screen.cols, COLS);
        assert_eq!(term.screen.rows, ROWS);
    }

    #[test]
    fn test_feed_byte() {
        let mut term = Terminal::new();
        term.feed_byte(b'H');
        term.feed_byte(b'i');
        term.feed_byte(b'\n');
        // Should have written "Hi\n" to screen
    }
}
