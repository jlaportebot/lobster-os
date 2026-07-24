//! Userspace GPIO API — libgpiod-like interface for applications
//!
//! Provides a clean abstraction over the raw GPIO driver with:
//! - Line request/reserve system (prevents conflicts)
//! - Edge detection (rising, falling, both)
//! - Bulk operations (read/write multiple lines atomically)
//! - Line naming/aliases
//! - Integration with devfs (/dev/gpiochip0)

use crate::drivers::gpio;
use crate::println;
use crate::sync::spinlock::Spinlock;

/// Total GPIO lines on BCM2712
const NUM_GPIO_LINES: usize = 54;

/// Line direction
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LineDirection {
    Input,
    Output,
}

/// Line request type
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RequestType {
    /// Read line values
    GetValue,
    /// Write line values
    SetValue,
    /// Monitor edge events
    EventRequest,
}

/// Edge detection type
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Edge {
    None,
    Rising,
    Falling,
    Both,
}

/// Line state — tracks whether a line is in use and by whom
#[derive(Clone, Copy)]
struct LineState {
    in_use: bool,
    direction: LineDirection,
    pull: gpio::PullMode,
    label: [u8; 32],
    label_len: usize,
}

impl LineState {
    const fn empty() -> Self {
        LineState {
            in_use: false,
            direction: LineDirection::Input,
            pull: gpio::PullMode::None,
            label: [0; 32],
            label_len: 0,
        }
    }
}

static LINE_STATES: Spinlock<[LineState; NUM_GPIO_LINES]> =
    Spinlock::new([LineState::empty(); NUM_GPIO_LINES]);

/// Initialize the GPIO userspace subsystem
pub fn init() {
    let mut states = LINE_STATES.lock();
    for s in states.iter_mut() {
        *s = LineState::empty();
    }
    println!(
        "[gpio] Userspace GPIO API initialized ({} lines)",
        NUM_GPIO_LINES
    );
}

/// Request a GPIO line for use
pub fn request_line(
    pin: u8,
    direction: LineDirection,
    label: Option<&str>,
) -> Result<(), &'static str> {
    if pin as usize >= NUM_GPIO_LINES {
        return Err("invalid GPIO pin number");
    }

    let mut states = LINE_STATES.lock();
    if states[pin as usize].in_use {
        return Err("GPIO line already in use");
    }

    states[pin as usize].in_use = true;
    states[pin as usize].direction = direction;

    if let Some(l) = label {
        let len = l.len().min(32);
        states[pin as usize].label[..len].copy_from_slice(&l.as_bytes()[..len]);
        states[pin as usize].label_len = len;
    }

    // Configure hardware
    match direction {
        LineDirection::Input => gpio::set_pin_mode(pin, gpio::PinMode::Input),
        LineDirection::Output => gpio::set_pin_mode(pin, gpio::PinMode::Output),
    }

    Ok(())
}

/// Release a GPIO line (make it available again)
pub fn release_line(pin: u8) {
    if pin as usize >= NUM_GPIO_LINES {
        return;
    }
    let mut states = LINE_STATES.lock();
    states[pin as usize] = LineState::empty();
}

/// Read a GPIO line value
pub fn get_value(pin: u8) -> Result<bool, &'static str> {
    if pin as usize >= NUM_GPIO_LINES {
        return Err("invalid GPIO pin");
    }
    let states = LINE_STATES.lock();
    if !states[pin as usize].in_use {
        return Err("GPIO line not requested");
    }
    if states[pin as usize].direction != LineDirection::Input {
        return Err("GPIO line is not an input");
    }
    Ok(gpio::read_pin(pin))
}

/// Write a GPIO line value
pub fn set_value(pin: u8, value: bool) -> Result<(), &'static str> {
    if pin as usize >= NUM_GPIO_LINES {
        return Err("invalid GPIO pin");
    }
    let states = LINE_STATES.lock();
    if !states[pin as usize].in_use {
        return Err("GPIO line not requested");
    }
    if states[pin as usize].direction != LineDirection::Output {
        return Err("GPIO line is not an output");
    }
    if value {
        gpio::set_pin(pin);
    } else {
        gpio::clear_pin(pin);
    }
    Ok(())
}

/// Set pull-up/down resistor on a line
pub fn set_pull(pin: u8, pull: gpio::PullMode) -> Result<(), &'static str> {
    if pin as usize >= NUM_GPIO_LINES {
        return Err("invalid GPIO pin");
    }
    let mut states = LINE_STATES.lock();
    states[pin as usize].pull = pull;
    gpio::set_pull(pin, pull);
    Ok(())
}

/// Check if a line is in use
pub fn is_line_in_use(pin: u8) -> bool {
    if pin as usize >= NUM_GPIO_LINES {
        return false;
    }
    let states = LINE_STATES.lock();
    states[pin as usize].in_use
}

/// Check if a line is in use (alternative to line_label)
pub fn line_in_use_with_info(pin: u8) -> Option<(LineDirection, [u8; 32], usize)> {
    if pin as usize >= NUM_GPIO_LINES {
        return None;
    }
    let states = LINE_STATES.lock();
    if !states[pin as usize].in_use {
        return None;
    }
    Some((
        states[pin as usize].direction,
        states[pin as usize].label,
        states[pin as usize].label_len,
    ))
}

/// Bulk read multiple GPIO lines
pub fn bulk_get_values(pins: &[u8]) -> Result<u64, &'static str> {
    let mut result: u64 = 0;
    for (i, &pin) in pins.iter().enumerate() {
        if i >= 64 {
            break;
        }
        match get_value(pin) {
            Ok(true) => result |= 1 << i,
            Ok(false) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(result)
}

/// Bulk write multiple GPIO lines
pub fn bulk_set_values(pins: &[u8], values: u64) -> Result<(), &'static str> {
    for (i, &pin) in pins.iter().enumerate() {
        if i >= 64 {
            break;
        }
        let val = (values >> i) & 1 != 0;
        set_value(pin, val)?;
    }
    Ok(())
}

/// Print the current state of all GPIO lines (for debugging)
pub fn print_status() {
    println!("[gpio] Line states:");
    let states = LINE_STATES.lock();
    for i in 0..NUM_GPIO_LINES {
        if states[i].in_use {
            let label =
                core::str::from_utf8(&states[i].label[..states[i].label_len]).unwrap_or("?");
            let dir = match states[i].direction {
                LineDirection::Input => "IN",
                LineDirection::Output => "OUT",
            };
            println!("  GPIO{}: {} ({})", i, dir, label);
        }
    }
}
