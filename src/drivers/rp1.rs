//! RP1 I/O Controller Driver for Raspberry Pi 5 (BCM2712)
//!
//! The RP1 is a separate I/O controller chip connected via PCIe to the BCM2712.
//! It provides: GPIO, UART (6x), I2C (6x), SPI (6x), PWM (4x), I2S (2x),
//! ADC, RIO (fast GPIO), and more.
//!
//! On Pi 5, the RP1 register space is mapped at 0x1F_0000_0000 in the ARM
//! physical address space (which corresponds to 0x4000_0000 on the RP1 bus).
//!
//! Reference: RP1 Peripherals Datasheet (RP-008370-DS-1)

use crate::memory::{phys_to_virt, PhysAddr, VirtAddr};
use crate::println;
use core::ptr::{read_volatile, write_volatile};

/// RP1 base physical address on Pi 5 (ARM view)
pub const RP1_BASE_PHYS: u64 = 0x1F_0000_0000;

/// RP1 register space size (64 MB)
const RP1_SIZE: u64 = 0x0400_0000;

/// RP1 peripheral offsets from base (matching RP1 datasheet)
pub mod offsets {
    /// SYSINFO registers
    pub const SYSINFO: u64 = 0x0000_0000;

    /// GPIO Bank 0 (GPIO 0-27)
    pub const GPIO_BANK0_BASE: u64 = 0x0000_1000;
    pub const GPIO_BANK0_REGS_SIZE: u64 = 0x1000;

    /// GPIO Bank 1 (GPIO 28-45, plus some special)
    pub const GPIO_BANK1_BASE: u64 = 0x0000_2000;
    pub const GPIO_BANK1_REGS_SIZE: u64 = 0x1000;

    /// PAD control registers
    pub const PADS_BANK0_BASE: u64 = 0x0000_3000;
    pub const PADS_BANK1_BASE: u64 = 0x0000_4000;

    /// RIO (fast GPIO access)
    pub const RIO_BASE: u64 = 0x0000_5000;

    /// UART 0-5
    pub const UART_BASE: u64 = 0x0003_0000;
    pub const UART_STRIDE: u64 = 0x4000;

    /// I2C 0-5
    pub const I2C_BASE: u64 = 0x0006_0000;
    pub const I2C_STRIDE: u64 = 0x4000;

    /// SPI 0-5
    pub const SPI_BASE: u64 = 0x0007_8000;
    pub const SPI_STRIDE: u64 = 0x4000;

    /// PWM 0-3
    pub const PWM_BASE: u64 = 0x0009_0000;
    pub const PWM_STRIDE: u64 = 0x4000;

    /// I2S 0-1
    pub const I2S_BASE: u64 = 0x000A_0000;
    pub const I2S_STRIDE: u64 = 0x4000;

    /// ADC
    pub const ADC_BASE: u64 = 0x000B_0000;

    /// DMA
    pub const DMA_BASE: u64 = 0x000C_0000;

    /// OTP
    pub const OTP_BASE: u64 = 0x000F_0000;
}

/// GPIO register offsets within a bank
pub mod gpio_regs {
    /// GPIO output register
    pub const OUT: u64 = 0x00;
    /// GPIO output enable register
    pub const OE: u64 = 0x04;
    /// GPIO input register (sync)
    pub const IN_SYNC: u64 = 0x08;
    /// GPIO input register (async)
    pub const IN_NOSYNC: u64 = 0x0C;
    /// GPIO interrupt status
    pub const INT_STATUS: u64 = 0x10;
    /// GPIO interrupt enable
    pub const INT_ENABLE: u64 = 0x14;
    /// GPIO interrupt type (level/edge)
    pub const INT_TYPE: u64 = 0x18;
    /// GPIO interrupt polarity
    pub const INT_POLARITY: u64 = 0x1C;
    /// GPIO debounce
    pub const DEBOUNCE: u64 = 0x20;

    /// Function select registers (3 bits per GPIO, 10 GPIOs per reg)
    pub const FUNCSEL_BASE: u64 = 0x80; // 12 registers for 30 GPIOs (0-29)
    pub const FUNCSEL_COUNT: usize = 12;
}

/// GPIO function select values
#[derive(Clone, Copy, PartialEq)]
#[repr(u32)]
pub enum GpioFunction {
    Null = 0x0, // No function
    Spi = 0x1,  // SPI
    Uart = 0x2, // UART
    I2c = 0x3,  // I2C
    Pwm = 0x4,  // PWM
    I2s = 0x5,  // I2S
    Pio = 0x6,  // PIO
    Clk = 0x7,  // Clock
    Pio1 = 0x8, // PIO1
    Pwm1 = 0x9, // PWM1
    Sio = 0xA,  // SIO
    Pio2 = 0xB, // PIO2
    Arm = 0xC,  // ARM JTAG
    Pci = 0xD,  // PCIe
    Sdio = 0xE, // SDIO
}

/// Pad control register bits
pub mod pad_bits {
    pub const OD: u32 = 1 << 7; // Output disable
    pub const IE: u32 = 1 << 6; // Input enable
    pub const DRIVE_SHIFT: u32 = 4; // Drive strength (2 bits)
    pub const DRIVE_2MA: u32 = 0 << 4;
    pub const DRIVE_4MA: u32 = 1 << 4;
    pub const DRIVE_8MA: u32 = 2 << 4;
    pub const DRIVE_12MA: u32 = 3 << 4;
    pub const PUE: u32 = 1 << 3; // Pull-up enable
    pub const PDE: u32 = 1 << 2; // Pull-down enable
    pub const SCHMITT: u32 = 1 << 1; // Schmitt trigger enable
    pub const SLEWFAST: u32 = 1 << 0; // Slew rate fast
}

/// RIO register offsets
pub mod rio_regs {
    pub const OUT: u64 = 0x00;
    pub const OE: u64 = 0x04;
    pub const IN_SYNC: u64 = 0x08;
    pub const IN_NOSYNC: u64 = 0x0C;
    pub const OUT_SET: u64 = 0x10;
    pub const OUT_CLR: u64 = 0x14;
    pub const OUT_XOR: u64 = 0x18;
    pub const OE_SET: u64 = 0x1C;
    pub const OE_CLR: u64 = 0x20;
    pub const OE_XOR: u64 = 0x24;
}

/// SYSINFO register offsets
pub mod sysinfo_regs {
    pub const IDENTITY: u64 = 0x00;
    pub const REVISION: u64 = 0x04;
    pub const DATE: u64 = 0x08;
    pub const GPIO_BANKS: u64 = 0x0C;
    pub const UART_COUNT: u64 = 0x10;
    pub const I2C_COUNT: u64 = 0x14;
    pub const SPI_COUNT: u64 = 0x18;
    pub const PWM_COUNT: u64 = 0x1C;
    pub const I2S_COUNT: u64 = 0x20;
    pub const ADC_COUNT: u64 = 0x24;
    pub const DMA_CHANNELS: u64 = 0x28;
}

/// RP1 driver state
static mut RP1_VIRT_BASE: Option<VirtAddr> = None;
static mut RP1_INITIALIZED: bool = false;

/// Initialize the RP1 driver
/// Maps the RP1 register space and verifies the device
pub fn init() {
    unsafe {
        if RP1_INITIALIZED {
            return;
        }

        // Map RP1 physical address to virtual
        let phys = PhysAddr::new(RP1_BASE_PHYS);
        let virt = phys_to_virt(phys);
        RP1_VIRT_BASE = Some(virt);

        // Verify RP1 is accessible by reading identity register
        let identity = rp1_read32(offsets::SYSINFO + sysinfo_regs::IDENTITY);
        let revision = rp1_read32(offsets::SYSINFO + sysinfo_regs::REVISION);

        println!(
            "[rp1] RP1 initialized at {:#018x}, identity={:#010x}, revision={:#010x}",
            virt.0, identity, revision
        );

        // Read peripheral counts
        let gpio_banks = rp1_read32(offsets::SYSINFO + sysinfo_regs::GPIO_BANKS);
        let uart_count = rp1_read32(offsets::SYSINFO + sysinfo_regs::UART_COUNT);
        let i2c_count = rp1_read32(offsets::SYSINFO + sysinfo_regs::I2C_COUNT);
        let spi_count = rp1_read32(offsets::SYSINFO + sysinfo_regs::SPI_COUNT);
        let pwm_count = rp1_read32(offsets::SYSINFO + sysinfo_regs::PWM_COUNT);
        let i2s_count = rp1_read32(offsets::SYSINFO + sysinfo_regs::I2S_COUNT);
        let adc_count = rp1_read32(offsets::SYSINFO + sysinfo_regs::ADC_COUNT);
        let dma_channels = rp1_read32(offsets::SYSINFO + sysinfo_regs::DMA_CHANNELS);

        println!(
            "[rp1] Peripherals: GPIO banks={}, UART={}, I2C={}, SPI={}, PWM={}, I2S={}, ADC={}, DMA={}",
            gpio_banks, uart_count, i2c_count, spi_count, pwm_count, i2s_count, adc_count, dma_channels
        );

        RP1_INITIALIZED = true;
    }
}

/// Get the RP1 virtual base address
pub fn virt_base() -> Option<VirtAddr> {
    unsafe { RP1_VIRT_BASE }
}

/// Check if RP1 is initialized
pub fn is_initialized() -> bool {
    unsafe { RP1_INITIALIZED }
}

/// Read a 32-bit register from RP1
#[inline]
pub unsafe fn rp1_read32(offset: u64) -> u32 {
    let base = RP1_VIRT_BASE.expect("RP1 not initialized").0;
    read_volatile((base + offset) as *const u32)
}

/// Write a 32-bit register to RP1
#[inline]
pub unsafe fn rp1_write32(offset: u64, val: u32) {
    let base = RP1_VIRT_BASE.expect("RP1 not initialized").0;
    write_volatile((base + offset) as *mut u32, val);
}

/// Read a 32-bit register from a specific RP1 peripheral
#[inline]
pub unsafe fn rp1_periph_read32(periph_base: u64, offset: u64) -> u32 {
    let base = RP1_VIRT_BASE.expect("RP1 not initialized").0;
    read_volatile((base + periph_base + offset) as *const u32)
}

/// Write a 32-bit register to a specific RP1 peripheral
#[inline]
pub unsafe fn rp1_periph_write32(periph_base: u64, offset: u64, val: u32) {
    let base = RP1_VIRT_BASE.expect("RP1 not initialized").0;
    write_volatile((base + periph_base + offset) as *mut u32, val);
}

/// GPIO bank identifier
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum GpioBank {
    Bank0 = 0,
    Bank1 = 1,
}

impl GpioBank {
    fn base_offset(self) -> u64 {
        match self {
            GpioBank::Bank0 => offsets::GPIO_BANK0_BASE,
            GpioBank::Bank1 => offsets::GPIO_BANK1_BASE,
        }
    }

    fn pads_base_offset(self) -> u64 {
        match self {
            GpioBank::Bank0 => offsets::PADS_BANK0_BASE,
            GpioBank::Bank1 => offsets::PADS_BANK1_BASE,
        }
    }

    fn max_gpio(self) -> u32 {
        match self {
            GpioBank::Bank0 => 27,
            GpioBank::Bank1 => 45,
        }
    }
}

/// Set GPIO function select
pub fn gpio_set_function(bank: GpioBank, gpio: u32, func: GpioFunction) {
    assert!(
        gpio <= bank.max_gpio(),
        "GPIO {} out of range for {:?}",
        gpio,
        bank
    );

    unsafe {
        let bank_base = bank.base_offset();
        let funcsel_reg = gpio_regs::FUNCSEL_BASE + ((gpio / 10) as u64 * 4);
        let shift = (gpio % 10) * 3;
        let mask: u32 = 0b111 << shift;
        let val = (func as u32) << shift;

        let current = rp1_periph_read32(bank_base, funcsel_reg);
        rp1_periph_write32(bank_base, funcsel_reg, (current & !mask) | val);
    }
}

/// Set GPIO output value
pub fn gpio_set_output(bank: GpioBank, gpio: u32, high: bool) {
    assert!(
        gpio <= bank.max_gpio(),
        "GPIO {} out of range for {:?}",
        gpio,
        bank
    );

    unsafe {
        let bank_base = bank.base_offset();
        let out_reg = gpio_regs::OUT;
        let current = rp1_periph_read32(bank_base, out_reg);
        if high {
            rp1_periph_write32(bank_base, out_reg, current | (1 << gpio));
        } else {
            rp1_periph_write32(bank_base, out_reg, current & !(1 << gpio));
        }
    }
}

/// Get GPIO input value
pub fn gpio_get_input(bank: GpioBank, gpio: u32) -> bool {
    assert!(
        gpio <= bank.max_gpio(),
        "GPIO {} out of range for {:?}",
        gpio,
        bank
    );

    unsafe {
        let bank_base = bank.base_offset();
        let in_reg = gpio_regs::IN_SYNC;
        let val = rp1_periph_read32(bank_base, in_reg);
        (val & (1 << gpio)) != 0
    }
}

/// Set GPIO output enable
pub fn gpio_set_oe(bank: GpioBank, gpio: u32, enabled: bool) {
    assert!(
        gpio <= bank.max_gpio(),
        "GPIO {} out of range for {:?}",
        gpio,
        bank
    );

    unsafe {
        let bank_base = bank.base_offset();
        let oe_reg = gpio_regs::OE;
        let current = rp1_periph_read32(bank_base, oe_reg);
        if enabled {
            rp1_periph_write32(bank_base, oe_reg, current | (1 << gpio));
        } else {
            rp1_periph_write32(bank_base, oe_reg, current & !(1 << gpio));
        }
    }
}

/// Configure GPIO pad control
pub fn gpio_set_pad(
    bank: GpioBank,
    gpio: u32,
    drive: u32,
    pue: bool,
    pde: bool,
    schmitt: bool,
    slewfast: bool,
) {
    assert!(
        gpio <= bank.max_gpio(),
        "GPIO {} out of range for {:?}",
        gpio,
        bank
    );

    unsafe {
        let pads_base = bank.pads_base_offset();
        let pad_offset = 0x04 + (gpio as u64 * 4); // PAD register for each GPIO
        let mut val = (drive & 0x3) << pad_bits::DRIVE_SHIFT;
        if pue {
            val |= pad_bits::PUE;
        }
        if pde {
            val |= pad_bits::PDE;
        }
        if schmitt {
            val |= pad_bits::SCHMITT;
        }
        if slewfast {
            val |= pad_bits::SLEWFAST;
        }
        // OD defaults to 0 (output enabled), IE defaults to 0 (input disabled)
        rp1_periph_write32(pads_base, pad_offset, val);
    }
}

/// RIO (fast GPIO) - set output
pub fn rio_set_output(gpio: u32, high: bool) {
    unsafe {
        let base = RP1_VIRT_BASE.expect("RP1 not initialized").0;
        let rio_base = base + offsets::RIO_BASE;
        if high {
            write_volatile((rio_base + rio_regs::OUT_SET) as *mut u32, 1 << gpio);
        } else {
            write_volatile((rio_base + rio_regs::OUT_CLR) as *mut u32, 1 << gpio);
        }
    }
}

/// RIO - set output enable
pub fn rio_set_oe(gpio: u32, enabled: bool) {
    unsafe {
        let base = RP1_VIRT_BASE.expect("RP1 not initialized").0;
        let rio_base = base + offsets::RIO_BASE;
        if enabled {
            write_volatile((rio_base + rio_regs::OE_SET) as *mut u32, 1 << gpio);
        } else {
            write_volatile((rio_base + rio_regs::OE_CLR) as *mut u32, 1 << gpio);
        }
    }
}

/// RIO - get input (sync)
pub fn rio_get_input_sync(gpio: u32) -> bool {
    unsafe {
        let base = RP1_VIRT_BASE.expect("RP1 not initialized").0;
        let rio_base = base + offsets::RIO_BASE;
        let val = read_volatile((rio_base + rio_regs::IN_SYNC) as *const u32);
        (val & (1 << gpio)) != 0
    }
}

/// RIO - get input (async/nosync)
pub fn rio_get_input_nosync(gpio: u32) -> bool {
    unsafe {
        let base = RP1_VIRT_BASE.expect("RP1 not initialized").0;
        let rio_base = base + offsets::RIO_BASE;
        let val = read_volatile((rio_base + rio_regs::IN_NOSYNC) as *const u32);
        (val & (1 << gpio)) != 0
    }
}

/// UART instance
#[derive(Clone, Copy, PartialEq)]
pub enum UartInstance {
    Uart0 = 0,
    Uart1 = 1,
    Uart2 = 2,
    Uart3 = 3,
    Uart4 = 4,
    Uart5 = 5,
}

impl UartInstance {
    fn base_offset(self) -> u64 {
        offsets::UART_BASE + (self as u64 * offsets::UART_STRIDE)
    }
}

/// Initialize a UART instance (PL011 compatible)
pub fn uart_init(instance: UartInstance, baudrate: u32) {
    unsafe {
        let base = instance.base_offset();

        // Disable UART
        rp1_periph_write32(base, 0x30, 0); // CR

        // Clear interrupts
        rp1_periph_write32(base, 0x44, 0x7FF); // ICR

        // Set baud rate
        // UART clock is 48 MHz on RP1
        const UART_CLK: u32 = 48_000_000;
        let ibrd = UART_CLK / (16 * baudrate);
        let fbrd = ((UART_CLK % (16 * baudrate)) * 64 + (8 * baudrate)) / (16 * baudrate);

        rp1_periph_write32(base, 0x24, ibrd); // IBRD
        rp1_periph_write32(base, 0x28, fbrd); // FBRD

        // Line control: 8 bits, no parity, 1 stop bit, FIFOs enabled
        rp1_periph_write32(base, 0x2C, (1 << 4) | (3 << 5) | (1 << 6)); // LCR_H

        // Enable UART, TX, RX
        rp1_periph_write32(base, 0x30, (1 << 0) | (1 << 8) | (1 << 9)); // CR
    }
}

/// UART send byte
pub fn uart_send(instance: UartInstance, byte: u8) {
    unsafe {
        let base = instance.base_offset();
        // Wait for TX FIFO not full
        while rp1_periph_read32(base, 0x18) & (1 << 5) != 0 {} // FR: TXFF
        rp1_periph_write32(base, 0x00, byte as u32); // DR
    }
}

/// UART receive byte (blocking)
pub fn uart_recv(instance: UartInstance) -> u8 {
    unsafe {
        let base = instance.base_offset();
        // Wait for RX FIFO not empty
        while rp1_periph_read32(base, 0x18) & (1 << 4) != 0 {} // FR: RXFE
        rp1_periph_read32(base, 0x00) as u8 // DR
    }
}

/// UART check if receive data available
pub fn uart_rx_ready(instance: UartInstance) -> bool {
    unsafe {
        let base = instance.base_offset();
        (rp1_periph_read32(base, 0x18) & (1 << 4)) == 0 // FR: RXFE = 0 means data available
    }
}

/// I2C instance
#[derive(Clone, Copy, PartialEq)]
pub enum I2cInstance {
    I2c0 = 0,
    I2c1 = 1,
    I2c2 = 2,
    I2c3 = 3,
    I2c4 = 4,
    I2c5 = 5,
}

impl I2cInstance {
    fn base_offset(self) -> u64 {
        offsets::I2C_BASE + (self as u64 * offsets::I2C_STRIDE)
    }
}

/// Initialize I2C instance
pub fn i2c_init(instance: I2cInstance, speed_khz: u32) {
    unsafe {
        let base = instance.base_offset();

        // Disable I2C
        rp1_periph_write32(base, 0x00, 0); // CTRL

        // Set clock divider for target speed
        // I2C clock is 48 MHz on RP1
        const I2C_CLK: u32 = 48_000_000;
        let divider = I2C_CLK / (2 * speed_khz * 1000) - 1;
        rp1_periph_write32(base, 0x04, divider); // CLK_DIV

        // Enable I2C
        rp1_periph_write32(base, 0x00, 1); // CTRL: EN
    }
}

/// I2C write then read (combined transaction)
pub fn i2c_write_read(
    instance: I2cInstance,
    _addr: u8,
    write_data: &[u8],
    read_data: &mut [u8],
) -> Result<(), &'static str> {
    unsafe {
        let base = instance.base_offset();

        // Write phase
        if !write_data.is_empty() {
            rp1_periph_write32(base, 0x08, write_data.len() as u32); // TX_CNT
            for &byte in write_data {
                rp1_periph_write32(base, 0x10, byte as u32); // TX_DATA
            }
            rp1_periph_write32(base, 0x00, 0x05); // CTRL: EN | START | TX

            // Wait for completion
            let mut timeout = 100000;
            while timeout > 0 {
                let status = rp1_periph_read32(base, 0x0C); // STATUS
                if status & 0x1 != 0 {
                    break;
                } // DONE
                timeout -= 1;
            }
            if timeout == 0 {
                return Err("I2C write timeout");
            }
        }

        // Read phase
        if !read_data.is_empty() {
            rp1_periph_write32(base, 0x08, read_data.len() as u32); // TX_CNT (reused as RX_CNT)
            rp1_periph_write32(base, 0x00, 0x09); // CTRL: EN | START | RX

            let mut timeout = 100000;
            for i in 0..read_data.len() {
                while timeout > 0 {
                    let status = rp1_periph_read32(base, 0x0C); // STATUS
                    if status & 0x2 != 0 {
                        break;
                    } // RX_DATA
                    timeout -= 1;
                }
                if timeout == 0 {
                    return Err("I2C read timeout");
                }
                read_data[i] = rp1_periph_read32(base, 0x14) as u8; // RX_DATA
            }
        }

        Ok(())
    }
}

/// SPI instance
#[derive(Clone, Copy, PartialEq)]
pub enum SpiInstance {
    Spi0 = 0,
    Spi1 = 1,
    Spi2 = 2,
    Spi3 = 3,
    Spi4 = 4,
    Spi5 = 5,
}

impl SpiInstance {
    fn base_offset(self) -> u64 {
        offsets::SPI_BASE + (self as u64 * offsets::SPI_STRIDE)
    }
}

/// PWM instance
#[derive(Clone, Copy, PartialEq)]
pub enum PwmInstance {
    Pwm0 = 0,
    Pwm1 = 1,
    Pwm2 = 2,
    Pwm3 = 3,
}

impl PwmInstance {
    fn base_offset(self) -> u64 {
        offsets::PWM_BASE + (self as u64 * offsets::PWM_STRIDE)
    }
}

/// I2S instance
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum I2sInstance {
    I2s0 = 0,
    I2s1 = 1,
}

impl I2sInstance {
    pub fn base_offset(self) -> u64 {
        offsets::I2S_BASE + (self as u64 * offsets::I2S_STRIDE)
    }
}

/// Print RP1 peripheral status
pub fn print_status() {
    unsafe {
        if !RP1_INITIALIZED {
            println!("[rp1] Not initialized");
            return;
        }

        let base = RP1_VIRT_BASE.unwrap().0;

        // Read a few key registers
        let identity =
            read_volatile((base + offsets::SYSINFO + sysinfo_regs::IDENTITY) as *const u32);
        let revision =
            read_volatile((base + offsets::SYSINFO + sysinfo_regs::REVISION) as *const u32);

        println!(
            "[rp1] Identity: {:#010x}, Revision: {:#010x}",
            identity, revision
        );

        // GPIO status
        for bank in [GpioBank::Bank0, GpioBank::Bank1] {
            let bank_base = bank.base_offset();
            let out_val = read_volatile((base + bank_base + gpio_regs::OUT) as *const u32);
            let oe_val = read_volatile((base + bank_base + gpio_regs::OE) as *const u32);
            let in_val = read_volatile((base + bank_base + gpio_regs::IN_SYNC) as *const u32);
            println!(
                "[rp1] GPIO {:?}: OUT={:#010x} OE={:#010x} IN={:#010x}",
                bank, out_val, oe_val, in_val
            );
        }
    }
}
