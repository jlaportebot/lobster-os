//! GPIO Character Device (/dev/gpiochip) for LobsterOS
//!
//! Implements a Linux-compatible gpiochip character device interface
//! for userspace GPIO control via libgpiod or direct ioctl.
//!
//! Uses RP1 GPIOs (banks 0 and 1, GPIOs 0-45).
//! Registered via devfs with ioctl support.

use crate::drivers::rp1::{gpio_get_input, gpio_set_oe, gpio_set_output, gpio_set_pad, GpioBank};
use crate::fs::devfs::{register_device, DeviceOps};
use crate::println;
use crate::sync::spinlock::Spinlock;
use alloc::format;
use alloc::string::{String, ToString};

/// GPIO line direction
#[derive(Clone, Copy, PartialEq, Debug)]
#[repr(u32)]
pub enum GpioDirection {
    Input = 0,
    Output = 1,
    AsIs = 2, // Don't change direction
}

/// GPIO line bias
#[derive(Clone, Copy, PartialEq, Debug)]
#[repr(u32)]
pub enum GpioBias {
    Disabled = 0,
    PullUp = 1,
    PullDown = 2,
}

/// GPIO line drive
#[derive(Clone, Copy, PartialEq, Debug)]
#[repr(u32)]
pub enum GpioDrive {
    PushPull = 0,
    OpenDrain = 1,
    OpenSource = 2,
}

/// GPIO line config flags
pub mod line_flags {
    pub const USED: u32 = 1 << 0;
    pub const ACTIVE_LOW: u32 = 1 << 1;
    pub const INPUT: u32 = 1 << 2;
    pub const OUTPUT: u32 = 1 << 3;
    pub const EDGE_RISING: u32 = 1 << 4;
    pub const EDGE_FALLING: u32 = 1 << 5;
    pub const EDGE_BOTH: u32 = (1 << 4) | (1 << 5);
    pub const BIAS_PULL_UP: u32 = 1 << 6;
    pub const BIAS_PULL_DOWN: u32 = 1 << 7;
    pub const DRIVE_OPEN_DRAIN: u32 = 1 << 8;
    pub const DRIVE_OPEN_SOURCE: u32 = 1 << 9;
}

/// GPIO line info (userspace visible)
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct GpioLineInfo {
    pub line_offset: u32,
    pub flags: u32,
    pub name: [u8; 32],
    pub consumer: [u8; 32],
}

/// GPIO chip info (userspace visible)
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct GpioChipInfo {
    pub name: [u8; 32],
    pub label: [u8; 32],
    pub lines: u32,
}

/// GPIO line request (for ioctl)
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct GpioLineRequest {
    pub line_offsets: [u32; 64],
    pub num_lines: u32,
    pub flags: u32,
    pub default_values: [u8; 64],
    pub consumer: [u8; 32],
    pub fd: i32, // Output: assigned file descriptor
}

/// GPIO line values (for get/set)
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct GpioLineValues {
    pub mask: u64,
    pub bits: u64,
}

/// GPIO line config (for ioctl)
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct GpioLineConfig {
    pub flags: u32,
    pub num_attrs: u32,
    // attrs follow (variable length)
}

/// GPIO line attribute
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct GpioLineAttribute {
    pub attr: u32,
    pub mask: u64,
    pub values: [u64; 4],
}

/// Attribute IDs
pub mod attr {
    pub const FLAGS: u32 = 1;
    pub const OUTPUT_VALUES: u32 = 2;
    pub const DEBOUNCE: u32 = 3;
}

/// Ioctl command numbers (matching Linux gpio-uapi.h)
pub mod ioctl {
    pub const GPIO_GET_CHIPINFO_IOCTL: u32 = 0x8040B401; // _IOR('B', 1, gpiochip_info)
    pub const GPIO_GET_LINEINFO_IOCTL: u32 = 0x8040B402; // _IOR('B', 2, gpio_line_info)
    pub const GPIO_GET_LINEINFO_WATCH_IOCTL: u32 = 0x8040B403; // _IOR('B', 3, ...)
    pub const GPIO_LINE_GET_VALUES_IOCTL: u32 = 0x8008B404; // _IOR('B', 4, gpio_line_values)
    pub const GPIO_LINE_SET_VALUES_IOCTL: u32 = 0x4008B405; // _IOW('B', 5, gpio_line_values)
    pub const GPIO_LINE_SET_CONFIG_IOCTL: u32 = 0x4040B406; // _IOW('B', 6, gpio_line_config)
    pub const GPIO_LINE_WATCH_IOCTL: u32 = 0x4040B407; // _IOW('B', 7, ...)
}

/// Maximum GPIO lines per chip
const MAX_LINES: usize = 46; // RP1 Bank 0 (0-27) + Bank 1 (28-45)

/// GPIO line state
#[derive(Clone, Debug)]
struct GpioLineState {
    offset: u32,
    bank: GpioBank,
    bank_gpio: u32,
    direction: GpioDirection,
    bias: GpioBias,
    drive: GpioDrive,
    active_low: bool,
    used: bool,
    consumer: String,
    output_value: bool,
    edge_rising: bool,
    edge_falling: bool,
    debounce_us: u32,
}

impl GpioLineState {
    fn new(offset: u32) -> Self {
        let (bank, bank_gpio) = if offset <= 27 {
            (GpioBank::Bank0, offset)
        } else {
            (GpioBank::Bank1, offset - 28)
        };

        Self {
            offset,
            bank,
            bank_gpio,
            direction: GpioDirection::AsIs,
            bias: GpioBias::Disabled,
            drive: GpioDrive::PushPull,
            active_low: false,
            used: false,
            consumer: String::new(),
            output_value: false,
            edge_rising: false,
            edge_falling: false,
            debounce_us: 0,
        }
    }
}

/// GPIO chip device state
struct GpioChipDev {
    lines: [Spinlock<GpioLineState>; MAX_LINES],
    label: String,
}

impl GpioChipDev {
    fn new() -> Self {
        let lines: [Spinlock<GpioLineState>; MAX_LINES] =
            core::array::from_fn(|i| Spinlock::new(GpioLineState::new(i as u32)));
        Self {
            lines,
            label: String::from("rp1-gpio"),
        }
    }

    fn get_chip_info(&self, info: &mut GpioChipInfo) {
        info.lines = MAX_LINES as u32;
        // Copy name
        let name = b"rp1-gpio";
        info.name[..name.len()].copy_from_slice(name);
        // Copy label
        let label = self.label.as_bytes();
        info.label[..label.len().min(32)].copy_from_slice(label);
    }

    fn get_line_info(&self, offset: u32, info: &mut GpioLineInfo) -> Result<(), &'static str> {
        if offset >= MAX_LINES as u32 {
            return Err("InvalidArgument");
        }
        let line = self.lines[offset as usize].lock();
        info.line_offset = line.offset;
        let mut flags = 0u32;
        if line.used {
            flags |= line_flags::USED;
        }
        if line.active_low {
            flags |= line_flags::ACTIVE_LOW;
        }
        match line.direction {
            GpioDirection::Input => flags |= line_flags::INPUT,
            GpioDirection::Output => flags |= line_flags::OUTPUT,
            _ => {}
        }
        if line.edge_rising {
            flags |= line_flags::EDGE_RISING;
        }
        if line.edge_falling {
            flags |= line_flags::EDGE_FALLING;
        }
        match line.bias {
            GpioBias::PullUp => flags |= line_flags::BIAS_PULL_UP,
            GpioBias::PullDown => flags |= line_flags::BIAS_PULL_DOWN,
            _ => {}
        }
        match line.drive {
            GpioDrive::OpenDrain => flags |= line_flags::DRIVE_OPEN_DRAIN,
            GpioDrive::OpenSource => flags |= line_flags::DRIVE_OPEN_SOURCE,
            _ => {}
        }
        info.flags = flags;
        // Copy name
        let name = format!("GPIO{}", line.offset);
        let name_bytes = name.as_bytes();
        info.name[..name_bytes.len().min(32)].copy_from_slice(name_bytes);
        // Copy consumer
        let consumer_bytes = line.consumer.as_bytes();
        info.consumer[..consumer_bytes.len().min(32)].copy_from_slice(consumer_bytes);
        Ok(())
    }

    fn get_line_values(&self, mask: u64, bits: &mut u64) -> Result<(), &'static str> {
        if mask == 0 {
            return Ok(());
        }
        for i in 0..MAX_LINES {
            if mask & (1u64 << i) != 0 {
                let line = self.lines[i].lock();
                let val = match line.direction {
                    GpioDirection::Input => {
                        let val = gpio_get_input(line.bank, line.bank_gpio);
                        if line.active_low {
                            !val
                        } else {
                            val
                        }
                    }
                    GpioDirection::Output => {
                        if line.active_low {
                            !line.output_value
                        } else {
                            line.output_value
                        }
                    }
                    _ => false,
                };
                if val {
                    *bits |= 1u64 << i;
                } else {
                    *bits &= !(1u64 << i);
                }
            }
        }
        Ok(())
    }

    fn set_line_values(&self, mask: u64, bits: u64) -> Result<(), &'static str> {
        if mask == 0 {
            return Ok(());
        }
        for i in 0..MAX_LINES {
            if mask & (1u64 << i) != 0 {
                let mut line = self.lines[i].lock();
                if line.direction != GpioDirection::Output {
                    return Err("InvalidArgument");
                }
                let val = (bits & (1u64 << i)) != 0;
                line.output_value = val;
                let hw_val = if line.active_low { !val } else { val };
                gpio_set_output(line.bank, line.bank_gpio, hw_val);
            }
        }
        Ok(())
    }

    fn set_line_config(&self, req: &GpioLineRequest) -> Result<(), &'static str> {
        for i in 0..req.num_lines as usize {
            let offset = req.line_offsets[i];
            if offset >= MAX_LINES as u32 {
                return Err("InvalidArgument");
            }

            let mut line = self.lines[offset as usize].lock();

            // Parse flags
            let flags = req.flags;
            let is_output = (flags & line_flags::OUTPUT) != 0;
            let is_input = (flags & line_flags::INPUT) != 0;

            if is_output && is_input {
                return Err("InvalidArgument");
            }

            line.used = (flags & line_flags::USED) != 0;
            line.active_low = (flags & line_flags::ACTIVE_LOW) != 0;
            line.edge_rising = (flags & line_flags::EDGE_RISING) != 0;
            line.edge_falling = (flags & line_flags::EDGE_FALLING) != 0;

            if is_output {
                line.direction = GpioDirection::Output;
                let default_val = if i < 64 {
                    req.default_values[i] != 0
                } else {
                    false
                };
                line.output_value = default_val;
                let hw_val = if line.active_low {
                    !default_val
                } else {
                    default_val
                };
                gpio_set_oe(line.bank, line.bank_gpio, true);
                gpio_set_output(line.bank, line.bank_gpio, hw_val);
            } else if is_input {
                line.direction = GpioDirection::Input;
                gpio_set_oe(line.bank, line.bank_gpio, false);
            }

            // Bias
            if (flags & line_flags::BIAS_PULL_UP) != 0 {
                line.bias = GpioBias::PullUp;
                gpio_set_pad(line.bank, line.bank_gpio, 1, true, false, true, false);
            } else if (flags & line_flags::BIAS_PULL_DOWN) != 0 {
                line.bias = GpioBias::PullDown;
                gpio_set_pad(line.bank, line.bank_gpio, 1, false, true, true, false);
            } else {
                line.bias = GpioBias::Disabled;
                gpio_set_pad(line.bank, line.bank_gpio, 1, false, false, true, false);
            }

            // Drive
            if (flags & line_flags::DRIVE_OPEN_DRAIN) != 0 {
                line.drive = GpioDrive::OpenDrain;
            } else if (flags & line_flags::DRIVE_OPEN_SOURCE) != 0 {
                line.drive = GpioDrive::OpenSource;
            } else {
                line.drive = GpioDrive::PushPull;
            }

            // Consumer label
            line.consumer = String::from_utf8_lossy(&req.consumer)
                .trim_end_matches('\0')
                .to_string();
        }
        Ok(())
    }
}

/// Global GPIO chip device
static mut GPIO_CHIP_DEV: Option<GpioChipDev> = None;
static mut GPIO_CHIP_DEV_IDX: Option<usize> = None;

/// IOCTL handler for gpiochip
fn gpiochip_ioctl(cmd: u64, arg: u64) -> i64 {
    unsafe {
        let dev = match GPIO_CHIP_DEV.as_ref() {
            Some(d) => d,
            None => return -1,
        };

        match cmd as u32 {
            ioctl::GPIO_GET_CHIPINFO_IOCTL => {
                let info_ptr = arg as *mut GpioChipInfo;
                dev.get_chip_info(&mut *info_ptr);
                0
            }
            ioctl::GPIO_GET_LINEINFO_IOCTL => {
                let info_ptr = arg as *mut GpioLineInfo;
                let offset = (*info_ptr).line_offset;
                if dev.get_line_info(offset, &mut *info_ptr).is_ok() {
                    0
                } else {
                    -22 // -EINVAL
                }
            }
            ioctl::GPIO_LINE_GET_VALUES_IOCTL => {
                let values_ptr = arg as *mut GpioLineValues;
                let mask = (*values_ptr).mask;
                let mut bits = 0u64;
                if dev.get_line_values(mask, &mut bits).is_ok() {
                    (*values_ptr).bits = bits;
                    0
                } else {
                    -22 // -EINVAL
                }
            }
            ioctl::GPIO_LINE_SET_VALUES_IOCTL => {
                let values_ptr = arg as *const GpioLineValues;
                let mask = (*values_ptr).mask;
                let bits = (*values_ptr).bits;
                if dev.set_line_values(mask, bits).is_ok() {
                    0
                } else {
                    -22 // -EINVAL
                }
            }
            ioctl::GPIO_LINE_SET_CONFIG_IOCTL => {
                let req_ptr = arg as *const GpioLineRequest;
                if dev.set_line_config(&*req_ptr).is_ok() {
                    0
                } else {
                    -22 // -EINVAL
                }
            }
            _ => -25, // -ENOTTY
        }
    }
}

/// Initialize the GPIO character device
pub fn init() {
    unsafe {
        GPIO_CHIP_DEV = Some(GpioChipDev::new());

        // Register as character device via devfs
        let ops = DeviceOps {
            read: None,
            write: None,
            ioctl: Some(gpiochip_ioctl),
        };

        let idx = register_device(
            "gpiochip0",
            254,   // Major for gpiochip
            0,     // Minor 0
            false, // Character device
            ops,
            0o666,
        );

        GPIO_CHIP_DEV_IDX = idx;
        println!(
            "[gpiochip] Registered /dev/gpiochip0 (rp1-gpio, {} lines)",
            MAX_LINES
        );
    }
}

/// Helper functions for kernel-internal GPIO use
pub mod kernel {
    use super::*;

    /// Set GPIO as output
    pub fn gpio_output(offset: u32, high: bool) -> Result<(), &'static str> {
        if offset >= MAX_LINES as u32 {
            return Err("InvalidArgument");
        }
        unsafe {
            let dev = GPIO_CHIP_DEV.as_ref().ok_or("NotReady")?;
            let mut line = dev.lines[offset as usize].lock();
            line.direction = GpioDirection::Output;
            line.output_value = high;
            let hw_val = if line.active_low { !high } else { high };
            gpio_set_oe(line.bank, line.bank_gpio, true);
            gpio_set_output(line.bank, line.bank_gpio, hw_val);
            Ok(())
        }
    }

    /// Get GPIO input value
    pub fn gpio_input(offset: u32) -> Result<bool, &'static str> {
        if offset >= MAX_LINES as u32 {
            return Err("InvalidArgument");
        }
        unsafe {
            let dev = GPIO_CHIP_DEV.as_ref().ok_or("NotReady")?;
            let line = dev.lines[offset as usize].lock();
            let val = gpio_get_input(line.bank, line.bank_gpio);
            Ok(if line.active_low { !val } else { val })
        }
    }

    /// Set GPIO direction
    pub fn gpio_set_direction(offset: u32, dir: GpioDirection) -> Result<(), &'static str> {
        if offset >= MAX_LINES as u32 {
            return Err("InvalidArgument");
        }
        unsafe {
            let dev = GPIO_CHIP_DEV.as_ref().ok_or("NotReady")?;
            let mut line = dev.lines[offset as usize].lock();
            line.direction = dir;
            match dir {
                GpioDirection::Output => {
                    gpio_set_oe(line.bank, line.bank_gpio, true);
                }
                GpioDirection::Input => {
                    gpio_set_oe(line.bank, line.bank_gpio, false);
                }
                _ => {}
            }
            Ok(())
        }
    }
}
