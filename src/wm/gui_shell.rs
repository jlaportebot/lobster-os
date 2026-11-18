//! GUI shell input — type shell commands into the Terminal window
//!
//! When the Terminal window is focused, keyboard characters are collected
//! in a [`LineBuffer`] (with backspace and echo). Enter runs the line
//! through the shell's command interpreter with the console mirror gate
//! open, so everything the command prints (built-ins, `println!` output and
//! EL0 program stdout) is fed through the UART mirror into the active
//! Terminal tab's VT screen.

use crate::sync::spinlock::Spinlock;
use crate::wm::window;

/// Longest command line the Terminal accepts.
const MAX_LINE: usize = 256;
/// Set-1 scancode of Enter.
const SC_ENTER: u8 = 0x1C;
/// Set-1 scancode of Backspace.
const SC_BACKSPACE: u8 = 0x0E;
/// Set-1 scancodes of '=' and keypad '+' (font scale up).
const SC_FONT_UP: [u8; 2] = [0x0D, 0x4E];
/// Set-1 scancodes of '-' and keypad '-' (font scale down).
const SC_FONT_DOWN: [u8; 2] = [0x0C, 0x4A];
/// Set-1 scancode of 'C' (Ctrl+C cancels the line).
const SC_C: u8 = 0x2E;
/// Set-1 scancode of 'L' (Ctrl+L clears the screen).
const SC_L: u8 = 0x26;
/// Erase the character before the cursor on a VT: back, blank, back.
const ERASE_LAST: &str = "\u{8} \u{8}";
/// Clear screen and home the cursor.
const CLEAR_SCREEN: &str = "\x1B[2J\x1B[H";

/// The command line being typed.
pub struct LineBuffer {
    bytes: [u8; MAX_LINE],
    len: usize,
}

impl LineBuffer {
    /// An empty line.
    pub const fn new() -> Self {
        Self {
            bytes: [0; MAX_LINE],
            len: 0,
        }
    }

    /// Append a printable ASCII character; false if the line is full or the
    /// character is not printable.
    pub fn push(&mut self, c: char) -> bool {
        if !(' '..='~').contains(&c) || self.len == MAX_LINE {
            return false;
        }
        self.bytes[self.len] = c as u8;
        self.len += 1;
        true
    }

    /// Remove the last character; false if the line was already empty.
    pub fn backspace(&mut self) -> bool {
        if self.len == 0 {
            return false;
        }
        self.len -= 1;
        true
    }

    /// The text typed so far.
    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.len]).unwrap_or("")
    }

    /// Hand back the line and start a new one.
    pub fn take(&mut self) -> alloc::string::String {
        let line = alloc::string::String::from(self.as_str());
        self.len = 0;
        line
    }

    /// Discard the line.
    pub fn clear(&mut self) {
        self.len = 0;
    }
}

impl Default for LineBuffer {
    fn default() -> Self {
        Self::new()
    }
}

static LINE: Spinlock<LineBuffer> = Spinlock::new(LineBuffer::new());

/// Whether the Terminal window currently owns keyboard input.
pub fn terminal_focused() -> bool {
    let fid = window::focused_id();
    fid != 0 && window::get_title(fid) == "Terminal"
}

/// Write text into the active Terminal tab through the UART mirror.
fn echo(text: &str) {
    crate::console::with_terminal_mirror(|| crate::drivers::uart::Uart10::puts(text));
}

/// Print a fresh prompt for the shell's current directory.
fn prompt() {
    echo(&crate::shell::gui_prompt());
}

/// Run the typed line and print the next prompt.
fn submit() {
    let line = LINE.lock().take();
    echo("\r\n");
    if !line.is_empty() {
        crate::console::with_terminal_mirror(|| crate::shell::execute_for_gui(&line));
    }
    prompt();
}

/// Ctrl+C: abandon the typed line.
fn cancel_line() {
    LINE.lock().clear();
    echo("^C\r\n");
    prompt();
}

/// Ctrl+L: clear the screen and redraw the prompt and the typed text.
fn clear_screen() {
    echo(CLEAR_SCREEN);
    prompt();
    let typed = alloc::string::String::from(LINE.lock().as_str());
    echo(&typed);
}

/// Terminal-specific Ctrl combinations. Returns true if consumed.
fn handle_ctrl(scancode: u8) -> bool {
    if SC_FONT_UP.contains(&scancode) {
        crate::apps::terminal::font_scale_up();
    } else if SC_FONT_DOWN.contains(&scancode) {
        crate::apps::terminal::font_scale_down();
    } else if scancode == SC_C {
        cancel_line();
    } else if scancode == SC_L {
        clear_screen();
    } else {
        return false;
    }
    true
}

/// Handle a keypress while the Terminal window is focused.
/// Returns true if the key was consumed (WM should not further dispatch).
pub fn handle_key(scancode: u8, ch: Option<char>) -> bool {
    if !terminal_focused() {
        return false;
    }
    if crate::usb::hid::ctrl_held() && handle_ctrl(scancode) {
        return true;
    }
    match scancode {
        SC_ENTER => submit(),
        SC_BACKSPACE => {
            if LINE.lock().backspace() {
                echo(ERASE_LAST);
            }
        }
        _ => return type_char(ch),
    }
    true
}

/// Append a printable character to the line and echo it.
fn type_char(ch: Option<char>) -> bool {
    let Some(c) = ch else {
        return false;
    };
    if c.is_control() {
        return false;
    }
    if LINE.lock().push(c) {
        let mut buf = [0u8; 4];
        echo(c.encode_utf8(&mut buf));
    }
    true
}

/// Current buffered line (for diagnostics).
pub fn current_line() -> alloc::string::String {
    alloc::string::String::from(LINE.lock().as_str())
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    #[test]
    fn push_and_take_roundtrip() {
        let mut line = LineBuffer::new();
        for c in "ls /sd".chars() {
            assert!(line.push(c));
        }
        assert_eq!(line.as_str(), "ls /sd");
        assert_eq!(line.take(), "ls /sd");
        assert_eq!(line.as_str(), "");
    }

    #[test]
    fn backspace_removes_last_and_reports_empty() {
        let mut line = LineBuffer::new();
        line.push('a');
        line.push('b');
        assert!(line.backspace());
        assert_eq!(line.as_str(), "a");
        assert!(line.backspace());
        assert!(!line.backspace());
    }

    #[test]
    fn rejects_control_and_non_ascii_characters() {
        let mut line = LineBuffer::new();
        assert!(!line.push('\n'));
        assert!(!line.push('\u{7f}'));
        assert!(!line.push('é'));
        assert_eq!(line.as_str(), "");
    }

    #[test]
    fn full_line_refuses_more_input() {
        let mut line = LineBuffer::new();
        for _ in 0..MAX_LINE {
            assert!(line.push('x'));
        }
        assert!(!line.push('y'));
        assert_eq!(line.as_str().len(), MAX_LINE);
    }

    #[test]
    fn clear_discards_the_line() {
        let mut line = LineBuffer::new();
        line.push('q');
        line.clear();
        assert_eq!(line.as_str(), "");
    }
}
