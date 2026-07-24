use crate::println;
use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};

/// Namespace types supported by LobsterOS
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NamespaceType {
    /// Process ID namespace (isolates PID space)
    Pid,
    /// Mount namespace (isolates mount points)
    Mount,
    /// UTS namespace (isolates hostname and domain name)
    Uts,
    /// User namespace (isolates user and group IDs)
    User,
    /// Network namespace (isolates network devices, stacks, ports)
    Net,
    /// IPC namespace (isolates System V IPC, POSIX message queues)
    Ipc,
}

/// Represents a namespace instance
#[derive(Debug, Clone)]
pub struct Namespace {
    /// Type of this namespace
    pub ty: NamespaceType,
    /// Unique identifier for the namespace
    pub id: u64,
    /// Optional human-readable name (e.g., hostname for UTS)
    pub name: Option<String>,
}

/// Simple namespace manager (in a real OS this would be more complex)
pub struct NamespaceManager {
    /// Next available namespace ID
    next_id: u64,
    /// Map of namespace ID to namespace
    namespaces: BTreeMap<u64, Namespace>,
}

impl Default for NamespaceManager {
    fn default() -> Self {
        Self::new()
    }
}

impl NamespaceManager {
    /// Create a new namespace manager
    pub const fn new() -> Self {
        Self {
            next_id: 1,
            namespaces: BTreeMap::new(),
        }
    }

    /// Create a new namespace of the given type
    pub fn create(&mut self, ty: NamespaceType, name: Option<String>) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        let ns = Namespace { ty, id, name };
        self.namespaces.insert(id, ns);
        id
    }

    /// Look up a namespace by ID
    pub fn get(&self, id: u64) -> Option<&Namespace> {
        self.namespaces.get(&id)
    }

    /// Remove a namespace (when no longer referenced)
    pub fn remove(&mut self, id: u64) -> Option<Namespace> {
        self.namespaces.remove(&id)
    }
}

/// Global namespace manager instance (in a real OS this would be per-task or similar)
static mut NAMESPACE_MANAGER: Option<NamespaceManager> = None;

/// Initialize the namespace subsystem (called during kernel init)
pub fn init() {
    unsafe {
        NAMESPACE_MANAGER = Some(NamespaceManager::new());
        crate::println!("[namespaces] subsystem initialized");
    }
}

/// Create a new namespace
pub fn create(ty: NamespaceType, name: Option<String>) -> Result<u64, &'static str> {
    unsafe {
        Ok(NAMESPACE_MANAGER
            .as_mut()
            .ok_or("namespace manager not initialized")?
            .create(ty, name))
    }
}

/// Look up a namespace by ID
pub fn get(id: u64) -> Option<&'static Namespace> {
    unsafe { NAMESPACE_MANAGER.as_ref().and_then(|m| m.get(id)) }
}

/// (Stub) This function is called by the scheduler when a task calls `unshare(2)`.
pub fn unshare(_flags: u32) -> Result<(), &'static str> {
    // In a real implementation, we would create new namespaces for the
    // calling task based on the flags.
    crate::println!("[namespaces] unshare called with flags: 0x{:x}", _flags);
    // For now, just acknowledge the call.
    Ok(())
}

/// (Stub) This function is called by the scheduler when a task calls `setns(2)`.
pub fn setns(_fd: u32, _nstype: u32) -> Result<(), &'static str> {
    crate::println!(
        "[namespaces] setns called with fd: {}, type: {}",
        _fd,
        _nstype
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_namespace_create_and_get() {
        unsafe { NAMESPACE_MANAGER = None };
        init();
        let id = create(NamespaceType::Pid, Some("init-pid-ns".to_string())).unwrap();
        assert_eq!(id, 1);
        let ns = get(id).expect("namespace should exist");
        assert_eq!(ns.ty, NamespaceType::Pid);
        assert_eq!(ns.name.as_deref(), Some("init-pid-ns"));
    }

    #[test]
    fn test_unshare_and_setns() {
        unsafe { NAMESPACE_MANAGER = None };
        init();
        assert!(unshare(0).is_ok());
        assert!(setns(0, 0).is_ok());
    }
}

/// Pivot root — change the root filesystem for the current task
///
/// This is used by container runtimes to set up a new root filesystem
/// without breaking the old one. The old root is moved to `put_old`,
/// and `new_root` becomes the new root.
///
/// # Arguments
/// * `new_root` - Path to the new root filesystem
/// * `put_old` - Path where the old root will be moved to
///
/// # Returns
/// Ok(()) on success, Err on failure
pub fn pivot_root(new_root: &str, put_old: &str) -> Result<(), &'static str> {
    use crate::fs::vfs;
    use crate::fs::vfs::FileType;

    // Validate that new_root exists and is a directory
    match vfs::stat(new_root) {
        Some(meta) if meta.file_type == FileType::Directory => {}
        Some(_) => return Err("new_root is not a directory"),
        None => return Err("new_root does not exist"),
    }

    // Validate that put_old exists and is a directory
    match vfs::stat(put_old) {
        Some(meta) if meta.file_type == FileType::Directory => {}
        Some(_) => return Err("put_old is not a directory"),
        None => return Err("put_old does not exist"),
    }

    // Check that put_old is under new_root
    if !put_old.starts_with(new_root) {
        return Err("put_old must be under new_root");
    }

    // In a full implementation, we'd:
    // 1. Create a mount namespace if not already in one
    // 2. Mount new_root at a temporary point
    // 3. Bind-mount new_root to itself
    // 4. Move the old root to put_old
    // 5. chroot to new_root
    // 6. Unmount put_old (lazy)

    println!(
        "[namespaces] pivot_root: new_root={}, put_old={}",
        new_root, put_old
    );

    // For now, we acknowledge the call and log it
    // The actual filesystem pivot requires VFS mount namespace support
    // which is tracked as a future enhancement
    Ok(())
}

/// Create a full container namespace set (PID, Mount, Network, UTS, IPC, User)
///
/// This is the equivalent of `unshare -p -n -m -u -i -r` for a new container.
/// Returns the namespace IDs for each created namespace.
pub fn create_container_ns() -> Result<ContainerNs, &'static str> {
    let pid_ns = create(NamespaceType::Pid, Some("container-pid".to_string()))?;
    let mount_ns = create(NamespaceType::Mount, Some("container-mount".to_string()))?;
    let net_ns = create(NamespaceType::Net, Some("container-net".to_string()))?;
    let uts_ns = create(NamespaceType::Uts, Some("container-uts".to_string()))?;
    let ipc_ns = create(NamespaceType::Ipc, Some("container-ipc".to_string()))?;
    let user_ns = create(NamespaceType::User, Some("container-user".to_string()))?;

    println!(
        "[namespaces] Created container namespaces: pid={} mount={} net={} uts={} ipc={} user={}",
        pid_ns, mount_ns, net_ns, uts_ns, ipc_ns, user_ns
    );

    Ok(ContainerNs {
        pid: pid_ns,
        mount: mount_ns,
        net: net_ns,
        uts: uts_ns,
        ipc: ipc_ns,
        user: user_ns,
    })
}

/// Container namespace set
#[derive(Debug, Clone, Copy)]
pub struct ContainerNs {
    pub pid: u64,
    pub mount: u64,
    pub net: u64,
    pub uts: u64,
    pub ipc: u64,
    pub user: u64,
}
