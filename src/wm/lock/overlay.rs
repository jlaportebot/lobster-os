//! The lock/login overlay as a state machine: which stage is showing, which
//! user is selected, and what a key or click does. No drawing and no global
//! state here, so every transition is host-tested.

use super::form::{Key, PasswordForm, Status, Submit};
use crate::users::auth::{AuthError, PasswordState};
use crate::users::throttle::Throttle;
use alloc::string::String;
use alloc::vec::Vec;

/// Why the overlay is up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// The signed-in user stepped away; only their password (or none) lifts it.
    Lock,
    /// Nobody is signed in: pick an account and log in.
    Greeter,
}

/// Which screen of the overlay shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// Account list (greeter only).
    Choose,
    /// Password entry for the selected account.
    Password,
}

/// One selectable account and whether it asks for a password.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    /// Login name.
    pub name: String,
    /// Password state at the time the overlay opened.
    pub password: PasswordState,
}

/// What the owner of the overlay must do after an input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Nothing to do beyond redrawing if needed.
    None,
    /// Authentication succeeded for this account: close the overlay.
    Done(String),
}

/// The overlay state.
pub struct Overlay {
    mode: Mode,
    stage: Stage,
    accounts: Vec<Account>,
    selected: usize,
    form: PasswordForm,
}

impl Overlay {
    /// Lock screen for `user`.
    #[must_use]
    pub fn lock(user: Account) -> Self {
        Self {
            mode: Mode::Lock,
            stage: Stage::Password,
            accounts: alloc::vec![user],
            selected: 0,
            form: PasswordForm::new(),
        }
    }

    /// Greeter listing `accounts` (must not be empty).
    #[must_use]
    pub fn greeter(accounts: Vec<Account>) -> Self {
        Self {
            mode: Mode::Greeter,
            stage: Stage::Choose,
            accounts,
            selected: 0,
            form: PasswordForm::new(),
        }
    }

    /// Why the overlay is up.
    #[must_use]
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Current screen.
    #[must_use]
    pub fn stage(&self) -> Stage {
        self.stage
    }

    /// Accounts on offer.
    #[must_use]
    pub fn accounts(&self) -> &[Account] {
        &self.accounts
    }

    /// Index of the highlighted account.
    #[must_use]
    pub fn selected(&self) -> usize {
        self.selected
    }

    /// The highlighted account.
    #[must_use]
    pub fn current(&self) -> Option<&Account> {
        self.accounts.get(self.selected)
    }

    /// The password form (for drawing).
    #[must_use]
    pub fn form(&self) -> &PasswordForm {
        &self.form
    }

    /// True when the selected account needs something typed.
    #[must_use]
    pub fn needs_typing(&self) -> bool {
        self.current()
            .is_some_and(|a| a.password != PasswordState::None)
    }

    /// Highlight account `index` and move on: straight to success for a
    /// passwordless account, else to password entry.
    pub fn choose(&mut self, index: usize) -> Outcome {
        let Some(account) = self.accounts.get(index) else {
            return Outcome::None;
        };
        self.selected = index;
        self.form.reset();
        if account.password == PasswordState::None {
            return Outcome::Done(account.name.clone());
        }
        self.stage = Stage::Password;
        Outcome::None
    }

    /// Handle a key. `check` verifies (user, password); `now_ms` is uptime.
    pub fn key(
        &mut self,
        key: Key,
        throttle: &mut Throttle,
        now_ms: u64,
        check: impl FnOnce(&str, &str) -> Result<(), AuthError>,
    ) -> Outcome {
        if key == Key::CapsLock {
            self.form.toggle_caps();
            return Outcome::None;
        }
        match self.stage {
            Stage::Choose => self.key_in_list(key),
            Stage::Password => self.key_in_form(key, throttle, now_ms, check),
        }
    }

    fn key_in_list(&mut self, key: Key) -> Outcome {
        match key {
            Key::Up => self.selected = self.selected.saturating_sub(1),
            Key::Down => self.selected = (self.selected + 1).min(self.accounts.len() - 1),
            Key::Enter => return self.choose(self.selected),
            _ => {}
        }
        Outcome::None
    }

    fn key_in_form(
        &mut self,
        key: Key,
        throttle: &mut Throttle,
        now_ms: u64,
        check: impl FnOnce(&str, &str) -> Result<(), AuthError>,
    ) -> Outcome {
        match key {
            Key::Char(c) => self.form.push(c),
            Key::Backspace => self.form.backspace(),
            Key::Enter => return self.submit(throttle, now_ms, check),
            Key::Escape => self.back(),
            _ => {}
        }
        Outcome::None
    }

    /// Submit the typed password (Enter or the Unlock button).
    pub fn submit(
        &mut self,
        throttle: &mut Throttle,
        now_ms: u64,
        check: impl FnOnce(&str, &str) -> Result<(), AuthError>,
    ) -> Outcome {
        let Some(name) = self.current().map(|a| a.name.clone()) else {
            return Outcome::None;
        };
        match self.form.submit(throttle, now_ms, |pw| check(&name, pw)) {
            Submit::Accepted => Outcome::Done(name),
            Submit::Rejected | Submit::Blocked => Outcome::None,
        }
    }

    /// Esc: clear the field, or return to the list in the greeter.
    fn back(&mut self) {
        let had_text = !self.form.is_empty();
        self.form.reset();
        if self.mode == Mode::Greeter && !had_text && self.accounts.len() > 1 {
            self.stage = Stage::Choose;
        }
    }

    /// True when the last attempt was refused for a reason other than a
    /// wrong password (locked or expired account).
    #[must_use]
    pub fn refusal(&self) -> Option<AuthError> {
        match self.form.status() {
            Status::Refused(why) => Some(why),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::users::throttle::{COOLDOWN_MS, MAX_FAILURES};

    fn acct(name: &str, password: PasswordState) -> Account {
        Account {
            name: String::from(name),
            password,
        }
    }

    fn right(user: &str, pw: &str) -> Result<(), AuthError> {
        if user == "alice" && pw == "pass" {
            Ok(())
        } else {
            Err(AuthError::InvalidPassword)
        }
    }

    fn type_in(o: &mut Overlay, t: &mut Throttle, text: &str, now: u64) {
        for c in text.chars() {
            o.key(Key::Char(c), t, now, right);
        }
    }

    #[test]
    fn lock_unlocks_with_the_right_password() {
        let mut o = Overlay::lock(acct("alice", PasswordState::Set));
        let mut t = Throttle::new();
        type_in(&mut o, &mut t, "pass", 0);
        assert_eq!(
            o.key(Key::Enter, &mut t, 0, right),
            Outcome::Done(String::from("alice"))
        );
    }

    #[test]
    fn lock_stays_locked_on_wrong_x5_then_cooldown_then_unlocks() {
        let mut o = Overlay::lock(acct("alice", PasswordState::Set));
        let mut t = Throttle::new();
        for _ in 0..MAX_FAILURES {
            type_in(&mut o, &mut t, "bad", 10);
            assert_eq!(o.key(Key::Enter, &mut t, 10, right), Outcome::None);
        }
        type_in(&mut o, &mut t, "pass", 20);
        assert_eq!(o.key(Key::Enter, &mut t, 20, right), Outcome::None);
        assert!(t.is_blocked(20));
        type_in(&mut o, &mut t, "pass", 10 + COOLDOWN_MS);
        let done = o.key(Key::Enter, &mut t, 10 + COOLDOWN_MS, right);
        assert_eq!(done, Outcome::Done(String::from("alice")));
    }

    #[test]
    fn passwordless_lock_unlocks_on_enter_with_empty_field() {
        let mut o = Overlay::lock(acct("alice", PasswordState::None));
        let mut t = Throttle::new();
        let ok = |_: &str, pw: &str| {
            if pw.is_empty() {
                Ok(())
            } else {
                Err(AuthError::InvalidPassword)
            }
        };
        assert!(!o.needs_typing());
        assert_eq!(
            o.key(Key::Enter, &mut t, 0, ok),
            Outcome::Done(String::from("alice"))
        );
    }

    #[test]
    fn escape_never_dismisses_the_lock_screen() {
        let mut o = Overlay::lock(acct("alice", PasswordState::Set));
        let mut t = Throttle::new();
        assert_eq!(o.key(Key::Escape, &mut t, 0, right), Outcome::None);
        assert_eq!(o.stage(), Stage::Password);
    }

    #[test]
    fn greeter_passwordless_account_logs_in_on_choose() {
        let mut o = Overlay::greeter(alloc::vec![
            acct("alice", PasswordState::Set),
            acct("bob", PasswordState::None),
        ]);
        assert_eq!(o.choose(1), Outcome::Done(String::from("bob")));
    }

    #[test]
    fn greeter_password_account_goes_to_password_stage_and_back() {
        let mut o = Overlay::greeter(alloc::vec![
            acct("alice", PasswordState::Set),
            acct("bob", PasswordState::None),
        ]);
        let mut t = Throttle::new();
        assert_eq!(o.choose(0), Outcome::None);
        assert_eq!(o.stage(), Stage::Password);
        o.key(Key::Escape, &mut t, 0, right);
        assert_eq!(o.stage(), Stage::Choose);
    }

    #[test]
    fn greeter_arrow_keys_clamp_to_the_list() {
        let mut o = Overlay::greeter(alloc::vec![
            acct("a", PasswordState::None),
            acct("b", PasswordState::None),
        ]);
        let mut t = Throttle::new();
        o.key(Key::Up, &mut t, 0, right);
        assert_eq!(o.selected(), 0);
        o.key(Key::Down, &mut t, 0, right);
        o.key(Key::Down, &mut t, 0, right);
        assert_eq!(o.selected(), 1);
    }

    #[test]
    fn caps_lock_toggles_without_touching_the_text() {
        let mut o = Overlay::lock(acct("alice", PasswordState::Set));
        let mut t = Throttle::new();
        o.key(Key::Char('a'), &mut t, 0, right);
        o.key(Key::CapsLock, &mut t, 0, right);
        assert!(o.form().caps());
        assert_eq!(o.form().len(), 1);
    }

    #[test]
    fn wrong_account_in_check_is_rejected() {
        let mut o = Overlay::lock(acct("mallory", PasswordState::Set));
        let mut t = Throttle::new();
        type_in(&mut o, &mut t, "pass", 0);
        assert_eq!(o.key(Key::Enter, &mut t, 0, right), Outcome::None);
    }
}
