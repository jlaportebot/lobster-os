//! Virtual desktops / workspaces — multiple independent desktop surfaces
//!
//! Each workspace has its own set of visible windows. Switching workspaces
//! hides all windows on the current one and shows all on the target.
//! Supports up to MAX_WORKSPACES (default 4) independent workspaces.

use crate::println;
use crate::sync::spinlock::Spinlock;
use crate::wm::compositor;

/// Maximum number of virtual desktops
pub const MAX_WORKSPACES: usize = 4;

/// Per-workspace window membership tracking.
/// Each workspace has a bitmap of window IDs that belong to it.
#[derive(Clone, Copy)]
struct Workspace {
    /// Window IDs assigned to this workspace
    window_ids: [u32; 16],
    window_count: usize,
    active: bool,
}

impl Workspace {
    const fn empty() -> Self {
        Workspace {
            window_ids: [0; 16],
            window_count: 0,
            active: false,
        }
    }
}

static WORKSPACES: Spinlock<[Workspace; MAX_WORKSPACES]> =
    Spinlock::new([Workspace::empty(); MAX_WORKSPACES]);
static ACTIVE_WS: Spinlock<usize> = Spinlock::new(0);

/// Initialize workspaces — mark workspace 0 as active
pub fn init() {
    let mut ws = WORKSPACES.lock();
    for w in ws.iter_mut() {
        *w = Workspace::empty();
    }
    ws[0].active = true;
    *ACTIVE_WS.lock() = 0;
    println!("[wm] Workspaces initialized ({} desktops)", MAX_WORKSPACES);
}

/// Get the active workspace index
pub fn active() -> usize {
    *ACTIVE_WS.lock()
}

/// Get the total number of workspaces
pub fn count() -> usize {
    MAX_WORKSPACES
}

/// Assign a window to a workspace
pub fn assign_window(ws_idx: usize, window_id: u32) {
    if ws_idx >= MAX_WORKSPACES {
        return;
    }
    let mut ws = WORKSPACES.lock();
    let w = &mut ws[ws_idx];
    if w.window_count < 16 {
        // Avoid duplicates
        for i in 0..w.window_count {
            if w.window_ids[i] == window_id {
                return;
            }
        }
        w.window_ids[w.window_count] = window_id;
        w.window_count += 1;
    }
}

/// Remove a window from all workspaces (called when window is destroyed)
pub fn remove_window(window_id: u32) {
    let mut ws = WORKSPACES.lock();
    for w in ws.iter_mut() {
        for i in 0..w.window_count {
            if w.window_ids[i] == window_id {
                // Shift remaining windows down
                for j in i..w.window_count.saturating_sub(1) {
                    w.window_ids[j] = w.window_ids[j + 1];
                }
                w.window_count -= 1;
                break;
            }
        }
    }
}

/// Check if a window belongs to the active workspace
pub fn is_window_on_active(window_id: u32) -> bool {
    let ws = WORKSPACES.lock();
    let active_idx = *ACTIVE_WS.lock();
    let w = &ws[active_idx];
    for i in 0..w.window_count {
        if w.window_ids[i] == window_id {
            return true;
        }
    }
    false
}

/// Switch to a workspace by index (0-based)
pub fn switch_to(target: usize) {
    if target >= MAX_WORKSPACES {
        return;
    }
    let current = *ACTIVE_WS.lock();
    if current == target {
        return;
    }

    let mut ws = WORKSPACES.lock();
    let mut active = ACTIVE_WS.lock();

    // Hide windows on current workspace
    let old_count = ws[current].window_count;
    for i in 0..old_count {
        let wid = ws[current].window_ids[i];
        compositor::set_layer_visible(wid, false);
    }
    ws[current].active = false;

    // Show windows on target workspace
    let new_count = ws[target].window_count;
    for i in 0..new_count {
        let wid = ws[target].window_ids[i];
        compositor::set_layer_visible(wid, true);
    }
    ws[target].active = true;
    *active = target;

    drop(ws);
    drop(active);

    compositor::mark_full_redraw();
    println!("[wm] Switched to workspace {}", target + 1);
}

/// Switch to the next workspace (wraps around)
pub fn switch_next() {
    let current = *ACTIVE_WS.lock();
    let next = (current + 1) % MAX_WORKSPACES;
    switch_to(next);
}

/// Switch to the previous workspace (wraps around)
pub fn switch_prev() {
    let current = *ACTIVE_WS.lock();
    let prev = if current == 0 {
        MAX_WORKSPACES - 1
    } else {
        current - 1
    };
    switch_to(prev);
}

/// Move a window to a different workspace
pub fn move_window_to(window_id: u32, target: usize) {
    if target >= MAX_WORKSPACES {
        return;
    }
    remove_window(window_id);
    assign_window(target, window_id);

    // If target is not active, hide the window
    let active_idx = *ACTIVE_WS.lock();
    if target != active_idx {
        compositor::set_layer_visible(window_id, false);
    }
    println!(
        "[wm] Moved window {} to workspace {}",
        window_id,
        target + 1
    );
}

/// Get window IDs for a workspace (for panel display)
pub fn workspace_windows(ws_idx: usize) -> alloc::vec::Vec<u32> {
    if ws_idx >= MAX_WORKSPACES {
        return alloc::vec::Vec::new();
    }
    let ws = WORKSPACES.lock();
    let mut result = alloc::vec::Vec::new();
    for i in 0..ws[ws_idx].window_count {
        result.push(ws[ws_idx].window_ids[i]);
    }
    result
}
