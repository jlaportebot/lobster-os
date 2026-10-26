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
use alloc::boxed::Box;
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

/// Maximum number of terminal tabs
pub const MAX_TABS: usize = 4;
/// Height of the tab bar at the top of the window's client area
pub const TAB_BAR_H: u32 = 22;
const TAB_BTN_W: u32 = 24;
const TAB_BTN_GAP: u32 = 4;
/// Window frame chrome (matches wm::window_frame: 2px border + 24px title)
const FRAME_BORDER: u32 = 2;
const FRAME_TITLE: u32 = 24;

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
    /// Tabs: `screen`/`parser` always hold the ACTIVE tab's state; every other
    /// tab's (screen, parser) is parked in `saved[idx]` and swapped in on switch.
    tab_count: usize,
    active_tab: usize,
    saved: [Option<Box<(ScreenBuffer, Parser)>>; MAX_TABS],
    /// Font scale: 1 = 8x16 glyphs, 2 = 16x32
    font_scale: u32,
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
            tab_count: 1,
            active_tab: 0,
            saved: [None, None, None, None],
            font_scale: 1,
        }
    }

    /// Open a new tab (fresh VT screen) and switch to it. Returns false at MAX_TABS.
    pub fn add_tab(&mut self) -> bool {
        if self.tab_count >= MAX_TABS {
            return false;
        }
        let idx = self.tab_count;
        let mut tab = Box::new((ScreenBuffer::new(), Parser::new()));
        let banner = alloc::format!("LobsterOS Terminal - tab {}\r\nlobster:/# ", idx + 1);
        for b in banner.bytes() {
            tab.1.feed(b, &mut tab.0);
        }
        self.saved[idx] = Some(tab);
        self.tab_count += 1;
        self.switch_tab(idx);
        true
    }

    /// Make tab `idx` the active tab (swaps its screen/parser into place).
    pub fn switch_tab(&mut self, idx: usize) {
        if idx >= self.tab_count || idx == self.active_tab {
            return;
        }
        if let Some(mut other) = self.saved[idx].take() {
            core::mem::swap(&mut self.screen, &mut other.0);
            core::mem::swap(&mut self.parser, &mut other.1);
            self.saved[self.active_tab] = Some(other);
            self.active_tab = idx;
            self.scrollback_offset = 0;
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
            // Set 1 scancodes: Z=0x2C X=0x2D C=0x2E V=0x2F A=0x1E D=0x20 E=0x12 L=0x26.
            // (Audit Q6: the old table fired Ctrl+C on Ctrl+Z, Ctrl+D on Ctrl+X,
            //  and Ctrl+E on Ctrl+D.)
            match scancode {
                0x2E => self.send_input(b"\x03"), // Ctrl+C = SIGINT
                0x2F => self.send_input(b"\x04"), // Ctrl+D = EOF
                0x1E => self.send_input(b"\x01"), // Ctrl+A = start of line
                0x12 => self.send_input(b"\x05"), // Ctrl+E = end of line
                0x26 => self.send_input(b"\x0C"), // Ctrl+L = clear screen
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

/// x offset (from the client area's left edge) of tab-bar button `i`
/// (`i == tab_count` is the '+' button).
fn tab_btn_offset(i: usize) -> u32 {
    4 + i as u32 * (TAB_BTN_W + TAB_BTN_GAP)
}

/// Set the terminal font scale (1 = 8x16, 2 = 16x32). Out-of-range values clamp.
pub fn set_font_scale(scale: u32) {
    get_terminal().font_scale = scale.clamp(1, 2);
    crate::wm::dirty::mark();
}

/// Current terminal font scale (1 or 2).
pub fn font_scale() -> u32 {
    get_terminal().font_scale
}

/// Ctrl+Plus: 1x -> 2x
pub fn font_scale_up() {
    set_font_scale(font_scale() + 1);
}

/// Ctrl+Minus: 2x -> 1x
pub fn font_scale_down() {
    set_font_scale(font_scale().saturating_sub(1));
}

/// Tab-bar click handling (screen coords). Returns true if consumed.
pub fn handle_click(cx: u32, cy: u32) -> bool {
    use crate::wm::window;
    let Some(win_id) = window::find_by_title("Terminal") else { return false };
    let Some((x, y, w, h)) = window::get_info(win_id) else { return false };
    let client_x = x + FRAME_BORDER;
    let client_y = y + FRAME_BORDER + FRAME_TITLE;
    let client_w = w.saturating_sub(2 * FRAME_BORDER);
    let _ = h;
    if cx < client_x || cx >= client_x + client_w || cy < client_y || cy >= client_y + TAB_BAR_H {
        return false;
    }
    let term = get_terminal();
    let rel_x = cx - client_x;
    // Buttons: tabs 0..tab_count, then '+' (if room)
    let slots = if term.tab_count < MAX_TABS { term.tab_count + 1 } else { term.tab_count };
    for i in 0..slots {
        let bx = tab_btn_offset(i);
        if rel_x >= bx && rel_x < bx + TAB_BTN_W && cy >= client_y + 2 && cy < client_y + TAB_BAR_H - 2 {
            window::focus(win_id);
            if i < term.tab_count {
                term.switch_tab(i);
            } else {
                term.add_tab();
            }
            crate::wm::dirty::mark();
            return true;
        }
    }
    false
}

/// Draw one glyph scaled `scale`x (integer) from the baked-in 8x16 font,
/// merging horizontal runs of set pixels into rects. Transparent background.
#[cfg(feature = "qemu_raspi5b")]
fn draw_glyph_scaled(
    fb: &crate::display::fb::FramebufferInfo,
    x: u32,
    y: u32,
    ch: char,
    fg: u32,
    scale: u32,
) {
    let code = ch as usize;
    if code >= 128 {
        return;
    }
    let glyph = &crate::display::console::FONT[code];
    for row in 0..16u32 {
        let bits = glyph[row as usize];
        let mut col = 0u32;
        while col < 8 {
            if (bits >> (7 - col)) & 1 == 0 {
                col += 1;
                continue;
            }
            let start = col;
            while col < 8 && (bits >> (7 - col)) & 1 != 0 {
                col += 1;
            }
            crate::wm::widget::fill_rect(
                fb,
                x + start * scale,
                y + row * scale,
                (col - start) * scale,
                scale,
                fg,
            );
        }
    }
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

        // Uniform themed frame; render the VT into the returned client rect
        let focused = window::focused_id() == id;
        let (client_x, client_y, client_w, client_h) = crate::wm::window_frame::draw_frame(
            fb, id, wx, wy, ww, wh, "Terminal", focused,
        );

        if client_w < 8 || client_h < 16 {
            continue;
        }

        let scale = term.font_scale;
        let line_height = 16u32 * scale;
        let char_width = 8u32 * scale;
        let theme = crate::wm::theme::current();

        // Clear client area with terminal background color
        crate::wm::widget::fill_rect(fb, client_x, client_y, client_w, client_h, 0xFF000000);

        // Tab bar: one button per tab + '+' (while under MAX_TABS)
        crate::wm::widget::fill_rect(fb, client_x, client_y, client_w, TAB_BAR_H, theme.title_bar_unfocused);
        let slots = if term.tab_count < MAX_TABS { term.tab_count + 1 } else { term.tab_count };
        for i in 0..slots {
            let bx = client_x + tab_btn_offset(i);
            let by = client_y + 2;
            let active = i == term.active_tab;
            let bg = if active { theme.win_border } else { theme.btn_bg };
            crate::wm::widget::fill_rect(fb, bx, by, TAB_BTN_W, TAB_BAR_H - 4, bg);
            let label = if i < term.tab_count {
                (b'1' + i as u8) as char
            } else {
                '+'
            };
            let mut lbuf = [0u8; 4];
            graphics::draw_text(
                fb,
                bx + (TAB_BTN_W - 8) / 2,
                by + (TAB_BAR_H - 4 - 16) / 2,
                label.encode_utf8(&mut lbuf),
                if active { 0xFFFFFFFF } else { theme.btn_text },
                bg,
            );
        }

        let body_y = client_y + TAB_BAR_H;
        let body_h = client_h.saturating_sub(TAB_BAR_H + 4);
        let visible_rows = (body_h / line_height) as usize;
        let visible_cols = (client_w.saturating_sub(4) / char_width) as usize;
        let scrollback = term.scrollback_offset;

        // Draw each visible cell
        for row_idx in 0..visible_rows.min(term.screen.rows) {
            let actual_row = scrollback + row_idx;
            if actual_row >= term.screen.rows {
                break;
            }
            let y = body_y + 4 + (row_idx as u32 * line_height);
            for col_idx in 0..visible_cols.min(term.screen.cols) {
                let cell = &term.screen.cells[actual_row][col_idx];
                let x = client_x + 4 + (col_idx as u32 * char_width);
                if cell.ch != ' ' && cell.ch != '\0' {
                    if scale == 1 {
                        graphics::draw_text(fb, x, y, &cell.ch.to_string(), cell.fg, cell.bg);
                    } else {
                        draw_glyph_scaled(fb, x, y, cell.ch, cell.fg, scale);
                    }
                }
            }
        }

        // Draw cursor
        if scrollback == 0 && term.dec_tcem {
            let cr = term.screen.cursor_row;
            let cc = term.screen.cursor_col;
            if cr < term.screen.rows && cc < term.screen.cols && cr < visible_rows && cc < visible_cols {
                let cx = client_x + 4 + (cc as u32 * char_width);
                let cy = body_y + 4 + (cr as u32 * line_height);
                crate::wm::widget::fill_rect(fb, cx, cy, char_width, line_height, 0xFFFFFFFF);
            }
        }
    }
}
