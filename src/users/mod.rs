//! User and group management for LobsterOS

pub mod passwd;
pub mod shadow;
pub mod group;
pub mod auth;
pub mod session;

#[allow(unused_imports)]
pub use passwd::{PasswdEntry, parse_passwd, read_passwd_file};
#[allow(unused_imports)]
pub use shadow::{ShadowEntry, parse_shadow, read_shadow_file};
#[allow(unused_imports)]
pub use group::{GroupEntry, parse_group, read_group_file};
#[allow(unused_imports)]
pub use auth::{hash_password, verify_password, AuthError, login_user, change_password};
#[allow(unused_imports)]
pub use session::{init as session_init, allocate_session, deallocate_session, get_session, get_active_sessions, get_user_sessions, get_utmp, get_wtmp, find_session_by_tty, find_session_by_pid, Session, SessionType};