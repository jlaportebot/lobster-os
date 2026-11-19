//! Validation for adding and removing accounts from the Users page.

use alloc::string::String;

/// Longest account name.
pub const MAX_NAME_LEN: usize = 32;
/// First uid handed to a regular account.
pub const FIRST_USER_UID: u32 = 1000;
/// Uids at or above this are system accounts such as `nobody`.
const SYSTEM_UID_CEILING: u32 = 60000;
/// Names that belong to the system or to the built-in desktop account.
const RESERVED: [&str; 7] = ["root", "nobody", "user", "daemon", "bin", "sys", "admin"];

/// Why a new account name was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameError {
    Empty,
    TooLong,
    BadChar(char),
    Reserved,
    Duplicate,
}

impl NameError {
    /// One-line explanation for the user.
    #[must_use]
    pub fn message(self) -> String {
        match self {
            NameError::Empty => String::from("Enter a user name."),
            NameError::TooLong => alloc::format!("Use at most {MAX_NAME_LEN} characters."),
            NameError::BadChar(c) => alloc::format!("'{c}' is not allowed in a user name."),
            NameError::Reserved => String::from("That name is reserved by the system."),
            NameError::Duplicate => String::from("A user with that name already exists."),
        }
    }
}

/// Why a removal was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoveError {
    Root,
    SystemAccount,
    CurrentUser,
    Unknown,
}

impl RemoveError {
    /// One-line explanation for the user.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            RemoveError::Root => "The root account cannot be removed.",
            RemoveError::SystemAccount => "System accounts cannot be removed.",
            RemoveError::CurrentUser => "You cannot remove the account you are using.",
            RemoveError::Unknown => "That user no longer exists.",
        }
    }
}

/// The name as typed, without surrounding whitespace.
#[must_use]
pub fn clean_name(raw: &str) -> &str {
    raw.trim()
}

/// Check `name` (already cleaned) against the naming rules and the names
/// in `existing`.
pub fn validate_new_name(name: &str, existing: &[String]) -> Result<(), NameError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(NameError::Empty);
    }
    if name.chars().count() > MAX_NAME_LEN {
        return Err(NameError::TooLong);
    }
    if let Some(bad) = name
        .chars()
        .find(|c| !c.is_ascii_alphanumeric() && *c != '_' && *c != '-')
    {
        return Err(NameError::BadChar(bad));
    }
    if RESERVED.iter().any(|r| r.eq_ignore_ascii_case(name)) {
        return Err(NameError::Reserved);
    }
    if existing.iter().any(|e| e.eq_ignore_ascii_case(name)) {
        return Err(NameError::Duplicate);
    }
    Ok(())
}

/// Check whether account `name` (with passwd uid `uid`, `None` when it is
/// not in the database) may be removed while `current` is logged in.
pub fn validate_removal(name: &str, uid: Option<u32>, current: &str) -> Result<(), RemoveError> {
    let Some(uid) = uid else {
        return Err(RemoveError::Unknown);
    };
    if uid == 0 {
        return Err(RemoveError::Root);
    }
    if !(FIRST_USER_UID..SYSTEM_UID_CEILING).contains(&uid) {
        return Err(RemoveError::SystemAccount);
    }
    if name == current {
        return Err(RemoveError::CurrentUser);
    }
    Ok(())
}

/// Lowest uid of at least [`FIRST_USER_UID`] above every regular uid in use.
#[must_use]
pub fn next_uid(taken: &[u32]) -> u32 {
    taken
        .iter()
        .filter(|uid| (FIRST_USER_UID..SYSTEM_UID_CEILING).contains(uid))
        .max()
        .map_or(FIRST_USER_UID, |highest| highest + 1)
}
#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    fn existing() -> Vec<String> {
        vec![String::from("root"), String::from("alice")]
    }

    #[test]
    fn accepts_a_normal_name() {
        assert_eq!(validate_new_name("bob", &existing()), Ok(()));
        assert_eq!(validate_new_name("dev_ops-2", &existing()), Ok(()));
    }

    #[test]
    fn rejects_empty_and_blank_names() {
        assert_eq!(validate_new_name("", &existing()), Err(NameError::Empty));
        assert_eq!(validate_new_name("   ", &existing()), Err(NameError::Empty));
    }

    #[test]
    fn rejects_duplicates_case_insensitively() {
        assert_eq!(
            validate_new_name("alice", &existing()),
            Err(NameError::Duplicate)
        );
        assert_eq!(
            validate_new_name("ALICE", &existing()),
            Err(NameError::Duplicate)
        );
    }

    #[test]
    fn rejects_reserved_names() {
        for name in ["root", "nobody", "user", "daemon"] {
            assert_eq!(
                validate_new_name(name, &[]),
                Err(NameError::Reserved),
                "{name}"
            );
        }
    }

    #[test]
    fn rejects_bad_characters() {
        assert_eq!(validate_new_name("a b", &[]), Err(NameError::BadChar(' ')));
        assert_eq!(validate_new_name("a/b", &[]), Err(NameError::BadChar('/')));
        assert_eq!(validate_new_name("a:b", &[]), Err(NameError::BadChar(':')));
    }

    #[test]
    fn rejects_overlong_names() {
        let long = "x".repeat(MAX_NAME_LEN + 1);
        assert_eq!(validate_new_name(&long, &[]), Err(NameError::TooLong));
        assert_eq!(validate_new_name(&"x".repeat(MAX_NAME_LEN), &[]), Ok(()));
    }

    #[test]
    fn trims_surrounding_whitespace_before_validating() {
        assert_eq!(clean_name("  carol \n"), "carol");
        assert_eq!(
            validate_new_name(&clean_name(" alice "), &existing()),
            Err(NameError::Duplicate)
        );
    }

    #[test]
    fn removal_refuses_root_and_system_accounts() {
        assert_eq!(
            validate_removal("root", Some(0), "user"),
            Err(RemoveError::Root)
        );
        assert_eq!(
            validate_removal("nobody", Some(65534), "user"),
            Err(RemoveError::SystemAccount)
        );
        assert_eq!(
            validate_removal("daemon", Some(1), "user"),
            Err(RemoveError::SystemAccount)
        );
    }

    #[test]
    fn removal_refuses_the_current_user() {
        assert_eq!(
            validate_removal("alice", Some(1000), "alice"),
            Err(RemoveError::CurrentUser)
        );
    }

    #[test]
    fn removal_allows_a_regular_other_user() {
        assert_eq!(validate_removal("alice", Some(1000), "user"), Ok(()));
    }

    #[test]
    fn removal_of_an_unknown_user_fails() {
        assert_eq!(
            validate_removal("ghost", None, "user"),
            Err(RemoveError::Unknown)
        );
    }

    #[test]
    fn next_uid_starts_at_1000_and_skips_taken_ones() {
        assert_eq!(next_uid(&[0, 65534]), 1000);
        assert_eq!(next_uid(&[0, 1000, 1001, 65534]), 1002);
    }
}
