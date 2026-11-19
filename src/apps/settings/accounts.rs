//! Adding and removing accounts on behalf of the Users page.

use super::users::{next_uid, validate_new_name, validate_removal, RemoveError};
use crate::users::{group, passwd, shadow};
use alloc::string::String;
use alloc::vec::Vec;

/// Login shell given to new accounts.
const DEFAULT_SHELL: &str = "/bin/sh";
/// Shadow hash meaning "locked: no password set".
const LOCKED_HASH: &str = "!";

/// Names of every account that exists.
#[must_use]
pub fn existing_names() -> Vec<String> {
    let mut names: Vec<String> = passwd::get_all_users()
        .into_iter()
        .map(|e| e.username)
        .collect();
    names.push(String::from(crate::users::DEFAULT_USER));
    names
}

/// Create account `name` with a home directory under /home and /sd/home.
pub fn add_user(name: &str) -> Result<(), String> {
    validate_new_name(name, &existing_names()).map_err(|e| e.message())?;
    let uids: Vec<u32> = passwd::get_all_users().iter().map(|e| e.uid).collect();
    let uid = next_uid(&uids);
    let home = alloc::format!("/home/{name}");
    let mut entries = passwd::read_passwd_file().unwrap_or_default();
    entries.push(passwd::PasswdEntry::new(
        name,
        uid,
        uid,
        &home,
        DEFAULT_SHELL,
    ));
    passwd::write_passwd_file(&entries)
        .map_err(|e| alloc::format!("Could not write passwd: {e:?}"))?;
    passwd::persist();
    store_shadow_and_group(name, uid);
    make_home(name);
    Ok(())
}

fn store_shadow_and_group(name: &str, gid: u32) {
    let mut shadows = shadow::read_shadow_file().unwrap_or_default();
    shadows.push(shadow::ShadowEntry::new(name, LOCKED_HASH));
    let _ = shadow::write_shadow_file(&shadows);
    let mut groups = group::read_group_file().unwrap_or_default();
    groups.push(group::GroupEntry::new(name, gid));
    let _ = group::write_group_file(&groups);
}

fn make_home(name: &str) {
    use crate::fs::vfs::create_dir;
    for dir in ["/home", "/sd/home"] {
        let _ = create_dir(dir);
        let _ = create_dir(&alloc::format!("{dir}/{name}"));
    }
}

/// Remove account `name` (its home directory is kept, like `userdel`).
pub fn remove_user(name: &str) -> Result<(), String> {
    let uid = passwd::find_user(name).map(|e| e.uid);
    validate_removal(name, uid, &crate::users::current_user()).map_err(RemoveError::message)?;
    let mut entries = passwd::read_passwd_file().unwrap_or_default();
    entries.retain(|e| e.username != name);
    passwd::write_passwd_file(&entries)
        .map_err(|e| alloc::format!("Could not write passwd: {e:?}"))?;
    passwd::persist();
    let mut shadows = shadow::read_shadow_file().unwrap_or_default();
    shadows.retain(|s| s.username != name);
    let _ = shadow::write_shadow_file(&shadows);
    let mut groups = group::read_group_file().unwrap_or_default();
    groups.retain(|g| g.name != name);
    let _ = group::write_group_file(&groups);
    Ok(())
}

/// One row of the Users page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserRow {
    pub name: String,
    pub uid: Option<u32>,
    pub home: String,
    pub is_current: bool,
}

/// Accounts to list: root, regular users and the implicit desktop user.
#[must_use]
pub fn rows() -> Vec<UserRow> {
    let current = crate::users::current_user();
    let mut rows: Vec<UserRow> = passwd::get_all_users()
        .into_iter()
        .filter(|e| e.uid == 0 || (1000..60000).contains(&e.uid))
        .map(|e| UserRow {
            is_current: e.username == current,
            uid: Some(e.uid),
            home: e.home,
            name: e.username,
        })
        .collect();
    if !rows.iter().any(|r| r.name == crate::users::DEFAULT_USER) {
        rows.push(UserRow {
            name: String::from(crate::users::DEFAULT_USER),
            uid: None,
            home: alloc::format!("/home/{}", crate::users::DEFAULT_USER),
            is_current: current == crate::users::DEFAULT_USER,
        });
    }
    rows
}
