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
const EC_SVC64: u32 = 0b010101; // SVC instruction from AArch64 (EL0 or EL1)
const EC_DATA_ABORT_LOW: u32 = 0b100100; // Data Abort from lower EL
const EC_DATA_ABORT_CUR: u32 = 0b100101; // Data Abort from current EL
const EC_INSN_ABORT_LOW: u32 = 0b100000; // Instruction Abort from lower EL
const EC_INSN_ABORT_CUR: u32 = 0b100001; // Instruction Abort from current EL
const EC_PC_ALIGN: u32 = 0b100010; // PC Alignment fault
const EC_SP_ALIGN: u32 = 0b100110; // SP Alignment fault
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
        EC_DATA_ABORT_CUR => {
            let wnr = (iss >> 6) & 1; // Write not Read
            let access_type = if wnr != 0 { "write" } else { "read" };
            println!(
                "[exception] Data Abort ({}) at ELR={:#018x} FAR={:#018x}",
                access_type, elr, far
            );
            println!(
                "[exception] ESR={:#018x} (EC={}, ISS={:#010x}) SPSR={:#018x}",
                esr, ec, iss, spsr
            );
            // Do not halt — allow continued execution for bring-up debugging.
        }
        EC_INSN_ABORT_CUR => {
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
            println!(
                "[exc] EC={:#04x} ISS={:#010x} SPSR={:#018x} FAR={:#018x} ELR={:#018x} IL={}",
                ec,
                iss,
                spsr,
                far,
                elr,
                (spsr >> 20) & 1
            );
            println!(
                "[exception] regs[0]={:#018x} regs[1]={:#018x} regs[29]={:#018x}",
                if _regs.is_null() { 0 } else { *_regs.add(0) },
                if _regs.is_null() { 0 } else { *_regs.add(1) },
                if _regs.is_null() { 0 } else { *_regs.add(29) }
            );
            // Bring-up: do not halt; the caller context resumes.
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
        EC_DATA_ABORT_LOW => {
            let wnr = (iss >> 6) & 1;
            let access_type = if wnr != 0 { "write" } else { "read" };
            println!(
                "[exception] EL0 Data Abort ({}) at ELR={:#018x} FAR={:#018x}",
                access_type, elr, far
            );
            // Route to page fault handler
            let is_write = wnr != 0;
            if !crate::sched::process::handle_page_fault(far, is_write) {
                // Could not handle - kill the process and return to kernel
                println!(
                    "[exception] Unhandled page fault, killing PID {}",
                    sched::current_pid()
                );
                sched::kill_current_and_exit(-1);
            }
        }
        EC_INSN_ABORT_LOW => {
            println!(
                "[exception] EL0 Instruction Abort at ELR={:#018x} FAR={:#018x}",
                elr, far
            );
            // This could be a page fault on instruction fetch
            if !crate::sched::process::handle_page_fault(far, false) {
                sched::kill_current_and_exit(-1);
            }
        }
        EC_PC_ALIGN | EC_SP_ALIGN => {
            println!("[exception] EL0 Alignment Fault at ELR={:#018x}", elr);
            sched::kill_current_and_exit(-1);
        }
        _ => {
            println!("[exception] EL0 exception at ELR={:#018x}", elr);
            println!(
                "[exception] ESR={:#018x} (EC={}, ISS={:#010x})",
                esr, ec, iss
            );
            println!("[exception] SPSR={:#018x} FAR={:#018x}", spsr, far);
            sched::kill_current_and_exit(-1);
        }
    }

    // Centralized cooperative exit check — runs for ALL exception types.
    // When set (by SYS_EXIT or kill_current_and_exit), modify SPSR_EL1 to
    // return to EL1 instead of EL0, and set ELR_EL1 to the saved kernel
    // return address so we resume in kernel code that launched the process.
    if crate::sched::cooperative_exit_pending() {
        crate::sched::clear_cooperative_exit();
        // S3: hard-mask IRQs for the ELR/SPSR rewrite + eret window.
        // The arch timer (once preempt is on) can otherwise land exactly
        // between the msr writes below and the handler's eret, producing
        // a PSTATE.IL-class UNDEF loop at the IRQ vector body (EC=0x00,
        // SPSR=0x3C5 — observed live). The main loop re-enables IRQs
        // after run_and_wait returns.
        unsafe {
            core::arch::asm!("msr DAIFSet, #0xF");
        }
        // S2: a background task's slice ends the same way — eret back to
        // the main loop, but keep the task alive and deliver the slice
        // marker instead of an exit code.
        let is_slice = crate::sched::slice_yield_pending();
        crate::sched::clear_slice_yield();
        let kr = crate::sched::kernel_return_address();
        unsafe {
            // S2: persist the EL0 frame BEFORE rewiring ELR/SPSR so the
            // next run_and_wait resumes the app where it yielded.
            if is_slice {
                crate::sched::save_slice_frame(crate::sched::current_pid(), regs, elr, spsr);
            }
            // Deliver the exit code (or slice marker) in x0: the resume
            // trampoline returns it to run_and_wait's caller.
            if !regs.is_null() {
                *regs.add(0) = if is_slice {
                    crate::sched::SLICE_YIELD_MARKER as u64
                } else {
                    crate::sched::coop_exit_code() as u64
                };
                // RESTORE_CONTEXT (boot.S) reloads ELR/SPSR from the
                // exception frame (offsets 256/264) and masks DAIF before
                // the eret — the cooperative return must therefore be
                // written into the FRAME (live msr writes would be
                // overwritten by the frame-backed restore).
                // EL1h (M=0b0101), all DAIF masked: 0x3C5 — kernel resume.
                // NOTE: `regs` is *mut u64 — add() counts ELEMENTS, so
                // byte offset 256 = add(32), 264 = add(33). (Using
                // add(256)/add(264) wrote 2KB past the frame — the eret
                // then loaded the ORIGINAL ELR/SPSR and returned to the
                // app after exit, executing garbage.)
                *regs.add(32) = kr; // TrapFrame.elr   (byte 256)
                *regs.add(33) = 0x3C5; // TrapFrame.spsr  (byte 264)
            }
        }
        // For slices the task stays Ready; the main loop will resume it
        // after its wake deadline. Only real exits park CURRENT_PID.
        if !is_slice {
            crate::sched::set_current_pid(0);
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

    let is_timer_irq = irq_id == 29 || (64..=67).contains(&irq_id);

    match irq_id {
        0..=15 => {
            // SGI (Software Generated Interrupt) — these are IPIs
            crate::smp::handle_ipi(irq_id);
        }
        29 => {
            // On QEMU raspi5b the bcm2712 wiring routes the EL1 physical
            // timer (CNTP) to PPI 29 — the BCM system-timer MMIO does not
            // exist there. On real hardware this is the BCM timer.
            static ARCH_TIMER_TICKS: core::sync::atomic::AtomicU64 =
                core::sync::atomic::AtomicU64::new(0);
            let n = ARCH_TIMER_TICKS.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
            if n < 5 {
                crate::println!("[irq] timer tick #{}", n);
            }
            #[cfg(feature = "qemu_raspi5b")]
            crate::arch::timer::Timer::handle_arch_timer_irq();
            #[cfg(not(feature = "qemu_raspi5b"))]
            crate::arch::timer::Timer::handle_irq();
            // Drive scheduler from timer tick
            sched::on_timer_tick();
            // Drive SMP per-core timer accounting + load balancing
            let core_id = crate::smp::current_core_id() as usize;
            crate::smp::runqueue::on_core_timer_tick(core_id);
            crate::smp::balance::on_timer_tick();
        }
        30 => {
            // Virtual timer (CNTV) — bcm2712 wiring routes it to PPI 30.
            // Used when 'preempt on virt' selects the virtual timer.
            crate::arch::timer::Timer::handle_virt_timer_irq();
            sched::on_timer_tick();
            let core_id = crate::smp::current_core_id() as usize;
            crate::smp::runqueue::on_core_timer_tick(core_id);
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

    // In cooperative QEMU mode, do NOT call schedule() from the IRQ handler.
    // schedule() does eret (noreturn) which corrupts the main loop context.
    // User processes are run directly from the shell's 'run' command instead.
    #[cfg(not(feature = "qemu_raspi5b"))]
    if is_timer_irq {
        if sched::schedule_pending() {
            sched::clear_schedule_flag();
            sched::schedule();
        }
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
