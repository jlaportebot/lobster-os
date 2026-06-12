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
            _ => {}
        }
        i += 1;
    }
    (fg, bg, bold, underline, blink)
}

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
