//! Calculator app — basic arithmetic with a clean GUI
//!
//! Supports: +, -, *, /, %, clear, decimal points, backspace

use crate::display::fb;
use crate::display::graphics;
use alloc::format;
use alloc::string::String;

const APP_NAME: &str = "Calculator";

const BUTTONS: &[&[&str]] = &[
    &["C", "⌫", "%", "/"],
    &["7", "8", "9", "*"],
    &["4", "5", "6", "-"],
    &["1", "2", "3", "+"],
    &["0", ".", "=", ""],
];

const NUM_BUTTONS: usize = 19;

/// Calculator app state
pub struct Calculator {
    display: String,
    previous: i64,
    operator: Option<char>,
    waiting_for_operand: bool,
    running: bool,
    button_rects: [(u32, u32, u32, u32); NUM_BUTTONS],
    /// Rolling tape of past evaluations (newest last), shown under the
    /// display when non-empty. B6.2.
    history: alloc::vec::Vec<alloc::string::String>,
}

/// Max tape entries kept.
const HISTORY_CAP: usize = 8;

impl Calculator {
    pub fn new() -> Self {
        Self {
            display: String::from("0"),
            previous: 0,
            operator: None,
            waiting_for_operand: true,
            running: true,
            button_rects: [(0, 0, 0, 0); NUM_BUTTONS],
            history: alloc::vec::Vec::new(),
        }
    }

    fn input_digit(&mut self, digit: char) {
        if self.waiting_for_operand {
            self.display.clear();
            self.waiting_for_operand = false;
        }
        if self.display.len() < 16 {
            if self.display == "0" && digit != '.' {
                self.display.clear();
            }
            self.display.push(digit);
        }
    }

    fn input_operator(&mut self, op: char) {
        if let Ok(value) = self.display.parse::<i64>() {
            if self.operator.is_some() && !self.waiting_for_operand {
                self.compute();
            }
            self.previous = value;
            self.operator = Some(op);
            self.waiting_for_operand = true;
        }
    }

    fn compute(&mut self) {
        if let (Some(op), Ok(current)) = (self.operator, self.display.parse::<i64>()) {
            let result = match op {
                '+' => self.previous + current,
                '-' => self.previous - current,
                '*' => self.previous * current,
                '/' => {
                    if current != 0 {
                        self.previous / current
                    } else {
                        self.display = String::from("Error: div by 0");
                        self.operator = None;
                        self.waiting_for_operand = true;
                        return;
                    }
                }
                '%' => {
                    if current != 0 {
                        self.previous % current
                    } else {
                        self.display = String::from("Error: mod by 0");
                        self.operator = None;
                        self.waiting_for_operand = true;
                        return;
                    }
                }
                _ => current,
            };

            self.display = format!("{}", result);
            self.previous = result;
        }
    }

    fn clear(&mut self) {
        self.display = String::from("0");
        self.previous = 0;
        self.operator = None;
        self.waiting_for_operand = true;
    }

    /// Compute `lhs op rhs` exactly as `compute()` does, without touching state.
    /// Returns None for division/modulo by zero (matches the "Error" display).
    fn apply_op(lhs: i64, op: char, rhs: i64) -> Option<i64> {
        match op {
            '+' => Some(lhs.wrapping_add(rhs)),
            '-' => Some(lhs.wrapping_sub(rhs)),
            '*' => Some(lhs.wrapping_mul(rhs)),
            '/' => {
                if rhs == 0 {
                    None
                } else {
                    Some(lhs / rhs)
                }
            }
            '%' => {
                if rhs == 0 {
                    None
                } else {
                    Some(lhs % rhs)
                }
            }
            _ => Some(rhs),
        }
    }

    fn backspace(&mut self) {
        if self.display.len() > 1 {
            self.display.pop();
        } else {
            self.display = String::from("0");
            self.waiting_for_operand = true;
        }
    }

    fn equals(&mut self) {
        if self.operator.is_some() {
            let lhs = self.previous;
            let op = self.operator.unwrap();
            let rhs_display = self.display.clone();
            self.compute();
            self.operator = None;
            self.waiting_for_operand = true;
            // Tape entry: "lhs op rhs = result"
            let entry = alloc::format!("{} {} {} = {}", lhs, op, rhs_display, self.display);
            if self.history.len() == HISTORY_CAP {
                self.history.remove(0);
            }
            self.history.push(entry);
        }
    }

    /// Keyboard input when the Calculator window is focused (B6.2).
    /// Returns true if the key was consumed.
    pub fn handle_key(&mut self, scancode: u8, ch: Option<char>) -> bool {
        let consumed = match ch {
            Some('0'..='9') => {
                self.input_digit(ch.unwrap());
                true
            }
            Some('.') => {
                self.input_digit('.');
                true
            }
            Some('+') => {
                self.input_operator('+');
                true
            }
            Some('-') => {
                self.input_operator('-');
                true
            }
            Some('*') => {
                self.input_operator('*');
                true
            }
            Some('/') => {
                self.input_operator('/');
                true
            }
            Some('%') => {
                self.input_operator('%');
                true
            }
            _ => match scancode {
                0x1C => {
                    self.equals();
                    true
                }
                0x0E => {
                    self.backspace();
                    true
                }
                0x01 => {
                    self.clear();
                    true
                }
                _ => false,
            },
        };
        consumed
    }

    /// Handle a button press
    pub fn handle_button(&mut self, label: &str) {
        match label {
            "C" => self.clear(),
            "⌫" => self.backspace(),
            "+" => self.input_operator('+'),
            "-" => self.input_operator('-'),
            "*" => self.input_operator('*'),
            "/" => self.input_operator('/'),
            "%" => self.input_operator('%'),
            "=" => self.equals(),
            "." => self.input_digit('.'),
            d if d.chars().all(|c| c.is_ascii_digit()) => {
                if let Some(c) = d.chars().next() {
                    self.input_digit(c);
                }
            }
            _ => {}
        }
    }

    /// Render the calculator
    pub fn render(&mut self, x: u32, y: u32, width: u32, height: u32) {
        let info = fb::get_info();

        // Title bar
        graphics::draw_rect(&info, x, y, width, 28, 0x2D2D30);
        graphics::draw_text(&info, x + 8, y + 6, APP_NAME, 0xFFFFFF, 0x2D2D30);

        // Display area
        let display_height = 60;
        let display_y = y + 28;
        graphics::draw_rect(&info, x, display_y, width, display_height, 0x1E1E1E);

        let display_text = if self.display.len() > 14 {
            &self.display[self.display.len() - 14..]
        } else {
            &self.display[..]
        };
        let text_x = x + width - (display_text.len() as u32) * 10 - 12;
        graphics::draw_text(
            &info,
            text_x,
            display_y + 20,
            display_text,
            0x4EC9B0,
            0x1E1E1E,
        );

        // History tape (B6.2): latest entries under the display, dimmed.
        if !self.history.is_empty() {
            let tape_y = display_y + display_height;
            let shown = self.history.len().min(3);
            for (i, entry) in self.history[self.history.len() - shown..]
                .iter()
                .rev()
                .enumerate()
            {
                let line = if entry.len() > 24 {
                    &entry[entry.len() - 24..]
                } else {
                    &entry[..]
                };
                graphics::draw_text(
                    &info,
                    x + 8,
                    tape_y + 4 + (i as u32) * 14,
                    line,
                    0x808080,
                    0x252526,
                );
            }
        }

        // Buttons
        let btn_area_y = display_y + display_height;
        let btn_area_h = height.saturating_sub(28 + display_height);
        let btn_w = width / 4;
        let btn_h = btn_area_h / 5;

        let mut idx = 0;
        for (row, row_buttons) in BUTTONS.iter().enumerate() {
            for (col, label) in row_buttons.iter().enumerate() {
                if label.is_empty() {
                    continue;
                }
                let bx = x + col as u32 * btn_w;
                let by = btn_area_y + row as u32 * btn_h;

                self.button_rects[idx] = (bx, by, btn_w, btn_h);

                let color = match *label {
                    "+" | "-" | "*" | "/" | "=" => 0xFF6B35,
                    "C" | "⌫" | "%" => 0x4D4D50,
                    _ => 0x3D3D40,
                };

                graphics::draw_rect(&info, bx + 2, by + 2, btn_w - 4, btn_h - 4, color);
                graphics::draw_text(
                    &info,
                    bx + btn_w / 2 - 5,
                    by + btn_h / 2 - 8,
                    label,
                    0xFFFFFF,
                    color,
                );

                idx += 1;
            }
        }
    }

    /// Handle mouse click on buttons
    pub fn handle_click(&mut self, px: u32, py: u32) {
        for (i, &(bx, by, bw, bh)) in self.button_rects.iter().enumerate() {
            if px >= bx && px < bx + bw && py >= by && py < by + bh {
                if let Some(row) = i.checked_div(4) {
                    if let Some(col) = i.checked_rem(4) {
                        if row < BUTTONS.len() && col < BUTTONS[row].len() {
                            let label = BUTTONS[row][col];
                            if !label.is_empty() {
                                self.handle_button(label);
                            }
                        }
                    }
                }
                break;
            }
        }
    }

    pub fn is_running(&self) -> bool {
        self.running
    }

    pub fn close(&mut self) {
        self.running = false;
    }
}

impl Default for Calculator {
    fn default() -> Self {
        Self::new()
    }
}

pub fn init() {
    crate::println!("[apps] Calculator initialized");
}

/// Global Calculator instance
static mut CALCULATOR: Option<Calculator> = None;

/// Get the global Calculator instance
/// Access the singleton calculator (for the WM keyboard hook).
pub fn instance() -> Option<&'static mut Calculator> {
    unsafe { CALCULATOR.as_mut() }
}

fn get_calculator() -> &'static mut Calculator {
    unsafe {
        if CALCULATOR.is_none() {
            CALCULATOR = Some(Calculator::new());
        }
        CALCULATOR.as_mut().expect("Calculator not initialized")
    }
}

/// Render Calculator to its window (QEMU direct-to-framebuffer mode)
#[cfg(feature = "qemu_raspi5b")]
pub fn render_to_windows(fb: &crate::display::fb::FramebufferInfo) {
    if let Some(win_id) = crate::wm::window::find_by_title("Calculator") {
        if let Some(win) = crate::wm::window::get(win_id) {
            if win.active {
                let focused = crate::wm::window::focused_id() == win_id;
                // Uniform themed frame, then the calculator UI in the client area
                let (cx, cy, cw, ch) = crate::wm::window_frame::draw_frame(
                    fb,
                    win_id,
                    win.x,
                    win.y,
                    win.w,
                    win.h,
                    "Calculator",
                    focused,
                );
                let calc = get_calculator();
                calc.render(cx, cy, cw, ch);
            }
        }
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    #[test]
    fn apply_op_handles_all_operators() {
        assert_eq!(Calculator::apply_op(5, '+', 3), Some(8));
        assert_eq!(Calculator::apply_op(5, '-', 3), Some(2));
        assert_eq!(Calculator::apply_op(5, '*', 3), Some(15));
        assert_eq!(Calculator::apply_op(6, '/', 3), Some(2));
        assert_eq!(Calculator::apply_op(5, '%', 3), Some(2));
    }

    #[test]
    fn apply_op_returns_none_for_div_and_mod_by_zero() {
        // Regression: `5 % 0` used to panic the whole kernel (audit Q1a).
        assert_eq!(Calculator::apply_op(5, '/', 0), None);
        assert_eq!(Calculator::apply_op(5, '%', 0), None);
        assert_eq!(Calculator::apply_op(0, '%', 0), None);
    }

    #[test]
    fn apply_op_unknown_operator_falls_back_to_rhs() {
        assert_eq!(Calculator::apply_op(10, '?', 4), Some(4));
    }
}
