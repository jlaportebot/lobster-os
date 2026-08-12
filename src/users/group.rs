//! /etc/group parser for LobsterOS

use crate::fs::vfs::read_file;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// A single entry in /etc/group
#[derive(Debug, Clone)]
pub struct GroupEntry {
    pub name: String,
    pub password: String, // Usually "x" or "*"
    pub gid: u32,
    pub members: Vec<String>, // List of usernames
}

impl GroupEntry {
    pub fn new(name: &str, gid: u32) -> Self {
        Self {
            name: name.to_string(),
            password: "x".to_string(),
            gid,
            members: Vec::new(),
        }
    }

    pub fn add_member(&mut self, username: &str) {
        if !self.members.iter().any(|m| m == username) {
            self.members.push(username.to_string());
        }
    }

    pub fn remove_member(&mut self, username: &str) {
        self.members.retain(|m| m != username);
    }

    pub fn has_member(&self, username: &str) -> bool {
        self.members.iter().any(|m| m == username)
    }
}

/// Parse a single /etc/group line
pub fn parse_group_line(line: &str) -> Option<GroupEntry> {
    if line.trim().is_empty() || line.starts_with('#') {
        return None;
    }

    let parts: Vec<&str> = line.split(':').collect();
    if parts.len() != 4 {
        return None;
    }

    let gid = parts[2].parse().ok()?;
    let members = if parts[3].is_empty() {
        Vec::new()
    } else {
        parts[3].split(',').map(|s| s.to_string()).collect()
    };

    Some(GroupEntry {
        name: parts[0].to_string(),
        password: parts[1].to_string(),
        gid,
        members,
    })
}

/// Parse entire /etc/group content
pub fn parse_group(content: &str) -> Vec<GroupEntry> {
    content.lines().filter_map(parse_group_line).collect()
}

/// Read and parse /etc/group from VFS
pub fn read_group_file() -> Result<Vec<GroupEntry>, crate::fs::vfs::VfsError> {
    match read_file("/etc/group") {
        Ok(data) => {
            let content =
                core::str::from_utf8(&data).map_err(|_| crate::fs::vfs::VfsError::IOError)?;
            Ok(parse_group(content))
        }
        Err(e) => Err(e),
    }
}

/// Find a group by name
pub fn find_group(name: &str) -> Option<GroupEntry> {
    read_group_file().ok()?.into_iter().find(|g| g.name == name)
}

/// Find a group by GID
pub fn find_group_by_gid(gid: u32) -> Option<GroupEntry> {
    read_group_file().ok()?.into_iter().find(|g| g.gid == gid)
}

/// Get all groups
pub fn get_all_groups() -> Vec<GroupEntry> {
    read_group_file().unwrap_or_default()
}

/// Check if group exists
pub fn group_exists(name: &str) -> bool {
    find_group(name).is_some()
}

/// Get groups for a user
pub fn get_user_groups(username: &str) -> Vec<GroupEntry> {
    get_all_groups()
        .into_iter()
        .filter(|g| g.has_member(username))
        .collect()
}

/// Get primary group for a user (from /etc/passwd gid)
pub fn get_primary_group(uid: u32) -> Option<GroupEntry> {
    crate::users::passwd::find_user_by_uid(uid).and_then(|u| find_group_by_gid(u.gid))
}

/// Write /etc/group file from group entries
pub fn write_group_file(entries: &[GroupEntry]) -> Result<(), crate::fs::vfs::VfsError> {
    use crate::fs::vfs::{create_dir, open, write};

    // Ensure /etc directory exists
    let _ = create_dir("/etc");

    // Open or create the group file
    let fd = open(
        "/etc/group",
        crate::fs::vfs::O_WRONLY | crate::fs::vfs::O_CREAT | crate::fs::vfs::O_TRUNC,
    )?; // O_WRONLY | O_CREAT | O_TRUNC

    // Build group content
    let mut content = String::new();
    for entry in entries {
        let members = entry.members.join(",");
        content.push_str(&alloc::format!(
            "{}:x:{}:{}\n",
            entry.name,
            entry.gid,
            members
        ));
    }

    // Write to file
    write(fd, content.as_bytes())?;
    crate::fs::vfs::close(fd)?;
    Ok(())
}
