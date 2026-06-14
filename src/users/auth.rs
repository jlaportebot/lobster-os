//! Authentication for LobsterOS

use crate::users::passwd;
use crate::users::shadow;
use alloc::format;
use alloc::string::String;

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

/// Hash a password using Argon2id
/// Returns hash in format: $argon2id$v=19$m=65536,t=3,p=4$salt$hash
pub fn hash_password(password: &str) -> String {
    // In a real implementation, this would use argon2 crate
    // For now, use a simple placeholder hash format
    // Real implementation would be:
    // let salt = generate_salt();
    // let hash = argon2::hash(password, &salt);
    // format!("$argon2id$v=19$m=65536,t=3,p=4${}${}$", salt_b64, hash_b64)

    // Placeholder: simple hash using SHA256
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(password.as_bytes());
    let hash = hasher.finalize();
    format!("$sha256${}", encode_hex(&hash))
}

fn encode_hex(bytes: &[u8]) -> String {
    let mut result = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        result.push(char::from_digit((b >> 4) as u32, 16).unwrap());
        result.push(char::from_digit((b & 0xF) as u32, 16).unwrap());
    }
    result
}

/// Verify a password against a hash
/// Supports: $argon2id$... and $sha256$...
pub fn verify_password(password: &str, hash: &str) -> bool {
    if hash.starts_with("$argon2id$") {
        // Argon2id verification would go here
        // For now, just compare
        hash_password(password) == hash
    } else if hash.starts_with("$sha256$") {
        hash_password(password) == hash
    } else {
        // Legacy or unknown format
        false
    }
}

/// Login a user with username and password
pub fn login_user(username: &str, password: &str) -> Result<passwd::PasswdEntry, AuthError> {
    // Find user in /etc/passwd
    let user = passwd::find_user(username).ok_or(AuthError::UserNotFound)?;

    // Find shadow entry
    let shadow_entry = shadow::find_shadow(username).ok_or(AuthError::ShadowMismatch)?;

    // Check if account is locked
    if shadow_entry.is_locked() {
        return Err(AuthError::AccountLocked);
    }

    // Check if account is expired
    let current_day = (crate::arch::timer::Timer::uptime_ms() / 86400000) as i64;
    if shadow_entry.is_expired(current_day) {
        return Err(AuthError::AccountExpired);
    }

    // Verify password
    if !verify_password(password, &shadow_entry.password_hash) {
        return Err(AuthError::InvalidPassword);
    }

    // Check if password is expired
    if shadow_entry.last_change > 0 && shadow_entry.max_age > 0 {
        let expiry = shadow_entry.last_change + shadow_entry.max_age;
        if current_day > expiry {
            return Err(AuthError::PasswordExpired);
        }
    }

    Ok(user)
}

/// Change a user's password
pub fn change_password(
    username: &str,
    old_password: &str,
    new_password: &str,
) -> Result<(), AuthError> {
    // Verify old password first
    login_user(username, old_password)?;

    // Hash new password
    let new_hash = hash_password(new_password);

    // Update /etc/shadow
    let mut entries = crate::users::shadow::read_shadow_file().unwrap_or_default();
    for entry in &mut entries {
        if entry.username == username {
            entry.password_hash = new_hash;
            entry.last_change = (crate::arch::timer::Timer::uptime_ms() / 86400000) as i64; // Approximate days since epoch
            break;
        }
    }
    crate::users::shadow::write_shadow_file(&entries)?;
    crate::println!(
        "[auth] Password changed for {} and written to /etc/shadow",
        username
    );
    Ok(())
}

/// Check if user is root (UID 0)
pub fn is_root(uid: u32) -> bool {
    uid == 0
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
