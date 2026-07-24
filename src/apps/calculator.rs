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
}

impl Calculator {
    pub fn new() -> Self {
        Self {
            display: String::from("0"),
            previous: 0,
            operator: None,
            waiting_for_operand: true,
            running: true,
            button_rects: [(0, 0, 0, 0); NUM_BUTTONS],
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
                '%' => self.previous % current,
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
            self.compute();
            self.operator = None;
            self.waiting_for_operand = true;
        }
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
        graphics::draw_text(&info, text_x, display_y + 20, display_text, 0x4EC9B0, 0x1E1E1E);

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
                graphics::draw_text(&info, bx + btn_w / 2 - 5, by + btn_h / 2 - 8, label, 0xFFFFFF, color);

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
