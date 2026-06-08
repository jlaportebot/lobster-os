//! PL011 UART driver for BCM2712 (Pi 5) or QEMU raspi4b
//!
//! Pi 5: UART10 at 0x10_7D00_1000, UARTCLK = 9.216 MHz
//! QEMU: PL011 at 0xFE201000 (BCM2711 mapping), UARTCLK = 48 MHz

/// UART base address — differs between real Pi 5 and QEMU raspi4b
#[cfg(not(feature = "qemu"))]
const UART10_BASE: u64 = 0x10_7D00_1000; // BCM2712 UART10
#[cfg(feature = "qemu")]
const UART10_BASE: u64 = 0xFE_201_000; // BCM2711 PL011 (QEMU raspi4b)

// PL011 Register offsets
const UART_DR: u64 = UART10_BASE + 0x000; // Data Register
const UART_RSRECR: u64 = UART10_BASE + 0x004; // Receive Status / Error Clear
const UART_FR: u64 = UART10_BASE + 0x018; // Flag Register
const UART_ILPR: u64 = UART10_BASE + 0x020; // IrDA Low-Power Counter
const UART_IBRD: u64 = UART10_BASE + 0x024; // Integer Baud Rate
const UART_FBRD: u64 = UART10_BASE + 0x028; // Fractional Baud Rate
const UART_LCRH: u64 = UART10_BASE + 0x02C; // Line Control
const UART_CR: u64 = UART10_BASE + 0x030; // Control Register
const UART_IFLS: u64 = UART10_BASE + 0x034; // Interrupt FIFO Level Select
const UART_IMSC: u64 = UART10_BASE + 0x038; // Interrupt Mask Set/Clear
const UART_RIS: u64 = UART10_BASE + 0x03C; // Raw Interrupt Status
const UART_MIS: u64 = UART10_BASE + 0x040; // Masked Interrupt Status
const UART_ICR: u64 = UART10_BASE + 0x044; // Interrupt Clear
const UART_DMACR: u64 = UART10_BASE + 0x048; // DMA Control
const UART_PERIPHID0: u64 = UART10_BASE + 0xFE0; // Peripheral ID 0

// Flag Register bits
const FR_TXFF: u32 = 1 << 5; // Transmit FIFO full
const FR_RXFE: u32 = 1 << 4; // Receive FIFO empty
const FR_BUSY: u32 = 1 << 3; // UART busy

// Control Register bits
const CR_UARTEN: u32 = 1 << 0; // UART Enable
const CR_TXE: u32 = 1 << 8; // Transmit Enable
const CR_RXE: u32 = 1 << 9; // Receive Enable
const CR_RTSEN: u32 = 1 << 14; // RTS Enable
const CR_CTSEN: u32 = 1 << 15; // CTS Enable

// Line Control Register bits
const LCRH_FEN: u32 = 1 << 4; // FIFO Enable
const LCRH_WLEN_8BIT: u32 = 0b11 << 5; // 8-bit word length
const LCRH_STP2: u32 = 1 << 3; // Two stop bits
const LCRH_PEN: u32 = 1 << 1; // Parity Enable

/// UART clock frequency
#[cfg(not(feature = "qemu"))]
const UART_CLOCK: u64 = 9_216_000; // Pi 5: 9.216 MHz
#[cfg(feature = "qemu")]
const UART_CLOCK: u64 = 48_000_000; // QEMU raspi4b: 48 MHz

/// Target baud rate
const BAUD_RATE: u64 = 115200;

pub struct Uart10;

impl Uart10 {
    /// Initialize UART10 for 115200 8N1 serial console
    pub unsafe fn init() {
        // Disable UART while configuring
        mmio_write32(UART_CR, 0);

        // Wait for any current transmission to complete
        while mmio_read32(UART_FR) & FR_BUSY != 0 {}

        // Disable all interrupts
        mmio_write32(UART_IMSC, 0);

        // Clear all pending interrupts
        mmio_write32(UART_ICR, 0x7FF);

        // Set baud rate: IBRD = UART_CLK / (16 * BAUD_RATE)
        // For 9216000 / (16 * 115200) = 5.0
        // IBRD = 5, FBRD = 0
        // But more precisely: BAUDDIV = UARTCLK / (16 * Baud)
        //   = 9216000 / (16 * 115200) = 9216000 / 1843200 = 5.0 exactly
        let baud_divider = (UART_CLOCK / (16 * BAUD_RATE)) as u32;
        let fractional =
            (((UART_CLOCK % (16 * BAUD_RATE)) * 64 + (8 * BAUD_RATE - 1)) / (8 * BAUD_RATE)) as u32;

        mmio_write32(UART_IBRD, baud_divider);
        mmio_write32(UART_FBRD, fractional);

        // Line Control: 8 bits, no parity, 1 stop bit, FIFOs enabled
        mmio_write32(UART_LCRH, LCRH_WLEN_8BIT | LCRH_FEN);

        // Set FIFO interrupt levels
        mmio_write32(UART_IFLS, 0); // RX 1/8, TX 1/8

        // Enable UART, TX, RX
        mmio_write32(UART_CR, CR_UARTEN | CR_TXE | CR_RXE);
    }

    /// Write a single byte to UART (blocking until TX FIFO has space)
    #[inline]
    pub fn putc(c: u8) {
        unsafe {
            // Wait until TX FIFO is not full
            while mmio_read32(UART_FR) & FR_TXFF != 0 {}
            mmio_write32(UART_DR, c as u32);
        }
    }

    /// Read a single byte from UART (blocking until RX FIFO has data)
    #[inline]
    pub fn getc() -> u8 {
        unsafe {
            while mmio_read32(UART_FR) & FR_RXFE != 0 {}
            mmio_read32(UART_DR) as u8
        }
    }

    /// Check if a byte is available to read (non-blocking)
    #[inline]
    pub fn has_data() -> bool {
        unsafe { mmio_read32(UART_FR) & FR_RXFE == 0 }
    }

    /// Write a string to UART
    pub fn puts(s: &str) {
        for byte in s.bytes() {
            Self::putc(byte);
        }
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
    core::ptr::write_volatile(addr as *mut u32, val);
}
