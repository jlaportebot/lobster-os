use alloc::string::String;
use alloc::vec::Vec;
use alloc::collections::BTreeMap;
use crate::println;

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
        NAMESPACE_MANAGER
            .as_mut()
            .ok_or("namespace manager not initialized")?
            .create(ty, name)
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
    crate::println!("[namespaces] setns called with fd: {}, type: {}", _fd, _nstype);
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