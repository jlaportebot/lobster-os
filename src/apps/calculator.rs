//! Calculator app — basic arithmetic with a clean GUI
//!
//! Supports: +, -, *, /, %, clear, decimal points, backspace, history tape
//! Uses shared window frame (src/wm/window_frame.rs)

use crate::apps::calculator_engine::CalculatorEngine;
use crate::display::fb;
use crate::display::graphics;

const APP_NAME: &str = "Calculator";

const BUTTONS: &[&[&str]] = &[
    &["C", "⌫", "%", "/"],
    &["7", "8", "9", "*"],
    &["4", "5", "6", "-"],
    &["1", "2", "3", "+"],
    &["+/-", "0", ".", "="],
];

const NUM_BUTTONS: usize = 20;

/// Calculator app state
pub struct Calculator {
    engine: CalculatorEngine,
    running: bool,
    button_rects: [(u32, u32, u32, u32); NUM_BUTTONS],
}

impl Calculator {
    pub fn new() -> Self {
        Self {
            engine: CalculatorEngine::new(),
            running: true,
            button_rects: [(0, 0, 0, 0); NUM_BUTTONS],
        }
    }

    /// Keyboard input when the Calculator window is focused.
    /// Returns true if the key was consumed.
    pub fn handle_key(&mut self, scancode: u8, ch: Option<char>) -> bool {
        let consumed = match ch {
            Some('0'..='9') => {
                self.engine.input_digit(ch.unwrap());
                true
            }
            Some('.') => {
                self.engine.input_decimal();
                true
            }
            Some('+') => {
                self.engine.input_operator('+');
                true
            }
            Some('-') => {
                self.engine.input_operator('-');
                true
            }
            Some('*') => {
                self.engine.input_operator('*');
                true
            }
            Some('/') => {
                self.engine.input_operator('/');
                true
            }
            Some('%') => {
                self.engine.input_operator('%');
                true
            }
            _ => match scancode {
                0x1C => {
                    self.engine.equals();
                    true
                }
                0x0E => {
                    self.engine.backspace();
                    true
                }
                0x01 => {
                    self.engine.clear();
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
            "C" => self.engine.clear(),
            "⌫" => self.engine.backspace(),
            "+" => self.engine.input_operator('+'),
            "-" => self.engine.input_operator('-'),
            "*" => self.engine.input_operator('*'),
            "/" => self.engine.input_operator('/'),
            "%" => self.engine.input_operator('%'),
            "=" => self.engine.equals(),
            "." => self.engine.input_decimal(),
            "+/-" => self.engine.negate(),
            d if d.chars().all(|c| c.is_ascii_digit()) => {
                if let Some(c) = d.chars().next() {
                    self.engine.input_digit(c);
                }
            }
            _ => {}
        }
    }

    /// Render the calculator
    pub fn render(&mut self, x: u32, y: u32, width: u32, height: u32) {
        let info = fb::get_info();

        // Display area
        let display_height = 60;
        let display_y = y;
        graphics::draw_rect(&info, x, display_y, width, display_height, 0x1E1E1E);

        let display_text = if self.engine.display().len() > 14 {
            &self.engine.display()[self.engine.display().len() - 14..]
        } else {
            self.engine.display()
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

        // History tape: latest entries under the display, dimmed.
        let tape_y = display_y + display_height;
        if !self.engine.history().is_empty() {
            let shown = self.engine.history().len().min(3);
            for (i, entry) in self.engine.history()[self.engine.history().len() - shown..]
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
        let btn_area_y = display_y
            + display_height
            + (if self.engine.history().is_empty() {
                0
            } else {
                3 * 14 + 8
            });
        let btn_area_h = height.saturating_sub(
            display_height
                + (if self.engine.history().is_empty() {
                    0
                } else {
                    3 * 14 + 8
                }),
        );
        let btn_w = width / 4;
        let btn_h = btn_area_h / 5;

        let mut idx = 0;
        for (row, row_buttons) in BUTTONS.iter().enumerate() {
            for (col, label) in row_buttons.iter().enumerate() {
                let bx = x + col as u32 * btn_w;
                let by = btn_area_y + row as u32 * btn_h;

                self.button_rects[idx] = (bx, by, btn_w, btn_h);

                let color = match *label {
                    "+" | "-" | "*" | "/" | "=" => 0xFF6B35,
                    "C" | "⌫" | "%" | "+/-" => 0x4D4D50,
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
                            self.handle_button(label);
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
                let fb = &crate::wm::scene::window_target(win_id, fb);
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
    fn calculator_new_has_zero_display() {
        let calc = Calculator::new();
        assert_eq!(calc.engine.display(), "0");
    }

    #[test]
    fn handle_button_digit() {
        let mut calc = Calculator::new();
        calc.handle_button("5");
        assert_eq!(calc.engine.display(), "5");
    }

    #[test]
    fn handle_button_operator() {
        let mut calc = Calculator::new();
        calc.handle_button("5");
        calc.handle_button("+");
        assert_eq!(calc.engine.display(), "5");
        // After operator, waiting_for_operand should be true
    }

    #[test]
    fn handle_button_equals() {
        let mut calc = Calculator::new();
        calc.handle_button("5");
        calc.handle_button("+");
        calc.handle_button("3");
        calc.handle_button("=");
        assert_eq!(calc.engine.display(), "8");
    }

    #[test]
    fn handle_button_decimal() {
        let mut calc = Calculator::new();
        calc.handle_button("1");
        calc.handle_button(".");
        calc.handle_button("5");
        assert_eq!(calc.engine.display(), "1.5");
    }

    #[test]
    fn handle_button_clear() {
        let mut calc = Calculator::new();
        calc.handle_button("5");
        calc.handle_button("+");
        calc.handle_button("3");
        calc.handle_button("C");
        assert_eq!(calc.engine.display(), "0");
    }

    #[test]
    fn handle_button_backspace() {
        let mut calc = Calculator::new();
        calc.handle_button("1");
        calc.handle_button("2");
        calc.handle_button("3");
        calc.handle_button("⌫");
        assert_eq!(calc.engine.display(), "12");
    }

    #[test]
    fn handle_button_negate() {
        let mut calc = Calculator::new();
        calc.handle_button("5");
        calc.handle_button("+/-");
        assert_eq!(calc.engine.display(), "-5");
        calc.handle_button("+/-");
        assert_eq!(calc.engine.display(), "5");
    }

    #[test]
    fn history_records_calculations() {
        let mut calc = Calculator::new();
        calc.handle_button("2");
        calc.handle_button("+");
        calc.handle_button("3");
        calc.handle_button("=");

        calc.handle_button("5");
        calc.handle_button("*");
        calc.handle_button("4");
        calc.handle_button("=");

        let history = calc.engine.history();
        assert_eq!(history.len(), 2);
        assert!(history[0].contains("2 + 3 = 5"));
        assert!(history[1].contains("5 * 4 = 20"));
    }
}
