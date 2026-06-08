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
use crate::sched::{self, find_task_index, TASK_TABLE};

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
pub fn deliver_pending_signals() -> bool {
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

                    // User-defined handler: modify trap frame to jump to handler
                    // In a full implementation, we would:
                    // 1. Save the current ELR/SPSR onto the user stack (sigreturn frame)
                    // 2. Set ELR = handler address
                    // 3. Set x0 = signal number
                    // 4. Set SPSR = EL0t
                    // For now, log and treat as terminate
                    println!(
                        "[signal] PID {}: user handler for signal {} at {:#x} (sigreturn not yet implemented)",
                        sched::current_pid(), sig, handler
                    );
                    task.exit_signal = sig;
                    sched::kill(sched::current_pid());
                    return true;
                }
            }
        }
    }

    false
}

/// Send a signal to the current process (from kernel, e.g. page fault → SIGSEGV)
pub fn signal_self(sig: u64) {
    let pid = sched::current_pid();
    kill(pid, sig);
}

/// sigreturn() — restore context from signal handler
/// Called from sys_rt_sigreturn syscall to restore the trap frame
pub fn sigreturn() -> Result<(), &'static str> {
    let pid = sched::current_pid();
    let task_idx = match sched::find_task_index(pid) {
        Some(i) => i,
        None => return Err("Task not found"),
    };

    // The sigreturn frame is at the current user stack pointer
    // We need to read the frame from user stack and restore registers
    unsafe {
        if let Some(ref mut task) = TASK_TABLE[task_idx] {
            let _sp = task.regs.sp;

            // Read the sigreturn frame from user stack
            // Frame layout (simplified):
            // x0-x30, sp, elr, spsr
            // For now, just return success - a full implementation would restore all registers
            // from the user stack frame

            // TODO: Read frame from user stack at sp and restore task.regs
            // For now, just return OK
        }
    }

    Ok(())
}
