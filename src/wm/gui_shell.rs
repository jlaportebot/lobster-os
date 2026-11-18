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

/// Set-1 scancode of the `C` key.
const SCANCODE_C: u8 = 0x2E;

/// Whether Ctrl is held. The QEMU input device reports modifiers in its own
/// bit layout (ctrl is bit 1), which `usb::hid` would misread as Shift, so
/// ask the device directly when it is the active keyboard source.
fn ctrl_down() -> bool {
    #[cfg(target_arch = "aarch64")]
    {
        if crate::drivers::qemu_input::is_present() {
            return crate::drivers::qemu_input::ctrl_held();
        }
    }
    crate::usb::hid::ctrl_held()
}

/// Ctrl+C in the Terminal: SIGINT to the foreground job (its exit brings the
/// prompt back); with no job, drop the half-typed line and reprompt.
fn handle_ctrl_c() {
    crate::console::with_terminal_mirror(|| {
        crate::drivers::uart::Uart10::puts("^C\r\n");
    });
    if crate::shell::foreground::is_active() {
        crate::shell::foreground::request_interrupt();
        return;
    }
    // SAFETY: the window manager (the only caller path) is single-threaded.
    unsafe {
        LINE_LEN = 0;
    }
    crate::console::with_terminal_mirror(|| {
        crate::drivers::uart::Uart10::puts("lobster:/# ");
    });
}

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
    if ctrl_down() {
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
            crate::console::with_terminal_mirror(|| {
                crate::drivers::uart::Uart10::puts("\r\n");
            });
            if !line.is_empty() {
                // Execute in the shell (output mirrors into the window)
                let _ = crate::shell::execute_for_gui(line);
            }
            // New prompt inside the window — unless a foreground job now
            // owns it (its exit prints the prompt).
            if !crate::shell::foreground::holds_prompt() {
                crate::console::with_terminal_mirror(|| {
                    crate::drivers::uart::Uart10::puts("lobster:/# ");
                });
            }
        }
        return true;
    }

    // Ctrl+C (set-1 'C' = 0x2E): interrupt the foreground job, or abandon
    // the line being typed.
    if scancode == SCANCODE_C && ctrl_down() {
        handle_ctrl_c();
        return true;
    }

    // Backspace (set-1)
    if scancode == 0x0E {
        unsafe {
            if LINE_LEN > 0 {
                LINE_LEN -= 1;
                // Erase the last character on the VT: BS + space + BS
                crate::console::with_terminal_mirror(|| {
                    crate::drivers::uart::Uart10::puts("\u{8} \u{8}");
                });
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
                    crate::console::with_terminal_mirror(|| {
                        crate::drivers::uart::Uart10::puts(s);
                    });
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
