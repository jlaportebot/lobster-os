//! User and group management for LobsterOS

pub mod auth;
pub mod group;
pub mod passwd;
pub mod session;
pub mod shadow;

#[allow(unused_imports)]
pub use auth::{change_password, hash_password, login_user, verify_password, AuthError};
#[allow(unused_imports)]
pub use group::{parse_group, read_group_file, GroupEntry};
#[allow(unused_imports)]
pub use passwd::{parse_passwd, read_passwd_file, PasswdEntry};
#[allow(unused_imports)]
pub use session::{
    allocate_session, deallocate_session, find_session_by_pid, find_session_by_tty,
    get_active_sessions, get_session, get_user_sessions, get_utmp, get_wtmp, init as session_init,
    Session, SessionType,
};
#[allow(unused_imports)]
pub use shadow::{parse_shadow, read_shadow_file, ShadowEntry};

/// Initialize user database — create default /etc/passwd, /etc/shadow, /etc/group if missing
pub fn init() {
    // Create default /etc/passwd if missing
    if read_passwd_file().is_err() {
        let default_users = alloc::vec![
            passwd::PasswdEntry::new("root", 0, 0, "/root", "/bin/sh"),
            passwd::PasswdEntry::new("nobody", 65534, 65534, "/nonexistent", "/bin/false"),
        ];
        let _ = passwd::write_passwd_file(&default_users);
    }

    // Create default /etc/shadow if missing
    if read_shadow_file().is_err() {
        // Root with no password (empty hash = no password required)
        let default_shadows = alloc::vec![
            shadow::ShadowEntry::new("root", ""),
            shadow::ShadowEntry::new("nobody", "*"),
        ];
        let _ = shadow::write_shadow_file(&default_shadows);
    }

    // Create default /etc/group if missing
    if read_group_file().is_err() {
        let default_groups = alloc::vec![
            group::GroupEntry::new("root", 0),
            group::GroupEntry::new("wheel", 10),
            group::GroupEntry::new("nogroup", 65534),
        ];
        let _ = group::write_group_file(&default_groups);
    }

    // Add root to wheel group
    if let Ok(mut groups) = read_group_file() {
        for g in &mut groups {
            if g.name == "wheel" && !g.has_member("root") {
                g.add_member("root");
            }
        }
        let _ = group::write_group_file(&groups);
    }
}
