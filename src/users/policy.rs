//! Rules for choosing a new password, shared by Settings, the shell and the
//! lock screen so they all accept and refuse the same things.

/// Shortest accepted password. Deliberately short: this is a single-seat
/// desktop screen lock, the PBKDF2 cost plus the 5-try/30 s lockout is the
/// brute-force defence, and a long minimum on an on-screen-keyboard-less
/// emulator mostly pushes people to remove the password entirely.
pub const MIN_PASSWORD_LEN: usize = 4;
/// Longest accepted password, in bytes (bounds PBKDF2 input and the field).
pub const MAX_PASSWORD_LEN: usize = 64;

/// What a validated request asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    /// Store the new password.
    Set,
    /// Remove the password (both fields empty); callers confirm first.
    Remove,
}

/// Why a password request was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyError {
    /// Shorter than [`MIN_PASSWORD_LEN`].
    TooShort,
    /// Longer than [`MAX_PASSWORD_LEN`].
    TooLong,
    /// New and confirmation differ.
    Mismatch,
    /// Contains a control character.
    BadCharacter,
}

impl PolicyError {
    /// Text for the dialog or shell.
    #[must_use]
    pub fn message(self) -> &'static str {
        match self {
            Self::TooShort => "Password must be at least 4 characters",
            Self::TooLong => "Password must be at most 64 characters",
            Self::Mismatch => "The two passwords do not match",
            Self::BadCharacter => "Password contains an unsupported character",
        }
    }
}

/// Validate a new password and its confirmation.
pub fn validate_change(new: &str, confirm: &str) -> Result<Change, PolicyError> {
    if new != confirm {
        return Err(PolicyError::Mismatch);
    }
    if new.is_empty() {
        return Ok(Change::Remove);
    }
    if new.chars().any(char::is_control) {
        return Err(PolicyError::BadCharacter);
    }
    match new.len() {
        n if n < MIN_PASSWORD_LEN => Err(PolicyError::TooShort),
        n if n > MAX_PASSWORD_LEN => Err(PolicyError::TooLong),
        _ => Ok(Change::Set),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matching_passwords_of_valid_length_are_accepted() {
        assert_eq!(validate_change("abcd", "abcd"), Ok(Change::Set));
    }

    #[test]
    fn mismatch_is_reported_before_length() {
        assert_eq!(validate_change("ab", "abc"), Err(PolicyError::Mismatch));
    }

    #[test]
    fn short_and_long_passwords_are_refused() {
        assert_eq!(validate_change("abc", "abc"), Err(PolicyError::TooShort));
        let long = "x".repeat(MAX_PASSWORD_LEN + 1);
        assert_eq!(validate_change(&long, &long), Err(PolicyError::TooLong));
    }

    #[test]
    fn two_empty_fields_mean_remove() {
        assert_eq!(validate_change("", ""), Ok(Change::Remove));
    }

    #[test]
    fn control_characters_are_refused() {
        assert_eq!(
            validate_change("ab\tcd", "ab\tcd"),
            Err(PolicyError::BadCharacter)
        );
    }
}
