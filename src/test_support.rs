//! Shared support for host unit tests.
//!
//! Kernel globals (`static mut` tables, spinlock-guarded registries, atomics
//! used as shared state) are process-wide, but the libtest runner executes
//! tests on multiple threads. Any test that reads or writes such state, directly
//! or through the functions it calls, must take [`serial_guard`] first so
//! tests cannot interleave on the same globals.

use std::sync::{Mutex, MutexGuard, PoisonError};

static GLOBAL_STATE_LOCK: Mutex<()> = Mutex::new(());

/// Serializes tests that touch process-wide kernel state.
///
/// Poison-tolerant: a failing test that panics while holding the guard does
/// not cascade into failures of every later test.
pub(crate) fn serial_guard() -> MutexGuard<'static, ()> {
    GLOBAL_STATE_LOCK
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}
