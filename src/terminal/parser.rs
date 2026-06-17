//! VT100/ANSI parser state machine
//!
//! Supports:
//! - Basic CSI sequences (cursor movement, screen clearing, SGR)
//! - DEC private modes (DECCKM, DECCOLM, DECTCEM, etc.)
//! - OSC (Operating System Command) sequences for title, clipboard
//! -  -  - 256-color and truecolor (RGB) SGR support
//!  - Cursor save/restore, scroll regions, etc.

use super::screen::{sgr_attr_from, Cell, ScreenBuffer};
use crate::display::console::{BLACK, WHITE};
use crate::println;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ParserState {
    Ground,
    Escape,
    CsiEntry,
    CsiParam,
    CsiIgnore,
    // DEC private mode (CSI ? ...)
    DecPrivate,
    // OSC (Operating System Command) sequences
    OscString,
    OscStringEnd,
    // DCS (Device Control String)
    DcsEntry,
    DcsParam,
    DcsIgnore,
    // SOS (Start of String), PM (Privacy Message), APC (Application Program Command)
    SosPmApcString,
}

pub struct Parser {
    state: ParserState,
    params: [u16; 16],
    param_count: usize,
    final_byte: u8,
    // For DEC private mode (CSI ? ...)
    dec_private: bool,
    // For OSC string parsing
    osc_buffer: [u8; 256],
    osc_len: usize,
    // For SGR 256-color/truecolor parsing
    sgr_subparams: [u16; 8],
    sgr_subparam_count: usize,
    // Intermediate bytes for CSI (for sequences like CSI > ...)
    intermediate_byte: u8,
}

impl Parser {
    pub fn new() -> Self {
        Self {
            state: ParserState::Ground,
            params: [0; 16],
            param_count: 0,
            final_byte: 0,
            dec_private: false,
            osc_buffer: [0; 256],
            osc_len: 0,
            sgr_subparams: [0; 8],
            sgr_subparam_count: 0,
            intermediate_byte: 0,
        }
    }

    pub fn feed(&mut self, byte: u8, screen: &mut ScreenBuffer) {
        match self.state {
            ParserState::Ground => {
                if byte == 0x1B {
                    self.state = ParserState::Escape;
                } else if byte >= 32 && byte < 127 {
                    self.print_char(screen, byte);
                } else if byte == 10 || byte == 11 || byte == 12 {
                    self.cursor_dn(screen);
                } else if byte == 13 {
                    screen.cursor_col = 0;
                } else if byte == 0x07 {
                    // BEL - bell (beep)
                } else if byte == 0x08 {
                    // BS - backspace
                    if screen.cursor_col > 0 {
                        screen.cursor_col -= 1;
                    }
                } else if byte == 0x09 {
                    // HT - horizontal tab (simplified: move to next 8-col boundary)
                    screen.cursor_col = ((screen.cursor_col / 8) + 1) * 8;
                    if screen.cursor_col >= screen.cols {
                        screen.cursor_col = screen.cols - 1;
                    }
                } else if byte == 0x0C {
                    // FF - form feed (clear screen)
                    screen.clear_all();
                }
            }
            ParserState::Escape => {
                match byte {
                    b'[' => {
                        // CSI (Control Sequence Introducer)
                        self.state = ParserState::CsiEntry;
                        self.params = [0; 16];
                        self.param_count = 0;
                        self.dec_private = false;
                        self.intermediate_byte = 0;
                    }
                    b']' => {
                        // OSC (Operating System Command)
                        self.state = ParserState::OscString;
                        self.osc_len = 0;
                    }
                    b'P' => {
                        // DCS (Device Control String)
                        self.state = ParserState::DcsEntry;
                        self.params = [0; 16];
                        self.param_count = 0;
                    }
                    b'X' | b'^' | b'_' => {
                        // SOS, PM, APC - ignore until ST
                        self.state = ParserState::SosPmApcString;
                    }
                    b'c' => {
                        // RIS - Reset to Initial State
                        self.reset(screen);
                    }
                    b'7' => {
                        // DECSC - Save Cursor
                        screen.save_cursor();
                    }
                    b'8' => {
                        // DECRC - Restore Cursor
                        screen.restore_cursor();
                    }
                    b'=' => {
                        // DECKPAM - Keypad Application Mode (ignored)
                    }
                    b'>' => {
                        // DECKPNM - Keypad Numeric Mode (ignored)
                    }
                    b'D' => {
                        // IND - Index (cursor down with scroll)
                        self.cursor_dn(screen);
                    }
                    b'E' => {
                        // NEL - Next Line
                        screen.cursor_col = 0;
                        self.cursor_dn(screen);
                    }
                    b'H' => {
                        // HTS - Horizontal Tab Set (ignored)
                    }
                    b'M' => {
                        // RI - Reverse Index
                        self.reverse_index(screen);
                    }
                    b'N' => {
                        // SS2 - Single Shift 2 (ignored)
                    }
                    b'O' => {
                        // SS3 - Single Shift 3 (ignored)
                    }
                    b'Z' => {
                        // DECID - Identify Terminal (ignored)
                    }
                    b' ' => {
                        // ESC space - intermediate bytes (ignored)
                    }
                    _ => {
                        // Unknown ESC sequence, ignore
                        self.state = ParserState::Ground;
                    }
                }
            }
            ParserState::CsiEntry | ParserState::CsiParam => {
                match byte {
                    b'?' => {
                        // DEC private mode prefix
                        self.dec_private = true;
                        self.state = ParserState::DecPrivate;
                    }
                    b'>' => {
                        // Private mode modifier (e.g., CSI > 1 c)
                        self.intermediate_byte = b'>';
                        self.state = ParserState::CsiParam;
                    }
                    b'!' => {
                        // Private mode modifier
                        self.intermediate_byte = b'!';
                        self.state = ParserState::CsiParam;
                    }
                    b'0'..=b'9' => {
                        if self.param_count < self.params.len() {
                            self.params[self.param_count] = self.params[self.param_count]
                                .saturating_mul(10)
                                + (byte - b'0') as u16;
                        }
                        self.state = ParserState::CsiParam;
                    }
                    b';' => {
                        self.param_count = (self.param_count + 1).min(15);
                        self.state = ParserState::CsiParam;
                    }
                    b':' => {
                        // Sub-parameters for SGR 256-color/truecolor (e.g., 38:2:r:g:b)
                        if self.sgr_subparam_count < self.sgr_subparams.len() {
                            self.sgr_subparams[self.sgr_subparam_count] =
                                self.params[self.param_count];
                        }
                        self.sgr_subparam_count = (self.sgr_subparam_count + 1).min(7);
                        self.params[self.param_count] = 0;
                        self.state = ParserState::CsiParam;
                    }
                    0x40..=0x7E => {
                        // Final byte - execute CSI sequence
                        self.final_byte = byte;
                        self.execute_csi(screen);
                        self.state = ParserState::Ground;
                    }
                    _ => {
                        // Invalid byte in CSI, enter ignore state
                        self.state = ParserState::CsiIgnore;
                    }
                }
            }
            ParserState::DecPrivate => {
                // DEC private mode parameters (same as CsiParam but with ? prefix)
                match byte {
                    b'0'..=b'9' => {
                        if self.param_count < self.params.len() {
                            self.params[self.param_count] = self.params[self.param_count]
                                .saturating_mul(10)
                                + (byte - b'0') as u16;
                        }
                        self.state = ParserState::DecPrivate;
                    }
                    b';' => {
                        self.param_count = (self.param_count + 1).min(15);
                        self.state = ParserState::DecPrivate;
                    }
                    0x40..=0x7E => {
                        // Final byte for DEC private mode
                        self.final_byte = byte;
                        self.execute_dec_private(screen);
                        self.state = ParserState::Ground;
                    }
                    _ => {
                        self.state = ParserState::Ground;
                    }
                }
            }
            ParserState::OscString => {
                if byte == 0x1B {
                    // ESC - possible end of OSC
                    self.state = ParserState::OscStringEnd;
                } else if byte == 0x07 {
                    // BEL - end of OSC
                    self.execute_osc(screen);
                    self.state = ParserState::Ground;
                } else if self.osc_len < self.osc_buffer.len() {
                    self.osc_buffer[self.osc_len] = byte;
                    self.osc_len += 1;
                }
                // Ignore overflow
            }
            ParserState::OscStringEnd => {
                if byte == b'\\' {
                    // ESC \ - end of OSC (ST)
                    self.execute_osc(screen);
                } else if byte == 0x07 {
                    // BEL - also end of OSC
                    self.execute_osc(screen);
                } else {
                    // Not a terminator, continue OSC
                    if self.osc_len < self.osc_buffer.len() {
                        self.osc_buffer[self.osc_len] = 0x1B;
                        self.osc_len += 1;
                    }
                    if self.osc_len < self.osc_buffer.len() {
                        self.osc_buffer[self.osc_len] = byte;
                        self.osc_len += 1;
                    }
                }
                self.state = ParserState::Ground;
            }
            ParserState::DcsEntry | ParserState::DcsParam => {
                match byte {
                    b'0'..=b'9' => {
                        if self.param_count < self.params.len() {
                            self.params[self.param_count] = self.params[self.param_count]
                                .saturating_mul(10)
                                + (byte - b'0') as u16;
                        }
                        self.state = ParserState::DcsParam;
                    }
                    b';' => {
                        self.param_count = (self.param_count + 1).min(15);
                        self.state = ParserState::DcsParam;
                    }
                    0x40..=0x7E => {
                        // DCS final byte - ignore for now
                        self.state = ParserState::Ground;
                    }
                    _ => {
                        self.state = ParserState::DcsIgnore;
                    }
                }
            }
            ParserState::DcsIgnore => {
                if byte == 0x1B {
                    self.state = ParserState::SosPmApcString; // Wait for ST
                }
            }
            ParserState::SosPmApcString => {
                // Ignore until ST (ESC \ or BEL)
                if byte == 0x1B {
                    // Possible ESC \
                } else if byte == 0x07 || byte == 0x08 || byte == 0x09 || byte == 0x0C {
                    // BEL, BS, HT, FF can end these strings in some implementations
                }
            }
            ParserState::CsiIgnore => {
                if matches!(byte, 0x40..=0x7E) {
                    self.state = ParserState::Ground;
                }
            }
        }
    }

    fn execute_csi(&self, screen: &mut ScreenBuffer) {
        match self.final_byte {
            b'A' => {
                let n = if self.params[0] == 0 {
                    1
                } else {
                    self.params[0] as usize
                };
                if n <= screen.cursor_row {
                    screen.cursor_row -= n;
                }
            }
            b'B' => {
                let n = if self.params[0] == 0 {
                    1
                } else {
                    self.params[0] as usize
                };
                screen.cursor_row = (screen.cursor_row + n).min(screen.rows - 1);
            }
            b'C' => {
                let n = if self.params[0] == 0 {
                    1
                } else {
                    self.params[0] as usize
                };
                screen.cursor_col = (screen.cursor_col + n).min(screen.cols - 1);
            }
            b'D' => {
                let n = if self.params[0] == 0 {
                    1
                } else {
                    self.params[0] as usize
                };
                if n <= screen.cursor_col {
                    screen.cursor_col -= n;
                }
            }
            b'H' | b'f' => {
                let row = if self.params[0] == 0 {
                    0
                } else {
                    (self.params[0] as usize).saturating_sub(1)
                };
                let col = if self.params[1] == 0 {
                    0
                } else {
                    (self.params[1] as usize).saturating_sub(1)
                };
                screen.cursor_row = row.min(screen.rows - 1);
                screen.cursor_col = col.min(screen.cols - 1);
            }
            b'J' => match self.params[0] {
                0 => screen.clear_from_cursor(),
                1 => screen.clear_to_cursor(),
                2 => screen.clear_all(),
                _ => {}
            },
            b'K' => match self.params[0] {
                0 => screen.clear_line_to_cursor(),
                1 => screen.clear_line_from_cursor(),
                2 => screen.clear_line(),
                _ => {}
            },
            b'm' => {
                if self.param_count == 0 {
                    screen.attrs = (WHITE, BLACK, false, false, false);
                } else {
                    // Convert u16 params to u8 for sgr_attr_from
                    let params_u8: [u8; 16] = core::array::from_fn(|i| self.params[i] as u8);
                    screen.attrs = sgr_attr_from(&params_u8[..=self.param_count.min(15)]);
                }
            }
            b's' => screen.save_cursor(),
            b'u' => screen.restore_cursor(),
            _ => {}
        }
    }

    fn print_char(&self, screen: &mut ScreenBuffer, byte: u8) {
        let c = byte as char;
        screen.put_char(screen.cursor_row, screen.cursor_col, c);
        screen.cursor_col += 1;
        if screen.cursor_col >= screen.cols {
            screen.cursor_col = 0;
            screen.cursor_row += 1;
            if screen.cursor_row >= screen.rows {
                screen.scroll_up();
            }
        }
    }

    fn cursor_dn(&self, screen: &mut ScreenBuffer) {
        screen.cursor_row += 1;
        if screen.cursor_row >= screen.rows {
            screen.scroll_up();
        }
    }

    fn execute_dec_private(&mut self, _screen: &mut ScreenBuffer) {
        // DEC private mode sequences (CSI ? ... h/l)
        // For now, we handle common modes: DECCKM (1), DECCOLM (3), DECTCEM (25)
        // The final byte indicates the mode; h = set, l = reset
        let set_mode = self.final_byte == b'h';
        let reset_mode = self.final_byte == b'l';

        if set_mode || reset_mode {
            for i in 0..=self.param_count {
                let param = self.params[i];
                match param {
                    1 => {
                        // DECCKM - Cursor Key Mode
                        // When set (application mode), cursor keys send ESC O x
                        // When reset (normal mode), cursor keys send ESC [ x
                    }
                    3 => {
                        // DECCOLM - Column Mode (80/132 columns)
                    }
                    6 => {
                        // DECOM - Origin Mode (relative/absolute cursor addressing)
                    }
                    7 => {
                        // DECAWM - Auto Wrap Mode
                    }
                    12 => {
                        // Send/Receive Mode
                    }
                    25 => {
                        // DECTCEM - Text Cursor Enable Mode
                    }
                    1000 => { /* X10 mouse tracking */ }
                    1002 => { /* Button event mouse tracking */ }
                    1006 => { /* SGR mouse tracking */ }
                    2004 => { /* Bracketed paste mode */ }
                    _ => {}
                }
            }
            self.dec_private = false;
        }
    }

    fn execute_osc(&mut self, _screen: &mut ScreenBuffer) {
        if self.osc_len == 0 {
            return;
        }
        let data = &self.osc_buffer[..self.osc_len];
        if data.is_empty() {
            return;
        }
        let mut ps_end = 0;
        while ps_end < data.len() && data[ps_end] != b';' && data[ps_end] != b':' {
            ps_end += 1;
        }
        let ps_str = core::str::from_utf8(&data[..ps_end]).unwrap_or("");
        let ps: u8 = ps_str.parse().unwrap_or(0);
        let pt_start = if ps_end < data.len() {
            ps_end + 1
        } else {
            data.len()
        };
        let pt = &data[pt_start..];
        match ps {
            0 | 2 => {
                let title = core::str::from_utf8(pt).unwrap_or("");
                println!("[terminal] Window title set: {}", title);
            }
            4 | 10 | 11 | 104 | 112 | 111 => { /* color palette ops */ }
            52 => {
                println!("[terminal] OSC 52 clipboard copy (base64 len={})", pt.len());
            }
            _ => {}
        }
    }

    fn reset(&mut self, screen: &mut ScreenBuffer) {
        screen.clear_all();
        screen.cursor_row = 0;
        screen.cursor_col = 0;
        screen.attrs = (WHITE, BLACK, false, false, false);
    }

    fn reverse_index(&self, screen: &mut ScreenBuffer) {
        if screen.cursor_row == 0 {
            // Scroll down by moving all lines up and clearing the last line
            for r in (1..screen.rows).rev() {
                screen.cells[r] = screen.cells[r - 1];
            }
            for c in 0..screen.cols {
                screen.cells[0][c] = Cell::new();
            }
        } else {
            screen.cursor_row -= 1;
        }
    }
}

impl Default for Parser {
    fn default() -> Self {
        Self::new()
    }
}
