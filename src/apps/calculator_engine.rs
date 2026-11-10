//! Calculator arithmetic engine — pure logic, no UI dependencies
//!
//! This module contains the core arithmetic logic for the calculator,
//! separated from the UI for testability. It handles:
//! - Decimal (floating-point) arithmetic
//! - Chained operators (e.g., 1 + 2 * 3 = 9)
//! - Percent operator
//! - Division/modulo by zero handling
//! - History/tape management

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write;

#[cfg(not(feature = "std"))]
use libm;

/// Maximum history/tape entries
pub const HISTORY_CAP: usize = 8;

/// Calculator engine state
#[derive(Clone, Debug, Default)]
pub struct CalculatorEngine {
    /// Current display value as string (for user input)
    display: String,
    /// Previous operand for binary operations
    previous: f64,
    /// Pending operator
    operator: Option<char>,
    /// Whether we're waiting for the next operand
    waiting_for_operand: bool,
    /// History tape of completed calculations
    history: Vec<String>,
}

impl CalculatorEngine {
    /// Create a new calculator engine
    pub fn new() -> Self {
        Self {
            display: String::from("0"),
            previous: 0.0,
            operator: None,
            waiting_for_operand: true,
            history: Vec::new(),
        }
    }

    /// Get the current display value
    pub fn display(&self) -> &str {
        &self.display
    }

    /// Get the history tape (most recent last)
    pub fn history(&self) -> &[String] {
        &self.history
    }

    /// Input a digit (0-9)
    pub fn input_digit(&mut self, digit: char) {
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

    /// Input a decimal point
    pub fn input_decimal(&mut self) {
        if self.waiting_for_operand {
            self.display = String::from("0.");
            self.waiting_for_operand = false;
        } else if !self.display.contains('.') {
            self.display.push('.');
        }
    }

    /// Input an operator (+, -, *, /, %)
    pub fn input_operator(&mut self, op: char) {
        if let Ok(value) = self.display.parse::<f64>() {
            if self.operator.is_some() && !self.waiting_for_operand {
                self.compute();
            } else {
                self.previous = value;
            }
            self.operator = Some(op);
            self.waiting_for_operand = true;
        }
    }

    /// Compute the pending operation
    fn compute(&mut self) {
        if let (Some(op), Ok(current)) = (self.operator, self.display.parse::<f64>()) {
            let result = Self::apply_op(self.previous, op, current);
            match result {
                Some(r) => {
                    // Format the result - remove trailing .0 for whole numbers
                    self.display = Self::format_number(r);
                    self.previous = r;
                }
                None => {
                    self.display = String::from("Error");
                    self.operator = None;
                    self.waiting_for_operand = true;
                }
            }
        }
    }

    /// Format a number for display (remove trailing .0)
    fn format_number(n: f64) -> String {
        // Check if it's an integer (no fractional part)
        #[cfg(feature = "std")]
        let truncated = n.trunc();
        #[cfg(not(feature = "std"))]
        let truncated = libm::trunc(n);

        if (n - truncated).abs() < f64::EPSILON && n.abs() < 1e10 {
            let mut s = String::new();
            write!(&mut s, "{}", truncated as i64).ok();
            s
        } else {
            // Use scientific notation for very large/small numbers
            if n.abs() >= 1e10 || (n.abs() < 1e-4 && n != 0.0) {
                let mut s = String::new();
                write!(&mut s, "{:.4e}", n).ok();
                s
            } else {
                let mut s = String::new();
                write!(&mut s, "{}", n).ok();
                s
            }
        }
    }

    /// Clear the calculator
    pub fn clear(&mut self) {
        self.display = String::from("0");
        self.previous = 0.0;
        self.operator = None;
        self.waiting_for_operand = true;
    }

    /// Handle backspace
    pub fn backspace(&mut self) {
        if self.waiting_for_operand {
            return;
        }
        if self.display.len() > 1 {
            self.display.pop();
        } else {
            self.display = String::from("0");
            self.waiting_for_operand = true;
        }
    }

    /// Handle equals (=)
    pub fn equals(&mut self) {
        if self.operator.is_some() && !self.waiting_for_operand {
            let lhs = self.previous;
            let op = self.operator.unwrap();
            let rhs_display = self.display.clone();
            self.compute();
            let result_display = self.display.clone();
            self.operator = None;
            self.waiting_for_operand = true;

            // Tape entry: "lhs op rhs = result"
            let mut entry = String::new();
            write!(&mut entry, "{} {} {} = {}", lhs, op, rhs_display, result_display).ok();
            if self.history.len() == HISTORY_CAP {
                self.history.remove(0);
            }
            self.history.push(entry);
        }
    }

    /// Negate the current display value (+/- button)
    pub fn negate(&mut self) {
        if let Ok(value) = self.display.parse::<f64>() {
            let negated = -value;
            self.display = Self::format_number(negated);
        }
    }

    /// Apply a binary operator (pure function, no state mutation)
    /// Returns None for division/modulo by zero
    pub fn apply_op(lhs: f64, op: char, rhs: f64) -> Option<f64> {
        match op {
            '+' => Some(lhs + rhs),
            '-' => Some(lhs - rhs),
            '*' => Some(lhs * rhs),
            '/' => {
                if rhs == 0.0 {
                    None
                } else {
                    Some(lhs / rhs)
                }
            }
            '%' => {
                if rhs == 0.0 {
                    None
                } else {
                    Some(lhs % rhs)
                }
            }
            _ => Some(rhs),
        }
    }

    /// Parse a number from string, handling scientific notation
    /// e.g., "2e+3" -> 2000.0, not 20000.0
    pub fn parse_number(s: &str) -> Result<f64, core::num::ParseFloatError> {
        s.parse::<f64>()
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    #[test]
    fn apply_op_handles_all_operators() {
        assert_eq!(CalculatorEngine::apply_op(5.0, '+', 3.0), Some(8.0));
        assert_eq!(CalculatorEngine::apply_op(5.0, '-', 3.0), Some(2.0));
        assert_eq!(CalculatorEngine::apply_op(5.0, '*', 3.0), Some(15.0));
        assert_eq!(CalculatorEngine::apply_op(6.0, '/', 3.0), Some(2.0));
        assert_eq!(CalculatorEngine::apply_op(5.0, '%', 3.0), Some(2.0));
    }

    #[test]
    fn apply_op_returns_none_for_div_and_mod_by_zero() {
        assert_eq!(CalculatorEngine::apply_op(5.0, '/', 0.0), None);
        assert_eq!(CalculatorEngine::apply_op(5.0, '%', 0.0), None);
        assert_eq!(CalculatorEngine::apply_op(0.0, '%', 0.0), None);
    }

    #[test]
    fn apply_op_unknown_operator_falls_back_to_rhs() {
        assert_eq!(CalculatorEngine::apply_op(10.0, '?', 4.0), Some(4.0));
    }

    #[test]
    fn decimal_handling() {
        let mut calc = CalculatorEngine::new();
        calc.input_digit('1');
        calc.input_decimal();
        calc.input_digit('5');
        assert_eq!(calc.display(), "1.5");

        calc.input_operator('+');
        calc.input_digit('2');
        calc.input_decimal();
        calc.input_digit('5');
        calc.equals();
        assert_eq!(calc.display(), "4");
    }

    #[test]
    fn operator_chaining() {
        let mut calc = CalculatorEngine::new();
        calc.input_digit('1');
        calc.input_operator('+');
        calc.input_digit('2');
        calc.input_operator('*');
        calc.input_digit('3');
        calc.equals();
        // 1 + 2 = 3, then 3 * 3 = 9 (left-to-right evaluation)
        assert_eq!(calc.display(), "9");
    }

    #[test]
    fn division_by_zero_shows_error() {
        let mut calc = CalculatorEngine::new();
        calc.input_digit('5');
        calc.input_operator('/');
        calc.input_digit('0');
        calc.equals();
        assert_eq!(calc.display(), "Error");
    }

    #[test]
    fn percent_operator() {
        let mut calc = CalculatorEngine::new();
        calc.input_digit('5');
        calc.input_operator('%');
        calc.input_digit('3');
        calc.equals();
        // 5 % 3 = 2
        assert_eq!(calc.display(), "2");
    }

    #[test]
    fn history_push_and_recall() {
        let mut calc = CalculatorEngine::new();
        assert!(calc.history().is_empty());

        calc.input_digit('2');
        calc.input_operator('+');
        calc.input_digit('3');
        calc.equals();

        calc.input_digit('5');
        calc.input_operator('*');
        calc.input_digit('4');
        calc.equals();

        let history = calc.history();
        assert_eq!(history.len(), 2);
        assert!(history[0].contains("2 + 3 = 5"));
        assert!(history[1].contains("5 * 4 = 20"));
    }

    #[test]
    fn history_capacity_limit() {
        let mut calc = CalculatorEngine::new();
        for i in 0..15 {
            calc.clear();
            calc.input_digit('1');
            calc.input_operator('+');
            calc.input_digit(char::from_digit(i as u32 % 10, 10).unwrap());
            calc.equals();
        }
        assert_eq!(calc.history().len(), HISTORY_CAP);
    }

    #[test]
    fn negate_button() {
        let mut calc = CalculatorEngine::new();
        calc.input_digit('5');
        calc.negate();
        assert_eq!(calc.display(), "-5");

        calc.negate();
        assert_eq!(calc.display(), "5");
    }

    #[test]
    fn backspace() {
        let mut calc = CalculatorEngine::new();
        calc.input_digit('1');
        calc.input_digit('2');
        calc.input_digit('3');
        assert_eq!(calc.display(), "123");

        calc.backspace();
        assert_eq!(calc.display(), "12");

        calc.backspace();
        calc.backspace();
        assert_eq!(calc.display(), "0");
    }

    #[test]
    fn clear_resets_state() {
        let mut calc = CalculatorEngine::new();
        calc.input_digit('5');
        calc.input_operator('+');
        calc.input_digit('3');
        calc.clear();
        assert_eq!(calc.display(), "0");
        assert!(calc.history().is_empty()); // Clear doesn't clear history
    }

    #[test]
    fn scientific_notation_parsing() {
        // Test that 2e+3 parses to 2000, not 20000
        let result = CalculatorEngine::parse_number("2e+3").unwrap();
        assert_eq!(result, 2000.0);

        let result = CalculatorEngine::parse_number("2e-3").unwrap();
        assert_eq!(result, 0.002);

        let result = CalculatorEngine::parse_number("1.5e2").unwrap();
        assert_eq!(result, 150.0);
    }

    #[test]
    fn format_number_removes_trailing_zero() {
        assert_eq!(CalculatorEngine::format_number(5.0), "5");
        assert_eq!(CalculatorEngine::format_number(5.5), "5.5");
        assert_eq!(CalculatorEngine::format_number(-3.0), "-3");
    }

    #[test]
    fn format_number_scientific_for_large() {
        let result = CalculatorEngine::format_number(1e12);
        assert!(result.contains('e'));

        let result = CalculatorEngine::format_number(1e-5);
        assert!(result.contains('e'));
    }
}
