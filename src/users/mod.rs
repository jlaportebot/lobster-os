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
