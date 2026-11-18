//! Timer-driven EL0 preemption (O5.2) and foreground job tracking (O5.3).
//!
//! The kernel main loop (desktop tick, input, shell) shares one CPU with
//! every EL0 task. A task that never traps would starve it, so the arch
//! timer IRQ taken *from EL0* ends the task's slice. Kernel code (EL1) is
//! never preempted by this mechanism: only IRQs whose saved SPSR says EL0
//! are eligible.
//!
//! Everything here is pure bookkeeping (no MMIO, no asm) so it is host
//! testable; the exception handler and the main loop call into it.

use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use super::Pid;

/// Period of the arch timer tick that drives preemption (see
/// `Timer::handle_arch_timer_irq`).
pub const TIMER_TICK_MS: u64 = 10;

/// Timer ticks an EL0 task may consume before its slice is ended.
///
/// Two ticks means a slice lasts 10-20 ms depending on where the task was
/// entered relative to the tick phase. One tick would make the effective
/// slice anywhere in 0-10 ms (too much context-switch churn); three or more
/// pushes the main loop's worst-case gap toward the 50 ms desktop budget.
pub const EL0_SLICE_TICKS: u32 = 2;

/// The desktop main loop must run at least this often while EL0 tasks spin.
pub const MAIN_LOOP_BUDGET_MS: u64 = 50;

/// Shell convention: a task ended by signal N reports status 128 + N.
const SIGNAL_STATUS_BASE: i64 = 128;

/// Exit status reported for a task terminated by signal `sig`.
pub fn status_for_signal(sig: u64) -> i64 {
    SIGNAL_STATUS_BASE + sig as i64
}

/// SPSR_EL1.M[4:0] mask: execution state bit plus exception level + SP.
const SPSR_MODE_MASK: u64 = 0x1F;

/// SPSR_EL1 mode value for AArch64 EL0t (the only way user code runs).
const SPSR_MODE_EL0T: u64 = 0x00;

/// True if an exception frame's SPSR says the interrupted code was EL0.
pub fn spsr_is_el0(spsr: u64) -> bool {
    spsr & SPSR_MODE_MASK == SPSR_MODE_EL0T
}

/// EL0 timer ticks consumed by the slice currently executing.
static SLICE_TICKS_USED: AtomicU32 = AtomicU32::new(0);

/// Start accounting a fresh slice (called right before the eret into EL0).
pub fn begin_slice() {
    SLICE_TICKS_USED.store(0, Ordering::Relaxed);
}

/// Account one timer tick taken from EL0; true when the slice is used up.
pub fn on_el0_tick() -> bool {
    let used = SLICE_TICKS_USED.fetch_add(1, Ordering::Relaxed) + 1;
    slice_expired(used, EL0_SLICE_TICKS)
}

/// True once `used` ticks reach the `budget`.
pub fn slice_expired(used: u32, budget: u32) -> bool {
    used >= budget
}

/// Worst-case gap between two main-loop iterations: one full slice (the
/// longest a task can run, including the tick-phase round-up) plus the
/// main loop's own work, bounded by `loop_overhead_ms`.
pub fn worst_case_loop_gap_ms(slice_ticks: u32, loop_overhead_ms: u64) -> u64 {
    (slice_ticks as u64) * TIMER_TICK_MS + loop_overhead_ms
}

/// One task that could be resumed by the main loop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Candidate {
    /// Slot in the task table.
    pub index: usize,
    /// Earliest uptime (ms) the task may run again (sleep deadline).
    pub wake_ms: u64,
    /// Round-robin ticket: lower ran longer ago.
    pub ticket: u64,
}

/// Pick the next task round-robin: among tasks whose wake time has arrived,
/// the one that ran longest ago (lowest ticket). Ordering by ticket alone
/// keeps a spinner (wake 0) from starving a task that just woke up.
pub fn pick_round_robin(candidates: &[Candidate], now_ms: u64) -> Option<usize> {
    candidates
        .iter()
        .filter(|c| c.wake_ms <= now_ms)
        .min_by_key(|c| c.ticket)
        .map(|c| c.index)
}

/// What to do with a signal sent to a task.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignalOutcome {
    /// End the task now.
    Terminate,
    /// Drop the signal (task ignores it).
    Ignore,
    /// Leave it pending for the task's own handler (delivered at its next
    /// syscall entry by `signal::deliver_pending_signals`).
    Queue,
}

/// Decide the fate of `sig` given the task's registered `handler`.
///
/// SIGKILL cannot be caught or ignored. Otherwise a user handler gets the
/// signal queued, `SIG_IGN` drops it, and `SIG_DFL` terminates (the only
/// default action reachable from the signals we send: INT/TERM/HUP).
pub fn signal_outcome(sig: u64, handler: u64) -> SignalOutcome {
    if sig == super::signal::SIGKILL {
        return SignalOutcome::Terminate;
    }
    match handler {
        super::signal::SIG_DFL => SignalOutcome::Terminate,
        super::signal::SIG_IGN => SignalOutcome::Ignore,
        _ => SignalOutcome::Queue,
    }
}

/// Longest observed gap between main-loop iterations.
#[derive(Clone, Copy, Debug)]
pub struct LoopGapTracker {
    last_ms: u64,
    max_gap_ms: u64,
}

impl LoopGapTracker {
    /// A tracker that has seen no iteration yet.
    pub const fn new() -> Self {
        Self {
            last_ms: 0,
            max_gap_ms: 0,
        }
    }

    /// Record an iteration at `now_ms`. The first one only sets the baseline.
    pub fn note(&mut self, now_ms: u64) {
        if self.last_ms != 0 {
            self.max_gap_ms = self.max_gap_ms.max(now_ms.saturating_sub(self.last_ms));
        }
        self.last_ms = now_ms;
    }

    /// Longest gap since the previous call; resets the window.
    pub fn take_max(&mut self) -> u64 {
        core::mem::take(&mut self.max_gap_ms)
    }
}

impl Default for LoopGapTracker {
    fn default() -> Self {
        Self::new()
    }
}

static LOOP_LAST_MS: AtomicU64 = AtomicU64::new(0);
static LOOP_MAX_GAP_MS: AtomicU64 = AtomicU64::new(0);

/// Main loop: record one iteration (desktop tick) at uptime `now_ms`.
pub fn note_main_loop_iteration(now_ms: u64) {
    let last = LOOP_LAST_MS.swap(now_ms, Ordering::Relaxed);
    if last != 0 {
        LOOP_MAX_GAP_MS.fetch_max(now_ms.saturating_sub(last), Ordering::Relaxed);
    }
}

/// Longest main-loop gap since the previous call; resets the window.
pub fn take_max_loop_gap_ms() -> u64 {
    LOOP_MAX_GAP_MS.swap(0, Ordering::Relaxed)
}

/// Start a fresh gap window and forget the current iteration's start, so
/// time spent inside a shell command (the one asking) is not counted.
pub fn restart_loop_gap_window() {
    LOOP_LAST_MS.store(0, Ordering::Relaxed);
    LOOP_MAX_GAP_MS.store(0, Ordering::Relaxed);
}

/// State of the single foreground job started by `run <path>`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ForegroundState {
    /// No foreground job.
    Idle,
    /// The job is running as a sliced task.
    Running(Pid),
    /// The job ended; the shell has not yet collected its status.
    Finished(Pid, i64),
}

/// Tracks the foreground job so the shell can wait for it asynchronously.
#[derive(Clone, Copy, Debug)]
pub struct ForegroundJob {
    state: ForegroundState,
}

impl ForegroundJob {
    /// A tracker with no job.
    pub const fn new() -> Self {
        Self {
            state: ForegroundState::Idle,
        }
    }

    /// Register `pid` as the foreground job; false if one is still active
    /// or its status is uncollected.
    pub fn start(&mut self, pid: Pid) -> bool {
        if self.state != ForegroundState::Idle {
            return false;
        }
        self.state = ForegroundState::Running(pid);
        true
    }

    /// Record that `pid` ended. Ignored unless it is the running job.
    pub fn finish(&mut self, pid: Pid, status: i64) -> bool {
        match self.state {
            ForegroundState::Running(p) if p == pid => {
                self.state = ForegroundState::Finished(pid, status);
                true
            }
            _ => false,
        }
    }

    /// PID of the running job, if any.
    pub fn running_pid(&self) -> Option<Pid> {
        match self.state {
            ForegroundState::Running(pid) => Some(pid),
            _ => None,
        }
    }

    /// Hand the finished job's (pid, status) to the shell exactly once.
    pub fn take_finished(&mut self) -> Option<(Pid, i64)> {
        match self.state {
            ForegroundState::Finished(pid, status) => {
                self.state = ForegroundState::Idle;
                Some((pid, status))
            }
            _ => None,
        }
    }
}

impl Default for ForegroundJob {
    fn default() -> Self {
        Self::new()
    }
}

/// The one foreground job. Only the single-core main loop and the exception
/// path (which runs with the main loop suspended) touch it.
static mut FOREGROUND: ForegroundJob = ForegroundJob::new();

/// Register `pid` as the foreground job; false if one is already active.
pub fn foreground_start(pid: Pid) -> bool {
    // SAFETY: single core; callers run either in the main loop or in the
    // exception path while the main loop is suspended, never concurrently.
    unsafe { (*core::ptr::addr_of_mut!(FOREGROUND)).start(pid) }
}

/// Note that `pid` ended with `status`; no-op unless it is the foreground job.
pub fn foreground_note_exit(pid: Pid, status: i64) -> bool {
    // SAFETY: same single-core exclusivity as `foreground_start`.
    unsafe { (*core::ptr::addr_of_mut!(FOREGROUND)).finish(pid, status) }
}

/// PID of the running foreground job, if any.
pub fn foreground_pid() -> Option<Pid> {
    // SAFETY: same single-core exclusivity as `foreground_start`.
    unsafe { (*core::ptr::addr_of!(FOREGROUND)).running_pid() }
}

/// Collect the finished job's (pid, status), once.
pub fn foreground_take_finished() -> Option<(Pid, i64)> {
    // SAFETY: same single-core exclusivity as `foreground_start`.
    unsafe { (*core::ptr::addr_of_mut!(FOREGROUND)).take_finished() }
}

/// Reset the foreground tracker (used by tests).
#[cfg(test)]
pub fn foreground_reset() {
    // SAFETY: tests hold the serial guard.
    unsafe { *core::ptr::addr_of_mut!(FOREGROUND) = ForegroundJob::new() };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(index: usize, wake_ms: u64, ticket: u64) -> Candidate {
        Candidate {
            index,
            wake_ms,
            ticket,
        }
    }

    #[test]
    fn el0_spsr_detected_and_el1_rejected() {
        assert!(spsr_is_el0(0x0));
        assert!(spsr_is_el0(0x6000_0000));
        assert!(!spsr_is_el0(0x3C5));
        assert!(!spsr_is_el0(0x3C4));
        assert!(!spsr_is_el0(0x10));
    }

    #[test]
    fn slice_expires_after_budget_ticks() {
        let _guard = crate::test_support::serial_guard();
        begin_slice();
        for _ in 0..EL0_SLICE_TICKS - 1 {
            assert!(!on_el0_tick());
        }
        assert!(on_el0_tick());
        begin_slice();
        assert!(!on_el0_tick());
    }

    #[test]
    fn desktop_stays_within_budget_while_tasks_spin() {
        let gap = worst_case_loop_gap_ms(EL0_SLICE_TICKS, 10);
        assert!(gap <= MAIN_LOOP_BUDGET_MS, "gap {gap} ms");
    }

    #[test]
    fn two_spinners_alternate() {
        let mut a = cand(0, 0, 1);
        let mut b = cand(1, 0, 2);
        let mut ticket = 3;
        let mut order = Vec::new();
        for _ in 0..6 {
            let pick = pick_round_robin(&[a, b], 100).expect("runnable");
            order.push(pick);
            if pick == 0 {
                a.ticket = ticket;
            } else {
                b.ticket = ticket;
            }
            ticket += 1;
        }
        assert_eq!(order, vec![0, 1, 0, 1, 0, 1]);
    }

    #[test]
    fn sleeper_not_starved_by_spinner() {
        let spinner = cand(0, 0, 1);
        let woken = cand(1, 50, 0);
        assert_eq!(pick_round_robin(&[spinner, woken], 40), Some(0));
        assert_eq!(pick_round_robin(&[spinner, woken], 60), Some(1));
    }

    #[test]
    fn nothing_picked_when_all_asleep() {
        assert_eq!(pick_round_robin(&[cand(0, 500, 1)], 100), None);
        assert_eq!(pick_round_robin(&[], 100), None);
    }

    #[test]
    fn loop_gap_tracks_longest_and_resets() {
        let mut t = LoopGapTracker::new();
        t.note(1000);
        t.note(1020);
        t.note(1090);
        t.note(1100);
        assert_eq!(t.take_max(), 70);
        t.note(1110);
        assert_eq!(t.take_max(), 10);
    }

    #[test]
    fn fp_snapshot_matches_boot_s_reservation() {
        assert_eq!(core::mem::size_of::<crate::sched::FpSnapshot>(), 528);
    }

    #[test]
    fn fp_snapshot_loads_into_context() {
        let mut snap = crate::sched::FpSnapshot {
            q: [[0; 2]; 16],
            fpcr: 0x0300_0000,
            fpsr: 0x8,
        };
        snap.q[3] = [7, 9];
        let mut ctx = crate::sched::FpContext::zeroed();
        ctx.load_snapshot(&snap);
        assert_eq!(ctx.q[3], [7, 9]);
        assert_eq!((ctx.fpcr, ctx.fpsr), (0x0300_0000, 0x8));
    }

    #[test]
    fn signal_status_follows_shell_convention() {
        assert_eq!(status_for_signal(2), 130);
        assert_eq!(status_for_signal(9), 137);
    }

    #[test]
    fn sigkill_always_terminates() {
        use crate::sched::signal::*;
        assert_eq!(signal_outcome(SIGKILL, SIG_IGN), SignalOutcome::Terminate);
        assert_eq!(signal_outcome(SIGKILL, 0x4000), SignalOutcome::Terminate);
    }

    #[test]
    fn int_and_term_respect_dispositions() {
        use crate::sched::signal::*;
        assert_eq!(signal_outcome(SIGINT, SIG_DFL), SignalOutcome::Terminate);
        assert_eq!(signal_outcome(SIGTERM, SIG_IGN), SignalOutcome::Ignore);
        assert_eq!(signal_outcome(SIGTERM, 0x4000), SignalOutcome::Queue);
    }

    #[test]
    fn foreground_runs_then_reports_once() {
        let mut job = ForegroundJob::new();
        assert!(job.start(7));
        assert_eq!(job.running_pid(), Some(7));
        assert!(job.finish(7, 3));
        assert_eq!(job.running_pid(), None);
        assert_eq!(job.take_finished(), Some((7, 3)));
        assert_eq!(job.take_finished(), None);
    }

    #[test]
    fn foreground_rejects_second_job_and_foreign_exit() {
        let mut job = ForegroundJob::new();
        assert!(job.start(7));
        assert!(!job.start(8));
        assert!(!job.finish(8, 0));
        assert_eq!(job.running_pid(), Some(7));
    }

    #[test]
    fn foreground_slot_reusable_after_collect() {
        let mut job = ForegroundJob::new();
        assert!(job.start(1));
        assert!(job.finish(1, status_for_signal(crate::sched::signal::SIGINT)));
        assert!(!job.start(2), "uncollected status blocks a new job");
        assert!(job.take_finished().is_some());
        assert!(job.start(2));
    }

    #[test]
    fn global_foreground_roundtrip() {
        let _guard = crate::test_support::serial_guard();
        foreground_reset();
        assert!(foreground_start(42));
        assert_eq!(foreground_pid(), Some(42));
        assert!(foreground_note_exit(42, status_for_signal(9)));
        assert_eq!(foreground_take_finished(), Some((42, 137)));
        assert_eq!(foreground_pid(), None);
    }
}
