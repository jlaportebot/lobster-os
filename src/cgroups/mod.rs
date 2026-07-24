use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

/// Controller types for cgroups
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControllerType {
    /// CPU controller (limits CPU usage)
    Cpu,
    /// Memory controller (limits memory usage)
    Memory,
    /// IO controller (limits I/O access)
    Io,
    /// PIDS controller (limits number of processes/tasks)
    Pids,
}

/// Represents a cgroup
#[derive(Debug, Clone)]
pub struct Cgroup {
    /// Hierarchy ID (which hierarchy this cgroup belongs to)
    pub hierarchy: u64,
    /// Unique ID within the hierarchy
    pub id: u64,
    /// Path of the cgroup (relative to hierarchy root)
    pub path: String,
    /// Controllers enabled for this cgroup
    pub controllers: Vec<ControllerType>,
}

/// Simple cgroup manager
pub struct CgroupManager {
    /// Next available hierarchy ID
    next_hierarchy_id: u64,
    /// Next available cgroup ID within a hierarchy
    next_cgroup_id: BTreeMap<u64, u64>,
    /// Map of (hierarchy_id, cgroup_id) to cgroup
    cgroups: BTreeMap<(u64, u64), Cgroup>,
}

impl Default for CgroupManager {
    fn default() -> Self {
        Self::new()
    }
}

impl CgroupManager {
    /// Create a new cgroup manager
    pub const fn new() -> Self {
        Self {
            next_hierarchy_id: 1,
            next_cgroup_id: BTreeMap::new(),
            cgroups: BTreeMap::new(),
        }
    }

    /// Create a new hierarchy and return its ID
    pub fn create_hierarchy(&mut self, _controllers: Vec<ControllerType>) -> u64 {
        let hid = self.next_hierarchy_id;
        self.next_hierarchy_id += 1;
        self.next_cgroup_id.insert(hid, 1); // Start cgroup IDs at 1 for each hierarchy
        hid
    }

    /// Create a new cgroup in the given hierarchy
    pub fn create_cgroup(
        &mut self,
        hierarchy: u64,
        path: String,
        controllers: Vec<ControllerType>,
    ) -> Result<u64, &'static str> {
        let next_id = self
            .next_cgroup_id
            .get_mut(&hierarchy)
            .ok_or("hierarchy does not exist")?;
        let cid = *next_id;
        *next_id += 1;
        let cgroup = Cgroup {
            hierarchy,
            id: cid,
            path,
            controllers,
        };
        self.cgroups.insert((hierarchy, cid), cgroup);
        Ok(cid)
    }

    /// Look up a cgroup by hierarchy and ID
    pub fn get(&self, hierarchy: u64, id: u64) -> Option<&Cgroup> {
        self.cgroups.get(&(hierarchy, id))
    }

    /// Remove a cgroup
    pub fn remove(&mut self, hierarchy: u64, id: u64) -> Option<Cgroup> {
        self.cgroups.remove(&(hierarchy, id))
    }
}

/// Global cgroup manager instance
static mut CGROUP_MANAGER: Option<CgroupManager> = None;

/// Initialize the cgroup subsystem
pub fn init() {
    unsafe {
        CGROUP_MANAGER = Some(CgroupManager::new());
        crate::println!("[cgroups] subsystem initialized");
    }
}

/// Create a new hierarchy with the given controllers
pub fn create_hierarchy(controllers: Vec<ControllerType>) -> Result<u64, &'static str> {
    unsafe {
        Ok(CGROUP_MANAGER
            .as_mut()
            .ok_or("cgroup manager not initialized")?
            .create_hierarchy(controllers))
    }
}

/// Create a new cgroup in the given hierarchy
pub fn create_cgroup(
    hierarchy: u64,
    path: String,
    controllers: Vec<ControllerType>,
) -> Result<u64, &'static str> {
    unsafe {
        CGROUP_MANAGER
            .as_mut()
            .ok_or("cgroup manager not initialized")?
            .create_cgroup(hierarchy, path, controllers)
    }
}

/// Look up a cgroup
pub fn get_cgroup(hierarchy: u64, id: u64) -> Option<&'static Cgroup> {
    unsafe { CGROUP_MANAGER.as_ref().and_then(|m| m.get(hierarchy, id)) }
}

/// (Stub) Apply resource limits to a task based on its cgroup membership.
pub fn apply_limits(_pid: u32) -> Result<(), &'static str> {
    crate::println!("[cgroups] applying limits for pid: {}", _pid);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hierarchy_and_cgroup_create() {
        unsafe { CGROUP_MANAGER = None };
        init();
        let hid = create_hierarchy(vec![ControllerType::Cpu, ControllerType::Memory]).unwrap();
        assert_eq!(hid, 1);
        let cid = create_cgroup(hid, "/test".to_string(), vec![ControllerType::Cpu]).unwrap();
        assert_eq!(cid, 1);
        let cg = get_cgroup(hid, cid).expect("cgroup should exist");
        assert_eq!(cg.path, "/test");
        assert!(cg.controllers.contains(&ControllerType::Cpu));
    }

    #[test]
    fn test_apply_limits() {
        unsafe { CGROUP_MANAGER = None };
        init();
        assert!(apply_limits(1).is_ok());
    }
}
