//! Window-manager runtime options (C2).
//!
//! Tunables that influence input routing. Defaults reproduce the behavior
//! the WM had before these options existed.

use core::sync::atomic::{AtomicBool, Ordering};

/// Pixels the pointer must travel (Euclidean, from the press point) after
/// pressing a title bar before the window actually starts to move. A
/// press+release that stays inside this radius is a plain click: no move,
/// no drag-to-edge snap.
pub const DRAG_THRESHOLD_PX: u32 = 4;

/// Focus-follows-z: hovering a window's chrome (title bar) raises/focuses
/// it without a click. Default OFF.
static FOCUS_FOLLOWS_Z: AtomicBool = AtomicBool::new(false);

pub fn focus_follows_z() -> bool {
    FOCUS_FOLLOWS_Z.load(Ordering::Relaxed)
}

pub fn set_focus_follows_z(on: bool) {
    FOCUS_FOLLOWS_Z.store(on, Ordering::Relaxed);
}

/// Shell entry point: `wmconfig ffz on|off` (no arg / unknown → status).
pub fn shell_command(args: &[&str]) {
    match (args.get(1).copied(), args.get(2).copied()) {
        (Some("ffz"), Some("on")) => {
            set_focus_follows_z(true);
            crate::println!("wmconfig: ffz on");
        }
        (Some("ffz"), Some("off")) => {
            set_focus_follows_z(false);
            crate::println!("wmconfig: ffz off");
        }
        _ => {
            crate::println!(
                "usage: wmconfig ffz on|off   (ffz currently {}, drag threshold {}px)",
                if focus_follows_z() { "on" } else { "off" },
                DRAG_THRESHOLD_PX
            );
        }
    }
}
