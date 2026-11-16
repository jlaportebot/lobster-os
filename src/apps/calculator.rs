//! Calculator app — basic arithmetic with a clean GUI
//!
//! Supports: +, -, *, /, %, clear, decimal points, backspace, history tape
//! Uses shared window frame (src/wm/window_frame.rs)

use crate::apps::calculator_engine::CalculatorEngine;
use crate::display::fb::FramebufferInfo;
use crate::display::graphics;
use crate::wm::hit_region::{self, HitKind};

/// Glyph advance of the display font, used to right-align the display text.
const GLYPH_W: u32 = 10;
/// Right padding of the display text.
const DISPLAY_PAD: u32 = 12;
/// Most characters shown on the display / on one history line.
const DISPLAY_MAX_CHARS: usize = 14;
const TAPE_MAX_CHARS: usize = 24;

/// Display strip background.
const COLOR_DISPLAY_BG: u32 = 0x1E1E1E;
/// Display digits.
const COLOR_DISPLAY_FG: u32 = 0x4EC9B0;
/// History tape text and background.
const COLOR_TAPE_FG: u32 = 0x808080;
const COLOR_TAPE_BG: u32 = 0x252526;
/// Key colours: operators, function keys, digits; shared label colour.
const COLOR_KEY_OPERATOR: u32 = 0xFF6B35;
const COLOR_KEY_FUNCTION: u32 = 0x4D4D50;
const COLOR_KEY_DIGIT: u32 = 0x3D3D40;
const COLOR_KEY_LABEL: u32 = 0xFFFFFF;
/// Vertical offset of the display text inside the display strip.
const DISPLAY_TEXT_TOP: u32 = 20;
/// Tape text offsets inside the tape area.
const TAPE_TEXT_LEFT: u32 = 8;
const TAPE_TEXT_TOP: u32 = 4;
/// Half a glyph width / height, used to centre a key label.
const LABEL_HALF_W: u32 = 5;
const LABEL_HALF_H: u32 = 8;

const BUTTONS: &[&[&str]] = &[
    &["C", "⌫", "%", "/"],
    &["7", "8", "9", "*"],
    &["4", "5", "6", "-"],
    &["1", "2", "3", "+"],
    &["+/-", "0", ".", "="],
];

/// Keys per row of the on-screen keypad.
const KEYS_PER_ROW: usize = 4;
/// Height of the display strip above the history tape and keypad.
const DISPLAY_HEIGHT: u32 = 60;
/// History tape: visible lines, line pitch and padding below the display.
const TAPE_LINES: usize = 3;
const TAPE_LINE_H: u32 = 14;
const TAPE_PAD: u32 = 8;
/// Gap trimmed from every side of a key so neighbours do not touch.
const KEY_INSET: u32 = 2;

/// Calculator app state
pub struct Calculator {
    engine: CalculatorEngine,
    running: bool,
}

impl Calculator {
    pub fn new() -> Self {
        Self {
            engine: CalculatorEngine::new(),
            running: true,
        }
    }

    /// Keyboard input when the Calculator window is focused.
    /// Returns true if the key was consumed.
    pub fn handle_key(&mut self, scancode: u8, ch: Option<char>) -> bool {
        let consumed = match ch {
            Some(digit @ '0'..='9') => {
                self.engine.input_digit(digit);
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

    /// Handle a keypad press identified by its flat key index (the tag of
    /// the hit region registered in `render`). False for an unknown tag.
    pub fn press_key(&mut self, tag: u32) -> bool {
        let Some(label) = key_label(tag) else {
            return false;
        };
        self.handle_button(label);
        true
    }

    /// Render the calculator into `target`; `(x, y)` is the client origin in
    /// screen coordinates. Registers one hit region per key for `win_id`.
    pub fn render(
        &mut self,
        target: &FramebufferInfo,
        win_id: u32,
        x: u32,
        y: u32,
        w: u32,
        h: u32,
    ) {
        self.draw_display(target, x, y, w);
        let tape_h = self.draw_history(target, x, y + DISPLAY_HEIGHT);
        let keys_y = y + DISPLAY_HEIGHT + tape_h;
        let keys_h = h.saturating_sub(DISPLAY_HEIGHT + tape_h);
        hit_region::set_handler(win_id, on_key_hit);
        draw_keypad(target, win_id, (x, y), (x, keys_y, w, keys_h));
    }

    fn draw_display(&self, target: &FramebufferInfo, x: u32, y: u32, width: u32) {
        graphics::draw_rect(target, x, y, width, DISPLAY_HEIGHT, COLOR_DISPLAY_BG);
        let text = self.engine.display();
        let text = &text[text.len().saturating_sub(DISPLAY_MAX_CHARS)..];
        let text_x = (x + width).saturating_sub(text.len() as u32 * GLYPH_W + DISPLAY_PAD);
        graphics::draw_text(
            target,
            text_x,
            y + DISPLAY_TEXT_TOP,
            text,
            COLOR_DISPLAY_FG,
            COLOR_DISPLAY_BG,
        );
    }

    /// Draw the dimmed history tape; returns the height it occupies.
    fn draw_history(&self, target: &FramebufferInfo, x: u32, y: u32) -> u32 {
        let history = self.engine.history();
        if history.is_empty() {
            return 0;
        }
        let shown = history.len().min(TAPE_LINES);
        for (i, entry) in history[history.len() - shown..].iter().rev().enumerate() {
            let line = &entry[entry.len().saturating_sub(TAPE_MAX_CHARS)..];
            let line_y = y + TAPE_TEXT_TOP + i as u32 * TAPE_LINE_H;
            graphics::draw_text(
                target,
                x + TAPE_TEXT_LEFT,
                line_y,
                line,
                COLOR_TAPE_FG,
                COLOR_TAPE_BG,
            );
        }
        TAPE_LINES as u32 * TAPE_LINE_H + TAPE_PAD
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

/// Label of the key with this flat index (row-major over `BUTTONS`).
fn key_label(tag: u32) -> Option<&'static str> {
    let index = tag as usize;
    BUTTONS
        .get(index / KEYS_PER_ROW)?
        .get(index % KEYS_PER_ROW)
        .copied()
}

/// Screen rectangle `(x, y, w, h)` of key `index` inside the keypad `area`.
fn key_cell(index: usize, area: (u32, u32, u32, u32)) -> (u32, u32, u32, u32) {
    let (ax, ay, aw, ah) = area;
    let cell_w = aw / KEYS_PER_ROW as u32;
    let cell_h = ah / BUTTONS.len() as u32;
    let col = (index % KEYS_PER_ROW) as u32;
    let row = (index / KEYS_PER_ROW) as u32;
    (ax + col * cell_w, ay + row * cell_h, cell_w, cell_h)
}

fn key_color(label: &str) -> u32 {
    match label {
        "+" | "-" | "*" | "/" | "=" => COLOR_KEY_OPERATOR,
        "C" | "⌫" | "%" | "+/-" => COLOR_KEY_FUNCTION,
        _ => COLOR_KEY_DIGIT,
    }
}

/// Draw every key and register its hit region (client-local coordinates,
/// tag = flat key index). Regions are cleared by `window_target` each frame.
fn draw_keypad(
    target: &FramebufferInfo,
    win_id: u32,
    origin: (u32, u32),
    area: (u32, u32, u32, u32),
) {
    for (index, label) in BUTTONS.iter().flat_map(|row| row.iter()).enumerate() {
        let (kx, ky, kw, kh) = key_cell(index, area);
        draw_key(target, label, (kx, ky, kw, kh));
        let (local_x, local_y) = (kx - origin.0, ky - origin.1);
        let tag = index as u32;
        hit_region::register(win_id, local_x, local_y, kw, kh, tag, HitKind::Button);
    }
}

/// Draw one key: inset background plus a centred label.
fn draw_key(target: &FramebufferInfo, label: &str, cell: (u32, u32, u32, u32)) {
    let (kx, ky, kw, kh) = cell;
    let color = key_color(label);
    let inner_w = kw.saturating_sub(2 * KEY_INSET);
    let inner_h = kh.saturating_sub(2 * KEY_INSET);
    graphics::draw_rect(
        target,
        kx + KEY_INSET,
        ky + KEY_INSET,
        inner_w,
        inner_h,
        color,
    );
    let label_x = kx + (kw / 2).saturating_sub(LABEL_HALF_W);
    let label_y = ky + (kh / 2).saturating_sub(LABEL_HALF_H);
    graphics::draw_text(target, label_x, label_y, label, COLOR_KEY_LABEL, color);
}

/// Router entry point: a press landed on a key region of the Calculator.
fn on_key_hit(hit: &hit_region::Hit) -> bool {
    let pressed = get_calculator().press_key(hit.region.tag);
    if pressed {
        crate::wm::dirty::mark();
    }
    pressed
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
                calc.render(fb, win_id, cx, cy, cw, ch);
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

    #[test]
    fn key_label_maps_flat_index_to_keypad_layout() {
        assert_eq!(key_label(0), Some("C"));
        assert_eq!(key_label(5), Some("8"));
        assert_eq!(key_label(19), Some("="));
    }

    #[test]
    fn key_label_rejects_out_of_range_tag() {
        assert_eq!(key_label(20), None);
        assert_eq!(key_label(u32::MAX), None);
    }

    #[test]
    fn key_cell_tiles_the_keypad_area_row_major() {
        let area = (100, 200, 400, 500);
        assert_eq!(key_cell(0, area), (100, 200, 100, 100));
        assert_eq!(key_cell(5, area), (200, 300, 100, 100));
        assert_eq!(key_cell(19, area), (400, 600, 100, 100));
    }

    #[test]
    fn press_key_sequence_adds_decimals_like_mouse_clicks() {
        let mut calc = Calculator::new();
        // 12.5 + 7.5 = typed by key tag exactly as the hit-region handler does.
        for label in ["1", "2", ".", "5", "+", "7", ".", "5", "="] {
            let tag = BUTTONS
                .iter()
                .flat_map(|row| row.iter())
                .position(|key| *key == label)
                .map(|index| index as u32);
            assert!(tag.is_some_and(|tag| calc.press_key(tag)));
        }
        assert_eq!(calc.engine.display(), "20");
    }

    #[test]
    fn press_key_ignores_unknown_tag() {
        let mut calc = Calculator::new();
        assert!(!calc.press_key(99));
        assert_eq!(calc.engine.display(), "0");
    }
}
