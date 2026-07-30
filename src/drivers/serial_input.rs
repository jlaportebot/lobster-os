//! Interrupt-driven serial input from UART10
//!
//! Provides:
//! - Ring buffer for received characters
//! - UART10 RX interrupt enable (SPI 121)
//! - Non-blocking read interface for the shell
//! - Line editing: backspace, echo, line buffering

use crate::drivers::uart::Uart10;

/// UART10 GIC SPI number
pub const UART10_SPI: u32 = 121;

/// Ring buffer size (must be power of 2)
const RX_BUF_SIZE: usize = 512;

/// Line buffer size for shell input
const LINE_BUF_SIZE: usize = 128;

/// RX ring buffer — stores raw bytes from UART
static mut RX_RING: [u8; RX_BUF_SIZE] = [0; RX_BUF_SIZE];
static mut RX_HEAD: usize = 0; // Where to write next (ISR side)
static mut RX_TAIL: usize = 0; // Where to read next (consumer side)

/// Line buffer — completed lines ready for shell
static mut LINE_BUF: [u8; LINE_BUF_SIZE] = [0; LINE_BUF_SIZE];
static mut LINE_LEN: usize = 0;

/// Number of complete lines available
static mut LINE_COUNT: usize = 0;

/// Current input line being assembled
static mut INPUT_BUF: [u8; 256] = [0; 256];
static mut INPUT_LEN: usize = 0;

/// Echo flag — when true, received chars are echoed back
static mut ECHO_ENABLED: bool = true;

/// Initialize serial input: enable UART10 RX interrupt
#[cfg(target_arch = "aarch64")]
pub unsafe fn init() {
    // Clear ring buffer state
    RX_HEAD = 0;
    RX_TAIL = 0;
    LINE_COUNT = 0;
    LINE_LEN = 0;
    INPUT_LEN = 0;
    ECHO_ENABLED = true;

    // UART base address — QEMU uses 0x7D00_1000, real Pi 5 uses 0x10_7D00_1000
    #[cfg(all(feature = "qemu", not(feature = "qemu_raspi5b")))]
    const UART10_BASE: u64 = 0x7D00_1000;
    #[cfg(feature = "qemu_raspi5b")]
    const UART10_BASE: u64 = 0x7D00_1000;
    #[cfg(not(feature = "qemu_raspi5b"))]
    const UART10_BASE: u64 = 0x10_7D00_1000;

    const UART_IMSC: u64 = UART10_BASE + 0x038;
    const UART_ICR: u64 = UART10_BASE + 0x044;
    // Enable RX interrupt + RX timeout interrupt
    core::ptr::write_volatile(UART_IMSC as *mut u32, (1 << 4) | (1 << 6));
    // Clear any pending UART interrupts
    core::ptr::write_volatile(UART_ICR as *mut u32, 0x7FF);

    // Enable UART10 SPI in GIC
    crate::arch::gic::Gic::enable_spi(UART10_SPI);
    crate::arch::gic::Gic::set_config(UART10_SPI, 0b10); // Edge-triggered
    crate::arch::gic::Gic::set_priority(UART10_SPI, 0x60); // Higher priority than timer

    crate::println!("[serial] UART10 RX interrupt enabled (SPI {})", UART10_SPI);
}

#[cfg(target_arch = "x86_64")]
pub unsafe fn init() {}

/// Called from IRQ handler when UART10 interrupt fires
#[cfg(target_arch = "aarch64")]
pub unsafe fn handle_irq() {
    #[cfg(all(feature = "qemu", not(feature = "qemu_raspi5b")))]
    const UART10_BASE: u64 = 0x7D00_1000;
    #[cfg(feature = "qemu_raspi5b")]
    const UART10_BASE: u64 = 0x7D00_1000;
    #[cfg(not(feature = "qemu_raspi5b"))]
    const UART10_BASE: u64 = 0x10_7D00_1000;
    const UART_DR: u64 = UART10_BASE + 0x000;
    const UART_FR: u64 = UART10_BASE + 0x018;
    const UART_RIS: u64 = UART10_BASE + 0x03C;
    const UART_ICR: u64 = UART10_BASE + 0x044;

    // Read masked interrupt status
    let ris = core::ptr::read_volatile(UART_RIS as *const u32);
    let rx_pending = (ris & (1 << 4)) != 0; // RX interrupt
    let rt_pending = (ris & (1 << 6)) != 0; // RX timeout

    if !rx_pending && !rt_pending {
        return;
    }

    // Drain all available bytes from RX FIFO
    while (core::ptr::read_volatile(UART_FR as *const u32) & (1 << 4)) == 0 {
        let byte = core::ptr::read_volatile(UART_DR as *const u32) as u8;

        // Push into ring buffer
        let next_head = (RX_HEAD + 1) & (RX_BUF_SIZE - 1);
        if next_head != RX_TAIL {
            // Buffer not full
            RX_RING[RX_HEAD] = byte;
            RX_HEAD = next_head;
        }
        // If buffer is full, byte is dropped — should be extremely rare
    }

    // Clear UART interrupts
    core::ptr::write_volatile(UART_ICR as *mut u32, (1 << 4) | (1 << 6));
}

#[cfg(target_arch = "x86_64")]
pub unsafe fn handle_irq() {}

/// Process a single byte directly (no ring buffer). Used for cooperative polling.
#[cfg(target_arch = "aarch64")]
pub unsafe fn feed_byte(byte: u8) {
    process_byte(byte);
}

#[cfg(target_arch = "x86_64")]
pub unsafe fn feed_byte(_byte: u8) {}

/// Process bytes from the ring buffer into line buffer
unsafe fn process_rx_bytes() {
    while RX_TAIL != RX_HEAD {
        let byte = RX_RING[RX_TAIL];
        RX_TAIL = (RX_TAIL + 1) & (RX_BUF_SIZE - 1);

        process_byte(byte);
    }
}

/// Process a single byte through the line editor
unsafe fn process_byte(byte: u8) {
    match byte {
        // Backspace / Delete
        0x7F | 0x08 => {
            if INPUT_LEN > 0 {
                INPUT_LEN -= 1;
                if ECHO_ENABLED {
                    Uart10::putc(0x08);
                    Uart10::putc(b' ');
                    Uart10::putc(0x08);
                }
            }
        }
        // Enter / Return
        b'\r' | b'\n' => {
            if ECHO_ENABLED {
                Uart10::puts("\r\n");
            }
            if INPUT_LEN > 0 && LINE_COUNT == 0 {
                let copy_len = INPUT_LEN.min(LINE_BUF_SIZE - LINE_LEN);
                LINE_BUF[LINE_LEN..LINE_LEN + copy_len].copy_from_slice(&INPUT_BUF[..copy_len]);
                LINE_LEN += copy_len;
                LINE_COUNT = 1;
            }
            INPUT_LEN = 0;
        }
        // Ctrl+C — cancel current input
        0x03 => {
            if ECHO_ENABLED {
                Uart10::puts("^C\r\n");
            }
            INPUT_LEN = 0;
        }
        // Ctrl+U — kill entire line
        0x15 => {
            if ECHO_ENABLED {
                for _ in 0..INPUT_LEN {
                    Uart10::putc(0x08);
                    Uart10::putc(b' ');
                    Uart10::putc(0x08);
                }
            }
            INPUT_LEN = 0;
        }
        0x1B => {}
        // Printable ASCII
        0x20..=0x7E => {
            if INPUT_LEN < INPUT_BUF.len() {
                INPUT_BUF[INPUT_LEN] = byte;
                INPUT_LEN += 1;
                if ECHO_ENABLED {
                    Uart10::putc(byte);
                }
            }
        }
        // Tab — echo spaces
        b'\t' => {
            if ECHO_ENABLED {
                Uart10::puts("    ");
            }
        }
        _ => {}
    }
}

/// Check if a complete line is available (non-blocking)
pub fn has_line() -> bool {
    unsafe { LINE_COUNT > 0 }
}

/// Read a complete line into the provided buffer.
/// Returns the number of bytes copied, or 0 if no line available.
/// This is non-blocking.
pub fn read_line(buf: &mut [u8]) -> usize {
    unsafe {
        if LINE_COUNT == 0 {
            return 0;
        }

        let len = LINE_LEN.min(buf.len());
        buf[..len].copy_from_slice(&LINE_BUF[..len]);

        // Reset line buffer
        LINE_LEN = 0;
        LINE_COUNT = 0;

        len
    }
}

/// Poll for a single byte (non-blocking, no line editing).
/// Returns Some(byte) if available, None otherwise.
pub fn poll_byte() -> Option<u8> {
    unsafe {
        if RX_HEAD != RX_TAIL {
            let byte = RX_RING[RX_TAIL];
            RX_TAIL = (RX_TAIL + 1) & (RX_BUF_SIZE - 1);
            Some(byte)
        } else {
            None
        }
    }
}

/// Blocking read of a single byte (spins until available)
pub fn read_byte() -> u8 {
    loop {
        if let Some(b) = poll_byte() {
            return b;
        }
        crate::arch::cpu::wfe();
    }
}

/// Set echo mode
pub fn set_echo(enabled: bool) {
    unsafe { ECHO_ENABLED = enabled };
}

/// Get current echo mode
pub fn echo_enabled() -> bool {
    unsafe { ECHO_ENABLED }
}

/// Get the number of bytes in the RX ring buffer
pub fn rx_available() -> usize {
    unsafe {
        if RX_HEAD >= RX_TAIL {
            RX_HEAD - RX_TAIL
        } else {
            RX_BUF_SIZE - RX_TAIL + RX_HEAD
        }
    }
}

// x86_64 stub implementations
#[cfg(target_arch = "x86_64")]
mod x86_64_stubs {
    use super::*;

    pub unsafe fn init() {}

    pub fn has_line() -> bool {
        false
    }

    pub fn read_line(_buf: &mut [u8]) -> usize {
        0
    }

    pub fn poll_byte() -> Option<u8> {
        None
    }

    pub fn read_byte() -> u8 {
        0
    }

    pub fn set_echo(_enabled: bool) {}

    pub fn echo_enabled() -> bool {
        false
    }

    pub fn rx_available() -> usize {
        0
    }
}

#[cfg(target_arch = "x86_64")]
pub use x86_64_stubs::*;
