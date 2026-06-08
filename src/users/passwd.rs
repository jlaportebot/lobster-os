//! /etc/passwd parser for LobsterOS

use crate::fs::vfs::read_file;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// A single entry in /etc/passwd
#[derive(Debug, Clone)]
pub struct PasswdEntry {
    pub username: String,
    pub password: String,        // Usually "x" or "*" (shadow passwords)
    pub uid: u32,
    pub gid: u32,
    pub gecos: String,           // Full name, room, phone, etc.
    pub home: String,
    pub shell: String,
}

impl PasswdEntry {
    pub fn new(username: &str, uid: u32, gid: u32, home: &str, shell: &str) -> Self {
        Self {
            username: username.to_string(),
            password: "x".to_string(),
            uid,
            gid,
            gecos: String::new(),
            home: home.to_string(),
            shell: shell.to_string(),
        }
    }

    pub fn is_system_user(&self) -> bool {
        self.uid < 1000
    }
}

/// Parse a single /etc/passwd line
pub fn parse_passwd_line(line: &str) -> Option<PasswdEntry> {
    if line.trim().is_empty() || line.starts_with('#') {
        return None;
    }

    let parts: Vec<&str> = line.split(':').collect();
    if parts.len() != 7 {
        return None;
    }

    let uid = parts[2].parse().ok()?;
    let gid = parts[3].parse().ok()?;

    Some(PasswdEntry {
        username: parts[0].to_string(),
        password: parts[1].to_string(),
        uid,
        gid,
        gecos: parts[4].to_string(),
        home: parts[5].to_string(),
        shell: parts[6].to_string(),
    })
}

/// Parse entire /etc/passwd content
pub fn parse_passwd(content: &str) -> Vec<PasswdEntry> {
    content
        .lines()
        .filter_map(parse_passwd_line)
        .collect()
}

/// Read and parse /etc/passwd from VFS
pub fn read_passwd_file() -> Result<Vec<PasswdEntry>, crate::fs::vfs::VfsError> {
    match read_file("/etc/passwd") {
        Ok(data) => {
            let content = core::str::from_utf8(&data).map_err(|_| crate::fs::vfs::VfsError::IoError)?;
            Ok(parse_passwd(content))
        }
        Err(e) => Err(e),
    }
}

/// Find a user by username
pub fn find_user(username: &str) -> Option<PasswdEntry> {
    read_passwd_file()
        .ok()?
        .into_iter()
        .find(|u| u.username == username)
}

/// Find a user by UID
pub fn find_user_by_uid(uid: u32) -> Option<PasswdEntry> {
    read_passwd_file()
        .ok()?
        .into_iter()
        .find(|u| u.uid == uid)
}

/// Get all users
pub fn get_all_users() -> Vec<PasswdEntry> {
    read_passwd_file().unwrap_or_default()
}

/// Check if user exists
pub fn user_exists(username: &str) -> bool {
    find_user(username).is_some()
}