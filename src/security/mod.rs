//! Security subsystem — credentials, capabilities, access control
//!
//! Provides:
//! - **Credentials**: per-process uid/gid/euid/egid + supplementary groups
//! - **Capabilities**: POSIX-like capability bounding set (CAP_*)
//! - **Access control**: permission checks for VFS operations
//! - **Privilege escalation**: controlled setuid/setgid support
//!
//! Design: capability-based with traditional UNIX uid/gid fallback.
//! Kernel tasks (uid 0) implicitly hold all capabilities.
//! User processes start with a restricted capability set that can be
//! expanded only by explicit grant from a capable process.

use crate::println;
use crate::sched::{self, find_task_index, Pid, TASK_TABLE};

// ═══════════════════════════════════════════════════════════════
// POSIX Capability bits
// ═══════════════════════════════════════════════════════════════

/// Maximum number of capability bits
pub const CAP_MAX: usize = 64;

/// Capability: change file ownership (CAP_CHOWN)
pub const CAP_CHOWN: u64 = 1 << 21;

/// Capability: override file permission checks (DAC)
pub const CAP_DAC_OVERRIDE: u64 = 1 << 0;
/// Capability: read+search bypass on files/dirs
pub const CAP_DAC_READ_SEARCH: u64 = 1 << 1;
/// Capability: owner override on file permission checks
pub const CAP_FOWNER: u64 = 1 << 2;
/// Capability: owner override on file group checks
pub const CAP_FSETID: u64 = 1 << 3;
/// Capability: kill any process
pub const CAP_KILL: u64 = 1 << 4;
/// Capability: set UID/GID / create setuid binaries
pub const CAP_SETGID: u64 = 1 << 5;
/// Capability: set UID / create setuid binaries
pub const CAP_SETUID: u64 = 1 << 6;
/// Capability: set Linux-specific capabilities
pub const CAP_SETPCAP: u64 = 1 << 7;
/// Capability: bind to ports < 1024
pub const CAP_NET_BIND_SERVICE: u64 = 1 << 8;
/// Capability: broadcast / multicast
pub const CAP_NET_BROADCAST: u64 = 1 << 9;
/// Capability: raw sockets / packet capture
pub const CAP_NET_RAW: u64 = 1 << 10;
/// Capability: administer IP firewall / routing
pub const CAP_NET_ADMIN: u64 = 1 << 11;
/// Capability: create device nodes
pub const CAP_MKNOD: u64 = 1 << 12;
/// Capability: mount/unmount filesystems
pub const CAP_SYS_MOUNT: u64 = 1 << 13;
/// Capability: reboot / shutdown
pub const CAP_SYS_BOOT: u64 = 1 << 14;
/// Capability: chroot
pub const CAP_SYS_CHROOT: u64 = 1 << 15;
/// Capability: clock_settime
pub const CAP_SYS_TIME: u64 = 1 << 16;
/// Capability: ptrace
pub const CAP_SYS_PTRACE: u64 = 1 << 17;
/// Capability: schedule priority / nice
pub const CAP_SYS_NICE: u64 = 1 << 18;
/// Capability: resource limits
pub const CAP_SYS_RESOURCE: u64 = 1 << 19;
/// Capability: shared memory / IPC
pub const CAP_SYS_IPC: u64 = 1 << 20;
/// Capability: all capabilities (full root)
pub const CAP_FULL_ROOT: u64 = u64::MAX;

// ═══════════════════════════════════════════════════════════════
// Permission bits (UNIX rwx)
// ═══════════════════════════════════════════════════════════════

/// Owner read
pub const PERM_OWNER_READ: u32 = 0o400;
/// Owner write
pub const PERM_OWNER_WRITE: u32 = 0o200;
/// Owner execute
pub const PERM_OWNER_EXEC: u32 = 0o100;
/// Group read
pub const PERM_GROUP_READ: u32 = 0o040;
/// Group write
pub const PERM_GROUP_WRITE: u32 = 0o020;
/// Group execute
pub const PERM_GROUP_EXEC: u32 = 0o010;
/// Others read
pub const PERM_OTHERS_READ: u32 = 0o004;
/// Others write
pub const PERM_OTHERS_WRITE: u32 = 0o002;
/// Others execute
pub const PERM_OTHERS_EXEC: u32 = 0o001;
/// Setuid bit
pub const PERM_SETUID: u32 = 0o4000;
/// Setgid bit
pub const PERM_SETGID: u32 = 0o2000;
/// Sticky bit
pub const PERM_STICKY: u32 = 0o1000;

// ═══════════════════════════════════════════════════════════════
// Process Credentials
// ═══════════════════════════════════════════════════════════════

/// Per-process capability sets (POSIX capability model)
#[derive(Clone, Copy)]
pub struct CapSet {
    /// Permitted capabilities (can be raised into effective)
    pub permitted: u64,
    /// Effective capabilities (currently active)
    pub effective: u64,
    /// Inheritable capabilities (preserved across execve)
    pub inheritable: u64,
    /// Bounding set (limits permitted across execve)
    pub bounding: u64,
    /// Ambient capabilities (auto-raised for non-setuid execve)
    pub ambient: u64,
}

impl CapSet {
    pub const fn empty() -> Self {
        Self {
            permitted: 0,
            effective: 0,
            inheritable: 0,
            bounding: 0,
            ambient: 0,
        }
    }

    /// Full root capability set
    pub const fn full_root() -> Self {
        Self {
            permitted: CAP_FULL_ROOT,
            effective: CAP_FULL_ROOT,
            inheritable: CAP_FULL_ROOT,
            bounding: CAP_FULL_ROOT,
            ambient: 0,
        }
    }

    /// Default capabilities for a normal user process
    pub const fn default_user() -> Self {
        Self {
            permitted: CAP_DAC_READ_SEARCH,
            effective: CAP_DAC_READ_SEARCH,
            inheritable: 0,
            bounding: CAP_DAC_READ_SEARCH,
            ambient: 0,
        }
    }

    /// Check if a specific capability is in the effective set
    pub fn has_cap(&self, cap: u64) -> bool {
        (self.effective & cap) != 0
    }

    /// Raise a capability into effective set (must be in permitted)
    pub fn raise_cap(&mut self, cap: u64) -> bool {
        if (self.permitted & cap) != 0 {
            self.effective |= cap;
            true
        } else {
            false
        }
    }

    /// Drop a capability from effective set
    pub fn drop_cap(&mut self, cap: u64) {
        self.effective &= !cap;
    }

    /// Drop a capability from all sets (irreversible within this exec)
    pub fn drop_cap_permanent(&mut self, cap: u64) {
        self.effective &= !cap;
        self.permitted &= !cap;
        self.inheritable &= !cap;
        self.ambient &= !cap;
    }
}

// ═══════════════════════════════════════════════════════════════
// Global credential table (indexed by task table index)
// ═══════════════════════════════════════════════════════════════

const MAX_CRED_SLOTS: usize = 32; // Must match MAX_TASKS

/// Saved set-UID and set-GID (for setuid support)
#[derive(Clone, Copy)]
struct SavedSetId {
    uid: u32,
    gid: u32,
}

/// Extended credentials (stored alongside Task, indexed by task slot)
#[derive(Clone, Copy)]
struct ExtCred {
    /// Capability sets
    caps: CapSet,
    /// Saved set-uid (for setuid revert)
    saved_uid: u32,
    /// Saved set-gid (for setgid revert)
    saved_gid: u32,
    /// Secure execution flag (set when setuid/setgid binary was execed)
    secure_exec: bool,
    /// No-new-privs flag (prevents gaining privileges via execve)
    no_new_privs: bool,
    /// Whether this slot is in use
    in_use: bool,
}

static mut EXT_CREDENTIALS: [ExtCred; MAX_CRED_SLOTS] = [ExtCred {
    caps: CapSet::empty(),
    saved_uid: 0,
    saved_gid: 0,
    secure_exec: false,
    no_new_privs: false,
    in_use: false,
}; MAX_CRED_SLOTS];

/// Initialize credentials for a new task (by task table index)
pub fn init_credentials(task_idx: usize, is_kernel: bool) {
    if task_idx >= MAX_CRED_SLOTS {
        return;
    }
    unsafe {
        let cred = &mut EXT_CREDENTIALS[task_idx];
        if is_kernel {
            cred.caps = CapSet::full_root();
            cred.saved_uid = 0;
            cred.saved_gid = 0;
        } else {
            cred.caps = CapSet::default_user();
            cred.saved_uid = 0;
            cred.saved_gid = 0;
        }
        cred.secure_exec = false;
        cred.no_new_privs = false;
        cred.in_use = true;
    }
}

/// Free credentials for a task slot
pub fn free_credentials(task_idx: usize) {
    if task_idx >= MAX_CRED_SLOTS {
        return;
    }
    unsafe {
        EXT_CREDENTIALS[task_idx].in_use = false;
        EXT_CREDENTIALS[task_idx].caps = CapSet::empty();
    }
}

/// Get the capability set for the current process
pub fn current_caps() -> CapSet {
    let pid = sched::current_pid();
    let idx = match find_task_index(pid) {
        Some(i) => i,
        None => return CapSet::empty(),
    };
    if idx < MAX_CRED_SLOTS {
        unsafe { EXT_CREDENTIALS[idx].caps }
    } else {
        CapSet::empty()
    }
}

/// Get effective capability bitmap for a task index (used by procfs)
pub fn get_capabilities(task_idx: usize) -> u64 {
    if task_idx < MAX_CRED_SLOTS && unsafe { EXT_CREDENTIALS[task_idx].in_use } {
        unsafe { EXT_CREDENTIALS[task_idx].caps.effective }
    } else {
        0
    }
}

/// Clone credentials from parent to child across fork
pub fn clone_credentials(parent_idx: usize, child_idx: usize, _uid: u32, _gid: u32) {
    if parent_idx >= MAX_CRED_SLOTS || child_idx >= MAX_CRED_SLOTS {
        return;
    }
    unsafe {
        if !EXT_CREDENTIALS[parent_idx].in_use {
            return;
        }
        // Copy parent's credential state to child
        EXT_CREDENTIALS[child_idx] = ExtCred {
            caps: EXT_CREDENTIALS[parent_idx].caps,
            saved_uid: EXT_CREDENTIALS[parent_idx].saved_uid,
            saved_gid: EXT_CREDENTIALS[parent_idx].saved_gid,
            secure_exec: EXT_CREDENTIALS[parent_idx].secure_exec,
            no_new_privs: EXT_CREDENTIALS[parent_idx].no_new_privs,
            in_use: true,
        };
    }
}

/// Get mutable reference to the capability set for a task
fn get_caps_mut(task_idx: usize) -> Option<&'static mut CapSet> {
    if task_idx < MAX_CRED_SLOTS {
        unsafe {
            if EXT_CREDENTIALS[task_idx].in_use {
                Some(&mut EXT_CREDENTIALS[task_idx].caps)
            } else {
                None
            }
        }
    } else {
        None
    }
}

// ═══════════════════════════════════════════════════════════════
// Access checks
// ═══════════════════════════════════════════════════════════════

/// Access mode for permission checks
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AccessMode {
    Read,
    Write,
    Execute,
}

/// Check if the current process can access a file with the given permissions and ownership.
/// `file_uid` / `file_gid` are the file's owner uid/gid.
/// `file_perm` is the UNIX permission mode (e.g., 0o755).
/// `mode` is the requested access type.
///
/// Returns true if access is allowed, false if denied.
pub fn check_access(file_uid: u32, file_gid: u32, file_perm: u32, mode: AccessMode) -> bool {
    let pid = sched::current_pid();
    let task_idx = match find_task_index(pid) {
        Some(i) => i,
        None => return false,
    };

    let (euid, egid, groups, group_count) = unsafe {
        if let Some(ref task) = TASK_TABLE[task_idx] {
            (task.euid, task.egid, task.groups, task.group_count)
        } else {
            return false;
        }
    };

    // Root (uid 0) with CAP_DAC_OVERRIDE bypasses all permission checks
    if euid == 0 {
        if let Some(caps) = get_caps_mut(task_idx) {
            if caps.has_cap(CAP_DAC_OVERRIDE) {
                return true;
            }
        } else {
            // Kernel task with no ext creds — allow
            return true;
        }
    }

    // Determine which permission bits to check
    let (read_bit, write_bit, exec_bit) = if euid == file_uid {
        (PERM_OWNER_READ, PERM_OWNER_WRITE, PERM_OWNER_EXEC)
    } else if is_in_group(egid, groups, group_count, file_gid) {
        (PERM_GROUP_READ, PERM_GROUP_WRITE, PERM_GROUP_EXEC)
    } else {
        (PERM_OTHERS_READ, PERM_OTHERS_WRITE, PERM_OTHERS_EXEC)
    };

    match mode {
        AccessMode::Read => (file_perm & read_bit) != 0,
        AccessMode::Write => (file_perm & write_bit) != 0,
        AccessMode::Execute => (file_perm & exec_bit) != 0,
    }
}

/// Check if a group ID matches the process's effective gid or supplementary groups
fn is_in_group(egid: u32, groups: [u32; 4], group_count: u32, target_gid: u32) -> bool {
    if egid == target_gid {
        return true;
    }
    for i in 0..group_count as usize {
        if i < groups.len() && groups[i] == target_gid {
            return true;
        }
    }
    false
}

/// Check if the current process has a specific capability
pub fn has_capability(cap: u64) -> bool {
    let pid = sched::current_pid();
    let idx = match find_task_index(pid) {
        Some(i) => i,
        None => return false,
    };

    // uid 0 implicitly has all capabilities
    let euid = unsafe { TASK_TABLE[idx].as_ref().map(|t| t.euid).unwrap_or(u32::MAX) };
    if euid == 0 {
        return true;
    }

    if idx < MAX_CRED_SLOTS {
        unsafe { EXT_CREDENTIALS[idx].caps.has_cap(cap) }
    } else {
        false
    }
}

/// Check if a specific process (by PID) has a capability
pub fn pid_has_capability(pid: Pid, cap: u64) -> bool {
    let idx = match find_task_index(pid) {
        Some(i) => i,
        None => return false,
    };

    let euid = unsafe { TASK_TABLE[idx].as_ref().map(|t| t.euid).unwrap_or(u32::MAX) };
    if euid == 0 {
        return true;
    }

    if idx < MAX_CRED_SLOTS {
        unsafe { EXT_CREDENTIALS[idx].caps.has_cap(cap) }
    } else {
        false
    }
}

// ═══════════════════════════════════════════════════════════════
// Credential manipulation syscalls
// ═══════════════════════════════════════════════════════════════

/// setuid: set real + effective + saved UID
/// Requires CAP_SETUID or setting to current uid/saved-uid.
pub fn sys_setuid(uid: u32) -> i64 {
    let pid = sched::current_pid();
    let idx = match find_task_index(pid) {
        Some(i) => i,
        None => return -1, // -ESRCH
    };

    // Check permission
    let (old_euid, old_uid) = unsafe {
        let task = TASK_TABLE[idx].as_ref().unwrap();
        (task.euid, task.uid)
    };

    let has_cap = has_capability(CAP_SETUID);
    let saved_uid = if idx < MAX_CRED_SLOTS {
        unsafe { EXT_CREDENTIALS[idx].saved_uid }
    } else {
        0
    };

    // Allowed if: CAP_SETUID, or setting to current real/saved/effective uid
    if !has_cap && uid != old_euid && uid != old_uid && uid != saved_uid {
        return -1; // -EPERM
    }

    unsafe {
        let task = TASK_TABLE[idx].as_mut().unwrap();
        task.uid = uid;
        task.euid = uid;
    }
    if idx < MAX_CRED_SLOTS {
        unsafe {
            EXT_CREDENTIALS[idx].saved_uid = uid;
        }
    }

    0
}

/// setgid: set real + effective + saved GID
pub fn sys_setgid(gid: u32) -> i64 {
    let pid = sched::current_pid();
    let idx = match find_task_index(pid) {
        Some(i) => i,
        None => return -1,
    };

    let (old_egid, old_gid) = unsafe {
        let task = TASK_TABLE[idx].as_ref().unwrap();
        (task.egid, task.gid)
    };

    let has_cap = has_capability(CAP_SETGID);
    let saved_gid = if idx < MAX_CRED_SLOTS {
        unsafe { EXT_CREDENTIALS[idx].saved_gid }
    } else {
        0
    };

    if !has_cap && gid != old_egid && gid != old_gid && gid != saved_gid {
        return -1;
    }

    unsafe {
        let task = TASK_TABLE[idx].as_mut().unwrap();
        task.gid = gid;
        task.egid = gid;
    }
    if idx < MAX_CRED_SLOTS {
        unsafe {
            EXT_CREDENTIALS[idx].saved_gid = gid;
        }
    }

    0
}

/// seteuid: set effective UID only
pub fn sys_seteuid(euid: u32) -> i64 {
    let pid = sched::current_pid();
    let idx = match find_task_index(pid) {
        Some(i) => i,
        None => return -1,
    };

    let (old_uid, _old_euid) = unsafe {
        let task = TASK_TABLE[idx].as_ref().unwrap();
        (task.uid, task.euid)
    };

    let saved_uid = if idx < MAX_CRED_SLOTS {
        unsafe { EXT_CREDENTIALS[idx].saved_uid }
    } else {
        0
    };

    // Can set euid to: real uid, saved uid, or any if CAP_SETUID
    if !has_capability(CAP_SETUID) && euid != old_uid && euid != saved_uid {
        return -1;
    }

    unsafe {
        TASK_TABLE[idx].as_mut().unwrap().euid = euid;
    }
    0
}

/// setegid: set effective GID only
pub fn sys_setegid(egid: u32) -> i64 {
    let pid = sched::current_pid();
    let idx = match find_task_index(pid) {
        Some(i) => i,
        None => return -1,
    };

    let (old_gid, _old_egid) = unsafe {
        let task = TASK_TABLE[idx].as_ref().unwrap();
        (task.gid, task.egid)
    };

    let saved_gid = if idx < MAX_CRED_SLOTS {
        unsafe { EXT_CREDENTIALS[idx].saved_gid }
    } else {
        0
    };

    if !has_capability(CAP_SETGID) && egid != old_gid && egid != saved_gid {
        return -1;
    }

    unsafe {
        TASK_TABLE[idx].as_mut().unwrap().egid = egid;
    }
    0
}

/// getuid / geteuid / getgid / getegid
pub fn sys_getuid() -> u32 {
    let pid = sched::current_pid();
    if let Some(idx) = find_task_index(pid) {
        unsafe { TASK_TABLE[idx].as_ref().map(|t| t.uid).unwrap_or(0) }
    } else {
        0
    }
}

pub fn sys_geteuid() -> u32 {
    let pid = sched::current_pid();
    if let Some(idx) = find_task_index(pid) {
        unsafe { TASK_TABLE[idx].as_ref().map(|t| t.euid).unwrap_or(0) }
    } else {
        0
    }
}

pub fn sys_getgid() -> u32 {
    let pid = sched::current_pid();
    if let Some(idx) = find_task_index(pid) {
        unsafe { TASK_TABLE[idx].as_ref().map(|t| t.gid).unwrap_or(0) }
    } else {
        0
    }
}

pub fn sys_getegid() -> u32 {
    let pid = sched::current_pid();
    if let Some(idx) = find_task_index(pid) {
        unsafe { TASK_TABLE[idx].as_ref().map(|t| t.egid).unwrap_or(0) }
    } else {
        0
    }
}

/// getgroups: fill caller's buffer with supplementary group list
/// Returns the number of groups written.
pub fn sys_getgroups(buf: &mut [u32]) -> usize {
    let pid = sched::current_pid();
    if let Some(idx) = find_task_index(pid) {
        let (groups, count) = unsafe {
            let task = TASK_TABLE[idx].as_ref().unwrap();
            (task.groups, task.group_count as usize)
        };
        let to_write = buf.len().min(count).min(8);
        buf[..to_write].copy_from_slice(&groups[..to_write]);
        return to_write;
    }
    0
}

/// setgroups: set supplementary groups (requires CAP_SETGID)
pub fn sys_setgroups(groups: &[u32]) -> i64 {
    if !has_capability(CAP_SETGID) {
        return -1; // -EPERM
    }
    if groups.len() > 8 {
        return -22; // -EINVAL
    }
    let pid = sched::current_pid();
    if let Some(idx) = find_task_index(pid) {
        unsafe {
            let task = TASK_TABLE[idx].as_mut().unwrap();
            task.groups = [0u32; 4];
            for (i, &g) in groups.iter().enumerate() {
                task.groups[i] = g;
            }
            task.group_count = groups.len() as u32;
        }
        return 0;
    }
    -1
}

// ═══════════════════════════════════════════════════════════════
// Capability manipulation
// ═══════════════════════════════════════════════════════════════

/// capget: return the current capability set for the calling process
pub fn sys_capget() -> CapSet {
    current_caps()
}

/// capset: set capability sets (subject to permitted/bounding constraints)
pub fn sys_capset(new: &CapSet) -> i64 {
    let pid = sched::current_pid();
    let idx = match find_task_index(pid) {
        Some(i) => i,
        None => return -1,
    };

    // No-new-privs prevents gaining capabilities
    let no_new_privs = if idx < MAX_CRED_SLOTS {
        unsafe { EXT_CREDENTIALS[idx].no_new_privs }
    } else {
        false
    };

    let caps = match get_caps_mut(idx) {
        Some(c) => c,
        None => return -1,
    };

    // New permitted must be a subset of current permitted (or bounding if no_new_privs)
    let limit = if no_new_privs {
        caps.bounding
    } else {
        caps.permitted
    };
    if (new.permitted & !limit) != 0 {
        return -1; // -EPERM: tried to gain capabilities
    }

    // New effective must be a subset of new permitted
    if (new.effective & !new.permitted) != 0 {
        return -1;
    }

    // New inheritable must be a subset of bounding set
    if (new.inheritable & !caps.bounding) != 0 {
        return -1;
    }

    caps.permitted = new.permitted;
    caps.effective = new.effective;
    caps.inheritable = new.inheritable;
    caps.ambient = new.ambient & new.permitted & caps.bounding;

    0
}

/// prctl: PR_SET_NO_NEW_PRIVS — prevent privilege escalation via execve
pub fn sys_set_no_new_privs() -> i64 {
    let pid = sched::current_pid();
    if let Some(idx) = find_task_index(pid) {
        if idx < MAX_CRED_SLOTS {
            unsafe {
                EXT_CREDENTIALS[idx].no_new_privs = true;
            }
            return 0;
        }
    }
    -1
}

/// prctl: PR_GET_NO_NEW_PRIVS
pub fn sys_get_no_new_privs() -> i64 {
    let pid = sched::current_pid();
    if let Some(idx) = find_task_index(pid) {
        if idx < MAX_CRED_SLOTS {
            return if unsafe { EXT_CREDENTIALS[idx].no_new_privs } {
                1
            } else {
                0
            };
        }
    }
    -1
}

// ═══════════════════════════════════════════════════════════════
// execve credential transformation
// ═══════════════════════════════════════════════════════════════

/// Transform credentials across an execve call.
/// If the file has setuid/setgid bits, the effective uid/gid are changed.
/// Capabilities are recalculated according to the inheritable + bounding sets.
///
/// `file_uid`, `file_gid`: ownership of the executed file
/// `file_mode`: permission mode of the executed file (includes setuid/setgid bits)
pub fn execve_credential_transform(task_idx: usize, file_uid: u32, file_gid: u32, file_mode: u32) {
    if task_idx >= MAX_CRED_SLOTS {
        return;
    }

    let is_setuid = (file_mode & PERM_SETUID) != 0;
    let is_setgid = (file_mode & PERM_SETGID) != 0;

    unsafe {
        let cred = &mut EXT_CREDENTIALS[task_idx];
        let task = TASK_TABLE[task_idx].as_mut().unwrap();

        // Save pre-exec uids for secure_exec detection
        let old_euid = task.euid;
        let old_egid = task.egid;

        // Setuid: set euid to file owner
        if is_setuid && file_uid != 0 {
            task.euid = file_uid;
            cred.saved_uid = file_uid;
        }

        // Setgid: set egid to file group
        if is_setgid && file_gid != 0 {
            task.egid = file_gid;
            cred.saved_gid = file_gid;
        }

        // Secure exec flag: set if euid or egid changed
        cred.secure_exec = (task.euid != old_euid) || (task.egid != old_egid);

        // Capability transformation across execve:
        // new_permitted = (inheritable & bounding) | ambient
        // If setuid: new_permitted is empty (drop privs unless ambient)
        let new_permitted = if is_setuid || is_setgid {
            // Setuid/setgid: only ambient capabilities survive
            cred.caps.ambient & cred.caps.bounding
        } else {
            // Normal exec: inheritable ∩ bounding + ambient
            (cred.caps.inheritable & cred.caps.bounding) | cred.caps.ambient
        };

        cred.caps.permitted = new_permitted;
        cred.caps.effective = new_permitted;

        // no_new_privs is preserved across execve (that's the whole point)
    }
}

// ═══════════════════════════════════════════════════════════════
// Initialization
// ═══════════════════════════════════════════════════════════════

/// Initialize the security subsystem
pub fn init() {
    // All slots start as not-in-use (handled by static init)
    println!("[security] Credential + capability subsystem initialized");
}

/// Print security debug info
pub fn print_debug() {
    println!("[security] === Security State ===");
    for i in 0..MAX_CRED_SLOTS {
        unsafe {
            let cred = &EXT_CREDENTIALS[i];
            if cred.in_use {
                println!(
                    "[security] slot {} caps: P={:#018x} E={:#018x} I={:#018x} B={:#018x} A={:#018x}",
                    i, cred.caps.permitted, cred.caps.effective,
                    cred.caps.inheritable, cred.caps.bounding, cred.caps.ambient
                );
            }
        }
    }
}
