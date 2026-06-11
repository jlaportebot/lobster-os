//! UART driver — PL011 for BCM2712 (Pi 5) / QEMU raspi4b, COM1 for x86_64

#[cfg(target_arch = "x86_64")]
use crate::arch::x86_64::cpu;

/// UART driver for both AArch64 (PL011) and x86_64 (COM1)
pub struct Uart10;

impl Uart10 {
    /// Initialize UART (AArch64: PL011, x86_64: COM1)
    pub unsafe fn init() {
        #[cfg(target_arch = "aarch64")]
        {
            Self::init_aarch64();
        }
        #[cfg(target_arch = "x86_64")]
        {
            crate::arch::x86_64::console::init();
        }
    }

    /// Write a single byte
    #[inline]
    pub fn putc(c: u8) {
        #[cfg(target_arch = "aarch64")]
        {
            Self::putc_aarch64(c);
        }
        #[cfg(target_arch = "x86_64")]
        {
            crate::arch::x86_64::console::serial_putc(c);
        }
    }

    /// Read a single byte (blocking)
    #[inline]
    pub fn getc() -> u8 {
        #[cfg(target_arch = "aarch64")]
        {
            Self::getc_aarch64()
        }
        #[cfg(target_arch = "x86_64")]
        {
            // TODO: implement serial input for x86_64
            loop {
                cpu::nop();
            }
        }
    }

    /// Check if a byte is available (non-blocking)
    #[inline]
    pub fn has_data() -> bool {
        #[cfg(target_arch = "aarch64")]
        {
            Self::has_data_aarch64()
        }
        #[cfg(target_arch = "x86_64")]
        {
            false // TODO: implement
        }
    }

    /// Write a string
    pub fn puts(s: &str) {
        #[cfg(target_arch = "aarch64")]
        {
            Self::puts_aarch64(s);
        }
        #[cfg(target_arch = "x86_64")]
        {
            crate::arch::x86_64::console::serial_write_str(s);
        }
    }
}

// AArch64-specific implementation
#[cfg(target_arch = "aarch64")]
impl Uart10 {
    /// UART base address — differs between real Pi 5 and QEMU raspi4b
    #[cfg(not(feature = "qemu"))]
    const UART10_BASE: u64 = 0x10_7D00_1000; // BCM2712 UART10
    #[cfg(feature = "qemu")]
    const UART10_BASE: u64 = 0xFE_201_000; // BCM2711 PL011 (QEMU raspi4b)

    // PL011 Register offsets
    const UART_DR: u64 = Self::UART10_BASE + 0x000;
    const UART_RSRECR: u64 = Self::UART10_BASE + 0x004;
    const UART_FR: u64 = Self::UART10_BASE + 0x018;
    const UART_ILPR: u64 = Self::UART10_BASE + 0x020;
    const UART_IBRD: u64 = Self::UART10_BASE + 0x024;
    const UART_FBRD: u64 = Self::UART10_BASE + 0x028;
    const UART_LCRH: u64 = Self::UART10_BASE + 0x02C;
    const UART_CR: u64 = Self::UART10_BASE + 0x030;
    const UART_IFLS: u64 = Self::UART10_BASE + 0x034;
    const UART_IMSC: u64 = Self::UART10_BASE + 0x038;
    const UART_RIS: u64 = Self::UART10_BASE + 0x03C;
    const UART_MIS: u64 = Self::UART10_BASE + 0x040;
    const UART_ICR: u64 = Self::UART10_BASE + 0x044;
    const UART_DMACR: u64 = Self::UART10_BASE + 0x048;
    const UART_PERIPHID0: u64 = Self::UART10_BASE + 0xFE0;

    // Flag Register bits
    const FR_TXFF: u32 = 1 << 5;
    const FR_RXFE: u32 = 1 << 4;
    const FR_BUSY: u32 = 1 << 3;

    // Control Register bits
    const CR_UARTEN: u32 = 1 << 0;
    const CR_TXE: u32 = 1 << 8;
    const CR_RXE: u32 = 1 << 9;
    const CR_RTSEN: u32 = 1 << 14;
    const CR_CTSEN: u32 = 1 << 15;

    // Line Control Register bits
    const LCRH_FEN: u32 = 1 << 4;
    const LCRH_WLEN_8BIT: u32 = 0b11 << 5;
    const LCRH_STP2: u32 = 1 << 3;
    const LCRH_PEN: u32 = 1 << 1;

    /// UART clock frequency
    #[cfg(not(feature = "qemu"))]
    const UART_CLOCK: u64 = 9_216_000;
    #[cfg(feature = "qemu")]
    const UART_CLOCK: u64 = 48_000_000;

    /// Target baud rate
    const BAUD_RATE: u64 = 115200;

    unsafe fn init_aarch64() {
        // Disable UART while configuring
        Self::mmio_write32(Self::UART_CR, 0);

        // Wait for any current transmission to complete
        while Self::mmio_read32(Self::UART_FR) & Self::FR_BUSY != 0 {}

        // Disable all interrupts
        Self::mmio_write32(Self::UART_IMSC, 0);

        // Clear all pending interrupts
        Self::mmio_write32(Self::UART_ICR, 0x7FF);

        // Set baud rate
        let baud_divider = (Self::UART_CLOCK / (16 * Self::BAUD_RATE)) as u32;
        let fractional = (((Self::UART_CLOCK % (16 * Self::BAUD_RATE)) * 64
            + (8 * Self::BAUD_RATE - 1))
            / (8 * Self::BAUD_RATE)) as u32;

        Self::mmio_write32(Self::UART_IBRD, baud_divider);
        Self::mmio_write32(Self::UART_FBRD, fractional);

        // Line Control: 8 bits, no parity, 1 stop bit, FIFOs enabled
        Self::mmio_write32(Self::UART_LCRH, Self::LCRH_WLEN_8BIT | Self::LCRH_FEN);

        // Set FIFO interrupt levels
        Self::mmio_write32(Self::UART_IFLS, 0);

        // Enable UART, TX, RX
        Self::mmio_write32(Self::UART_CR, Self::CR_UARTEN | Self::CR_TXE | Self::CR_RXE);
    }

    #[inline]
    fn putc_aarch64(c: u8) {
        unsafe {
            while Self::mmio_read32(Self::UART_FR) & Self::FR_TXFF != 0 {}
            Self::mmio_write32(Self::UART_DR, c as u32);
        }
    }

    #[inline]
    fn getc_aarch64() -> u8 {
        unsafe {
            while Self::mmio_read32(Self::UART_FR) & Self::FR_RXFE != 0 {}
            Self::mmio_read32(Self::UART_DR) as u8
        }
    }

    #[inline]
    fn has_data_aarch64() -> bool {
        unsafe { Self::mmio_read32(Self::UART_FR) & Self::FR_RXFE == 0 }
    }

    fn puts_aarch64(s: &str) {
        for byte in s.bytes() {
            Self::putc_aarch64(byte);
        }
    }

    /// Read 32-bit MMIO register
    #[inline]
    unsafe fn mmio_read32(addr: u64) -> u32 {
        core::ptr::read_volatile(addr as *const u32)
    }

    /// Write 32-bit MMIO register
    #[inline]
    unsafe fn mmio_write32(addr: u64, val: u32) {
        core::ptr::write_volatile(addr as *mut u32, val)
    }
}
