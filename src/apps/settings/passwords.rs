//! Password flows of the Users page: change (or remove) the signed-in
//! user's password, and give a new account an optional one. Each step is one
//! dialog; [`plan`] decides what to do with an answer and is pure, so the
//! whole flow is host-tested without a window.

use super::actions::{queue_dialog, queue_secret};
use super::state::{Pending, State};
use crate::users::policy::{validate_change, Change};
use crate::wm::dialog::DialogResult;
use alloc::string::String;

/// Dialog title for the change-password flow.
const CHANGE_TITLE: &str = "Change password";
/// Dialog title for choosing a password for a new account.
const NEW_USER_TITLE: &str = "New account password";

/// Where a password flow is waiting for an answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    /// Asked for the current password.
    Current { user: String },
    /// Asked for the new password (empty removes it).
    New { user: String, current: String },
    /// Asked to repeat the new password.
    Confirm { user: String, new: String },
    /// Asked whether to really remove the password.
    RemoveConfirm { user: String },
    /// Asked for the password of the account being created (empty = none).
    NewUserPassword { name: String },
    /// Asked to repeat the new account's password.
    NewUserConfirm { name: String, password: String },
}

/// What the dialog answered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Answer {
    /// Prompt text, or None when cancelled.
    Text(Option<String>),
    /// Yes/no confirmation.
    Yes(bool),
}

/// Kind of dialog to ask next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AskKind {
    /// Hidden-text prompt.
    Secret,
    /// Yes/no.
    Confirm,
}

/// What to do after an answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Plan {
    /// Ask another question.
    Ask {
        step: Step,
        kind: AskKind,
        title: &'static str,
        message: String,
    },
    /// Store `password` for `user`.
    Set { user: String, password: String },
    /// Remove the password of `user`.
    Remove { user: String },
    /// Create account `name` with an optional password.
    Create {
        name: String,
        password: Option<String>,
    },
    /// Stop and show this line (empty = silently cancelled).
    Notice(String),
}

fn notice(text: &str) -> Plan {
    Plan::Notice(String::from(text))
}

fn ask_secret(step: Step, title: &'static str, message: &str) -> Plan {
    Plan::Ask {
        step,
        kind: AskKind::Secret,
        title,
        message: String::from(message),
    }
}

/// Decide the next move. `verify(user, password)` checks a current password.
pub fn plan(step: Step, answer: Answer, verify: &dyn Fn(&str, &str) -> bool) -> Plan {
    match (step, answer) {
        (_, Answer::Text(None)) | (_, Answer::Yes(false)) => notice(""),
        (Step::Current { user }, Answer::Text(Some(current))) => {
            if !verify(&user, &current) {
                return notice("The current password is wrong.");
            }
            let message = "New password (leave empty to remove it):";
            ask_secret(Step::New { user, current }, CHANGE_TITLE, message)
        }
        (Step::New { user, .. }, Answer::Text(Some(new))) if new.is_empty() => Plan::Ask {
            message: alloc::format!(
                "Remove the password for '{user}'? Anyone at the keyboard will be able to log in."
            ),
            step: Step::RemoveConfirm { user },
            kind: AskKind::Confirm,
            title: CHANGE_TITLE,
        },
        (Step::New { user, .. }, Answer::Text(Some(new))) => ask_secret(
            Step::Confirm { user, new },
            CHANGE_TITLE,
            "Repeat the new password:",
        ),
        (Step::Confirm { user, new }, Answer::Text(Some(again))) => {
            match validate_change(&new, &again) {
                Ok(Change::Set) => Plan::Set {
                    user,
                    password: new,
                },
                Ok(Change::Remove) => Plan::Remove { user },
                Err(why) => notice(why.message()),
            }
        }
        (Step::RemoveConfirm { user }, Answer::Yes(true)) => Plan::Remove { user },
        (Step::NewUserPassword { name }, Answer::Text(Some(password))) => {
            plan_new_user_password(name, password)
        }
        (Step::NewUserConfirm { name, password }, Answer::Text(Some(again))) => {
            match validate_change(&password, &again) {
                Ok(_) => Plan::Create {
                    name,
                    password: Some(password),
                },
                Err(why) => notice(why.message()),
            }
        }
        _ => notice(""),
    }
}

fn plan_new_user_password(name: String, password: String) -> Plan {
    if password.is_empty() {
        return Plan::Create {
            name,
            password: None,
        };
    }
    match validate_change(&password, &password) {
        Ok(_) => ask_secret(
            Step::NewUserConfirm { name, password },
            NEW_USER_TITLE,
            "Repeat the password:",
        ),
        Err(why) => notice(why.message()),
    }
}

fn verify_current(user: &str, password: &str) -> bool {
    crate::users::auth::check_password(user, password).is_ok()
}

/// Start the change-password flow for the signed-in user.
pub fn start(st: &mut State) {
    use crate::users::auth::{password_state, PasswordState};
    let user = crate::users::current_user();
    let step = match password_state(&user) {
        PasswordState::Set => Step::Current { user },
        _ => Step::New {
            user,
            current: String::new(),
        },
    };
    let message = match step {
        Step::Current { .. } => "Current password:",
        _ => "New password (leave empty to keep none):",
    };
    st.pending = Some(Pending::Password(step));
    queue_secret(st, CHANGE_TITLE, message);
}

/// After the name prompt of "Add user": validate the name, then ask for an
/// optional password. Returns the notice to show when it stops here.
pub fn begin_new_user(st: &mut State, name: &str) -> String {
    let existing = super::accounts::existing_names();
    if let Err(why) = super::users::validate_new_name(name, &existing) {
        return why.message();
    }
    let step = Step::NewUserPassword {
        name: String::from(name),
    };
    st.pending = Some(Pending::Password(step));
    queue_secret(st, NEW_USER_TITLE, "Password (leave empty for none):");
    String::new()
}

fn answer_from(result: DialogResult) -> Answer {
    match result {
        DialogResult::Prompt(text) => Answer::Text(text),
        DialogResult::Confirm(yes) => Answer::Yes(yes),
        _ => Answer::Text(None),
    }
}

fn apply_set(user: &str, password: &str) -> String {
    match crate::users::auth::set_password(user, password) {
        Ok(()) => String::from(if password.is_empty() {
            "Password removed."
        } else {
            "Password changed."
        }),
        Err(e) => alloc::format!("Could not save the password: {e}"),
    }
}

fn apply_create(name: &str, password: Option<&str>) -> String {
    if let Err(reason) = super::accounts::add_user(name) {
        return reason;
    }
    match password {
        Some(pw) => match crate::users::auth::set_password(name, pw) {
            Ok(()) => alloc::format!("User '{name}' created with a password."),
            Err(e) => alloc::format!("User '{name}' created, but the password failed: {e}"),
        },
        None => alloc::format!("User '{name}' created."),
    }
}

/// Process the dialog answer for `step`; returns the page notice.
pub fn advance(st: &mut State, step: Step, result: DialogResult) -> String {
    match plan(step, answer_from(result), &verify_current) {
        Plan::Ask {
            step,
            kind,
            title,
            message,
        } => {
            st.pending = Some(Pending::Password(step));
            match kind {
                AskKind::Secret => queue_secret(st, title, &message),
                AskKind::Confirm => queue_dialog(st, title, &message, false),
            }
            String::new()
        }
        Plan::Set { user, password } => apply_set(&user, &password),
        Plan::Remove { user } => apply_set(&user, ""),
        Plan::Create { name, password } => apply_create(&name, password.as_deref()),
        Plan::Notice(text) => text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(text: &str) -> String {
        String::from(text)
    }

    fn text(t: &str) -> Answer {
        Answer::Text(Some(s(t)))
    }

    fn accept(_: &str, pw: &str) -> bool {
        pw == "old1"
    }

    fn current(user: &str) -> Step {
        Step::Current { user: s(user) }
    }

    #[test]
    fn wrong_current_password_stops_the_flow() {
        let out = plan(current("u"), text("nope"), &accept);
        assert_eq!(out, Plan::Notice(s("The current password is wrong.")));
    }

    #[test]
    fn right_current_password_asks_for_the_new_one() {
        let out = plan(current("u"), text("old1"), &accept);
        assert!(matches!(
            out,
            Plan::Ask {
                step: Step::New { .. },
                kind: AskKind::Secret,
                ..
            }
        ));
    }

    #[test]
    fn empty_new_password_asks_to_confirm_removal() {
        let step = Step::New {
            user: s("u"),
            current: s("old1"),
        };
        let out = plan(step, text(""), &accept);
        assert!(matches!(
            out,
            Plan::Ask {
                step: Step::RemoveConfirm { .. },
                kind: AskKind::Confirm,
                ..
            }
        ));
    }

    #[test]
    fn removal_needs_a_yes() {
        let step = Step::RemoveConfirm { user: s("u") };
        assert_eq!(
            plan(step.clone(), Answer::Yes(true), &accept),
            Plan::Remove { user: s("u") }
        );
        assert_eq!(plan(step, Answer::Yes(false), &accept), Plan::Notice(s("")));
    }

    #[test]
    fn matching_confirmation_sets_the_password() {
        let step = Step::Confirm {
            user: s("u"),
            new: s("abcd"),
        };
        let out = plan(step, text("abcd"), &accept);
        assert_eq!(
            out,
            Plan::Set {
                user: s("u"),
                password: s("abcd")
            }
        );
    }

    #[test]
    fn mismatched_or_short_passwords_are_refused() {
        let step = Step::Confirm {
            user: s("u"),
            new: s("abcd"),
        };
        assert!(
            matches!(plan(step, text("abce"), &accept), Plan::Notice(m) if m.contains("do not match"))
        );
        let short = Step::Confirm {
            user: s("u"),
            new: s("ab"),
        };
        assert!(
            matches!(plan(short, text("ab"), &accept), Plan::Notice(m) if m.contains("at least 4"))
        );
    }

    #[test]
    fn cancelling_any_dialog_ends_quietly() {
        let step = Step::New {
            user: s("u"),
            current: s(""),
        };
        assert_eq!(plan(step, Answer::Text(None), &accept), Plan::Notice(s("")));
    }

    #[test]
    fn new_user_without_password_is_created_directly() {
        let out = plan(Step::NewUserPassword { name: s("bob") }, text(""), &accept);
        assert_eq!(
            out,
            Plan::Create {
                name: s("bob"),
                password: None
            }
        );
    }

    #[test]
    fn new_user_password_is_confirmed_then_created() {
        let first = plan(
            Step::NewUserPassword { name: s("bob") },
            text("pass1"),
            &accept,
        );
        assert!(matches!(
            first,
            Plan::Ask {
                step: Step::NewUserConfirm { .. },
                ..
            }
        ));
        let step = Step::NewUserConfirm {
            name: s("bob"),
            password: s("pass1"),
        };
        let done = plan(step.clone(), text("pass1"), &accept);
        assert_eq!(
            done,
            Plan::Create {
                name: s("bob"),
                password: Some(s("pass1"))
            }
        );
        assert!(matches!(
            plan(step, text("other"), &accept),
            Plan::Notice(_)
        ));
    }
}
