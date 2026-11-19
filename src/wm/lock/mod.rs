//! Session lock and login overlay (roadmap O3.8).
//!
//! One full-screen overlay serves three jobs: the lock screen (Super+L, the
//! start-menu Lock entry, the shell `lock` command), the post-logout greeter
//! and the boot greeter. While it is up every keyboard and mouse event is
//! routed here by `wm::input` before any shortcut, hit region or menu sees
//! it; windows, timers and processes keep running underneath. Nothing about
//! the locked state is persisted: a reboot always starts unlocked (or at the
//! boot greeter if accounts need it).

pub mod commands;
pub mod form;
pub mod layout;
pub mod overlay;
mod session;
pub mod view;

use crate::display::fb::FramebufferInfo;
use crate::sync::spinlock::Spinlock;
use crate::usb::hid::HidEvent;
use crate::users::auth::{self, AuthError, PasswordState};
use crate::users::throttle::Throttle;
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use layout::{Hit, Screen};
use overlay::{Account, Outcome, Overlay, Stage};

/// Scancode (set 1) of the `L` key, for Super+L.
const SC_L: u8 = 0x26;
/// Milliseconds per minute, for the clock redraw check.
const MS_PER_SECOND: u64 = 1000;

static STATE: Spinlock<Option<Overlay>> = Spinlock::new(None);
static THROTTLE: Spinlock<Throttle> = Spinlock::new(Throttle::new());
static ACTIVE: AtomicBool = AtomicBool::new(false);
static REDRAW: AtomicBool = AtomicBool::new(false);
/// Last clock minute / cooldown second drawn, to redraw only on change.
static LAST_STAMP: AtomicU64 = AtomicU64::new(0);

/// Where an input event goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Delivery {
    /// The normal desktop pipeline (shortcuts, menus, hit regions, windows).
    Desktop,
    /// Only the lock overlay.
    Overlay,
}

/// Decide where `event` goes. While the overlay is up everything, including
/// Super, Alt+Tab, Alt+F4, mouse moves and clicks, belongs to it.
#[must_use]
pub fn delivery(overlay_active: bool, _event: &HidEvent) -> Delivery {
    if overlay_active {
        Delivery::Overlay
    } else {
        Delivery::Desktop
    }
}

/// True while the lock screen or greeter is showing.
#[must_use]
pub fn is_active() -> bool {
    ACTIVE.load(Ordering::Acquire)
}

/// True when `scancode` with Super held asks to lock the session.
#[must_use]
pub fn is_lock_chord(scancode: u8, gui_held: bool) -> bool {
    gui_held && scancode == SC_L
}

fn account_for(name: &str) -> Account {
    Account {
        name: String::from(name),
        password: auth::password_state(name),
    }
}

fn accounts() -> Vec<Account> {
    crate::users::login_users()
        .iter()
        .map(|n| account_for(n))
        .collect()
}

fn show(overlay: Overlay) {
    *STATE.lock() = Some(overlay);
    ACTIVE.store(true, Ordering::Release);
    REDRAW.store(true, Ordering::Relaxed);
}

fn hide() {
    *STATE.lock() = None;
    ACTIVE.store(false, Ordering::Release);
    crate::wm::compositor::mark_full_redraw();
    crate::wm::dirty::mark();
    #[cfg(feature = "qemu_raspi5b")]
    crate::wm::scene::damage_all();
}

/// Close every transient surface so nothing from the desktop stays live
/// behind (or reappears after) the overlay.
fn dismiss_desktop_surfaces() {
    crate::wm::start_menu::close();
    crate::wm::tray_popups::close();
    crate::wm::context_menu::hide();
    crate::wm::alt_tab::close(false);
    crate::wm::controls::drag::end();
    crate::wm::input::cancel_pointer_capture();
}

/// Lock the session of the current user (no-op if already locked).
pub fn lock() {
    if is_active() {
        return;
    }
    dismiss_desktop_surfaces();
    let user = crate::users::current_user();
    crate::println!("[lock] session locked for {}", user);
    show(Overlay::lock(account_for(&user)));
}

/// End the session: close every window, stop the user's processes, clear
/// per-session state, then show the greeter.
pub fn log_out() {
    dismiss_desktop_surfaces();
    session::end_current();
    show(Overlay::greeter(accounts()));
}

/// Show the greeter before the desktop exists (boot).
pub fn begin_boot_greeter() {
    show(Overlay::greeter(accounts()));
}

/// Deliver one HID event while the boot greeter runs.
pub fn deliver_boot_event(event: HidEvent) {
    crate::wm::input::deliver_boot_event(&event);
}

/// Whether boot should stop at the greeter: more than one login account, or
/// any login account with a password.
#[must_use]
pub fn greeter_needed() -> bool {
    let states: Vec<PasswordState> = crate::users::login_users()
        .iter()
        .map(|n| auth::password_state(n))
        .collect();
    greeter_required(&states)
}

/// Pure form of [`greeter_needed`].
#[must_use]
pub fn greeter_required(states: &[PasswordState]) -> bool {
    states.len() > 1 || states.iter().any(|s| *s != PasswordState::None)
}

fn uptime_ms() -> u64 {
    crate::arch::timer::Timer::uptime_ms()
}

fn check(user: &str, password: &str) -> Result<(), AuthError> {
    auth::check_password(user, password)
}

fn run_overlay(step: impl FnOnce(&mut Overlay, &mut Throttle, u64) -> Outcome) {
    let now = uptime_ms();
    let outcome = {
        let mut state = STATE.lock();
        let mut throttle = THROTTLE.lock();
        match state.as_mut() {
            Some(o) => step(o, &mut throttle, now),
            None => Outcome::None,
        }
    };
    REDRAW.store(true, Ordering::Relaxed);
    if let Outcome::Done(user) = outcome {
        finish(&user);
    }
}

fn finish(user: &str) {
    let greeter = STATE
        .lock()
        .as_ref()
        .is_some_and(|o| o.mode() == overlay::Mode::Greeter);
    if greeter {
        crate::users::login(user);
        crate::apps::settings::store::load_at_boot();
    }
    crate::println!(
        "[lock] {} for {}",
        if greeter { "login" } else { "unlocked" },
        user
    );
    hide();
}

/// Feed a key press to the overlay.
pub fn key_press(scancode: u8) {
    let shift = crate::usb::hid::shift_held();
    let caps = STATE.lock().as_ref().is_some_and(|o| o.form().caps());
    let key = form::key_from_scancode(scancode, shift, caps);
    run_overlay(|o, t, now| o.key(key, t, now, check));
}

/// Feed a left click at (`x`, `y`) to the overlay.
pub fn click(x: u32, y: u32) {
    let screen = crate::display::fb::device_info();
    let target = STATE.lock().as_ref().map(|o| match o.stage() {
        Stage::Choose => Screen::List {
            count: o.accounts().len(),
            selected: o.selected(),
        },
        Stage::Password => Screen::Password,
    });
    let Some(target) = target else { return };
    match layout::hit(screen.width, screen.height, target, x, y) {
        Hit::Account(index) => run_overlay(|o, _, _| o.choose(index)),
        Hit::Button => run_overlay(|o, t, now| o.submit(t, now, check)),
        Hit::Nothing => {}
    }
}

/// Redraw check, called once per frame: true when the overlay changed or its
/// clock/countdown ticked, so the caller must repaint the screen.
pub fn take_redraw() -> bool {
    if !is_active() {
        return false;
    }
    let now = uptime_ms();
    let cooldown = THROTTLE.lock().remaining_ms(now);
    let minute = crate::clock::local_at(now).map_or(0, |t| u64::from(t.minute));
    let stamp = if cooldown > 0 {
        1_000_000 + now / MS_PER_SECOND
    } else {
        minute
    };
    let ticked = LAST_STAMP.swap(stamp, Ordering::Relaxed) != stamp;
    REDRAW.swap(false, Ordering::Relaxed) || ticked
}

/// Paint the overlay into `target` (wallpaper, dimming, content).
pub fn draw(target: &FramebufferInfo) {
    let now = uptime_ms();
    let state = STATE.lock();
    let throttle = THROTTLE.lock();
    if let Some(o) = state.as_ref() {
        view::draw(target, o, &throttle, now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn greeter_is_skipped_for_a_lone_passwordless_user() {
        assert!(!greeter_required(&[PasswordState::None]));
    }

    #[test]
    fn greeter_is_shown_for_a_lone_user_with_a_password() {
        assert!(greeter_required(&[PasswordState::Set]));
    }

    #[test]
    fn greeter_is_shown_for_two_passwordless_users() {
        assert!(greeter_required(&[
            PasswordState::None,
            PasswordState::None
        ]));
    }

    #[test]
    fn locked_only_account_still_needs_the_greeter() {
        assert!(greeter_required(&[PasswordState::Locked]));
    }

    #[test]
    fn locked_overlay_takes_every_kind_of_event() {
        let events = [
            HidEvent::KeyPress {
                scancode: 0x5B,
                extended: false,
            },
            HidEvent::KeyPress {
                scancode: 0x0F,
                extended: false,
            },
            HidEvent::KeyPress {
                scancode: 0x3E,
                extended: false,
            },
            HidEvent::KeyRelease {
                scancode: 0x38,
                extended: false,
            },
            HidEvent::MouseMove { dx: 3, dy: -2 },
            HidEvent::MouseButton {
                left: true,
                right: false,
                middle: false,
            },
            HidEvent::MouseButton {
                left: false,
                right: true,
                middle: false,
            },
        ];
        for ev in &events {
            assert_eq!(delivery(true, ev), Delivery::Overlay);
            assert_eq!(delivery(false, ev), Delivery::Desktop);
        }
    }

    #[test]
    fn super_l_is_the_lock_chord_only_with_super_held() {
        assert!(is_lock_chord(0x26, true));
        assert!(!is_lock_chord(0x26, false));
        assert!(!is_lock_chord(0x20, true));
    }
}
