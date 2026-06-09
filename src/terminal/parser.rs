//! VT100/ANSI parser state machine

use super::screen::{sgr_attr_from, ScreenBuffer};
use crate::display::console::{BLACK, WHITE};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ParserState {
    Ground,
    Escape,
    CsiEntry,
    CsiParam,
}

pub struct Parser {
    state: ParserState,
    params: [u8; 16],
    param_count: usize,
    final_byte: u8,
}

impl Parser {
    pub fn new() -> Self {
        Self {
            state: ParserState::Ground,
            params: [0; 16],
            param_count: 0,
            final_byte: 0,
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
                }
            }
            ParserState::Escape => {
                if byte == b'[' {
                    self.state = ParserState::CsiEntry;
                    self.params = [0; 16];
                    self.param_count = 0;
                } else {
                    self.state = ParserState::Ground;
                }
            }
            ParserState::CsiEntry | ParserState::CsiParam => {
                if byte >= b'0' && byte <= b'9' {
                    if self.param_count < self.params.len() {
                        self.params[self.param_count] =
                            self.params[self.param_count] * 10 + (byte - b'0');
                    }
                    self.state = ParserState::CsiParam;
                } else if byte == b';' {
                    self.param_count = (self.param_count + 1).min(15);
                    self.state = ParserState::CsiParam;
                } else if byte >= 0x40 && byte <= 0x7E {
                    self.final_byte = byte;
                    self.execute_csi(screen);
                    self.state = ParserState::Ground;
                } else {
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
                    screen.attrs = sgr_attr_from(&self.params[..=self.param_count.min(15)]);
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
}
