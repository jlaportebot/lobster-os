//! Password entry state shared by the lock screen and the login greeter:
//! the typed text, the last outcome, Caps Lock tracking and the submit rule
//! (cooldown, strike counting). Pure, so the whole state machine is
//! host-tested; the caller supplies the clock and the password check.

use crate::users::auth::AuthError;
use crate::users::policy::MAX_PASSWORD_LEN;
use crate::users::throttle::{Strike, Throttle};
use alloc::vec::Vec;

/// Scancode set 1 code of the Caps Lock key (the input device delivers set 1).
const SC_CAPS_LOCK: u8 = 0x3A;
/// Set 1 code of Backspace.
const SC_BACKSPACE: u8 = 0x0E;
/// Set 1 code of Return (and of Keypad Enter, which differs only by prefix).
const SC_ENTER: u8 = 0x1C;
/// Set 1 code of Escape.
const SC_ESCAPE: u8 = 0x01;
/// Set 1 code of Tab.
const SC_TAB: u8 = 0x0F;
/// Set 1 code of the Up arrow.
const SC_UP: u8 = 0x48;
/// Set 1 code of the Down arrow.
const SC_DOWN: u8 = 0x50;

/// A key as the lock screen understands it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    /// A printable ASCII character (Shift and Caps Lock already applied).
    Char(char),
    /// Delete the last character.
    Backspace,
    /// Submit.
    Enter,
    /// Go back / clear.
    Escape,
    /// Move the selection up.
    Up,
    /// Move the selection down (Tab also lands here).
    Down,
    /// Caps Lock toggled.
    CapsLock,
    /// Anything else: swallowed, never reaches the desktop.
    Other,
}

/// Translate a set 1 scancode plus modifier state into a [`Key`]. `caps` is the
/// tracked Caps Lock state; it flips the case of letters only.
#[must_use]
pub fn key_from_scancode(scancode: u8, shift: bool, caps: bool) -> Key {
    match scancode {
        SC_CAPS_LOCK => Key::CapsLock,
        SC_BACKSPACE => Key::Backspace,
        SC_ENTER => Key::Enter,
        SC_ESCAPE => Key::Escape,
        SC_UP => Key::Up,
        SC_DOWN | SC_TAB => Key::Down,
        _ => {
            let letter_shift = shift != caps;
            let Some(plain) = crate::usb::hid::scancode_to_ascii(scancode, false) else {
                return Key::Other;
            };
            let effective = if plain.is_ascii_alphabetic() {
                letter_shift
            } else {
                shift
            };
            match crate::usb::hid::scancode_to_ascii(scancode, effective) {
                Some(c) if (' '..='~').contains(&c) => Key::Char(c),
                _ => Key::Other,
            }
        }
    }
}

/// Result of the last submit, shown under the field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Nothing to report.
    Idle,
    /// Wrong password; tries left before a cooldown.
    Wrong(u32),
    /// The account cannot log in (locked, expired, ...).
    Refused(AuthError),
}

/// What a submit did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Submit {
    /// Correct: the caller unlocks or logs in.
    Accepted,
    /// Wrong or refused; the field was cleared.
    Rejected,
    /// A cooldown is running; the attempt was not even checked.
    Blocked,
}

/// Typed password and last outcome.
pub struct PasswordForm {
    text: Vec<u8>,
    status: Status,
    caps: bool,
}

impl PasswordForm {
    /// Empty form.
    #[must_use]
    pub fn new() -> Self {
        Self {
            text: Vec::new(),
            status: Status::Idle,
            caps: false,
        }
    }

    /// Number of typed characters (drawn as dots).
    #[must_use]
    pub fn len(&self) -> usize {
        self.text.len()
    }

    /// True when nothing is typed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// Last submit result.
    #[must_use]
    pub fn status(&self) -> Status {
        self.status
    }

    /// Tracked Caps Lock state.
    #[must_use]
    pub fn caps(&self) -> bool {
        self.caps
    }

    /// Flip the tracked Caps Lock state.
    pub fn toggle_caps(&mut self) {
        self.caps = !self.caps;
    }

    /// Append a character (ignored past the maximum length).
    pub fn push(&mut self, c: char) {
        if self.text.len() < MAX_PASSWORD_LEN && c.is_ascii() {
            self.text.push(c as u8);
            self.status = Status::Idle;
        }
    }

    /// Delete the last character.
    pub fn backspace(&mut self) {
        if let Some(last) = self.text.last_mut() {
            *last = 0;
            self.text.pop();
        }
    }

    /// Erase the typed text, overwriting it first so it does not linger.
    pub fn wipe(&mut self) {
        self.text.fill(0);
        self.text.clear();
    }

    /// Forget the typed text and any message (new user, back, re-lock).
    pub fn reset(&mut self) {
        self.wipe();
        self.status = Status::Idle;
    }

    /// Try the typed password. `check` receives it and says whether it is
    /// right; it is not called while a cooldown runs.
    pub fn submit(
        &mut self,
        throttle: &mut Throttle,
        now_ms: u64,
        check: impl FnOnce(&str) -> Result<(), AuthError>,
    ) -> Submit {
        if throttle.is_blocked(now_ms) {
            self.wipe();
            return Submit::Blocked;
        }
        let verdict = check(core::str::from_utf8(&self.text).unwrap_or(""));
        self.wipe();
        match verdict {
            Ok(()) => {
                throttle.record_success();
                self.status = Status::Idle;
                Submit::Accepted
            }
            Err(AuthError::InvalidPassword) => {
                let left = match throttle.record_failure(now_ms) {
                    Strike::Retry(left) => left,
                    Strike::CooldownStarted => 0,
                };
                self.status = Status::Wrong(left);
                Submit::Rejected
            }
            Err(other) => {
                self.status = Status::Refused(other);
                Submit::Rejected
            }
        }
    }
}

impl Default for PasswordForm {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::users::throttle::{COOLDOWN_MS, MAX_FAILURES};

    fn wrong(_: &str) -> Result<(), AuthError> {
        Err(AuthError::InvalidPassword)
    }

    fn typed(text: &str) -> PasswordForm {
        let mut form = PasswordForm::new();
        text.chars().for_each(|c| form.push(c));
        form
    }

    #[test]
    fn correct_password_is_accepted_and_text_wiped() {
        let mut form = typed("secret");
        let mut t = Throttle::new();
        let out = form.submit(&mut t, 0, |pw| {
            assert_eq!(pw, "secret");
            Ok(())
        });
        assert_eq!(out, Submit::Accepted);
        assert!(form.is_empty());
    }

    #[test]
    fn wrong_password_reports_tries_left_and_clears_field() {
        let mut form = typed("nope");
        let mut t = Throttle::new();
        assert_eq!(form.submit(&mut t, 0, wrong), Submit::Rejected);
        assert_eq!(form.status(), Status::Wrong(MAX_FAILURES - 1));
        assert!(form.is_empty());
    }

    #[test]
    fn locked_wrong_x5_then_cooldown_then_unlock() {
        let mut form = PasswordForm::new();
        let mut t = Throttle::new();
        for _ in 0..MAX_FAILURES {
            assert_eq!(form.submit(&mut t, 1_000, wrong), Submit::Rejected);
        }
        assert_eq!(form.status(), Status::Wrong(0));
        let mut called = false;
        let blocked = form.submit(&mut t, 2_000, |_| {
            called = true;
            Ok(())
        });
        assert_eq!(blocked, Submit::Blocked);
        assert!(!called, "the check must not run during a cooldown");
        let after = 1_000 + COOLDOWN_MS;
        assert_eq!(form.submit(&mut t, after, |_| Ok(())), Submit::Accepted);
    }

    #[test]
    fn refused_account_does_not_count_as_a_strike() {
        let mut form = PasswordForm::new();
        let mut t = Throttle::new();
        let out = form.submit(&mut t, 0, |_| Err(AuthError::AccountLocked));
        assert_eq!(out, Submit::Rejected);
        assert_eq!(form.status(), Status::Refused(AuthError::AccountLocked));
        assert_eq!(t, Throttle::new());
    }

    #[test]
    fn typing_clears_the_previous_message() {
        let mut form = PasswordForm::new();
        form.submit(&mut Throttle::new(), 0, wrong);
        form.push('a');
        assert_eq!(form.status(), Status::Idle);
    }

    #[test]
    fn length_is_capped_and_backspace_removes() {
        let mut form = PasswordForm::new();
        for _ in 0..MAX_PASSWORD_LEN + 10 {
            form.push('x');
        }
        assert_eq!(form.len(), MAX_PASSWORD_LEN);
        form.backspace();
        assert_eq!(form.len(), MAX_PASSWORD_LEN - 1);
    }

    #[test]
    fn scancodes_map_with_shift_and_caps() {
        assert_eq!(key_from_scancode(0x1E, false, false), Key::Char('a'));
        assert_eq!(key_from_scancode(0x1E, true, false), Key::Char('A'));
        assert_eq!(key_from_scancode(0x1E, false, true), Key::Char('A'));
        assert_eq!(key_from_scancode(0x1E, true, true), Key::Char('a'));
        assert_eq!(key_from_scancode(0x02, false, true), Key::Char('1'));
        assert_eq!(key_from_scancode(0x02, true, false), Key::Char('!'));
    }

    #[test]
    fn control_scancodes_map_to_actions() {
        assert_eq!(key_from_scancode(SC_ENTER, false, false), Key::Enter);
        assert_eq!(
            key_from_scancode(SC_BACKSPACE, false, false),
            Key::Backspace
        );
        assert_eq!(key_from_scancode(SC_CAPS_LOCK, false, false), Key::CapsLock);
        assert_eq!(key_from_scancode(0x5B, false, false), Key::Other);
    }
}
