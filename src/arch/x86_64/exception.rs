//! x86_64 Exception handling — IDT, fault handlers, syscalls
//!
//! This module handles:
//! - IDT (Interrupt Descriptor Table) setup
//! - Exception handlers (page fault, GP fault, etc.)
//! - Hardware interrupt handlers (IRQs via LAPIC)
//! - Syscall entry (syscall/sysret)

use crate::arch::x86_64::cpu;
use crate::println;
use crate::sched;
use core::arch::global_asm;

/// IDT entry (16 bytes for 64-bit interrupt gate)
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct IdtEntry {
    offset_low: u16,  // Offset bits 0-15
    selector: u16,    // Code segment selector
    ist: u8,          // Interrupt Stack Table index
    type_attr: u8,    // Gate type, DPL, present
    offset_mid: u16,  // Offset bits 16-31
    offset_high: u32, // Offset bits 32-63
    reserved: u32,    // Reserved
}

impl IdtEntry {
    /// Create a new interrupt gate entry
    pub const fn new(handler: u64, selector: u16, dpl: u8, ist: u8) -> Self {
        let type_attr = 0x80 | (dpl & 0x3) << 5 | 0xE; // Present, DPL, 64-bit interrupt gate
        Self {
            offset_low: handler as u16,
            selector,
            ist,
            type_attr,
            offset_mid: (handler >> 16) as u16,
            offset_high: (handler >> 32) as u32,
            reserved: 0,
        }
    }

    /// Create an empty entry
    pub const fn empty() -> Self {
        Self {
            offset_low: 0,
            selector: 0,
            ist: 0,
            type_attr: 0,
            offset_mid: 0,
            offset_high: 0,
            reserved: 0,
        }
    }
}

/// IDT with 256 entries
pub struct Idt {
    entries: [IdtEntry; 256],
}

impl Idt {
    pub const fn new() -> Self {
        Self {
            entries: [IdtEntry::empty(); 256],
        }
    }

    pub fn set_handler(&mut self, vector: u8, handler: u64, dpl: u8, ist: u8) {
        self.entries[vector as usize] = IdtEntry::new(handler, 0x08, dpl, ist); // 0x08 = kernel code segment
    }

    pub fn load(&self) {
        let ptr = IdtPtr {
            limit: (core::mem::size_of::<IdtEntry>() * 256 - 1) as u16,
            base: self.entries.as_ptr() as u64,
        };
        unsafe {
            core::arch::asm!("lidt [{}]", in(reg) &ptr, options(nomem, nostack, preserves_flags));
        }
    }
}

#[repr(C, packed)]
struct IdtPtr {
    limit: u16,
    base: u64,
}

/// Exception vectors (x86_64 exceptions)
pub mod vector {
    pub const DIVIDE_ERROR: u8 = 0;
    pub const DEBUG: u8 = 1;
    pub const NMI: u8 = 2;
    pub const BREAKPOINT: u8 = 3;
    pub const OVERFLOW: u8 = 4;
    pub const BOUND_RANGE: u8 = 5;
    pub const INVALID_OPCODE: u8 = 6;
    pub const DEVICE_NOT_AVAILABLE: u8 = 7;
    pub const DOUBLE_FAULT: u8 = 8;
    pub const COPROCESSOR_SEGMENT: u8 = 9;
    pub const INVALID_TSS: u8 = 10;
    pub const SEGMENT_NOT_PRESENT: u8 = 11;
    pub const STACK_SEGMENT_FAULT: u8 = 12;
    pub const GENERAL_PROTECTION: u8 = 13;
    pub const PAGE_FAULT: u8 = 14;
    pub const RESERVED: u8 = 15;
    pub const X87_FPU: u8 = 16;
    pub const ALIGNMENT_CHECK: u8 = 17;
    pub const MACHINE_CHECK: u8 = 18;
    pub const SIMD: u8 = 19;
    pub const VIRTUALIZATION: u8 = 20;
    pub const CONTROL_PROTECTION: u8 = 21;
    // 22-31 reserved
    // 32-255: hardware interrupts (IRQs) + syscalls
    pub const SYSCALL: u8 = 0x80; // Linux-style syscall vector
}

/// IST (Interrupt Stack Table) indices
pub mod ist {
    pub const NORMAL: u8 = 0;
    pub const DOUBLE_FAULT: u8 = 1;
    pub const NMI: u8 = 2;
    pub const TIMER: u8 = 3;
    pub const KEYBOARD: u8 = 4;
}

/// Global IDT instance
static mut IDT: Idt = Idt::new();

/// Initialize the IDT
pub unsafe fn init_idt() {
    let idt = &mut IDT;

    // Exception handlers (IST=0 for most, IST=1 for double fault)
    idt.set_handler(
        vector::DIVIDE_ERROR,
        handle_divide_error as u64,
        0,
        ist::NORMAL,
    );
    idt.set_handler(vector::DEBUG, handle_debug as u64, 0, ist::NORMAL);
    idt.set_handler(vector::NMI, handle_nmi as u64, 0, ist::NMI);
    idt.set_handler(vector::BREAKPOINT, handle_breakpoint as u64, 3, ist::NORMAL); // DPL=3 for user breakpoints
    idt.set_handler(vector::OVERFLOW, handle_overflow as u64, 0, ist::NORMAL);
    idt.set_handler(
        vector::BOUND_RANGE,
        handle_bound_range as u64,
        0,
        ist::NORMAL,
    );
    idt.set_handler(
        vector::INVALID_OPCODE,
        handle_invalid_opcode as u64,
        0,
        ist::NORMAL,
    );
    idt.set_handler(
        vector::DEVICE_NOT_AVAILABLE,
        handle_device_not_available as u64,
        0,
        ist::NORMAL,
    );
    idt.set_handler(
        vector::DOUBLE_FAULT,
        handle_double_fault as u64,
        0,
        ist::DOUBLE_FAULT,
    );
    idt.set_handler(
        vector::INVALID_TSS,
        handle_invalid_tss as u64,
        0,
        ist::NORMAL,
    );
    idt.set_handler(
        vector::SEGMENT_NOT_PRESENT,
        handle_segment_not_present as u64,
        0,
        ist::NORMAL,
    );
    idt.set_handler(
        vector::STACK_SEGMENT_FAULT,
        handle_stack_segment_fault as u64,
        0,
        ist::NORMAL,
    );
    idt.set_handler(
        vector::GENERAL_PROTECTION,
        handle_general_protection as u64,
        0,
        ist::NORMAL,
    );
    idt.set_handler(vector::PAGE_FAULT, handle_page_fault as u64, 0, ist::NORMAL);
    idt.set_handler(vector::X87_FPU, handle_x87_fpu as u64, 0, ist::NORMAL);
    idt.set_handler(
        vector::ALIGNMENT_CHECK,
        handle_alignment_check as u64,
        0,
        ist::NORMAL,
    );
    idt.set_handler(
        vector::MACHINE_CHECK,
        handle_machine_check as u64,
        0,
        ist::NORMAL,
    );
    idt.set_handler(vector::SIMD, handle_simd as u64, 0, ist::NORMAL);
    idt.set_handler(
        vector::VIRTUALIZATION,
        handle_virtualization as u64,
        0,
        ist::NORMAL,
    );
    idt.set_handler(
        vector::CONTROL_PROTECTION,
        handle_control_protection as u64,
        0,
        ist::NORMAL,
    );

    // Syscall entry (DPL=3 so userspace can call it)
    idt.set_handler(vector::SYSCALL, syscall_entry as u64, 3, ist::NORMAL);

    // Hardware IRQs will be set up by LAPIC initialization
    // Vectors 0x20-0x2F for IRQ0-15 (legacy PIC remapped)
    // Vectors 0x30+ for LAPIC/MSI interrupts

    idt.load();
}

/// Get a mutable reference to the IDT for runtime modifications (e.g., adding IRQ handlers)
pub fn idt_mut() -> &'static mut Idt {
    unsafe { &mut IDT }
}

// ============================================================
// Exception handlers (called from assembly stubs)
// ============================================================

/// Saved register state on exception entry
#[repr(C)]
pub struct ExceptionFrame {
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub r11: u64,
    pub r10: u64,
    pub r9: u64,
    pub r8: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub rbp: u64,
    pub rbx: u64,
    pub rdx: u64,
    pub rcx: u64,
    pub rax: u64,
    pub vector: u64,     // Pushed by assembly stub
    pub error_code: u64, // Pushed by CPU (if applicable)
    pub rip: u64,        // Return instruction pointer
    pub cs: u64,         // Code segment
    pub rflags: u64,     // RFLAGS
    pub rsp: u64,        // Stack pointer (for privilege change)
    pub ss: u64,         // Stack segment (for privilege change)
}

// Assembly stubs that save registers and call Rust handlers
global_asm!(include_str!("exception.S"));

// Rust handlers called from assembly

#[no_mangle]
pub unsafe extern "C" fn handle_divide_error(frame: *mut ExceptionFrame) {
    println!("[exception] Divide error at RIP={:#x}", (*frame).rip);
    loop {
        cpu::wfe();
    }
}

#[no_mangle]
pub unsafe extern "C" fn handle_debug(frame: *mut ExceptionFrame) {
    println!("[exception] Debug exception at RIP={:#x}", (*frame).rip);
    loop {
        cpu::wfe();
    }
}

#[no_mangle]
pub unsafe extern "C" fn handle_nmi(frame: *mut ExceptionFrame) {
    println!("[exception] NMI at RIP={:#x}", (*frame).rip);
    loop {
        cpu::wfe();
    }
}

#[no_mangle]
pub unsafe extern "C" fn handle_breakpoint(frame: *mut ExceptionFrame) {
    println!("[exception] Breakpoint at RIP={:#x}", (*frame).rip);
    // Continue execution (skip INT3)
    (*frame).rip += 1;
}

#[no_mangle]
pub unsafe extern "C" fn handle_overflow(frame: *mut ExceptionFrame) {
    println!("[exception] Overflow at RIP={:#x}", (*frame).rip);
    loop {
        cpu::wfe();
    }
}

#[no_mangle]
pub unsafe extern "C" fn handle_bound_range(frame: *mut ExceptionFrame) {
    println!(
        "[exception] Bound range exceeded at RIP={:#x}",
        (*frame).rip
    );
    loop {
        cpu::wfe();
    }
}

#[no_mangle]
pub unsafe extern "C" fn handle_invalid_opcode(frame: *mut ExceptionFrame) {
    println!("[exception] Invalid opcode at RIP={:#x}", (*frame).rip);
    loop {
        cpu::wfe();
    }
}

#[no_mangle]
pub unsafe extern "C" fn handle_device_not_available(frame: *mut ExceptionFrame) {
    println!(
        "[exception] Device not available (FPU/SSE) at RIP={:#x}",
        (*frame).rip
    );
    // This happens when using FPU/SSE without enabling it
    // Enable it and retry
    enable_fpu();
    // The instruction will be re-executed after iret
}

#[no_mangle]
pub unsafe extern "C" fn handle_double_fault(frame: *mut ExceptionFrame) {
    println!(
        "[exception] DOUBLE FAULT at RIP={:#x}, error_code={:#x}",
        (*frame).rip,
        (*frame).error_code
    );
    loop {
        cpu::wfe();
    }
}

#[no_mangle]
pub unsafe extern "C" fn handle_invalid_tss(frame: *mut ExceptionFrame) {
    println!(
        "[exception] Invalid TSS at RIP={:#x}, error_code={:#x}",
        (*frame).rip,
        (*frame).error_code
    );
    loop {
        cpu::wfe();
    }
}

#[no_mangle]
pub unsafe extern "C" fn handle_segment_not_present(frame: *mut ExceptionFrame) {
    println!(
        "[exception] Segment not present at RIP={:#x}, error_code={:#x}",
        (*frame).rip,
        (*frame).error_code
    );
    loop {
        cpu::wfe();
    }
}

#[no_mangle]
pub unsafe extern "C" fn handle_stack_segment_fault(frame: *mut ExceptionFrame) {
    println!(
        "[exception] Stack segment fault at RIP={:#x}, error_code={:#x}",
        (*frame).rip,
        (*frame).error_code
    );
    loop {
        cpu::wfe();
    }
}

#[no_mangle]
pub unsafe extern "C" fn handle_general_protection(frame: *mut ExceptionFrame) {
    println!(
        "[exception] General protection fault at RIP={:#x}, error_code={:#x}",
        (*frame).rip,
        (*frame).error_code
    );
    loop {
        cpu::wfe();
    }
}

#[no_mangle]
pub unsafe extern "C" fn handle_page_fault(frame: *mut ExceptionFrame) {
    let error_code = (*frame).error_code;
    let cr2: u64;
    unsafe {
        core::arch::asm!("mov {}, cr2", out(reg) cr2, options(nomem, nostack, preserves_flags))
    };

    let present = error_code & 1 != 0;
    let write = error_code & 2 != 0;
    let user = error_code & 4 != 0;
    let reserved = error_code & 8 != 0;
    let instruction_fetch = error_code & 16 != 0;

    println!(
        "[exception] Page fault at RIP={:#x} CR2={:#x} (P={} W={} U={} R={} I={})",
        (*frame).rip,
        cr2,
        present,
        write,
        user,
        reserved,
        instruction_fetch
    );

    // Try to handle via page fault handler in memory management
    let is_write = write;
    if !crate::sched::process::handle_page_fault(cr2, is_write) {
        println!("[exception] Unhandled page fault, killing current process");
        sched::kill(sched::current_pid());
    }
}

#[no_mangle]
pub unsafe extern "C" fn handle_x87_fpu(frame: *mut ExceptionFrame) {
    println!("[exception] x87 FPU exception at RIP={:#x}", (*frame).rip);
    loop {
        cpu::wfe();
    }
}

#[no_mangle]
pub unsafe extern "C" fn handle_alignment_check(frame: *mut ExceptionFrame) {
    println!(
        "[exception] Alignment check at RIP={:#x}, error_code={:#x}",
        (*frame).rip,
        (*frame).error_code
    );
    loop {
        cpu::wfe();
    }
}

#[no_mangle]
pub unsafe extern "C" fn handle_machine_check(frame: *mut ExceptionFrame) {
    println!("[exception] Machine check at RIP={:#x}", (*frame).rip);
    loop {
        cpu::wfe();
    }
}

#[no_mangle]
pub unsafe extern "C" fn handle_simd(frame: *mut ExceptionFrame) {
    println!("[exception] SIMD exception at RIP={:#x}", (*frame).rip);
    loop {
        cpu::wfe();
    }
}

#[no_mangle]
pub unsafe extern "C" fn handle_virtualization(frame: *mut ExceptionFrame) {
    println!(
        "[exception] Virtualization exception at RIP={:#x}",
        (*frame).rip
    );
    loop {
        cpu::wfe();
    }
}

#[no_mangle]
pub unsafe extern "C" fn handle_control_protection(frame: *mut ExceptionFrame) {
    println!(
        "[exception] Control protection exception at RIP={:#x}, error_code={:#x}",
        (*frame).rip,
        (*frame).error_code
    );
    loop {
        cpu::wfe();
    }
}

/// Syscall handler entry point (called from assembly via INT 0x80 or SYSCALL)
#[no_mangle]
pub unsafe extern "C" fn handle_syscall(frame: *mut ExceptionFrame) {
    // Syscall number in RAX, args in RDI, RSI, RDX, R10, R8, R9
    let syscall_num = (*frame).rax;
    let args = [
        (*frame).rdi,
        (*frame).rsi,
        (*frame).rdx,
        (*frame).r10,
        (*frame).r8,
        (*frame).r9,
    ];

    let ret = crate::sched::syscall::dispatch(syscall_num, &args);

    // Return value in RAX
    (*frame).rax = ret;
}

/// Assembly entry for syscall (INT 0x80)
#[no_mangle]
pub unsafe extern "C" fn syscall_entry(frame: *mut ExceptionFrame) {
    handle_syscall(frame);
}

/// Enable FPU/SSE for the current task
pub fn enable_fpu() {
    unsafe {
        let mut cr0: u64;
        core::arch::asm!("mov {}, cr0", out(reg) cr0, options(nomem, nostack, preserves_flags));
        cr0 &= !(1 << 2); // Clear EM (bit 2)
        cr0 |= 1 << 1; // Set MP (bit 1)
        core::arch::asm!("mov cr0, {}", in(reg) cr0, options(nomem, nostack, preserves_flags));

        let mut cr4: u64;
        core::arch::asm!("mov {}, cr4", out(reg) cr4, options(nomem, nostack, preserves_flags));
        cr4 |= (1 << 9) | (1 << 10); // OSFXSR | OSXMMEXCPT
        core::arch::asm!("mov cr4, {}", in(reg) cr4, options(nomem, nostack, preserves_flags));
    }
}

/// Initialize exception handling (IDT, etc.)
pub unsafe fn init() {
    init_idt();
    enable_fpu();
    println!("[x86_64] Exception handling initialized (IDT + FPU/SSE)");
}
