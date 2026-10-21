//! GUI shell input — type shell commands into the Terminal window
//!
//! When the Terminal window is focused, HID keyboard characters are
//! captured into a line buffer (with backspace + echo). Enter executes the
//! line through the shell's command interpreter. All shell output goes
//! through println! → UART → mirror_serial_byte → the terminal's VT
//! screen, so results appear inside the window.

use crate::wm::window;

const MAX_LINE: usize = 256;

static mut LINE: [u8; MAX_LINE] = [0; MAX_LINE];
static mut LINE_LEN: usize = 0;

/// Whether the Terminal window currently owns keyboard input.
pub fn terminal_focused() -> bool {
    let fid = window::focused_id();
    fid != 0 && window::get_title(fid) == "Terminal"
}

/// Handle a keypress while the Terminal window is focused.
/// Returns true if the key was consumed (WM should not further dispatch).
pub fn handle_key(scancode: u8, ch: Option<char>) -> bool {
    if !terminal_focused() {
        return false;
    }

    // Ctrl+Plus / Ctrl+Minus: terminal font scale (set-1: '=' 0x0D, '-' 0x0C,
    // keypad + 0x4E, keypad - 0x4A)
    if crate::usb::hid::ctrl_held() {
        match scancode {
            0x0D | 0x4E => {
                crate::apps::terminal::font_scale_up();
                return true;
            }
            0x0C | 0x4A => {
                crate::apps::terminal::font_scale_down();
                return true;
            }
            _ => {}
        }
    }

    // Enter → execute the buffered line (set-1 scancode, matching
    // input.rs and hid::scancode_to_ascii)
    if scancode == 0x1C {
        unsafe {
            let line = core::str::from_utf8(&LINE[..LINE_LEN]).unwrap_or("");
            LINE_LEN = 0;
            // Echo the newline into the VT via the serial mirror path
            crate::drivers::uart::Uart10::puts("\r\n");
            if !line.is_empty() {
                // Execute in the shell (output mirrors into the window)
                let _ = crate::shell::execute_for_gui(line);
            }
            // New prompt inside the window
            crate::drivers::uart::Uart10::puts("lobster:/# ");
        }
        return true;
    }

    // Backspace (set-1)
    if scancode == 0x0E {
        unsafe {
            if LINE_LEN > 0 {
                LINE_LEN -= 1;
                // Erase the last character on the VT: BS + space + BS
                crate::drivers::uart::Uart10::puts("\u{8} \u{8}");
            }
        }
        return true;
    }

    // Printable character → append + echo
    if let Some(c) = ch {
        if !c.is_control() && (c as u32) >= 0x20 {
            unsafe {
                if LINE_LEN < MAX_LINE {
                    LINE[LINE_LEN] = c as u8;
                    LINE_LEN += 1;
                    // Echo (mirrors into the VT)
                    let mut buf = [0u8; 4];
                    let s = c.encode_utf8(&mut buf);
                    crate::drivers::uart::Uart10::puts(s);
                }
            }
            return true;
        }
    }

    false
}

/// Current buffered line (for debugging).
pub fn current_line() -> &'static str {
    unsafe { core::str::from_utf8(&LINE[..LINE_LEN]).unwrap_or("") }
}
