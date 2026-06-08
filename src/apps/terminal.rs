//! Terminal Emulator for LobsterOS — PTY-based
//!
//! Opens /dev/ptmx to get a PTY master, spawns a shell on the slave side,
//! reads shell output from master, sends keyboard input to master.

use crate::fs::vfs::{close, open};
use crate::ipc::{pty_master_read, pty_master_write, pty_slave_read, pty_slave_write};
use crate::println;
use crate::wm::widget::{WidgetId, WidgetTable};
use crate::sched;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

/// Static holder for PTY slave fd used when spawning shell task
static mut SHELL_PTY_SLAVE_FD: Option<usize> = None;

/// Wrapper function for shell task entry point
fn shell_task_entry() -> ! {
    if let Some(fd) = unsafe { SHELL_PTY_SLAVE_FD } {
        crate::shell::run_with_pty(Some(fd));
    } else {
        crate::shell::run_with_pty(None);
    }
}

/// Terminal dimensions
pub const TERM_COLS: u32 = 80;
pub const TERM_ROWS: u32 = 24;
const SCROLLBACK_LINES: usize = 1000;

/// Terminal emulator state
pub struct Terminal {
    pub widget_id: Option<WidgetId>,
    /// PTY master file descriptor
    pty_master_fd: Option<usize>,
    /// PTY slave file descriptor (for reference)
    pty_slave_fd: Option<usize>,
    /// Screen buffer: each row is a string
    screen: Vec<String>,
    /// Cursor position (row, col)
    cursor_row: usize,
    cursor_col: usize,
    /// Scrollback buffer
    scrollback: Vec<String>,
    /// Current scrollback offset (0 = at bottom)
    scrollback_offset: usize,
    /// Shell PID
    shell_pid: Option<u64>,
    /// Terminal size
    width: u32,
    height: u32,
    /// Input buffer for keyboard
    input_buf: [u8; 256],
    input_len: usize,
}

impl Terminal {
    pub fn new() -> Self {
        Self {
            widget_id: None,
            pty_master_fd: None,
            pty_slave_fd: None,
            screen: vec![String::new(); TERM_ROWS as usize],
            cursor_row: 0,
            cursor_col: 0,
            scrollback: Vec::new(),
            scrollback_offset: 0,
            shell_pid: None,
            width: TERM_COLS,
            height: TERM_ROWS,
            input_buf: [0; 256],
            input_len: 0,
        }
    }

    /// Initialize the terminal: open PTY, spawn shell
    pub fn init(&mut self) -> Result<(), &'static str> {
        // Open PTY master/slave pair via IPC
        let (master_fd, slave_fd) = crate::ipc::pty_open().ok_or("Failed to open PTY")?;

        self.pty_master_fd = Some(master_fd);
        self.pty_slave_fd = Some(slave_fd);

        println!(
            "[terminal] PTY opened: master_fd={}, slave_fd={}",
            master_fd, slave_fd
        );

        // Spawn a new task to run the shell with the PTY slave
        unsafe {
            SHELL_PTY_SLAVE_FD = Some(slave_fd);
        }
        let shell_pid = crate::sched::create_kernel_task(
            shell_task_entry,
            crate::sched::Priority::Normal,
        );

        if let Some(pid) = shell_pid {
            println!("[terminal] Spawned shell on PTY as PID {}", pid);
            self.shell_pid = Some(pid);
        } else {
            println!("[terminal] WARNING: Failed to spawn shell task");
        }

        // Initialize screen
        self.screen = vec![String::new(); self.height as usize];
        self.cursor_row = 0;
        self.cursor_col = 0;

        Ok(())
    }

/// Initialize the terminal widget in the WM
    pub fn init_widget(&mut self, registry: &mut WidgetTable, x: u32, y: u32, w: u32, h: u32) {
        self.width = w / 8;
        self.height = h / 16;
        self.screen = vec![String::new(); self.height as usize];
        let id = registry.add_text_input(x, y, w, h);
        self.widget_id = id;
        println!("[terminal] Terminal widget created with id {:?}", id);
    }

    /// Spawn shell process with PTY slave as stdio
    fn spawn_shell_on_pty(&mut self, slave_fd: usize) {
        // We need to set up the shell with the PTY slave fd
        // For now, we'll set up the current process to use the PTY
        // In a full implementation, we'd spawn a new process with the PTY fd

        // Store the slave fd for reference
        self.pty_slave_fd = Some(slave_fd);

        // TODO: Actually spawn a shell process with the PTY slave
        // For now, the shell runs in the main kernel task and we
        // manually pump the PTY in tick()
    }

    /// Main tick: read from PTY master, update screen
    pub fn tick(&mut self) {
        if let Some(master_fd) = self.pty_master_fd {
            let mut buf = [0u8; 256];
            let n = pty_master_read(master_fd, &mut buf);
            if n > 0 {
                self.process_output(&buf[..n]);
            }
        }
    }

    /// Process output from shell (VT100/ANSI)
    fn process_output(&mut self, data: &[u8]) {
        for &byte in data {
            match byte {
                b'\n' => self.newline(),
                b'\r' => self.cursor_col = 0,
                b'\x08' => self.backspace(), // Backspace
                b'\t' => self.tab(),
                b'\x1b' => { // ESC - start of escape sequence
                    // We need to parse the full escape sequence
                    // For simplicity, collect until we have a complete sequence
                    // In a real implementation, use the VT100 parser
                }
                c if c >= 32 && c <= 126 => self.put_char(c as char),
                _ => {}
            }
        }
    }

    /// Write a character at cursor position
    fn put_char(&mut self, ch: char) {
        if self.cursor_row >= self.screen.len() {
            return;
        }
        let row = &mut self.screen[self.cursor_row];
        if self.cursor_col >= row.len() {
            row.push(ch);
        } else {
            // Insert at cursor position
            let mut chars: Vec<char> = row.chars().collect();
            if self.cursor_col < chars.len() {
                chars[self.cursor_col] = ch;
            } else {
                chars.push(ch);
            }
            *row = chars.into_iter().collect();
        }
        self.cursor_col += 1;
        if self.cursor_col >= self.width as usize {
            self.newline();
        }
    }

    /// Move to next line
    fn newline(&mut self) {
        self.cursor_row += 1;
        self.cursor_col = 0;
        if self.cursor_row >= self.height as usize {
            self.scroll_screen_up();
        }
        // Ensure row exists
        if self.cursor_row >= self.screen.len() {
            self.screen.push(String::new());
        }
    }

    /// Scroll screen up by one line
    fn scroll_screen_up(&mut self) {
        // Add current top line to scrollback
        if !self.screen.is_empty() {
            self.scrollback.push(self.screen[0].clone());
            if self.scrollback.len() > SCROLLBACK_LINES {
                self.scrollback.remove(0);
            }
        }
        self.screen.remove(0);
        self.screen.push(String::new());
        self.cursor_row = self.height as usize - 1;
    }

    /// Handle backspace
    fn backspace(&mut self) {
        if self.cursor_col > 0 {
            self.cursor_col -= 1;
            if self.cursor_row < self.screen.len() {
                let row = &mut self.screen[self.cursor_row];
                if self.cursor_col < row.len() {
                    row.remove(self.cursor_col);
                }
            }
        } else if self.cursor_row > 0 {
            // Move to end of previous line
            self.cursor_row -= 1;
            if self.cursor_row < self.screen.len() {
                self.cursor_col = self.screen[self.cursor_row].len();
            }
        }
    }

    /// Handle tab
    fn tab(&mut self) {
        let tab_width = 8;
        let spaces = tab_width - (self.cursor_col % tab_width);
        for _ in 0..spaces {
            self.put_char(' ');
        }
    }

    /// Write text to the terminal (from PTY master)
    pub fn write(&mut self, text: &str) {
        for ch in text.chars() {
            match ch {
                '\n' => self.newline(),
                '\r' => self.cursor_col = 0,
                '\x08' => self.backspace(),
                '\t' => self.tab(),
                c if c >= ' ' && c <= '~' => self.put_char(c),
                _ => {}
            }
        }
    }

    /// Send input to PTY master (keyboard -> shell)
    pub fn send_input(&mut self, data: &[u8]) {
        if let Some(master_fd) = self.pty_master_fd {
            let _ = pty_master_write(master_fd, data);
        }
    }

    /// Handle a key press
    pub fn handle_key(&mut self, key: u8) {
        match key {
            b'\n' | b'\r' => {
                self.send_input(b"\r\n");
            }
            b'\x08' | b'\x7f' => { // Backspace/DEL
                self.send_input(b"\x7f");
            }
            b'\t' => {
                self.send_input(b"\t");
            }
            b'\x1b' => { // ESC - start of escape sequence
                self.send_input(b"\x1b");
            }
            c if c >= 32 && c <= 126 => {
                self.send_input(&[c]);
            }
            _ => {}
        }
    }

    /// Handle special key combinations (Ctrl, Alt)
    pub fn handle_special_key(&mut self, scancode: u8, shift: bool, ctrl: bool, alt: bool) {
        use crate::usb::hid;

        if ctrl {
            match scancode {
                0x2C => self.send_input(b"\x03"), // Ctrl+C = SIGINT
                0x2D => self.send_input(b"\x04"), // Ctrl+D = EOF
                0x1E => self.send_input(b"\x01"), // Ctrl+A = start of line
                0x20 => self.send_input(b"\x05"), // Ctrl+E = end of line
                0x0F => self.send_input(b"\x0C"), // Ctrl+L = clear screen
                _ => {}
            }
        } else if alt {
            match scancode {
                0x2B => {
                    // Alt+Tab - cycle focus (handled by WM)
                }
                _ => {}
            }
        } else {
            // Regular key
            if let Some(ch) = hid::scancode_to_ascii(scancode, shift) {
                self.handle_key(ch as u8);
            }
        }
    }

    /// Render the terminal to the framebuffer
    pub fn render(&self, fb: &crate::display::fb::FramebufferInfo, registry: &WidgetTable) {
        if let Some(id) = self.widget_id {
            if let Some(widget) = registry.get(id) {
                if let crate::wm::widget::WidgetKind::TextInput(input) = widget {
                    let theme = crate::wm::theme::current();
                    let px = input.x;
                    let py = input.y;

                    // Draw background
                    crate::wm::widget::fill_rect(fb, px, py, input.w, input.h, theme.input_bg);

                    // Draw visible screen lines
                    let line_height = 16;
                    let char_width = 8;
                    let visible_rows = (input.h / line_height) as usize;

                    for i in 0..visible_rows.min(self.screen.len()) {
                        let row_idx = self.scrollback_offset + i;
                        if row_idx < self.screen.len() {
                            let line = &self.screen[row_idx];
                            let y = py + 4 + (i as u32 * line_height);
                            crate::display::graphics::draw_text(
                                fb,
                                px + 4,
                                y,
                                line,
                                theme.input_text,
                                theme.input_bg,
                            );
                        }
                    }

                    // Draw cursor if not in scrollback
                    if self.scrollback_offset == 0 && self.cursor_row < self.screen.len() {
                        let cx = px + 4 + (self.cursor_col as u32 * char_width);
                        let cy = py + 4 + (self.cursor_row as u32 * line_height);
                        // Draw cursor block
                        crate::wm::widget::fill_rect(fb, cx, cy, char_width, line_height, theme.input_text);
                    }
                }
            }
        }
    }

    /// Scroll up in scrollback
    pub fn scroll_up(&mut self) {
        if self.scrollback_offset < self.scrollback.len() {
            self.scrollback_offset += 1;
        }
    }

    /// Scroll down in scrollback
    pub fn scroll_down(&mut self) {
        if self.scrollback_offset > 0 {
            self.scrollback_offset -= 1;
        }
    }

    /// Clear screen
    pub fn clear(&mut self) {
        for row in &mut self.screen {
            row.clear();
        }
        self.cursor_row = 0;
        self.cursor_col = 0;
        self.send_input(b"\x1b[2J\x1b[H"); // ESC [ 2 J = clear screen, ESC [ H = home
    }
}

impl Default for Terminal {
    fn default() -> Self {
        Self::new()
    }
}

/// Global terminal instance
static mut TERMINAL: Terminal = Terminal {
    widget_id: None,
    pty_master_fd: None,
    pty_slave_fd: None,
    screen: Vec::new(),
    cursor_row: 0,
    cursor_col: 0,
    scrollback: Vec::new(),
    scrollback_offset: 0,
    shell_pid: None,
    width: TERM_COLS,
    height: TERM_ROWS,
    input_buf: [0; 256],
    input_len: 0,
};

/// Initialize the terminal application
pub fn init() {
    unsafe {
        TERMINAL = Terminal::new();
    }
    println!("[terminal] Terminal application initialized");
}

/// Get the global terminal instance
pub fn get_terminal() -> &'static mut Terminal {
    unsafe { &mut TERMINAL }
}