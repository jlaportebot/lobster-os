//! Session management for LobsterOS
//! Handles TTY allocation, login sessions, utmp/wtmp logging

use crate::arch::timer::Timer;
use crate::users::passwd;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// Maximum number of concurrent sessions
pub const MAX_SESSIONS: usize = 16;

/// Session types
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SessionType {
    Login,      // User login session
    Tty,        // Raw TTY session
    Ssh,        // SSH session (future)
    Serial,     // Serial console
}

/// Session entry
#[derive(Debug, Clone)]
pub struct Session {
    pub id: usize,
    pub session_type: SessionType,
    pub tty: String,           // e.g., "tty1", "ttyS0"
    pub username: String,
    pub pid: u64,
    pub start_time: u64,       // Timer uptime_ms()
    pub active: bool,
}

/// UTMP entry (current logins)
#[derive(Debug, Clone)]
pub struct UtmpEntry {
    pub ut_type: i16,          // Type of login
    pub ut_pid: i32,           // Process ID
    pub ut_line: [u8; 32],     // TTY name
    pub ut_id: [u8; 4],        // Inittab ID
    pub ut_user: [u8; 32],     // Username
    pub ut_host: [u8; 256],    // Hostname
    pub ut_exit: i32,          // Exit status
    pub ut_session: i32,       // Session ID
    pub ut_tv_sec: i64,        // Time entry was made
    pub ut_addr: [u8; 4],      // IP address
}

impl UtmpEntry {
    pub fn new_login(pid: u64, tty: &str, user: &str) -> Self {
        let mut entry = Self::default();
        entry.ut_type = 7; // USER_PROCESS
        entry.ut_pid = pid as i32;
        entry.ut_line[..tty.len().min(32)].copy_from_slice(tty.as_bytes());
        entry.ut_user[..user.len().min(32)].copy_from_slice(user.as_bytes());
        entry.ut_tv_sec = (Timer::uptime_ms() / 1000) as i64;
        entry.ut_session = pid as i32;
        entry
    }

    pub fn new_dead_process(pid: u64, tty: &str) -> Self {
        let mut entry = Self::default();
        entry.ut_type = 8; // DEAD_PROCESS
        entry.ut_pid = pid as i32;
        entry.ut_line[..tty.len().min(32)].copy_from_slice(tty.as_bytes());
        entry.ut_tv_sec = (Timer::uptime_ms() / 1000) as i64;
        entry
    }
}

impl Default for UtmpEntry {
    fn default() -> Self {
        Self {
            ut_type: 0,
            ut_pid: 0,
            ut_line: [0; 32],
            ut_id: [0; 4],
            ut_user: [0; 32],
            ut_host: [0; 256],
            ut_exit: 0,
            ut_session: 0,
            ut_tv_sec: 0,
            ut_addr: [0; 4],
        }
    }
}

/// WTMP entry (login history)
#[derive(Debug, Clone)]
pub struct WtmpEntry {
    pub ut_type: i16,
    pub ut_pid: i32,
    pub ut_line: [u8; 32],
    pub ut_id: [u8; 4],
    pub ut_user: [u8; 32],
    pub ut_host: [u8; 256],
    pub ut_exit: i32,
    pub ut_session: i32,
    pub ut_tv_sec: i64,
    pub ut_addr: [u8; 4],
}

impl Default for WtmpEntry {
    fn default() -> Self {
        Self {
            ut_type: 0,
            ut_pid: 0,
            ut_line: [0; 32],
            ut_id: [0; 4],
            ut_user: [0; 32],
            ut_host: [0; 256],
            ut_exit: 0,
            ut_session: 0,
            ut_tv_sec: 0,
            ut_addr: [0; 4],
        }
    }
}

/// Session manager state
struct SessionManager {
    sessions: [Option<Session>; MAX_SESSIONS],
    next_id: usize,
    utmp_entries: Vec<UtmpEntry>,
    wtmp_entries: Vec<WtmpEntry>,
}

static mut SESSION_MANAGER: SessionManager = SessionManager {
    sessions: [None; MAX_SESSIONS],
    next_id: 1,
    utmp_entries: Vec::new(),
    wtmp_entries: Vec::new(),
};

/// Initialize session manager
pub fn init() {
    unsafe {
        SESSION_MANAGER.sessions = [None; MAX_SESSIONS];
        SESSION_MANAGER.next_id = 1;
        SESSION_MANAGER.utmp_entries = Vec::new();
        SESSION_MANAGER.wtmp_entries = Vec::new();
    }
    crate::println!("[session] Session manager initialized");
}

/// Allocate a new session
pub fn allocate_session(
    session_type: SessionType,
    tty: &str,
    username: &str,
    pid: u64,
) -> Result<usize, &'static str> {
    unsafe {
        // Find free slot
        for i in 0..MAX_SESSIONS {
            if SESSION_MANAGER.sessions[i].is_none() {
                let session = Session {
                    id: SESSION_MANAGER.next_id,
                    session_type,
                    tty: tty.to_string(),
                    username: username.to_string(),
                    pid,
                    start_time: Timer::uptime_ms(),
                    active: true,
                };
                SESSION_MANAGER.next_id += 1;
                SESSION_MANAGER.sessions[i] = Some(session);
                
                // Add to utmp
                let utmp = UtmpEntry::new_login(pid, tty, username);
                SESSION_MANAGER.utmp_entries.push(utmp);
                
                crate::println!("[session] Allocated session {} on {} for {}", session.id, tty, username);
                return Ok(i);
            }
        }
        Err("No free session slots")
    }
}

/// Deallocate a session
pub fn deallocate_session(session_idx: usize) -> bool {
    unsafe {
        if session_idx >= MAX_SESSIONS {
            return false;
        }
        if let Some(session) = SESSION_MANAGER.sessions[session_idx].take() {
            // Add to utmp as dead process
            let utmp = UtmpEntry::new_dead_process(session.pid, &session.tty);
            SESSION_MANAGER.utmp_entries.push(utmp);
            
            // Add to wtmp for history
            let wtmp = WtmpEntry {
                ut_type: 8, // DEAD_PROCESS
                ut_pid: session.pid as i32,
                ut_line: {
                    let mut arr = [0u8; 32];
                    arr[..session.tty.len().min(32)].copy_from_slice(session.tty.as_bytes());
                    arr
                },
                ut_id: [0; 4],
                ut_user: {
                    let mut arr = [0u8; 32];
                    arr[..session.username.len().min(32)].copy_from_slice(session.username.as_bytes());
                    arr
                },
                ut_host: [0; 256],
                ut_exit: 0,
                ut_session: session.id as i32,
                ut_tv_sec: (Timer::uptime_ms() / 1000) as i64,
                ut_addr: [0; 4],
            };
            SESSION_MANAGER.wtmp_entries.push(wtmp);
            
            crate::println!("[session] Deallocated session {} ({} on {})", session.id, session.username, session.tty);
            true
        } else {
            false
        }
    }
}

/// Get session by index
pub fn get_session(session_idx: usize) -> Option<Session> {
    unsafe {
        SESSION_MANAGER.sessions[session_idx].clone()
    }
}

/// Get all active sessions
pub fn get_active_sessions() -> Vec<Session> {
    unsafe {
        SESSION_MANAGER.sessions
            .iter()
            .filter_map(|s| s.as_ref().filter(|s| s.active).cloned())
            .collect()
    }
}

/// Get sessions for a user
pub fn get_user_sessions(username: &str) -> Vec<Session> {
    unsafe {
        SESSION_MANAGER.sessions
            .iter()
            .filter_map(|s| {
                s.as_ref().filter(|s| s.active && s.username == username).cloned()
            })
            .collect()
    }
}

/// Get utmp entries (current logins)
pub fn get_utmp() -> Vec<UtmpEntry> {
    unsafe {
        SESSION_MANAGER.utmp_entries.clone()
    }
}

/// Get wtmp entries (login history)
pub fn get_wtmp() -> Vec<WtmpEntry> {
    unsafe {
        SESSION_MANAGER.wtmp_entries.clone()
    }
}

/// Clean up dead sessions
pub fn cleanup_dead_sessions() {
    unsafe {
        // In a real implementation, check if processes are still alive
        // For now, just a placeholder
    }
}

/// Find session by TTY
pub fn find_session_by_tty(tty: &str) -> Option<(usize, Session)> {
    unsafe {
        for (i, session) in SESSION_MANAGER.sessions.iter().enumerate() {
            if let Some(s) = session {
                if s.active && s.tty == tty {
                    return Some((i, s.clone()));
                }
            }
        }
        None
    }
}

/// Find session by PID
pub fn find_session_by_pid(pid: u64) -> Option<(usize, Session)> {
    unsafe {
        for (i, session) in SESSION_MANAGER.sessions.iter().enumerate() {
            if let Some(s) = session {
                if s.active && s.pid == pid {
                    return Some((i, s.clone()));
                }
            }
        }
        None
    }
}