//! Tearing down a desktop session for log out: windows, the user's
//! processes and per-session state.

use crate::sched::{TaskState, MAX_TASKS, TASK_TABLE};
use alloc::vec::Vec;

/// Workspace shown after log out.
const FIRST_WORKSPACE: usize = 0;
/// Console the desktop session is registered on.
const SESSION_TTY: &str = "tty1";

/// Close every window. EL0 app windows also stop their process, like the
/// close button does.
fn close_windows() {
    for id in 1..=crate::wm::window::max_id() {
        if !crate::wm::window::exists(id) {
            continue;
        }
        match crate::wm::user_windows::owner_of_wm(id) {
            Some(pid) => {
                crate::wm::user_windows::close_all_for_pid(pid);
                crate::sched::kill(pid);
            }
            None => crate::wm::window::destroy(id),
        }
    }
}

fn user_pids() -> Vec<u64> {
    let mut pids = Vec::new();
    for slot in 0..MAX_TASKS {
        // SAFETY: the WM runs on the main loop and nothing else holds a
        // reference into the task table while a log out is processed.
        let task = unsafe { &*core::ptr::addr_of!(TASK_TABLE[slot]) };
        if let Some(t) = task
            .as_ref()
            .filter(|t| t.is_user && t.state != TaskState::Zombie)
        {
            pids.push(t.pid);
        }
    }
    pids
}

/// Stop every remaining EL0 process (background apps without a window).
fn end_user_processes() {
    for pid in user_pids() {
        crate::wm::user_windows::close_all_for_pid(pid);
        crate::sched::kill(pid);
    }
}

fn end_login_sessions() {
    while let Some((index, _)) = crate::users::find_session_by_tty(SESSION_TTY) {
        if !crate::users::deallocate_session(index) {
            break;
        }
    }
}

/// End the current user's desktop session.
pub fn end_current() {
    crate::println!("[lock] logging out {}", crate::users::current_user());
    close_windows();
    end_user_processes();
    crate::wm::dialog::init();
    crate::wm::notifications::clear_history();
    crate::wm::workspace::switch_to(FIRST_WORKSPACE);
    end_login_sessions();
    crate::wm::dirty::mark();
}
