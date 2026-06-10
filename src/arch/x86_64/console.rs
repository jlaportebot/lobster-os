//! x86_64 Console — Serial port (COM1) and VGA text mode output

use crate::arch::x86_64::cpu;
use core::fmt;

/// COM1 serial port base address
const COM1: u16 = 0x3F8;

/// Initialize serial console
pub fn init() {
    unsafe {
        // Disable all interrupts
        cpu::outb(COM1 + 1, 0x00); // Disable all interrupts
        cpu::outb(COM1 + 3, 0x80); // Enable DLAB (set baud rate divisor)
        cpu::outb(COM1 + 0, 0x03); // Set divisor to 3 (lo byte) 38400 baud
        cpu::outb(COM1 + 1, 0x00); //                  (hi byte)
        cpu::outb(COM1 + 3, 0x03); // 8 bits, no parity, one stop bit
        cpu::outb(COM1 + 2, 0xC7); // Enable FIFO, clear them, with 14-byte threshold
        cpu::outb(COM1 + 4, 0x0B); // IRQs enabled, RTS/DSR set
    }
}

/// Write a string to serial port
pub fn serial_write_str(s: &str) {
    for &byte in s.as_bytes() {
        serial_putc(byte);
    }
}

/// Write a single byte to serial port
pub fn serial_putc(c: u8) {
    unsafe {
        // Wait for transmitter holding register to be empty
        while cpu::inb(COM1 + 5) & 0x20 == 0 {
            cpu::nop();
        }
        cpu::outb(COM1, c);
    }
}

/// VGA text mode buffer
const VGA_BUFFER: *mut u8 = 0xFFFF_8000_000B_8000 as *mut u8;
const VGA_WIDTH: usize = 80;
const VGA_HEIGHT: usize = 25;

static mut VGA_POS: usize = 0;

/// Write to VGA text buffer
pub fn vga_write_str(s: &str) {
    for &byte in s.as_bytes() {
        vga_putc(byte);
    }
}

fn vga_putc(c: u8) {
    unsafe {
        if c == b'\n' {
            // Newline: move to start of next line
            let row = VGA_POS / (VGA_WIDTH * 2);
            VGA_POS = (row + 1) * VGA_WIDTH * 2;
        } else if c == b'\r' {
            // Carriage return: move to start of current line
            let row = VGA_POS / (VGA_WIDTH * 2);
            VGA_POS = row * VGA_WIDTH * 2;
        } else {
            // Normal character
            let pos = VGA_POS;
            VGA_BUFFER.add(pos).write(c);
            VGA_BUFFER.add(pos + 1).write(0x0F); // White on black
            VGA_POS += 2;
        }

        // Scroll if needed
        if VGA_POS >= VGA_WIDTH * VGA_HEIGHT * 2 {
            scroll();
        }
    }
}

fn scroll() {
    unsafe {
        // Move all lines up by one
        for row in 1..VGA_HEIGHT {
            let src = VGA_BUFFER.add(row * VGA_WIDTH * 2);
            let dst = VGA_BUFFER.add((row - 1) * VGA_WIDTH * 2);
            core::ptr::copy(src, dst, VGA_WIDTH * 2);
        }
        // Clear last line
        let last = VGA_BUFFER.add((VGA_HEIGHT - 1) * VGA_WIDTH * 2);
        for i in 0..VGA_WIDTH * 2 {
            if i % 2 == 0 {
                last.add(i).write(b' ');
            } else {
                last.add(i).write(0x0F);
            }
        }
        VGA_POS = (VGA_HEIGHT - 1) * VGA_WIDTH * 2;
    }
}
