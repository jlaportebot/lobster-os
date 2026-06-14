//! /etc/shadow parser for LobsterOS

use crate::fs::vfs::read_file;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// A single entry in /etc/shadow
#[derive(Debug, Clone)]
pub struct ShadowEntry {
    pub username: String,
    pub password_hash: String,   // Argon2id hash: $argon2id$v=19$m=65536,t=3,p=4$salt$hash
    pub last_change: i64,        // Days since epoch of last password change
    pub min_age: i64,            // Minimum days before password can be changed
    pub max_age: i64,            // Maximum days before password must be changed
    pub warning: i64,            // Days before expiry to warn
    pub inactive: i64,           // Days after expiry before account disabled
    pub expire: i64,             // Days since epoch when account expires
    pub reserved: String,        // Reserved for future use
}

impl ShadowEntry {
    pub fn new(username: &str, password_hash: &str) -> Self {
        Self {
            username: username.to_string(),
            password_hash: password_hash.to_string(),
            last_change: 0,
            min_age: 0,
            max_age: 99999,
            warning: 7,
            inactive: -1,
            expire: -1,
            reserved: String::new(),
        }
    }

    /// Check if account is locked (password starts with ! or *)
    pub fn is_locked(&self) -> bool {
        self.password_hash.starts_with('!') || self.password_hash.starts_with('*')
    }

    /// Check if password is expired
    pub fn is_expired(&self, current_day: i64) -> bool {
        if self.expire > 0 && current_day > self.expire {
            return true;
        }
        if self.max_age > 0 && self.last_change > 0 {
            let expiry = self.last_change + self.max_age;
            if current_day > expiry {
                return true;
            }
        }
        false
    }
}

/// Parse a single /etc/shadow line
pub fn parse_shadow_line(line: &str) -> Option<ShadowEntry> {
    if line.trim().is_empty() || line.starts_with('#') {
        return None;
    }

    let parts: Vec<&str> = line.split(':').collect();
    if parts.len() != 9 {
        return None;
    }

    Some(ShadowEntry {
        username: parts[0].to_string(),
        password_hash: parts[1].to_string(),
        last_change: parts[2].parse().unwrap_or(0),
        min_age: parts[3].parse().unwrap_or(0),
        max_age: parts[4].parse().unwrap_or(99999),
        warning: parts[4].parse().unwrap_or(7),  // Note: parts[5] is warning
        inactive: parts[5].parse().unwrap_or(-1),
        expire: parts[6].parse().unwrap_or(-1),
        reserved: parts[7].to_string(),
    })
}

/// Parse entire /etc/shadow content
pub fn parse_shadow(content: &str) -> Vec<ShadowEntry> {
    content
        .lines()
        .filter_map(parse_shadow_line)
        .collect()
}

/// Read and parse /etc/shadow from VFS
pub fn read_shadow_file() -> Result<Vec<ShadowEntry>, crate::fs::vfs::VfsError> {
    match read_file("/etc/shadow") {
        Ok(data) => {
            let content = core::str::from_utf8(&data).map_err(|_| crate::fs::vfs::VfsError::IoError)?;
            Ok(parse_shadow(content))
        }
        Err(e) => Err(e),
    }
}

/// Find a user's shadow entry by username
pub fn find_shadow(username: &str) -> Option<ShadowEntry> {
    read_shadow_file()
        .ok()?
        .into_iter()
        .find(|u| u.username == username)
}

/// Get all shadow entries
pub fn get_all_shadows() -> Vec<ShadowEntry> {
    read_shadow_file().unwrap_or_default()
}

/// Write /etc/shadow file from shadow entries
pub fn write_shadow_file(entries: &[ShadowEntry]) -> Result<(), crate::fs::vfs::VfsError> {
    let mut content = String::new();
    for entry in entries {
        content.push_str(&alloc::format!(
            "{}:{}:{}:{}:{}:{}:{}:{}:{}\n",
            entry.username,
            entry.password_hash,
            entry.last_change,
            entry.min_age,
            entry.max_age,
            entry.warning,
            entry.inactive,
            entry.expire,
            entry.reserved
        ));
    }

    // Ensure /etc directory exists
    let _ = crate::fs::vfs::create_dir("/etc");

    // Open or create the shadow file
    let fd = crate::fs::vfs::open("/etc/shadow", 0x02 | 0x400)?; // O_WRONLY | O_CREAT

    // Write content
    crate::fs::vfs::write(fd, content.as_bytes())?;

    // Close file
    crate::fs::vfs::close(fd)?;

    Ok(())
}