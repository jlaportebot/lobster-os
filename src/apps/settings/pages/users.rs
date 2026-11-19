//! Users: accounts from passwd, add and remove.

use super::{heading, note, setting, Flow};
use crate::apps::settings::accounts::{self, UserRow};
use crate::apps::settings::model::Action;
use crate::apps::settings::state::State;
use crate::apps::settings::users::validate_removal;
use crate::display::icons::Glyph;
use crate::users::auth::PasswordState;
use crate::wm::controls::{ButtonKind, Ui};
use alloc::string::String;

/// Height of an account row.
const USER_ROW_H: u32 = 64;
/// Width of the Add user button cell.
const ADD_W: u32 = 140;

pub fn draw(ui: &mut Ui, flow: &mut Flow, st: &mut State) {
    heading(ui, flow, "ACCOUNTS");
    let current = crate::users::current_user();
    for (i, row) in accounts::rows().iter().enumerate() {
        let r = flow.take(USER_ROW_H);
        ui.card(r);
        let removable = validate_removal(&row.name, row.uid, &current).is_ok();
        let trailing = Some((
            Action::RemoveUser(i as u8).key(),
            "Remove",
            ButtonKind::Danger,
            removable,
        ));
        ui.list_row(r, Some(Glyph::Home), &title(row), &subtitle(row), trailing);
    }
    flow.space(8);
    heading(ui, flow, "MANAGE");
    let add = setting(
        ui,
        flow,
        "Add a user",
        "Creates the account and its home folder",
        ADD_W,
    );
    ui.button(
        Action::AddUser.key(),
        add,
        "Add user",
        ButtonKind::Primary,
        true,
    );
    let pw = setting(
        ui,
        flow,
        "Change password",
        "Leave the new password empty to remove it",
        ADD_W,
    );
    ui.button(
        Action::ChangePassword.key(),
        pw,
        "Change password",
        ButtonKind::Secondary,
        true,
    );
    note(ui, flow, &st.notice);
}

fn title(row: &UserRow) -> String {
    if row.is_current {
        alloc::format!("{} (you)", row.name)
    } else {
        row.name.clone()
    }
}

fn subtitle(row: &UserRow) -> String {
    let password = match crate::users::auth::password_state(&row.name) {
        PasswordState::Set => "password set",
        PasswordState::None => "no password",
        PasswordState::Locked => "locked",
    };
    match row.uid {
        Some(uid) => alloc::format!("uid {}  \u{b7}  {}  \u{b7}  {}", uid, row.home, password),
        None => alloc::format!(
            "desktop account  \u{b7}  {}  \u{b7}  {}",
            row.home,
            password
        ),
    }
}
