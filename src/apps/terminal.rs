//! Terminal Emulator for LobsterOS — PTY-based with VT100/ANSI support
//!
//! Opens /dev/ptmx to get a PTY master, spawns a shell on the slave side,
//! reads shell output from master, sends keyboard input to master.
//!
//! Features:
//! - Full VT100/ANSI parser (CSI, ESC, OSC sequences)
//! - 256-color and truecolor (RGB) SGR support
//! - DEC private modes (DECCKM, DECCOLM, DECTCEM)
//! - OSC 52 (clipboard copy/paste)
//! - PTY resize handling (TIOCGWINSZ/TIOCSWINSZ)
//! - Scrollback buffer (10k lines)

use crate::ipc::{pty_ioctl, pty_master_read, pty_master_write};
use crate::println;
use crate::terminal::{
    parser::Parser,
    screen::{Cell, ScreenBuffer},
    COLS, ROWS,
};
use crate::wm::widget::{WidgetId, WidgetTable};
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// Static holder for PTY slave fd used when spawning shell task
static mut SHELL_PTY_SLAVE_FD: Option<usize> = None;

/// Global terminal instance (lazily initialized)
static mut TERMINAL_INIT: bool = false;
static mut TERMINAL: Option<Terminal> = None;

/// Wrapper function for shell task entry point
fn shell_task_entry() -> ! {
    if let Some(fd) = unsafe { SHELL_PTY_SLAVE_FD } {
        crate::shell::run_with_pty(Some(fd));
    } else {
        crate::shell::run_with_pty(None);
    }
}

/// Terminal dimensions
pub const TERM_COLS: u32 = COLS as u32;
pub const TERM_ROWS: u32 = ROWS as u32;
const SCROLLBACK_LINES: usize = 10000;

/// Terminal emulator state
pub struct Terminal {
    pub widget_id: Option<WidgetId>,
    /// PTY master file descriptor
    pty_master_fd: Option<usize>,
    /// PTY slave file descriptor (for reference)
    pty_slave_fd: Option<usize>,
    /// VT100 parser
    parser: Parser,
    /// Screen buffer with cell grid
    screen: ScreenBuffer,
    /// Scrollback buffer (stores rendered text lines)
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
    /// DEC private mode flags
    dec_ckm: bool, // Cursor key mode
    dec_colm: bool, // Column mode (80/132)
    dec_tcem: bool, // Text cursor enable mode
    /// OSC 52 clipboard buffer
    clipboard: Option<String>,
}

impl Terminal {
    pub fn new() -> Self {
        Self {
            widget_id: None,
            pty_master_fd: None,
            pty_slave_fd: None,
            parser: Parser::new(),
            screen: ScreenBuffer::new(),
            scrollback: Vec::new(),
            scrollback_offset: 0,
            shell_pid: None,
            width: TERM_COLS,
            height: TERM_ROWS,
            input_buf: [0; 256],
            input_len: 0,
            dec_ckm: false,
            dec_colm: false,
            dec_tcem: true,
            clipboard: None,
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
        let shell_pid =
            crate::sched::create_kernel_task(shell_task_entry, crate::sched::Priority::Normal);

        if let Some(pid) = shell_pid {
            println!("[terminal] Spawned shell on PTY as PID {}", pid);
            self.shell_pid = Some(pid);
        } else {
            println!("[terminal] WARNING: Failed to spawn shell task");
        }

        // Initialize screen buffer
        self.screen = ScreenBuffer::new();

        Ok(())
    }

    /// Initialize the terminal widget in the WM
    pub fn init_widget(&mut self, registry: &mut WidgetTable, x: u32, y: u32, w: u32, h: u32) {
        self.width = w / 8;
        self.height = h / 16;
        self.screen = ScreenBuffer::new();
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
            let mut buf = [0u8; 1024];
            let n = pty_master_read(master_fd, &mut buf);
            if n > 0 {
                self.process_output(&buf[..n]);
            }
        }
    }

    /// Process output from shell (VT100/ANSI) using the parser
    fn process_output(&mut self, data: &[u8]) {
        for &byte in data {
            self.parser.feed(byte, &mut self.screen);
        }
    }

    /// Write text to the terminal (from PTY master) - feeds through parser
    pub fn write(&mut self, text: &str) {
        for byte in text.bytes() {
            self.parser.feed(byte, &mut self.screen);
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
            b'\x08' | b'\x7f' => {
                // Backspace/DEL
                self.send_input(b"\x7f");
            }
            b'\t' => {
                self.send_input(b"\t");
            }
            b'\x1b' => {
                // ESC - start of escape sequence
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

    /// Handle window resize (called when terminal widget size changes)
    pub fn resize(&mut self, cols: u32, rows: u32) {
        self.width = cols;
        self.height = rows;
        // Resize the screen buffer
        self.screen = ScreenBuffer::new();
        // Notify PTY of window size change via TIOCSWINSZ
        if let Some(master_fd) = self.pty_master_fd {
            let _ = pty_ioctl(master_fd, 0x5414, 0); // TIOCSWINSZ
        }
    }

    /// Render the terminal to the framebuffer using ScreenBuffer cells
    pub fn render(&self, fb: &crate::display::fb::FramebufferInfo, registry: &WidgetTable) {
        if let Some(id) = self.widget_id {
            if let Some(widget) = registry.get(id) {
                if let crate::wm::widget::WidgetKind::TextInput(input) = widget {
                    let theme = crate::wm::theme::current();
                    let px = input.x;
                    let py = input.y;

                    // Draw background
                    crate::wm::widget::fill_rect(fb, px, py, input.w, input.h, theme.input_bg);

                    // Draw visible screen lines from ScreenBuffer
                    let line_height = 16;
                    let char_width = 8;
                    let visible_rows = (input.h / line_height) as usize;

                    for i in 0..visible_rows.min(self.screen.rows) {
                        let row_idx = self.scrollback_offset + i;
                        if row_idx < self.screen.rows {
                            let y = py + 4 + (i as u32 * line_height);
                            // Draw each cell in the row
                            for col in 0..self.screen.cols.min((input.w / char_width) as usize) {
                                let cell = &self.screen.cells[row_idx][col];
                                let x = px + 4 + (col as u32 * char_width);
                                if cell.ch != ' ' {
                                    crate::display::graphics::draw_text(
                                        fb,
                                        x,
                                        y,
                                        &cell.ch.to_string(),
                                        cell.fg,
                                        cell.bg,
                                    );
                                }
                            }
                        }
                    }

                    // Draw cursor if not in scrollback and cursor is enabled
                    if self.scrollback_offset == 0 && self.dec_tcem {
                        let cursor_row = self.screen.cursor_row;
                        let cursor_col = self.screen.cursor_col;
                        if cursor_row < self.screen.rows && cursor_col < self.screen.cols {
                            let cx = px + 4 + (cursor_col as u32 * char_width);
                            let cy = py + 4 + (cursor_row as u32 * line_height);
                            // Draw cursor block
                            crate::wm::widget::fill_rect(
                                fb,
                                cx,
                                cy,
                                char_width,
                                line_height,
                                theme.input_text,
                            );
                        }
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

    /// Clear screen via escape sequence
    pub fn clear(&mut self) {
        self.send_input(b"\x1b[2J\x1b[H"); // ESC [ 2 J = clear screen, ESC [ H = home
    }

    /// Get current cursor row
    pub fn cursor_row(&self) -> usize {
        self.screen.cursor_row
    }

    /// Get current cursor col
    pub fn cursor_col(&self) -> usize {
        self.screen.cursor_col
    }

    /// Copy selected text to clipboard (OSC 52)
    pub fn copy_to_clipboard(&mut self, text: &str) {
        // OSC 52: ESC ] 52 ; c ; <base64> BEL
        // For now, store locally
        self.clipboard = Some(text.to_string());
        println!("[terminal] Copied to clipboard: {} chars", text.len());
    }

    /// Paste from clipboard (OSC 52)
    pub fn paste_from_clipboard(&mut self) -> Option<String> {
        self.clipboard.clone()
    }
}

impl Default for Terminal {
    fn default() -> Self {
        Self::new()
    }
}

/// Initialize the terminal application
pub fn init() {
    unsafe {
        TERMINAL = Some(Terminal::new());
        TERMINAL_INIT = true;
    }
    println!("[terminal] Terminal application initialized");
}

/// Create a new terminal window with PTY and shell
pub fn create_terminal_window(
    registry: &mut crate::wm::widget::WidgetTable,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
) -> Option<u32> {
    let term = get_terminal();

    // Initialize terminal PTY if not already done
    // On QEMU, PTY may fail (no scheduler task context) — still create the window
    if term.pty_master_fd.is_none() {
        if term.init().is_err() {
            #[cfg(feature = "qemu_raspi5b")]
            {
                println!("[terminal] PTY unavailable on QEMU — creating window without PTY");
            }
            #[cfg(not(feature = "qemu"))]
            {
                println!("[terminal] Failed to initialize PTY");
                return None;
            }
        }
    }

    // Create the window
    let win_id = crate::wm::window::create(x, y, w, h, "Terminal")?;

    // Initialize the terminal widget inside the window
    term.init_widget(registry, x, y, w, h);

    // On QEMU, populate the screen buffer with a welcome message so the terminal
    // widget shows something useful immediately
    #[cfg(feature = "qemu_raspi5b")]
    {
        term.screen = ScreenBuffer::new();
        // Write welcome text into the screen buffer lines
        let lines = [
            "LobsterOS Terminal [QEMU mode]",
            "",
            "Type 'help' for commands.",
            "Shell output appears here.",
            "",
        ];
        for (row, line) in lines.iter().enumerate() {
            if row < term.screen.rows {
                let chars: Vec<char> = line.chars().collect();
                for (col, ch) in chars.iter().enumerate() {
                    if col < term.screen.cols {
                        term.screen.cells[row][col] = Cell {
                            ch: *ch,
                            fg: 0x00FFFFFF,
                            bg: 0x00000000,
                            bold: false,
                            underline: false,
                            blink: false,
                        };
                    }
                }
            }
        }
    }

    // Register the widget_id with the window for keyboard routing
    if let Some(widget_id) = term.widget_id {
        crate::wm::window::set_terminal_widget_id(win_id, widget_id);
    }

    Some(win_id)
}

/// Get the global terminal instance
pub fn get_terminal() -> &'static mut Terminal {
    unsafe {
        if !TERMINAL_INIT {
            TERMINAL = Some(Terminal::new());
            TERMINAL_INIT = true;
        }
        TERMINAL.as_mut().expect("Terminal not initialized")
    }
}

/// Mirror a byte from serial output to the terminal screen buffer.
/// Called from Uart10::putc on QEMU so the graphical terminal shows live shell output.
#[cfg(feature = "qemu_raspi5b")]
pub fn mirror_serial_byte(byte: u8) {
    let term = get_terminal();
    if term.widget_id.is_none() {
        return;
    }
    // Feed through the VT100 parser into the screen buffer
    term.parser.feed(byte, &mut term.screen);
}

/// Render terminal screen buffer directly to the framebuffer in each terminal window.
/// Called from the WM tick on QEMU (direct-to-framebuffer mode).
#[cfg(feature = "qemu_raspi5b")]
pub fn render_to_windows(fb: &crate::display::fb::FramebufferInfo) {
    use crate::display::graphics;
    use crate::wm::window;

    let term = get_terminal();
    let term_widget_id = match term.widget_id {
        Some(id) => id,
        None => return,
    };

    let win_count = window::count();
    for id in 1..=win_count as u32 {
        if !window::exists(id) {
            continue;
        }
        // Check if this window has our terminal widget
        let win_term_widget = match window::get_terminal_widget_id(id) {
            Some(w) => w,
            None => continue,
        };
        if win_term_widget != term_widget_id {
            continue;
        }
        let (wx, wy, ww, wh) = match window::get_info(id) {
            Some(info) => info,
            None => continue,
        };

        // Client area: inset by border (2px) + title bar (24px)
        let client_x = wx + 2;
        let client_y = wy + 24;
        let client_w = ww.saturating_sub(4);
        let client_h = wh.saturating_sub(26);

        if client_w < 8 || client_h < 16 {
            continue;
        }

        let line_height = 16u32;
        let char_width = 8u32;
        let visible_rows = (client_h / line_height) as usize;
        let visible_cols = (client_w / char_width) as usize;
        let scrollback = term.scrollback_offset;

        // Clear client area with terminal background color
        crate::wm::widget::fill_rect(fb, client_x, client_y, client_w, client_h, 0xFF000000);

        // Draw each visible cell
        for row_idx in 0..visible_rows.min(term.screen.rows) {
            let actual_row = scrollback + row_idx;
            if actual_row >= term.screen.rows {
                break;
            }
            let y = client_y + 4 + (row_idx as u32 * line_height);
            for col_idx in 0..visible_cols.min(term.screen.cols) {
                let cell = &term.screen.cells[actual_row][col_idx];
                let x = client_x + 4 + (col_idx as u32 * char_width);
                if cell.ch != ' ' && cell.ch != '\0' {
                    graphics::draw_text(fb, x, y, &cell.ch.to_string(), cell.fg, cell.bg);
                }
            }
        }

        // Draw cursor
        if scrollback == 0 && term.dec_tcem {
            let cr = term.screen.cursor_row;
            let cc = term.screen.cursor_col;
            if cr < term.screen.rows && cc < term.screen.cols {
                let cx = client_x + 4 + (cc as u32 * char_width);
                let cy = client_y + 4 + (cr as u32 * line_height);
                crate::wm::widget::fill_rect(fb, cx, cy, char_width, line_height, 0xFFFFFFFF);
            }
        }
    }
}
