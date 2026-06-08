//! BCM2712 GPIO controller driver

/// GPIO base address
const GPIO_BASE: u64 = 0x10_7D50_4000;

/// GPIO function select registers
const GPFSEL0: u64 = GPIO_BASE + 0x00;
const GPFSEL1: u64 = GPIO_BASE + 0x04;
const GPFSEL2: u64 = GPIO_BASE + 0x08;
const GPFSEL3: u64 = GPIO_BASE + 0x0C;
const GPFSEL4: u64 = GPIO_BASE + 0x10;
const GPFSEL5: u64 = GPIO_BASE + 0x14;

/// GPIO output set registers
const GPSET0: u64 = GPIO_BASE + 0x1C;
const GPSET1: u64 = GPIO_BASE + 0x20;

/// GPIO output clear registers
const GPCLR0: u64 = GPIO_BASE + 0x28;
const GPCLR1: u64 = GPIO_BASE + 0x2C;

/// GPIO level registers
const GPLEV0: u64 = GPIO_BASE + 0x34;
const GPLEV1: u64 = GPIO_BASE + 0x38;

/// GPIO pull-up/down registers
const GPPUD: u64 = GPIO_BASE + 0x94;
const GPPUDCLK0: u64 = GPIO_BASE + 0x98;
const GPPUDCLK1: u64 = GPIO_BASE + 0x9C;

/// GPIO pin modes
#[derive(Debug, Clone, Copy)]
pub enum PinMode {
    Input = 0,
    Output = 1,
    Alt0 = 4,
    Alt1 = 5,
    Alt2 = 6,
    Alt3 = 7,
    Alt4 = 3,
    Alt5 = 2,
}

/// Set a GPIO pin mode
pub fn set_pin_mode(pin: u8, mode: PinMode) {
    if pin > 53 {
        return;
    }

    let reg = GPFSEL0 + ((pin / 10) * 4) as u64;
    let shift = (pin % 10) * 3;
    let mask = 0b111 << shift;

    // Read the current value
    let mut val = unsafe { core::ptr::read_volatile(reg as *const u32) };

    // Clear the bits for this pin
    val &= !mask;

    // Set the new mode
    val |= (mode as u32) << shift;

    // Write back
    unsafe { core::ptr::write_volatile(reg as *mut u32, val) };
}

/// Set a GPIO pin high
pub fn set_pin(pin: u8) {
    if pin > 53 {
        return;
    }

    let reg = if pin < 32 { GPSET0 } else { GPSET1 };
    let bit = if pin < 32 { pin } else { pin - 32 };

    unsafe { core::ptr::write_volatile(reg as *mut u32, 1 << bit) };
}

/// Set a GPIO pin low
pub fn clear_pin(pin: u8) {
    if pin > 53 {
        return;
    }

    let reg = if pin < 32 { GPCLR0 } else { GPCLR1 };
    let bit = if pin < 32 { pin } else { pin - 32 };

    unsafe { core::ptr::write_volatile(reg as *mut u32, 1 << bit) };
}

/// Read a GPIO pin level
pub fn read_pin(pin: u8) -> bool {
    if pin > 53 {
        return false;
    }

    let reg = if pin < 32 { GPLEV0 } else { GPLEV1 };
    let bit = if pin < 32 { pin } else { pin - 32 };
    let val = unsafe { core::ptr::read_volatile(reg as *const u32) };

    (val & (1 << bit)) != 0
}

/// Set pull-up/down for a GPIO pin
pub fn set_pull(pin: u8, pull: PullMode) {
    if pin > 53 {
        return;
    }

    // Set the pull mode
    unsafe { core::ptr::write_volatile(GPPUD as *mut u32, pull as u32) };

    // Wait for the control signal to be set
    crate::arch::cpu::nop();
    crate::arch::cpu::nop();
    crate::arch::cpu::nop();
    crate::arch::cpu::nop();
    crate::arch::cpu::nop();

    // Clock the pin
    let reg = if pin < 32 { GPPUDCLK0 } else { GPPUDCLK1 };
    let bit = if pin < 32 { pin } else { pin - 32 };
    unsafe { core::ptr::write_volatile(reg as *mut u32, 1 << bit) };

    // Wait for the control signal to be set
    crate::arch::cpu::nop();
    crate::arch::cpu::nop();
    crate::arch::cpu::nop();
    crate::arch::cpu::nop();
    crate::arch::cpu::nop();

    // Clear the control signal
    unsafe { core::ptr::write_volatile(GPPUD as *mut u32, 0) };
    unsafe { core::ptr::write_volatile(reg as *mut u32, 0) };
}

/// Pull-up/down modes
#[derive(Debug, Clone, Copy)]
pub enum PullMode {
    None = 0,
    Down = 1,
    Up = 2,
}
