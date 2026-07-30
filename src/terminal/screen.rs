//! Terminal screen buffer - 2D cell grid

use crate::display::console::{BLACK, BLUE, CYAN, GREEN, MAGENTA, RED, WHITE, YELLOW};
use alloc::string::String;

/// Character cell in the terminal screen
#[derive(Clone, Copy, Debug)]
pub struct Cell {
    pub ch: char,
    pub fg: u32,
    pub bg: u32,
    pub bold: bool,
    pub underline: bool,
    pub blink: bool,
}

impl Cell {
    pub const fn new() -> Self {
        Self {
            ch: ' ',
            fg: WHITE,
            bg: BLACK,
            bold: false,
            underline: false,
            blink: false,
        }
    }
}

impl Default for Cell {
    fn default() -> Self {
        Self::new()
    }
}

/// SGR attributes from CSI params
/// Supports:
/// - Basic 16-color (30-37, 40-47, 90-97, 100-107)
/// - 256-color palette (38:5:n, 48:5:n)
/// - Truecolor RGB (38:2:r:g:b, 48:2:r:g:b)
/// - Basic attributes: 0=reset, 1=bold, 4=underline, 5=blink, 7=reverse, 22=normal, 24=no underline, 25=no blink, 27=no reverse
pub fn sgr_attr_from(params: &[u8]) -> (u32, u32, bool, bool, bool) {
    let mut fg = WHITE;
    let mut bg = BLACK;
    let mut bold = false;
    let mut underline = false;
    let mut blink = false;

    let mut i = 0;
    while i < params.len() {
        match params[i] {
            0 => {
                fg = WHITE;
                bg = BLACK;
                bold = false;
                underline = false;
                blink = false;
            }
            1 => bold = true,
            4 => underline = true,
            5 => blink = true,
            7 => { /* reverse - swap fg/bg */ }
            22 => {
                bold = false;
            }
            24 => {
                underline = false;
            }
            25 => {
                blink = false;
            }
            27 => { /* no reverse */ }
            30..=37 => {
                fg = match params[i] {
                    30 => BLACK,
                    31 => RED,
                    32 => GREEN,
                    33 => YELLOW,
                    34 => BLUE,
                    35 => MAGENTA,
                    36 => CYAN,
                    37 => WHITE,
                    _ => WHITE,
                }
            }
            38 => {
                // Extended foreground color: 38:5:n (256-color) or 38:2:r:g:b (truecolor)
                if i + 1 < params.len() {
                    match params[i + 1] {
                        5 => {
                            // 256-color palette
                            if i + 2 < params.len() {
                                let idx = params[i + 2] as u8;
                                fg = palette256_color(idx);
                            }
                            i += 2;
                        }
                        2 => {
                            // Truecolor RGB
                            if i + 4 < params.len() {
                                let r = params[i + 2] as u32;
                                let g = params[i + 3] as u32;
                                let b = params[i + 4] as u32;
                                fg = (r << 16) | (g << 8) | b;
                            }
                            i += 4;
                        }
                        _ => {}
                    }
                }
            }
            39 => {
                // Default foreground
                fg = WHITE;
            }
            40..=47 => {
                bg = match params[i] {
                    40 => BLACK,
                    41 => RED,
                    42 => GREEN,
                    43 => YELLOW,
                    44 => BLUE,
                    45 => MAGENTA,
                    46 => CYAN,
                    47 => WHITE,
                    _ => BLACK,
                }
            }
            48 => {
                // Extended background color: 48:5:n (256-color) or 48:2:r:g:b (truecolor)
                if i + 1 < params.len() {
                    match params[i + 1] {
                        5 => {
                            // 256-color palette
                            if i + 2 < params.len() {
                                let idx = params[i + 2] as u8;
                                bg = palette256_color(idx);
                            }
                            i += 2;
                        }
                        2 => {
                            // Truecolor RGB
                            if i + 4 < params.len() {
                                let r = params[i + 2] as u32;
                                let g = params[i + 3] as u32;
                                let b = params[i + 4] as u32;
                                bg = (r << 16) | (g << 8) | b;
                            }
                            i += 4;
                        }
                        _ => {}
                    }
                }
            }
            49 => {
                // Default background
                bg = BLACK;
            }
            90..=97 => {
                // Bright foreground colors
                fg = match params[i] {
                    90 => BRIGHT_BLACK,
                    91 => BRIGHT_RED,
                    92 => BRIGHT_GREEN,
                    93 => BRIGHT_YELLOW,
                    94 => BRIGHT_BLUE,
                    95 => BRIGHT_MAGENTA,
                    96 => BRIGHT_CYAN,
                    97 => BRIGHT_WHITE,
                    _ => WHITE,
                }
            }
            100..=107 => {
                // Bright background colors
                bg = match params[i] {
                    100 => BRIGHT_BLACK,
                    101 => BRIGHT_RED,
                    102 => BRIGHT_GREEN,
                    103 => BRIGHT_YELLOW,
                    104 => BRIGHT_BLUE,
                    105 => BRIGHT_MAGENTA,
                    106 => BRIGHT_CYAN,
                    107 => BRIGHT_WHITE,
                    _ => BLACK,
                }
            }
            _ => {}
        }
        i += 1;
    }
    (fg, bg, bold, underline, blink)
}

/// 256-color palette lookup
/// Standard xterm 256-color palette:
/// 0-15: standard 16 colors
/// 16-231: 6x6x6 color cube (RGB)
/// 232-255: grayscale ramp
fn palette256_color(idx: u8) -> u32 {
    if idx <= 15 {
        // Standard 16 colors
        match idx {
            0 => BLACK,
            1 => RED,
            2 => GREEN,
            3 => YELLOW,
            4 => BLUE,
            5 => MAGENTA,
            6 => CYAN,
            7 => WHITE,
            8 => BRIGHT_BLACK,
            9 => BRIGHT_RED,
            10 => BRIGHT_GREEN,
            11 => BRIGHT_YELLOW,
            12 => BRIGHT_BLUE,
            13 => BRIGHT_MAGENTA,
            14 => BRIGHT_CYAN,
            15 => BRIGHT_WHITE,
            _ => WHITE,
        }
    } else if idx <= 231 {
        // 6x6x6 color cube
        let idx = idx - 16;
        let r = (idx / 36) as u32;
        let g = ((idx % 36) / 6) as u32;
        let b = (idx % 6) as u32;
        // Map 0-5 to 0x00, 0x5F, 0x87, 0xAF, 0xD7, 0xFF
        let cube = [0x00, 0x5F, 0x87, 0xAF, 0xD7, 0xFF];
        (cube[r as usize] << 16) | (cube[g as usize] << 8) | cube[b as usize]
    } else {
        // Grayscale ramp (232-255)
        let idx = idx - 232;
        let v = 0x08 + idx as u32 * 10;
        (v << 16) | (v << 8) | v
    }
}

/// Bright color constants for 90-97 / 100-107 SGR
const BRIGHT_BLACK: u32 = 0x808080;
const BRIGHT_RED: u32 = 0xFF5555;
const BRIGHT_GREEN: u32 = 0x55FF55;
const BRIGHT_YELLOW: u32 = 0xFFFF55;
const BRIGHT_BLUE: u32 = 0x5555FF;
const BRIGHT_MAGENTA: u32 = 0xFF55FF;
const BRIGHT_CYAN: u32 = 0x55FFFF;
const BRIGHT_WHITE: u32 = 0xFFFFFF;

/// Screen buffer - 2D grid of cells
pub struct ScreenBuffer {
    pub cells: [[Cell; 80]; 25],
    pub cursor_row: usize,
    pub cursor_col: usize,
    pub cursor_row_saved: usize,
    pub cursor_col_saved: usize,
    pub cols: usize,
    pub rows: usize,
    pub attrs: (u32, u32, bool, bool, bool),
    pub dirty: bool,
}

impl ScreenBuffer {
    pub fn new() -> Self {
        let mut cells = [[Cell::new(); 80]; 25];
        for row in cells.iter_mut() {
            for cell in row.iter_mut() {
                *cell = Cell::new();
            }
        }
        Self {
            cells,
            cursor_row: 0,
            cursor_col: 0,
            cursor_row_saved: 0,
            cursor_col_saved: 0,
            cols: 80,
            rows: 25,
            attrs: (WHITE, BLACK, false, false, false),
            dirty: true,
        }
    }

    pub fn put_char(&mut self, row: usize, col: usize, ch: char) {
        if row < self.rows && col < self.cols {
            let (fg, bg, bold, underline, blink) = self.attrs;
            self.cells[row][col] = Cell {
                ch,
                fg,
                bg,
                bold,
                underline,
                blink,
            };
        }
    }

    pub fn clear_all(&mut self) {
        for row in 0..self.rows {
            for col in 0..self.cols {
                self.cells[row][col] = Cell::new();
            }
        }
        self.cursor_row = 0;
        self.cursor_col = 0;
        self.dirty = true;
    }

    pub fn clear_from_cursor(&mut self) {
        // Clear to end of screen
        for r in self.cursor_row..self.rows {
            for c in if r == self.cursor_row {
                self.cursor_col
            } else {
                0
            }..self.cols
            {
                self.cells[r][c] = Cell::new();
            }
        }
        self.dirty = true;
    }

    pub fn clear_to_cursor(&mut self) {
        for r in 0..=self.cursor_row {
            for c in 0..(if r == self.cursor_row {
                self.cursor_col + 1
            } else {
                self.cols
            }) {
                self.cells[r][c] = Cell::new();
            }
        }
        self.dirty = true;
    }

    pub fn clear_line(&mut self) {
        for c in 0..self.cols {
            self.cells[self.cursor_row][c] = Cell::new();
        }
        self.dirty = true;
    }

    pub fn clear_line_to_cursor(&mut self) {
        for c in self.cursor_col..self.cols {
            self.cells[self.cursor_row][c] = Cell::new();
        }
        self.dirty = true;
    }

    pub fn clear_line_from_cursor(&mut self) {
        for c in 0..=self.cursor_col {
            self.cells[self.cursor_row][c] = Cell::new();
        }
        self.dirty = true;
    }

    pub fn scroll_up(&mut self) {
        for r in 0..self.rows - 1 {
            self.cells[r] = self.cells[r + 1];
        }
        for c in 0..self.cols {
            self.cells[self.rows - 1][c] = Cell::new();
        }
        // Keep cursor at bottom row after scroll
        self.cursor_row = self.rows - 1;
        self.dirty = true;
    }

    pub fn scroll_down(&mut self) {
        for r in (1..self.rows).rev() {
            self.cells[r] = self.cells[r - 1];
        }
        for c in 0..self.cols {
            self.cells[0][c] = Cell::new();
        }
        self.dirty = true;
    }

    pub fn save_cursor(&mut self) {
        self.cursor_row_saved = self.cursor_row;
        self.cursor_col_saved = self.cursor_col;
    }

    pub fn restore_cursor(&mut self) {
        self.cursor_row = self.cursor_row_saved;
        self.cursor_col = self.cursor_col_saved;
    }

    pub fn cursor_home(&mut self) {
        self.cursor_row = 0;
        self.cursor_col = 0;
    }

    /// Clear the screen (alias for clear_all)
    pub fn clear(&mut self) {
        self.clear_all();
    }

    /// Get a row as a string
    pub fn get_row_string(&self, row: usize) -> String {
        if row >= self.rows {
            return String::new();
        }
        let mut s = String::new();
        for c in 0..self.cols {
            s.push(self.cells[row][c].ch);
        }
        s
    }
}

impl Default for ScreenBuffer {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Scrollback Ring Buffer ────────────────────────────────────────────
//
// When lines scroll off the top of the visible screen, they're stored in
// a ring buffer. Users scroll back with Shift+PageUp/PageDown. The
// buffer holds up to SCROLLBACK_MAX_LINES lines of text.

/// Maximum scrollback lines (10k lines × 80 chars ≈ 800KB)
pub const SCROLLBACK_MAX_LINES: usize = 10_000;

/// Scrollback ring buffer entry
#[derive(Clone, Copy)]
pub struct ScrollbackLine {
    pub cells: [Cell; 80],
    pub len: usize,
}

/// Scrollback ring buffer
pub struct Scrollback {
    lines: [ScrollbackLine; SCROLLBACK_MAX_LINES],
    head: usize,          // next write position
    count: usize,         // total lines stored
    scroll_offset: usize, // how many lines back from bottom (0 = live)
}

impl Scrollback {
    pub const fn new() -> Self {
        Self {
            lines: [ScrollbackLine {
                cells: [Cell::new(); 80],
                len: 0,
            }; SCROLLBACK_MAX_LINES],
            head: 0,
            count: 0,
            scroll_offset: 0,
        }
    }

    /// Push a line into scrollback
    pub fn push_line(&mut self, row: &[Cell; 80]) {
        let mut line = ScrollbackLine {
            cells: [Cell::new(); 80],
            len: 80,
        };
        line.cells.copy_from_slice(row);
        self.lines[self.head] = line;
        self.head = (self.head + 1) % SCROLLBACK_MAX_LINES;
        if self.count < SCROLLBACK_MAX_LINES {
            self.count += 1;
        }
    }

    /// Get a scrollback line relative to the oldest (0 = oldest)
    pub fn get_line(&self, index: usize) -> Option<&ScrollbackLine> {
        if index >= self.count {
            return None;
        }
        let start = if self.count < SCROLLBACK_MAX_LINES {
            0
        } else {
            self.head
        };
        let actual = (start + index) % SCROLLBACK_MAX_LINES;
        Some(&self.lines[actual])
    }

    /// Scroll up (increase offset)
    pub fn scroll_up(&mut self, lines: usize) {
        self.scroll_offset = (self.scroll_offset + lines).min(self.count);
    }

    /// Scroll down (decrease offset)
    pub fn scroll_down(&mut self, lines: usize) {
        if self.scroll_offset > lines {
            self.scroll_offset -= lines;
        } else {
            self.scroll_offset = 0;
        }
    }

    /// Reset scroll to live mode
    pub fn reset_scroll(&mut self) {
        self.scroll_offset = 0;
    }

    /// Get current scroll offset
    pub fn offset(&self) -> usize {
        self.scroll_offset
    }

    /// Get total lines in buffer
    pub fn line_count(&self) -> usize {
        self.count
    }

    /// Check if we're in scrollback mode (not live)
    pub fn is_scrolling(&self) -> bool {
        self.scroll_offset > 0
    }
}

impl Default for Scrollback {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Copy/Paste Buffer ─────────────────────────────────────────────────

/// Global clipboard for terminal copy/paste
static mut CLIPBOARD: [u8; 4096] = [0; 4096];
static mut CLIPBOARD_LEN: usize = 0;

/// Copy text to clipboard
pub fn copy_to_clipboard(text: &[u8]) {
    unsafe {
        let len = text.len().min(CLIPBOARD.len());
        CLIPBOARD[..len].copy_from_slice(&text[..len]);
        CLIPBOARD_LEN = len;
    }
}

/// Paste from clipboard
pub fn paste_from_clipboard() -> &'static [u8] {
    unsafe { &CLIPBOARD[..CLIPBOARD_LEN] }
}

/// Check if clipboard has content
pub fn clipboard_has_content() -> bool {
    unsafe { CLIPBOARD_LEN > 0 }
}

/// Get clipboard content as a string slice
pub fn clipboard_str() -> &'static str {
    unsafe { core::str::from_utf8_unchecked(&CLIPBOARD[..CLIPBOARD_LEN]) }
}
