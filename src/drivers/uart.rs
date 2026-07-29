//! UART driver — PL011 for BCM2712 (Pi 5) / QEMU raspi4b/raspi5b, COM1 for x86_64

#[cfg(target_arch = "x86_64")]
use crate::arch::x86_64::cpu;

/// UART driver for both AArch64 (PL011) and x86_64 (COM1)
pub struct Uart10;

impl Uart10 {
    /// Initialize UART (AArch64: PL011, x86_64: COM1)
    pub unsafe fn init() {
        #[cfg(target_arch = "aarch64")]
        {
            aarch64_uart::init_aarch64();
        }
        #[cfg(target_arch = "x86_64")]
        {
            crate::arch::x86_64::console::init();
        }
    }

    /// Initialize UART with virtual base address (called after MMU is enabled)
    /// This updates the MMIO base to use virtual addresses
    pub unsafe fn init_virtual(uart_virt_base: u64) {
        #[cfg(target_arch = "aarch64")]
        {
            aarch64_uart::init_virtual(uart_virt_base);
        }
    }

    /// Write a single byte
    #[inline]
    pub fn putc(c: u8) {
        #[cfg(target_arch = "aarch64")]
        {
            aarch64_uart::putc_aarch64(c);
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
            aarch64_uart::getc_aarch64()
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
            aarch64_uart::has_data_aarch64()
        }
        #[cfg(target_arch = "x86_64")]
        {
            false // TODO: implement
        }
    }

    /// Non-blocking read of a single byte. Returns None if no data.
    #[inline]
    pub fn read_byte_nb() -> Option<u8> {
        if Self::has_data() {
            Some(Self::getc())
        } else {
            None
        }
    }

    /// Alias for has_data — true if UART has received data available to read.
    #[inline]
    pub fn rx_available() -> bool {
        Self::has_data()
    }

    /// Write a string
    pub fn puts(s: &str) {
        #[cfg(target_arch = "aarch64")]
        {
            aarch64_uart::puts_aarch64(s);
        }
        #[cfg(target_arch = "x86_64")]
        {
            crate::arch::x86_64::console::serial_write_str(s);
        }
    }
}

// AArch64-specific implementation
#[cfg(target_arch = "aarch64")]
mod aarch64_uart {
    // use crate::println;  // unused

    /// UART physical base address — differs between real Pi 5 and QEMU
    #[cfg(all(not(feature = "qemu"), not(feature = "qemu_raspi5b")))]
    const UART10_PHYS_BASE: u64 = 0x10_7D00_1000; // BCM2712 UART10 (real Pi 5)
    #[cfg(all(feature = "qemu", not(feature = "qemu_raspi5b")))]
    const UART10_PHYS_BASE: u64 = 0xFE_201_000; // BCM2711 PL011 (QEMU raspi4b)
    #[cfg(feature = "qemu_raspi5b")]
    const UART10_PHYS_BASE: u64 = 0x7D00_1000; // BCM2711-style UART10 (QEMU raspi5b)

    /// Virtual base address (set after MMU enable)
    pub(crate) static mut UART_VIRT_BASE: u64 = 0;
    pub(crate) static mut MMIO_USE_VIRT: bool = false;

    /// Get the current MMIO base (physical before MMU, virtual after)
    #[inline]
    fn mmio_base() -> u64 {
        unsafe {
            if MMIO_USE_VIRT && UART_VIRT_BASE != 0 {
                UART_VIRT_BASE
            } else {
                UART10_PHYS_BASE
            }
        }
    }

    // PL011 Register offsets (relative to base)
    const UART_DR_OFF: u64 = 0x000;
    const UART_RSRECR_OFF: u64 = 0x004;
    const UART_FR_OFF: u64 = 0x018;
    const UART_ILPR_OFF: u64 = 0x020;
    const UART_IBRD_OFF: u64 = 0x024;
    const UART_FBRD_OFF: u64 = 0x028;
    const UART_LCRH_OFF: u64 = 0x02C;
    const UART_CR_OFF: u64 = 0x030;
    const UART_IFLS_OFF: u64 = 0x034;
    const UART_IMSC_OFF: u64 = 0x038;
    const UART_RIS_OFF: u64 = 0x03C;
    const UART_MIS_OFF: u64 = 0x040;
    const UART_ICR_OFF: u64 = 0x044;
    const UART_DMACR_OFF: u64 = 0x048;
    const UART_PERIPHID0_OFF: u64 = 0xFE0;

    /// Get register address using current base (physical or virtual)
    #[inline]
    fn reg_addr(offset: u64) -> u64 {
        mmio_base() + offset
    }

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
    #[cfg(all(not(feature = "qemu"), not(feature = "qemu_raspi5b")))]
    const UART_CLOCK: u64 = 9_216_000;
    #[cfg(all(feature = "qemu", not(feature = "qemu_raspi5b")))]
    const UART_CLOCK: u64 = 48_000_000;
    #[cfg(feature = "qemu_raspi5b")]
    const UART_CLOCK: u64 = 48_000_000;

    /// Target baud rate
    const BAUD_RATE: u64 = 115200;

    pub(crate) unsafe fn init_aarch64() {
        // Disable UART while configuring
        mmio_write32(reg_addr(UART_CR_OFF), 0);

        // Wait for any current transmission to complete
        while mmio_read32(reg_addr(UART_FR_OFF)) & FR_BUSY != 0 {}

        // Disable all interrupts
        mmio_write32(reg_addr(UART_IMSC_OFF), 0);

        // Clear all pending interrupts
        mmio_write32(reg_addr(UART_ICR_OFF), 0x7FF);

        // Set baud rate
        let baud_divider = (UART_CLOCK / (16 * BAUD_RATE)) as u32;
        let fractional =
            (((UART_CLOCK % (16 * BAUD_RATE)) * 64 + (8 * BAUD_RATE - 1)) / (8 * BAUD_RATE)) as u32;

        mmio_write32(reg_addr(UART_IBRD_OFF), baud_divider);
        mmio_write32(reg_addr(UART_FBRD_OFF), fractional);

        // Line Control: 8 bits, no parity, 1 stop bit, FIFOs enabled
        mmio_write32(reg_addr(UART_LCRH_OFF), LCRH_WLEN_8BIT | LCRH_FEN);

        // Set FIFO interrupt levels
        mmio_write32(reg_addr(UART_IFLS_OFF), 0);

        // Enable UART, TX, RX
        mmio_write32(reg_addr(UART_CR_OFF), CR_UARTEN | CR_TXE | CR_RXE);
    }

    #[inline]
    pub(crate) fn putc_aarch64(c: u8) {
        unsafe {
            while mmio_read32(reg_addr(UART_FR_OFF)) & FR_TXFF != 0 {}
            mmio_write32(reg_addr(UART_DR_OFF), c as u32);
        }
        // On QEMU, mirror serial output to the graphical terminal screen buffer
        // so the framebuffer terminal displays live shell interaction.
        #[cfg(feature = "qemu")]
        crate::apps::terminal::mirror_serial_byte(c);
    }

    #[inline]
    pub(crate) fn getc_aarch64() -> u8 {
        unsafe {
            while mmio_read32(reg_addr(UART_FR_OFF)) & FR_RXFE != 0 {}
            mmio_read32(reg_addr(UART_DR_OFF)) as u8
        }
    }

    #[inline]
    pub(crate) fn has_data_aarch64() -> bool {
        unsafe { mmio_read32(reg_addr(UART_FR_OFF)) & FR_RXFE == 0 }
    }

    pub(crate) fn puts_aarch64(s: &str) {
        for byte in s.bytes() {
            putc_aarch64(byte);
        }
    }

    /// Initialize UART with virtual base address (called after MMU is enabled)
    pub(crate) unsafe fn init_virtual(uart_virt_base: u64) {
        UART_VIRT_BASE = uart_virt_base;
        MMIO_USE_VIRT = true;
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
