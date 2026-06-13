//! Signal handling support for user processes
//!
//! Provides:
//! - Signal types (SIGSEGV, SIGKILL, SIGTERM, etc.)
//! - Signal actions (sigaction)
//! - Signal masks and pending sets
//! - kill(), sigaction(), sigprocmask() syscalls
//!
//! Signal fields are now stored directly in the Task struct:
//! - pending_signals: u64 (bitmask of pending signals)
//! - signal_mask: u64 (current signal mask)
//! - signal_handlers: [u64; 64] (per-signal handler, SIG_DFL=0 or SIG_IGN=1 or user addr)

use crate::println;
use crate::sched::{self, find_task_index, Task, TrapFrame, TASK_TABLE};

// ── Signal action constants ──
pub const SIG_DFL: u64 = 0; // Default action
pub const SIG_IGN: u64 = 1; // Ignore signal

// ── Standard POSIX signals (1-31) ──
pub const SIGHUP: u64 = 1;
pub const SIGINT: u64 = 2;
pub const SIGQUIT: u64 = 3;
pub const SIGILL: u64 = 4;
pub const SIGTRAP: u64 = 5;
pub const SIGABRT: u64 = 6;
pub const SIGBUS: u64 = 7;
pub const SIGFPE: u64 = 8;
pub const SIGKILL: u64 = 9;
pub const SIGUSR1: u64 = 10;
pub const SIGSEGV: u64 = 11;
pub const SIGUSR2: u64 = 12;
pub const SIGPIPE: u64 = 13;
pub const SIGALRM: u64 = 14;
pub const SIGTERM: u64 = 15;
pub const SIGCHLD: u64 = 17;
pub const SIGCONT: u64 = 18;
pub const SIGSTOP: u64 = 19;
pub const SIGTSTP: u64 = 20;
pub const SIGTTIN: u64 = 21;
pub const SIGTTOU: u64 = 22;

// ── Realtime signals (32-63) ──
pub const SIGRTMIN: u64 = 32;
pub const SIGRTMAX: u64 = 63;

/// Signal action flags
pub const SA_NOCLDSTOP: u64 = 1;
pub const SA_NOCLDWAIT: u64 = 2;
pub const SA_RESTART: u64 = 4;
pub const SA_SIGINFO: u64 = 8;

/// sigaction structure (for syscall interface)
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SigAction {
    pub handler: u64,
    pub sa_flags: u64,
    pub sa_mask: u64,
}

impl SigAction {
    pub const fn new() -> Self {
        Self {
            handler: SIG_DFL,
            sa_flags: 0,
            sa_mask: 0,
        }
    }
}

impl Default for SigAction {
    fn default() -> Self {
        Self::new()
    }
}

/// Maximum standard signal number
const MAX_STD_SIG: u64 = 31;
/// Maximum signal number including realtime
const MAX_SIG: u64 = 63;

/// Default action for a signal
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SignalDefault {
    Terminate,
    CoreDump,
    Ignore,
    Stop,
    Continue,
}

/// Get the default action for a signal
pub fn default_action(sig: u64) -> SignalDefault {
    match sig {
        SIGKILL => SignalDefault::Terminate,
        SIGTERM => SignalDefault::Terminate,
        SIGINT => SignalDefault::Terminate,
        SIGHUP => SignalDefault::Terminate,
        SIGQUIT => SignalDefault::CoreDump,
        SIGILL => SignalDefault::CoreDump,
        SIGABRT => SignalDefault::CoreDump,
        SIGBUS => SignalDefault::CoreDump,
        SIGFPE => SignalDefault::CoreDump,
        SIGSEGV => SignalDefault::CoreDump,
        SIGPIPE => SignalDefault::Terminate,
        SIGALRM => SignalDefault::Terminate,
        SIGUSR1 => SignalDefault::Terminate,
        SIGUSR2 => SignalDefault::Terminate,
        SIGCHLD => SignalDefault::Ignore,
        SIGSTOP => SignalDefault::Stop,
        SIGTSTP => SignalDefault::Stop,
        SIGTTIN => SignalDefault::Stop,
        SIGTTOU => SignalDefault::Stop,
        SIGCONT => SignalDefault::Continue,
        SIGTRAP => SignalDefault::CoreDump,
        _ if sig >= SIGRTMIN => SignalDefault::Terminate,
        _ => SignalDefault::Terminate,
    }
}

/// Send a signal to a process
/// Returns 0 on success, negative errno on failure
pub fn kill(pid: u64, sig: u64) -> i64 {
    if sig == 0 {
        // Just check if process exists
        return if find_task_index(pid).is_some() {
            0
        } else {
            -3
        }; // -ESRCH
    }

    // Validate signal number
    if sig > MAX_SIG {
        return -22; // -EINVAL
    }

    // Find target task
    let task_idx = match find_task_index(pid) {
        idx @ Some(_) => idx,
        None => {
            println!("[signal] kill: PID {} not found", pid);
            return -3; // -ESRCH
        }
    };

    // Set pending signal bit directly in Task
    unsafe {
        if let Some(ref mut task) = TASK_TABLE[task_idx.unwrap()] {
            let bit = if sig <= 31 { sig - 1 } else { sig - 1 };
            task.pending_signals |= 1 << bit;

            // Wake the task if blocked (interruptible sleep)
            if task.state == crate::sched::TaskState::Blocked {
                task.state = crate::sched::TaskState::Ready;
            }
        }
    }

    println!("[signal] kill: sent signal {} to PID {}", sig, pid);
    0
}

/// Set signal action for the current process
pub fn sigaction(sig: u64, act: Option<&SigAction>, oldact: Option<&mut SigAction>) -> i64 {
    if sig == 0 || sig > MAX_SIG {
        return -22; // -EINVAL
    }

    // Cannot change SIGKILL or SIGSTOP actions
    if sig == SIGKILL || sig == SIGSTOP {
        return -22; // -EINVAL
    }

    let task_idx = match find_task_index(sched::current_pid()) {
        Some(idx) => idx,
        None => return -3, // -ESRCH
    };

    // Save old action
    if let Some(old) = oldact {
        unsafe {
            if let Some(ref task) = TASK_TABLE[task_idx] {
                old.handler = task.signal_handlers[sig as usize];
                old.sa_flags = 0;
                old.sa_mask = task.signal_mask;
            }
        }
    }

    // Set new action
    if let Some(new_act) = act {
        unsafe {
            if let Some(ref mut task) = TASK_TABLE[task_idx] {
                task.signal_handlers[sig as usize] = new_act.handler;
            }
        }
        if new_act.handler == SIG_DFL {
            println!("[signal] sigaction: signal {} → default", sig);
        } else if new_act.handler == SIG_IGN {
            println!("[signal] sigaction: signal {} → ignore", sig);
        } else {
            println!(
                "[signal] sigaction: signal {} → handler {:#x}",
                sig, new_act.handler
            );
        }
    }

    0
}

/// Set signal mask (block/unblock signals)
pub fn sigprocmask(how: u64, set: Option<u64>, oldset: Option<&mut u64>) -> i64 {
    const SIG_BLOCK: u64 = 0;
    const SIG_UNBLOCK: u64 = 1;
    const SIG_SETMASK: u64 = 2;

    let task_idx = match find_task_index(sched::current_pid()) {
        Some(idx) => idx,
        None => return -3,
    };

    unsafe {
        if let Some(ref mut task) = TASK_TABLE[task_idx] {
            // Save old mask
            if let Some(old) = oldset {
                *old = task.signal_mask;
            }

            // Apply new mask
            if let Some(new_set) = set {
                // Cannot block SIGKILL or SIGSTOP
                let safe_set = new_set & !((1 << (SIGKILL - 1)) | (1 << (SIGSTOP - 1)));

                match how {
                    SIG_BLOCK => task.signal_mask |= safe_set,
                    SIG_UNBLOCK => task.signal_mask &= !safe_set,
                    SIG_SETMASK => task.signal_mask = safe_set,
                    _ => return -22, // -EINVAL
                }
            }
        }
    }

    0
}

/// Check for and deliver pending signals
/// Called before exception return to user space
/// Returns true if a signal was delivered (and ELR/SPSR should be modified)
/// # Safety
/// `regs` must be a valid pointer to a TrapFrame that lives long enough
pub unsafe fn deliver_pending_signals(regs: *mut TrapFrame) -> bool {
    let task_idx = match find_task_index(sched::current_pid()) {
        Some(idx) => idx,
        None => return false,
    };

    let (pending, blocked) = unsafe {
        if let Some(ref task) = TASK_TABLE[task_idx] {
            (task.pending_signals, task.signal_mask)
        } else {
            return false;
        }
    };

    // Compute deliverable signals: pending & ~blocked
    let deliverable = pending & !blocked;

    if deliverable == 0 {
        return false;
    }

    // Find lowest-numbered deliverable signal
    for sig in 1..=MAX_SIG {
        let bit = sig - 1;
        if deliverable & (1 << bit) != 0 {
            unsafe {
                if let Some(ref mut task) = TASK_TABLE[task_idx] {
                    // Clear the pending bit
                    task.pending_signals &= !(1 << bit);

                    let handler = task.signal_handlers[sig as usize];

                    if handler == SIG_IGN {
                        // Ignore the signal
                        continue;
                    }

                    if handler == SIG_DFL {
                        // Execute default action
                        let action = default_action(sig);
                        match action {
                            SignalDefault::Terminate | SignalDefault::CoreDump => {
                                println!(
                                    "[signal] PID {} terminated by signal {}",
                                    sched::current_pid(),
                                    sig
                                );
                                task.exit_signal = sig;
                                sched::kill(sched::current_pid());
                                return true;
                            }
                            SignalDefault::Ignore => continue,
                            SignalDefault::Stop => {
                                println!(
                                    "[signal] PID {} stopped by signal {}",
                                    sched::current_pid(),
                                    sig
                                );
                                task.state = crate::sched::TaskState::Blocked;
                                return true;
                            }
                            SignalDefault::Continue => {
                                if task.state == crate::sched::TaskState::Blocked {
                                    task.state = crate::sched::TaskState::Ready;
                                }
                                continue;
                            }
                        }
                    }

                    // User-defined handler: set up signal frame
                    if let Some(regs) = regs.as_mut() {
                        setup_signal_frame(regs, sig, handler, task);
                        println!(
                            "[signal] PID {}: delivering signal {} to handler {:#x}",
                            sched::current_pid(),
                            sig,
                            handler
                        );
                        return true;
                    }
                }
            }
        }
    }

    false
}

/// Set up signal frame on user stack for handler execution
/// This saves the current context and prepares for signal handler call
fn setup_signal_frame(regs: &mut TrapFrame, sig: u64, handler: u64, task: &mut Task) {
    // The user stack pointer
    let sp = regs.sp;

    // Align stack to 16 bytes (AArch64 requirement)
    let aligned_sp = (sp - 16) & !0xF;

    // Calculate space needed for sigreturn frame
    // We need to save: TrapFrame + signal info + ucontext
    // For simplicity, we'll save just the TrapFrame
    let frame_size = core::mem::size_of::<TrapFrame>() as u64 + 128; // Extra space for siginfo/ucontext
    let frame_sp = (aligned_sp - frame_size) & !0xF;

    // Copy the current TrapFrame to user stack (sigreturn frame)
    // This is what sigreturn will restore
    let frame_ptr = frame_sp as *mut u8;
    unsafe {
        // Write the TrapFrame to user stack
        core::ptr::copy_nonoverlapping(
            regs as *const TrapFrame as *const u8,
            frame_ptr,
            core::mem::size_of::<TrapFrame>(),
        );
    }

    // Set up signal handler call:
    // x0 = signal number
    // x1 = siginfo_t* (simplified, use 0 for now)
    // x2 = ucontext_t* (point to saved TrapFrame)
    regs.x0 = sig;
    regs.x1 = 0; // siginfo_t pointer (not implemented)
    regs.x2 = frame_sp; // ucontext_t pointer (points to saved TrapFrame)

    // Set ELR to signal handler
    regs.elr = handler;

    // Set SPSR for EL0t (EL0 with SPSel=0, D=A=I=F=0)
    // M[3:0] = 0000 (EL0t)
    // DAIF = 0 (interrupts enabled)
    regs.spsr = 0x0000;

    // Set new user stack pointer
    regs.sp = frame_sp;

    // Mark that we're in a signal handler (for sigreturn)
    // We could use a flag in Task, but for now just note it
    task.signal_handlers[sig as usize] |= 1 << 63; // High bit = in handler
}

/// Send a signal to the current process (from kernel, e.g. page fault → SIGSEGV)
pub fn signal_self(sig: u64) {
    let pid = sched::current_pid();
    kill(pid, sig);
}

/// sigreturn() — restore context from signal handler
/// Called from sys_rt_sigreturn syscall to restore the trap frame
/// # Safety
/// `regs` must be a valid pointer to a TrapFrame that lives long enough
pub unsafe fn sigreturn(regs: *mut TrapFrame) -> Result<(), &'static str> {
    let pid = sched::current_pid();
    let task_idx = match sched::find_task_index(pid) {
        Some(i) => i,
        None => return Err("Task not found"),
    };

    unsafe {
        if let Some(ref mut task) = TASK_TABLE[task_idx] {
            let sp = (*regs).sp;

            // Read the sigreturn frame from user stack
            // The frame was saved at sp by setup_signal_frame
            let frame_ptr = sp as *const TrapFrame;
            let saved_frame = core::ptr::read_volatile(frame_ptr);

            // Restore the trap frame
            *regs = saved_frame;

            // Clear the "in handler" flag
            // Find which signal we're returning from
            for sig in 1..=MAX_SIG {
                let handler = task.signal_handlers[sig as usize];
                if handler & (1 << 63) != 0 {
                    task.signal_handlers[sig as usize] &= !(1 << 63);
                    break;
                }
            }
        }
    }

    Ok(())
}
