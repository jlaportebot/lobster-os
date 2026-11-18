//! Foreground job control for the shell (O5.2 / O5.3).
//!
//! `run <path>` starts the program as a sliced task and returns at once, so
//! the desktop keeps ticking. The main loop calls [`poll`] every iteration:
//! it delivers a pending Ctrl+C, and when the job has ended it prints its
//! exit status and brings the prompt back where the command was typed
//! (serial console or the Terminal window).

use core::sync::atomic::{AtomicBool, Ordering};

use crate::drivers::uart::Uart10;
use crate::sched::preempt;

/// The foreground job was started from the Terminal window (not serial).
static FROM_GUI: AtomicBool = AtomicBool::new(false);

/// A command line is currently executing on behalf of the Terminal window.
static GUI_EXECUTING: AtomicBool = AtomicBool::new(false);

/// Ctrl+C was typed; the main loop delivers SIGINT at its next [`poll`].
/// Deferred because key handlers may run in IRQ context, where tearing a
/// task down (frees frames) is not safe.
static INTERRUPT_REQUESTED: AtomicBool = AtomicBool::new(false);

/// The command being executed started the foreground job, so its own
/// prompt is deferred until the job ends.
static PROMPT_DEFERRED: AtomicBool = AtomicBool::new(false);

/// Prompt shown inside the Terminal window.
const GUI_PROMPT: &str = "lobster:/# ";

/// Mark whether the command being executed came from the Terminal window.
pub(super) fn set_gui_executing(on: bool) {
    GUI_EXECUTING.store(on, Ordering::Relaxed);
}

/// True while a foreground `run` job is executing.
pub fn is_active() -> bool {
    preempt::foreground_pid().is_some()
}

/// Register `pid` as the foreground job; false if one is already running.
pub(super) fn begin(pid: crate::sched::Pid) -> bool {
    if !preempt::foreground_start(pid) {
        return false;
    }
    FROM_GUI.store(GUI_EXECUTING.load(Ordering::Relaxed), Ordering::Relaxed);
    PROMPT_DEFERRED.store(true, Ordering::Relaxed);
    true
}

/// Ask for the foreground job to be interrupted (Ctrl+C). Safe from any
/// context; the main loop does the work.
pub fn request_interrupt() {
    INTERRUPT_REQUESTED.store(true, Ordering::Relaxed);
}

/// Main-loop hook: deliver a pending Ctrl+C, then report a finished job.
pub fn poll() {
    if INTERRUPT_REQUESTED.swap(false, Ordering::Relaxed) {
        crate::sched::interrupt_foreground();
    }
    let Some((pid, status)) = preempt::foreground_take_finished() else {
        return;
    };
    // The reap in sched may not have run yet for a task that exited by
    // itself; make sure its memory and windows are gone before the prompt.
    crate::sched::reap_zombies();
    let gui = FROM_GUI.load(Ordering::Relaxed);
    announce_exit(gui, pid, status);
    super::set_last_exit_code(status as i32);
}

/// Print the exit line and the prompt in the place the job was started.
fn announce_exit(gui: bool, pid: crate::sched::Pid, status: i64) {
    if gui {
        crate::console::with_terminal_mirror(|| {
            crate::println!("run: PID {} exited with code {}", pid, status);
            Uart10::puts(GUI_PROMPT);
        });
        return;
    }
    crate::println!("run: PID {} exited with code {}", pid, status);
    Uart10::puts(&super::build_prompt());
}

/// True if the command just executed started the foreground job, so its
/// prompt must wait until the job ends. Later commands (typed while the job
/// runs) get their prompt back immediately. Consumes the flag.
pub fn holds_prompt() -> bool {
    PROMPT_DEFERRED.swap(false, Ordering::Relaxed)
}
