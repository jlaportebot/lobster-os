//! Exception handlers — called from assembly vector table in boot.S
//!
//! Handles:
//! - Synchronous exceptions at EL1 (kernel faults)
//! - SVC syscalls from EL0 (user → kernel)
//! - IRQs (hardware interrupts + timer-driven scheduling)
//! - FIQs (fast interrupts)
//! - SErrors (system errors)

use crate::arch::gic;
use crate::println;
use crate::sched;

/// ESR_EL1.EC values (Exception Class)
const EC_SVC64: u32 = 0b111101; // SVC instruction from AArch64
const EC_DATA_ABORT: u32 = 0b100101;
const EC_INSN_ABORT: u32 = 0b100000;
const EC_PC_ALIGN: u32 = 0b100001;
const EC_SP_ALIGN: u32 = 0b100010;
#[allow(dead_code)]
const EC_TRAP_FP: u32 = 0b111000;

/// Synchronous exception handler (EL1h) — kernel-space fault
#[no_mangle]
pub unsafe extern "C" fn handle_sync_exception(
    spsr: u64,
    elr: u64,
    esr: u64,
    far: u64,
    _regs: *mut u64,
) {
    let ec = ((esr >> 26) & 0x3F) as u32;
    let iss = (esr & 0x1FFFFFF) as u32;

    match ec {
        EC_SVC64 => {
            let svc_num = iss & 0xFFFF;
            println!(
                "[exception] SVC #{} from EL1 (unexpected in kernel)",
                svc_num
            );
        }
        EC_DATA_ABORT => {
            let wnr = (iss >> 6) & 1; // Write not Read
            let access_type = if wnr != 0 { "write" } else { "read" };
            println!(
                "[exception] Data Abort ({}) at ELR={:#018x} FAR={:#018x}",
                access_type, elr, far
            );
            println!(
                "[exception] ESR={:#018x} (EC={}, ISS={:#010x})",
                esr, ec, iss
            );
            loop {
                crate::arch::cpu::wfe();
            }
        }
        EC_INSN_ABORT => {
            println!(
                "[exception] Instruction Abort at ELR={:#018x} FAR={:#018x}",
                elr, far
            );
            loop {
                crate::arch::cpu::wfe();
            }
        }
        EC_PC_ALIGN => {
            println!("[exception] PC Alignment Fault at ELR={:#018x}", elr);
            loop {
                crate::arch::cpu::wfe();
            }
        }
        EC_SP_ALIGN => {
            println!("[exception] SP Alignment Fault at ELR={:#018x}", elr);
            loop {
                crate::arch::cpu::wfe();
            }
        }
        _ => {
            println!("[exception] Synchronous exception at ELR={:#018x}", elr);
            println!(
                "[exception] ESR={:#018x} (EC={}, ISS={:#010x})",
                esr, ec, iss
            );
            println!("[exception] SPSR={:#018x} FAR={:#018x}", spsr, far);
            loop {
                crate::arch::cpu::wfe();
            }
        }
    }
}

/// Synchronous exception from EL0 — dispatches syscalls
#[no_mangle]
pub unsafe extern "C" fn handle_sync_exception_el0(
    spsr: u64,
    elr: u64,
    esr: u64,
    far: u64,
    regs: *mut u64,
) {
    let ec = ((esr >> 26) & 0x3F) as u32;
    let iss = (esr & 0x1FFFFFF) as u32;

    match ec {
        EC_SVC64 => {
            // SVC from EL0 — this is a syscall
            let svc_num = iss & 0xFFFF;

            // Extract syscall arguments from saved registers
            // Register save order in boot.S: x0-x30 on stack
            // x0=regs[0], x1=regs[1], ..., x8=regs[8], etc.
            let syscall_num = if !regs.is_null() {
                *regs.add(8) // x8 = syscall number
            } else {
                svc_num as u64
            };

            let mut args = [0u64; 6];
            if !regs.is_null() {
                args[0] = *regs.add(0); // x0
                args[1] = *regs.add(1); // x1
                args[2] = *regs.add(2); // x2
                args[3] = *regs.add(3); // x3
                args[4] = *regs.add(4); // x4
                args[5] = *regs.add(5); // x5
            }

            // Check for and deliver pending signals BEFORE running the syscall
            // This ensures signals interrupt syscalls properly (EINTR)
            // SAFETY: `regs` is a valid pointer to the current trap frame on the exception stack
            if unsafe {
                crate::sched::signal::deliver_pending_signals(regs as *mut crate::sched::TrapFrame)
            } {
                // Signal was delivered, ELR/SPSR/regs have been modified
                // Return to the signal handler instead of the original instruction
            } else {
                // No pending signals, dispatch the syscall
                crate::sched::syscall::set_current_trap_frame(regs as *mut crate::sched::TrapFrame);
                let ret = crate::sched::syscall::dispatch(syscall_num, &args);

                // Write return value back to x0 in the saved frame
                if !regs.is_null() {
                    *regs.add(0) = ret;
                }

                // Clear the trap frame pointer
                crate::sched::syscall::set_current_trap_frame(core::ptr::null_mut());
            }
        }
        EC_DATA_ABORT => {
            let wnr = (iss >> 6) & 1;
            let access_type = if wnr != 0 { "write" } else { "read" };
            println!(
                "[exception] EL0 Data Abort ({}) at ELR={:#018x} FAR={:#018x}",
                access_type, elr, far
            );
            println!(
                "[exception] ESR={:#018x} (EC={}, ISS={:#010x})",
                esr, ec, iss
            );
            // Route to page fault handler
            let is_write = wnr != 0;
            if !crate::sched::process::handle_page_fault(far, is_write) {
                // Could not handle - kill the process
                println!(
                    "[exception] Unhandled page fault, killing PID {}",
                    sched::current_pid()
                );
                sched::kill(sched::current_pid());
            }
        }
        EC_INSN_ABORT => {
            println!(
                "[exception] EL0 Instruction Abort at ELR={:#018x} FAR={:#018x}",
                elr, far
            );
            // This could be a page fault on instruction fetch
            if !crate::sched::process::handle_page_fault(far, false) {
                sched::kill(sched::current_pid());
            }
        }
        EC_PC_ALIGN | EC_SP_ALIGN => {
            println!("[exception] EL0 Alignment Fault at ELR={:#018x}", elr);
            sched::kill(sched::current_pid());
        }
        _ => {
            println!("[exception] EL0 exception at ELR={:#018x}", elr);
            println!(
                "[exception] ESR={:#018x} (EC={}, ISS={:#010x})",
                esr, ec, iss
            );
            println!("[exception] SPSR={:#018x} FAR={:#018x}", spsr, far);
            sched::kill(sched::current_pid());
        }
    }
}

/// IRQ handler — hardware interrupts + timer-driven scheduling
#[no_mangle]
pub unsafe extern "C" fn handle_irq(_spsr: u64, _elr: u64, _esr: u64, _far: u64, _regs: *mut u64) {
    let irq_id = gic::Gic::acknowledge();

    if irq_id >= 1020 {
        return;
    }

    match irq_id {
        0..=15 => {
            // SGI (Software Generated Interrupt) — these are IPIs
            crate::smp::handle_ipi(irq_id);
        }
        29 => {
            crate::arch::timer::Timer::handle_irq();
            // Drive scheduler from timer tick
            sched::on_timer_tick();
            // Drive SMP per-core timer accounting + load balancing
            let core_id = crate::smp::current_core_id() as usize;
            crate::smp::runqueue::on_core_timer_tick(core_id);
            crate::smp::balance::on_timer_tick();
        }
        121 => {
            // UART10 RX interrupt
            crate::drivers::serial_input::handle_irq();
        }
        64..=67 => {
            crate::arch::timer::Timer::handle_bcm_timer_irq(irq_id);
            // Also drive scheduler from BCM timer
            sched::on_timer_tick();
            // Drive SMP per-core timer accounting + load balancing
            let core_id = crate::smp::current_core_id() as usize;
            crate::smp::runqueue::on_core_timer_tick(core_id);
            crate::smp::balance::on_timer_tick();
        }
        274 => {
            // SDIO2 interrupt (WiFi)
            crate::drivers::sdio::handle_interrupt();
            crate::drivers::wifi::handle_interrupt();
        }
        _ => {
            println!("[irq] Unhandled IRQ {}", irq_id);
        }
    }

    // Check if a context switch is pending
    if sched::schedule_pending() {
        sched::clear_schedule_flag();
        // The actual context switch happens on exception return
        // In a full implementation, we'd modify the return frame here
        sched::schedule();
    }

    gic::Gic::end_of_interrupt(irq_id);
}

/// FIQ handler
#[no_mangle]
pub unsafe extern "C" fn handle_fiq(_spsr: u64, _elr: u64, _esr: u64, _far: u64, _regs: *mut u64) {
    println!("[fiq] Unexpected FIQ!");
}

/// SError handler
#[no_mangle]
pub unsafe extern "C" fn handle_serror(_spsr: u64, elr: u64, esr: u64, _far: u64, _regs: *mut u64) {
    println!(
        "[serror] System Error at ELR={:#018x} ESR={:#018x}",
        elr, esr
    );
    loop {
        crate::arch::cpu::wfe();
    }
}
