//! Boot-time login greeter. It drives the same overlay as the lock screen
//! (`wm::lock`) from its own small poll loop, because the desktop is not
//! composited yet: account list, password entry, cooldown and the
//! 5-wrong-tries lockout are shared with Super+L and log out.

use crate::display::fb;
use crate::usb::hid;
use crate::wm::{compositor, input, lock};

/// True when the greeter should be shown at boot: more than one login
/// account, or any login account protected by a password.
pub fn needed() -> bool {
    lock::greeter_needed()
}

fn feed(event: hid::HidEvent) {
    lock::deliver_boot_event(event);
}

/// Show the greeter and block until a user is logged in (click, or arrow
/// keys + Enter, then the password when the account has one).
pub fn run() {
    let info = fb::get_info();
    if info.width == 0 {
        return;
    }
    lock::begin_boot_greeter();
    crate::println!("[greeter] waiting for login");
    while lock::is_active() {
        let mut changed = false;
        while let Some(event) = hid::poll() {
            changed = true;
            feed(event);
        }
        if changed || lock::take_redraw() {
            compositor::restore_software_cursor(&info);
            if lock::is_active() {
                lock::draw(&info);
            }
            compositor::draw_software_cursor(&info);
        }
        core::hint::spin_loop();
    }
    compositor::restore_software_cursor(&info);
    let (cx, cy) = input::cursor_pos();
    input::set_cursor_pos(cx, cy);
}
