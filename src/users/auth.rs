//! Authentication for LobsterOS

use crate::users::passwd;
use crate::users::password::{self, Verdict};
use crate::users::shadow;
use alloc::string::String;

/// Milliseconds in a day, for shadow-file day counts.
const MS_PER_DAY: u64 = 86_400_000;

/// Authentication errors
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthError {
    UserNotFound,
    InvalidPassword,
    AccountLocked,
    AccountExpired,
    PasswordExpired,
    ShadowMismatch,
    InternalError,
    IoError,
}

impl From<crate::fs::vfs::VfsError> for AuthError {
    fn from(_: crate::fs::vfs::VfsError) -> Self {
        AuthError::IoError
    }
}

impl core::fmt::Display for AuthError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            AuthError::UserNotFound => write!(f, "User not found"),
            AuthError::InvalidPassword => write!(f, "Invalid password"),
            AuthError::AccountLocked => write!(f, "Account locked"),
            AuthError::AccountExpired => write!(f, "Account expired"),
            AuthError::PasswordExpired => write!(f, "Password expired"),
            AuthError::ShadowMismatch => write!(f, "Shadow entry mismatch"),
            AuthError::InternalError => write!(f, "Internal error"),
            AuthError::IoError => write!(f, "I/O error"),
        }
    }
}

/// Hash a password for storage: PBKDF2-HMAC-SHA-256 with a random salt
/// (`$pbkdf2-sha256$<iterations>$<salt>$<hash>`, see [`crate::users::password`]).
pub fn hash_password(password: &str) -> String {
    password::hash(password)
}

/// Verify a password against a stored hash (current or legacy `$sha256$`).
pub fn verify_password(password: &str, hash: &str) -> bool {
    password::verify(password, hash) != Verdict::Mismatch
}

/// True when the stored hash means "no password set" (empty field).
pub fn is_passwordless(stored_hash: &str) -> bool {
    stored_hash.is_empty()
}

/// Whether an account needs a password, derived from its stored hash field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasswordState {
    /// No password: an empty entry (or the legacy `!` placeholder written by
    /// older Settings and `useradd` builds) unlocks with an empty password.
    None,
    /// A password hash is stored.
    Set,
    /// Login disabled (`*`, or `!` in front of a hash).
    Locked,
}

impl PasswordState {
    /// Classify a shadow hash field.
    #[must_use]
    pub fn from_hash(stored: &str) -> Self {
        match stored {
            "" | "!" => Self::None,
            s if s.starts_with('*') || s.starts_with('!') => Self::Locked,
            _ => Self::Set,
        }
    }
}

/// True if `name` is the implicit desktop user or has a passwd entry.
pub fn account_exists(name: &str) -> bool {
    name == crate::users::DEFAULT_USER || passwd::user_exists(name)
}

/// Password state of an account; accounts without a shadow entry have none.
pub fn password_state(username: &str) -> PasswordState {
    shadow::find_shadow(username).map_or(PasswordState::None, |e| {
        PasswordState::from_hash(&e.password_hash)
    })
}

/// Pure decision of [`check_password`]: what to do with `password` given the
/// account's stored hash field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Accept as is.
    Accept,
    /// Accept, and re-hash the stored value in the current format.
    AcceptAndRehash,
    /// Refuse for this reason.
    Reject(AuthError),
}

/// Judge `password` against a stored hash field without touching the disk.
/// A passwordless account accepts only the empty password.
#[must_use]
pub fn decide(stored: &str, password: &str) -> Decision {
    match PasswordState::from_hash(stored) {
        PasswordState::Locked => Decision::Reject(AuthError::AccountLocked),
        PasswordState::None if password.is_empty() => Decision::Accept,
        PasswordState::None => Decision::Reject(AuthError::InvalidPassword),
        PasswordState::Set => match password::verify(password, stored) {
            Verdict::Mismatch => Decision::Reject(AuthError::InvalidPassword),
            Verdict::Match => Decision::Accept,
            Verdict::MatchNeedsRehash => Decision::AcceptAndRehash,
        },
    }
}

/// Check `password` for `username` (desktop and shell share this). A correct
/// password stored in the legacy format is transparently re-hashed.
pub fn check_password(username: &str, password: &str) -> Result<(), AuthError> {
    if !account_exists(username) {
        return Err(AuthError::UserNotFound);
    }
    let stored = shadow::find_shadow(username)
        .map(|e| e.password_hash)
        .unwrap_or_default();
    match decide(&stored, password) {
        Decision::Accept => Ok(()),
        Decision::Reject(why) => Err(why),
        Decision::AcceptAndRehash => {
            if set_password(username, password).is_err() {
                crate::println!("[auth] could not upgrade hash for {}", username);
            }
            Ok(())
        }
    }
}

/// Login a user with username and password
pub fn login_user(username: &str, password: &str) -> Result<passwd::PasswdEntry, AuthError> {
    let user = passwd::find_user(username).ok_or(AuthError::UserNotFound)?;
    check_password(username, password)?;
    let Some(entry) = shadow::find_shadow(username) else {
        return Ok(user);
    };
    let current_day = (crate::arch::timer::Timer::uptime_ms() / MS_PER_DAY) as i64;
    if entry.is_expired(current_day) {
        return Err(AuthError::AccountExpired);
    }
    Ok(user)
}

/// Change a user's password
pub fn change_password(
    username: &str,
    old_password: &str,
    new_password: &str,
) -> Result<(), AuthError> {
    check_password(username, old_password)?;
    set_password(username, new_password)
}

/// Check if user is root (UID 0)
pub fn is_root(uid: u32) -> bool {
    uid == 0
}

/// Set password directly (admin mode, skips old password verification).
/// An empty `new_password` removes the password. The hash is salted PBKDF2
/// and is written to /etc/shadow, which the persistent prefix map keeps on
/// the disk.
pub fn set_password(username: &str, new_password: &str) -> Result<(), AuthError> {
    if !account_exists(username) {
        return Err(AuthError::UserNotFound);
    }
    let new_hash = if new_password.is_empty() {
        String::new()
    } else {
        hash_password(new_password)
    };
    let today = (crate::arch::timer::Timer::uptime_ms() / MS_PER_DAY) as i64;
    let mut entries = shadow::read_shadow_file().unwrap_or_default();
    match entries.iter_mut().find(|e| e.username == username) {
        Some(entry) => {
            entry.password_hash = new_hash;
            entry.last_change = today;
        }
        None => entries.push(shadow::ShadowEntry::new(username, &new_hash)),
    }
    shadow::write_shadow_file(&entries)?;
    Ok(())
}

/// Check if user can sudo (in sudoers)
pub fn can_sudo(username: &str) -> bool {
    // In a real implementation, check /etc/sudoers
    // For now, allow root and users in wheel group
    if username == "root" {
        return true;
    }

    let groups = crate::users::group::get_user_groups(username);
    groups.iter().any(|g| g.name == "wheel" || g.name == "sudo")
}

/// Get effective UID for sudo
pub fn sudo_uid(username: &str, target_user: Option<&str>) -> Result<u32, AuthError> {
    if !can_sudo(username) {
        return Err(AuthError::InvalidPassword); // Not authorized
    }

    let target = target_user.unwrap_or("root");
    let target_entry = passwd::find_user(target).ok_or(AuthError::UserNotFound)?;

    Ok(target_entry.uid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_state_classifies_hash_fields() {
        assert_eq!(PasswordState::from_hash(""), PasswordState::None);
        assert_eq!(PasswordState::from_hash("!"), PasswordState::None);
        assert_eq!(PasswordState::from_hash("*"), PasswordState::Locked);
        assert_eq!(
            PasswordState::from_hash("!$pbkdf2-sha256$1$a$b"),
            PasswordState::Locked
        );
        assert_eq!(
            PasswordState::from_hash("$pbkdf2-sha256$1$a$b"),
            PasswordState::Set
        );
    }

    #[test]
    fn passwordless_account_accepts_only_empty_password() {
        assert_eq!(decide("", ""), Decision::Accept);
        assert_eq!(
            decide("", "x"),
            Decision::Reject(AuthError::InvalidPassword)
        );
    }

    #[test]
    fn locked_account_rejects_everything() {
        assert_eq!(decide("*", ""), Decision::Reject(AuthError::AccountLocked));
    }

    #[test]
    fn set_password_hash_accepts_right_and_rejects_wrong() {
        let stored = hash_password("s3cret");
        assert_eq!(decide(&stored, "s3cret"), Decision::Accept);
        assert_eq!(
            decide(&stored, "S3cret"),
            Decision::Reject(AuthError::InvalidPassword)
        );
        assert_eq!(
            decide(&stored, ""),
            Decision::Reject(AuthError::InvalidPassword)
        );
    }

    #[test]
    fn legacy_hash_is_accepted_and_flagged_for_rehash() {
        let legacy = "$sha256$5e884898da28047151d0e56f8dc6292773603d0d6aabbdd62a11ef721d1542d8";
        assert_eq!(decide(legacy, "password"), Decision::AcceptAndRehash);
        assert!(verify_password("password", legacy));
        assert!(!verify_password("nope", legacy));
    }

    #[test]
    fn garbage_hash_never_authenticates() {
        assert_eq!(
            decide("garbage", "garbage"),
            Decision::Reject(AuthError::InvalidPassword)
        );
    }
}
