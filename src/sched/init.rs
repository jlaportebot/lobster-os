//! Init system — service management, runlevels, and boot orchestration
//!
//! Provides a systemd-like service manager for the kernel:
//! - Service definitions with dependencies, restart policies, and capabilities
//! - Runlevels (0=halt, 1=single, 2=multi, 3=network, 4=graphical, 6=reboot)
//! - Service lifecycle: start/stop/restart with dependency ordering
//! - Auto-restart on crash with exponential backoff
//! - Boot sequence orchestrated by runlevel transitions
//!
//! Services are defined in-kernel (no config file parser yet) and managed
//! by the global ServiceManager singleton.

use crate::println;
use crate::sched::{self, Pid, Priority};

// ═══════════════════════════════════════════════════════════════
// Runlevels
// ═══════════════════════════════════════════════════════════════

/// System runlevel — determines which services should be running.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Runlevel {
    /// 0 — System halt
    Halt = 0,
    /// 1 — Single-user / recovery (minimal services)
    Single = 1,
    /// 2 — Multi-user (no networking)
    Multi = 2,
    /// 3 — Multi-user with networking
    Network = 3,
    /// 4 — Graphical (WM + compositor)
    Graphical = 4,
    /// 6 — Reboot
    Reboot = 6,
}

impl Runlevel {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Runlevel::Halt),
            1 => Some(Runlevel::Single),
            2 => Some(Runlevel::Multi),
            3 => Some(Runlevel::Network),
            4 => Some(Runlevel::Graphical),
            6 => Some(Runlevel::Reboot),
            _ => None,
        }
    }

    pub fn as_u8(self) -> u8 {
        self as u8
    }

    pub fn name(self) -> &'static str {
        match self {
            Runlevel::Halt => "halt",
            Runlevel::Single => "single",
            Runlevel::Multi => "multi",
            Runlevel::Network => "network",
            Runlevel::Graphical => "graphical",
            Runlevel::Reboot => "reboot",
        }
    }
}

/// Which runlevels a service should be active in.
#[derive(Clone, Copy)]
pub struct RunlevelMask(u8);

impl RunlevelMask {
    pub const NONE: RunlevelMask = RunlevelMask(0);
    pub const HALT: RunlevelMask = RunlevelMask(1 << 0);
    pub const SINGLE: RunlevelMask = RunlevelMask(1 << 1);
    pub const MULTI: RunlevelMask = RunlevelMask(1 << 2);
    pub const NETWORK: RunlevelMask = RunlevelMask(1 << 3);
    pub const GRAPHICAL: RunlevelMask = RunlevelMask(1 << 4);
    pub const REBOOT: RunlevelMask = RunlevelMask(1 << 6);

    /// Mask for levels 2+ (normal operation)
    pub const NORMAL: RunlevelMask =
        RunlevelMask(RunlevelMask::MULTI.0 | RunlevelMask::NETWORK.0 | RunlevelMask::GRAPHICAL.0);
    /// Mask for levels 3+
    pub const NETWORKING: RunlevelMask =
        RunlevelMask(RunlevelMask::NETWORK.0 | RunlevelMask::GRAPHICAL.0);
    /// All non-special levels
    pub const ALL: RunlevelMask =
        RunlevelMask(RunlevelMask::MULTI.0 | RunlevelMask::NETWORK.0 | RunlevelMask::GRAPHICAL.0);

    pub fn contains(self, rl: Runlevel) -> bool {
        (self.0 & (1 << rl.as_u8())) != 0
    }

    pub fn union(a: RunlevelMask, b: RunlevelMask) -> RunlevelMask {
        RunlevelMask(a.0 | b.0)
    }
}

// ═══════════════════════════════════════════════════════════════
// Service state
// ═══════════════════════════════════════════════════════════════

/// Current state of a managed service.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ServiceState {
    /// Service definition loaded but never started
    Inactive,
    /// Dependencies being resolved / service is starting
    Starting,
    /// Service process is running
    Running,
    /// Service is being stopped
    Stopping,
    /// Service exited on its own (may auto-restart)
    Stopped,
    /// Service crashed and is awaiting restart
    Crashed,
    /// Service failed to start or crashed too many times
    Failed,
}

impl ServiceState {
    pub fn name(self) -> &'static str {
        match self {
            ServiceState::Inactive => "inactive",
            ServiceState::Starting => "starting",
            ServiceState::Running => "running",
            ServiceState::Stopping => "stopping",
            ServiceState::Stopped => "stopped",
            ServiceState::Crashed => "crashed",
            ServiceState::Failed => "failed",
        }
    }
}

// ═══════════════════════════════════════════════════════════════
// Restart policy
// ═══════════════════════════════════════════════════════════════

/// When to automatically restart a crashed service.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RestartPolicy {
    /// Never restart
    Never,
    /// Restart on non-zero exit
    OnFailure,
    /// Always restart (even on clean exit)
    Always,
}

// ═══════════════════════════════════════════════════════════════
// Service configuration
// ═══════════════════════════════════════════════════════════════

/// Maximum service name length
const MAX_SVC_NAME: usize = 32;

/// Maximum dependencies per service
const MAX_DEPS: usize = 8;

/// Maximum restart attempts before marking Failed
const MAX_RESTART_ATTEMPTS: u32 = 5;

/// Base restart delay in ms (doubles each attempt: exponential backoff)
const RESTART_BASE_DELAY_MS: u64 = 500;

/// A service descriptor — defines what to run and how.
pub struct ServiceConfig {
    /// Unique service name (e.g. "net-dhcp", "wm-compositor")
    name: [u8; MAX_SVC_NAME],
    name_len: usize,

    /// VFS path to the ELF binary (e.g. "/sbin/dhcpd")
    exec: [u8; 128],
    exec_len: usize,

    /// Runlevels this service should be active in
    runlevels: RunlevelMask,

    /// Scheduling priority for the service process
    priority: Priority,

    /// Restart policy
    restart: RestartPolicy,

    /// Service dependency names (must start before this one)
    after: [[u8; MAX_SVC_NAME]; MAX_DEPS],
    after_len: [usize; MAX_DEPS],
    after_count: usize,

    /// Service dependency names (must start together / required)
    requires: [[u8; MAX_SVC_NAME]; MAX_DEPS],
    requires_len: [usize; MAX_DEPS],
    requires_count: usize,

    /// Whether this is a critical service (failure → kernel alert)
    critical: bool,
}

impl Clone for ServiceConfig {
    fn clone(&self) -> Self {
        Self {
            name: self.name,
            name_len: self.name_len,
            exec: self.exec,
            exec_len: self.exec_len,
            runlevels: self.runlevels,
            priority: self.priority,
            restart: self.restart,
            after: self.after,
            after_len: self.after_len,
            after_count: self.after_count,
            requires: self.requires,
            requires_len: self.requires_len,
            requires_count: self.requires_count,
            critical: self.critical,
        }
    }
}

impl Copy for ServiceConfig {}

impl ServiceConfig {
    /// Create a builder-style service config
    pub fn new(name: &str, exec: &str) -> Self {
        let mut s = ServiceConfig {
            name: [0; MAX_SVC_NAME],
            name_len: 0,
            exec: [0; 128],
            exec_len: 0,
            runlevels: RunlevelMask::NONE,
            priority: Priority::Normal,
            restart: RestartPolicy::OnFailure,
            after: [[0u8; MAX_SVC_NAME]; MAX_DEPS],
            after_len: [0; MAX_DEPS],
            after_count: 0,
            requires: [[0u8; MAX_SVC_NAME]; MAX_DEPS],
            requires_len: [0; MAX_DEPS],
            requires_count: 0,
            critical: false,
        };
        s.set_name(name);
        s.set_exec(exec);
        s
    }

    pub fn set_name(&mut self, s: &str) {
        self.name_len = s.len().min(MAX_SVC_NAME);
        self.name[..self.name_len].copy_from_slice(&s.as_bytes()[..self.name_len]);
    }

    pub fn set_exec(&mut self, s: &str) {
        self.exec_len = s.len().min(128);
        self.exec[..self.exec_len].copy_from_slice(&s.as_bytes()[..self.exec_len]);
    }

    pub fn runlevels(mut self, mask: RunlevelMask) -> Self {
        self.runlevels = mask;
        self
    }

    pub fn priority(mut self, p: Priority) -> Self {
        self.priority = p;
        self
    }

    pub fn restart(mut self, policy: RestartPolicy) -> Self {
        self.restart = policy;
        self
    }

    pub fn after(mut self, dep: &str) -> Self {
        if self.after_count < MAX_DEPS {
            let idx = self.after_count;
            let len = dep.len().min(MAX_SVC_NAME);
            self.after[idx][..len].copy_from_slice(&dep.as_bytes()[..len]);
            self.after_len[idx] = len;
            self.after_count += 1;
        }
        self
    }

    pub fn requires(mut self, dep: &str) -> Self {
        if self.requires_count < MAX_DEPS {
            let idx = self.requires_count;
            let len = dep.len().min(MAX_SVC_NAME);
            self.requires[idx][..len].copy_from_slice(&dep.as_bytes()[..len]);
            self.requires_len[idx] = len;
            self.requires_count += 1;
        }
        self
    }

    pub fn critical(mut self, val: bool) -> Self {
        self.critical = val;
        self
    }

    pub fn name_str(&self) -> &str {
        core::str::from_utf8(&self.name[..self.name_len]).unwrap_or("?")
    }

    pub fn exec_str(&self) -> &str {
        core::str::from_utf8(&self.exec[..self.exec_len]).unwrap_or("")
    }

    fn after_str(&self, idx: usize) -> &str {
        if idx >= self.after_count {
            return "";
        }
        core::str::from_utf8(&self.after[idx][..self.after_len[idx]]).unwrap_or("")
    }

    fn requires_str(&self, idx: usize) -> &str {
        if idx >= self.requires_count {
            return "";
        }
        core::str::from_utf8(&self.requires[idx][..self.requires_len[idx]]).unwrap_or("")
    }
}

// ═══════════════════════════════════════════════════════════════
// Runtime service state
// ═══════════════════════════════════════════════════════════════

/// Runtime tracking for a service instance.
struct ServiceInstance {
    /// Index into the config table
    config_idx: usize,
    /// Current state
    state: ServiceState,
    /// PID of the running process (0 if not running)
    pid: Pid,
    /// Number of restart attempts since last successful start
    restart_count: u32,
    /// Next restart time (uptime ms); 0 = no pending restart
    next_restart_ms: u64,
    /// Uptime when the service was last started
    started_at_ms: u64,
}

impl ServiceInstance {
    const fn empty() -> Self {
        Self {
            config_idx: 0,
            state: ServiceState::Inactive,
            pid: 0,
            restart_count: 0,
            next_restart_ms: 0,
            started_at_ms: 0,
        }
    }
}

// ═══════════════════════════════════════════════════════════════
// Service manager
// ═══════════════════════════════════════════════════════════════

/// Maximum number of registered services
const MAX_SERVICES: usize = 32;

/// Global service manager state
static mut SERVICE_CONFIGS: [Option<ServiceConfig>; MAX_SERVICES] = [None; MAX_SERVICES];
static mut SERVICE_INSTANCES: [ServiceInstance; MAX_SERVICES] =
    [const { ServiceInstance::empty() }; MAX_SERVICES];
static mut SERVICE_COUNT: usize = 0;
static mut CURRENT_RUNLEVEL: Runlevel = Runlevel::Halt;
static mut MANAGER_INITIALIZED: bool = false;

/// Initialize the service manager and register built-in services.
pub fn init() {
    unsafe {
        CURRENT_RUNLEVEL = Runlevel::Halt;
        SERVICE_COUNT = 0;
        MANAGER_INITIALIZED = false;

        // Register built-in service definitions
        // These represent the kernel subsystems that can be managed as services.
        // Since we don't have separate user-space binaries yet, they use
        // placeholder exec paths that the spawn logic will handle.

        register_service(
            ServiceConfig::new("fs-vfs", "/sbin/vfsd")
                .runlevels(RunlevelMask::SINGLE)
                .priority(Priority::High)
                .restart(RestartPolicy::Always)
                .critical(true),
        );

        register_service(
            ServiceConfig::new("net-loopback", "/sbin/lo")
                .runlevels(RunlevelMask::NETWORKING)
                .priority(Priority::Normal)
                .restart(RestartPolicy::OnFailure)
                .after("fs-vfs"),
        );

        register_service(
            ServiceConfig::new("net-eth", "/sbin/ethd")
                .runlevels(RunlevelMask::NETWORKING)
                .priority(Priority::Normal)
                .restart(RestartPolicy::OnFailure)
                .after("net-loopback")
                .requires("net-loopback"),
        );

        register_service(
            ServiceConfig::new("net-dhcp", "/sbin/dhcpd")
                .runlevels(RunlevelMask::NETWORKING)
                .priority(Priority::Normal)
                .restart(RestartPolicy::OnFailure)
                .after("net-eth")
                .requires("net-eth"),
        );

        register_service(
            ServiceConfig::new("net-dns", "/sbin/dnsd")
                .runlevels(RunlevelMask::NETWORKING)
                .priority(Priority::Normal)
                .restart(RestartPolicy::OnFailure)
                .after("net-dhcp"),
        );

        register_service(
            ServiceConfig::new("usb-hid", "/sbin/hidd")
                .runlevels(RunlevelMask::NORMAL)
                .priority(Priority::Normal)
                .restart(RestartPolicy::OnFailure)
                .after("fs-vfs"),
        );

        register_service(
            ServiceConfig::new("audio-server", "/sbin/audiod")
                .runlevels(RunlevelMask::NORMAL)
                .priority(Priority::Low)
                .restart(RestartPolicy::OnFailure)
                .after("fs-vfs"),
        );

        register_service(
            ServiceConfig::new("wm-compositor", "/sbin/compositor")
                .runlevels(RunlevelMask::GRAPHICAL)
                .priority(Priority::Normal)
                .restart(RestartPolicy::OnFailure)
                .after("fs-vfs")
                .after("usb-hid")
                .requires("fs-vfs")
                .critical(true),
        );

        register_service(
            ServiceConfig::new("wm-theme", "/sbin/themed")
                .runlevels(RunlevelMask::GRAPHICAL)
                .priority(Priority::Low)
                .restart(RestartPolicy::OnFailure)
                .after("wm-compositor")
                .requires("wm-compositor"),
        );

        register_service(
            ServiceConfig::new("shell-svc", "/sbin/shelld")
                .runlevels(RunlevelMask::ALL)
                .priority(Priority::Normal)
                .restart(RestartPolicy::Always)
                .after("fs-vfs"),
        );

        MANAGER_INITIALIZED = true;
    }

    println!(
        "[init] Service manager initialized with {} built-in services",
        unsafe { SERVICE_COUNT }
    );
}

/// Register a new service. Returns its index or -1 on failure.
fn register_service(config: ServiceConfig) -> i32 {
    unsafe {
        if SERVICE_COUNT >= MAX_SERVICES {
            println!("[init] Cannot register service: table full");
            return -1;
        }

        let idx = SERVICE_COUNT;
        SERVICE_CONFIGS[idx] = Some(config);
        SERVICE_INSTANCES[idx] = ServiceInstance {
            config_idx: idx,
            state: ServiceState::Inactive,
            pid: 0,
            restart_count: 0,
            next_restart_ms: 0,
            started_at_ms: 0,
        };
        SERVICE_COUNT += 1;

        println!(
            "[init] Registered service '{}' → idx {}",
            SERVICE_CONFIGS[idx].as_ref().unwrap().name_str(),
            idx
        );
        idx as i32
    }
}

/// Find a service index by name.
fn find_service(name: &str) -> Option<usize> {
    unsafe {
        for i in 0..SERVICE_COUNT {
            if let Some(ref cfg) = SERVICE_CONFIGS[i] {
                if cfg.name_str() == name {
                    return Some(i);
                }
            }
        }
    }
    None
}

// ═══════════════════════════════════════════════════════════════
// Service lifecycle
// ═══════════════════════════════════════════════════════════════

/// Maximum depth for recursive dependency resolution (prevents stack overflow
/// and limits cycle-detection cost).
const MAX_DEP_DEPTH: usize = 8;

/// Internal: recursively start a service and its dependencies.
/// `visiting` tracks the current DFS path for cycle detection.
/// Returns true if the service is running after the call.
fn start_service_recursive(idx: usize, depth: usize, visiting: &mut [bool; MAX_SERVICES]) -> bool {
    if depth > MAX_DEP_DEPTH {
        println!(
            "[init] Dependency chain too deep (>{}), aborting",
            MAX_DEP_DEPTH
        );
        return false;
    }
    if idx >= unsafe { SERVICE_COUNT } {
        return false;
    }

    // Cycle detection
    if visiting[idx] {
        println!(
            "[init] Dependency cycle detected involving service idx {}",
            idx
        );
        return false;
    }

    // Already running or starting?
    unsafe {
        let state = SERVICE_INSTANCES[idx].state;
        if state == ServiceState::Running || state == ServiceState::Starting {
            return true;
        }
    }

    let cfg = unsafe {
        match SERVICE_CONFIGS[idx].as_ref() {
            Some(c) => c,
            None => return false,
        }
    };

    // Mark as visiting (DFS path)
    visiting[idx] = true;

    // --- Resolve 'after' dependencies (soft: auto-start if not running) ---
    for d in 0..cfg.after_count {
        let dep_name = cfg.after_str(d);
        if let Some(dep_idx) = find_service(dep_name) {
            let dep_state = unsafe { SERVICE_INSTANCES[dep_idx].state };
            if dep_state != ServiceState::Running
                && dep_state != ServiceState::Starting
                && dep_state != ServiceState::Stopped
            {
                println!(
                    "[init] Auto-starting 'after' dependency '{}' for '{}'",
                    dep_name,
                    cfg.name_str()
                );
                if !start_service_recursive(dep_idx, depth + 1, visiting) {
                    println!(
                        "[init] Failed to start 'after' dep '{}' for '{}'",
                        dep_name,
                        cfg.name_str()
                    );
                    // Soft dep failure: continue, it might be optional
                }
            }
        }
        // Unregistered after deps are silently skipped (optional)
    }

    // --- Resolve 'requires' dependencies (hard: must start successfully) ---
    for d in 0..cfg.requires_count {
        let dep_name = cfg.requires_str(d);
        if let Some(dep_idx) = find_service(dep_name) {
            let dep_state = unsafe { SERVICE_INSTANCES[dep_idx].state };
            if dep_state != ServiceState::Running && dep_state != ServiceState::Starting {
                println!(
                    "[init] Auto-starting required dependency '{}' for '{}'",
                    dep_name,
                    cfg.name_str()
                );
                if !start_service_recursive(dep_idx, depth + 1, visiting) {
                    println!(
                        "[init] Cannot start '{}': required '{}' failed to start",
                        cfg.name_str(),
                        dep_name
                    );
                    visiting[idx] = false;
                    unsafe {
                        SERVICE_INSTANCES[idx].state = ServiceState::Failed;
                    }
                    return false;
                }
            }
        // If already running/starting, we're good
        } else {
            println!(
                "[init] Cannot start '{}': required '{}' not registered",
                cfg.name_str(),
                dep_name
            );
            visiting[idx] = false;
            unsafe {
                SERVICE_INSTANCES[idx].state = ServiceState::Failed;
            }
            return false;
        }
    }

    // Done visiting — clear DFS path before spawning (borrowed data released)
    visiting[idx] = false;

    // Attempt to spawn the service process
    unsafe {
        let cfg = SERVICE_CONFIGS[idx].as_ref().unwrap();
        let inst = &mut SERVICE_INSTANCES[idx];

        // Try to load the ELF binary from VFS
        let exec_path = cfg.exec_str();
        let elf_data = crate::fs::vfs::read_file(exec_path).ok();

        if let Some(data) = elf_data {
            if !data.is_empty() {
                match sched::spawn_user_process(&data, cfg.priority) {
                    Some(pid) => {
                        inst.pid = pid;
                        inst.state = ServiceState::Running;
                        inst.started_at_ms = crate::arch::ArchTimer::uptime_ms();
                        inst.restart_count = 0;
                        inst.next_restart_ms = 0;
                        println!("[init] Started '{}' → PID {}", cfg.name_str(), pid);
                        return true;
                    }
                    None => {
                        println!("[init] Failed to spawn '{}'", cfg.name_str());
                        inst.state = ServiceState::Failed;
                        return false;
                    }
                }
            }
        }

        // ELF not found in VFS — for built-in kernel services, mark as
        // "running" since the kernel init sequence already set them up.
        // This handles the case where e.g. VFS, networking, and WM are
        // initialized by kernel_init() before the service manager runs.
        if is_builtin_service(cfg.name_str()) {
            inst.pid = 0; // kernel thread, no userspace PID
            inst.state = ServiceState::Running;
            let _install_time = crate::arch::ArchTimer::uptime_ms();
            inst.restart_count = 0;
            inst.next_restart_ms = 0;
            println!(
                "[init] Marked built-in service '{}' as running",
                cfg.name_str()
            );
            return true;
        }

        println!(
            "[init] No ELF at '{}' and '{}' is not a built-in service",
            exec_path,
            cfg.name_str()
        );
        inst.state = ServiceState::Failed;
        false
    }
}

/// Stop a service by index.
fn stop_service(idx: usize) -> bool {
    unsafe {
        if idx >= SERVICE_COUNT {
            return false;
        }

        let inst = &mut SERVICE_INSTANCES[idx];
        let cfg = match SERVICE_CONFIGS[idx].as_ref() {
            Some(c) => c,
            None => return false,
        };

        match inst.state {
            ServiceState::Running => {
                inst.state = ServiceState::Stopping;
                if inst.pid != 0 {
                    sched::kill(inst.pid);
                    println!("[init] Stopping '{}' (PID {})", cfg.name_str(), inst.pid);
                }
                inst.pid = 0;
                inst.state = ServiceState::Stopped;
                inst.next_restart_ms = 0;
                true
            }
            ServiceState::Crashed | ServiceState::Failed => {
                inst.state = ServiceState::Stopped;
                inst.next_restart_ms = 0;
                inst.pid = 0;
                true
            }
            ServiceState::Inactive | ServiceState::Stopped => true,
            _ => false,
        }
    }
}

/// Restart a service (stop + start with recursive deps).
fn restart_service(idx: usize) -> bool {
    stop_service(idx);
    let mut visiting = [false; MAX_SERVICES];
    start_service_recursive(idx, 0, &mut visiting)
}

/// Public entry point: start a service by index with full dependency resolution.
fn start_service(idx: usize) -> bool {
    let mut visiting = [false; MAX_SERVICES];
    start_service_recursive(idx, 0, &mut visiting)
}

/// Compute a topological ordering of all registered services using Kahn's algorithm.
/// Returns a Vec of service indices in dependency order (providers before dependents).
fn topological_order() -> alloc::vec::Vec<usize> {
    let count = unsafe { SERVICE_COUNT };
    let mut in_degree = [0usize; MAX_SERVICES];
    let mut adj: [[bool; MAX_SERVICES]; MAX_SERVICES] = [[false; MAX_SERVICES]; MAX_SERVICES];

    // Build adjacency: if service i depends on j, then j → i (j must come before i)
    unsafe {
        for i in 0..count {
            if let Some(ref cfg_i) = SERVICE_CONFIGS[i] {
                // 'after' deps
                for d in 0..cfg_i.after_count {
                    let dep_name = cfg_i.after_str(d);
                    if let Some(j) = find_service(dep_name) {
                        if !adj[j][i] {
                            adj[j][i] = true;
                            in_degree[i] += 1;
                        }
                    }
                }
                // 'requires' deps
                for d in 0..cfg_i.requires_count {
                    let dep_name = cfg_i.requires_str(d);
                    if let Some(j) = find_service(dep_name) {
                        if !adj[j][i] {
                            adj[j][i] = true;
                            in_degree[i] += 1;
                        }
                    }
                }
            }
        }
    }

    // Kahn's algorithm
    let mut queue = alloc::vec::Vec::new();
    for i in 0..count {
        if in_degree[i] == 0 {
            queue.push(i);
        }
    }

    let mut order = alloc::vec::Vec::with_capacity(count);
    while let Some(node) = queue.pop() {
        order.push(node);
        for successor in 0..count {
            if adj[node][successor] {
                in_degree[successor] -= 1;
                if in_degree[successor] == 0 {
                    queue.push(successor);
                }
            }
        }
    }

    // Cycle detection: if not all nodes are in order, there's a cycle
    if order.len() != count {
        println!(
            "[init] Warning: dependency cycle detected (ordered {}/{} services)",
            order.len(),
            count
        );
        // Add remaining services anyway (best-effort)
        for i in 0..count {
            if !order.contains(&i) {
                order.push(i);
            }
        }
    }

    order
}

/// Check if a service name corresponds to a kernel built-in subsystem
/// that was already initialized during boot.
fn is_builtin_service(name: &str) -> bool {
    matches!(
        name,
        "fs-vfs"
            | "net-loopback"
            | "net-eth"
            | "net-dhcp"
            | "net-dns"
            | "usb-hid"
            | "audio-server"
            | "wm-compositor"
            | "wm-theme"
            | "shell-svc"
    )
}

/// Cascade a service failure to all services that *require* it.
/// When a required service hits `Failed`, every dependent that declared it
/// in `requires[]` is also marked `Failed` (and stopped if running).
/// The cascade continues recursively until no further dependents remain.
fn cascade_failure(failed_name: &str) {
    // Collect indices that require the failed service (avoid borrowing while mutating)
    let mut to_fail: alloc::vec::Vec<usize> = alloc::vec::Vec::new();
    unsafe {
        for i in 0..SERVICE_COUNT {
            if let Some(ref cfg) = SERVICE_CONFIGS[i] {
                for d in 0..cfg.requires_count {
                    if cfg.requires_str(d) == failed_name {
                        let inst = &SERVICE_INSTANCES[i];
                        if inst.state == ServiceState::Running
                            || inst.state == ServiceState::Starting
                            || inst.state == ServiceState::Crashed
                        {
                            to_fail.push(i);
                            break; // one match per service is enough
                        }
                    }
                }
            }
        }
    }

    for idx in to_fail {
        let dep_name;
        unsafe {
            let inst = &mut SERVICE_INSTANCES[idx];
            dep_name = SERVICE_CONFIGS[idx]
                .as_ref()
                .map(|c| c.name_str())
                .unwrap_or("???");
            // Stop the process if it's running
            if inst.pid != 0 {
                sched::kill(inst.pid);
                inst.pid = 0;
            }
            inst.state = ServiceState::Failed;
            println!(
                "[init] Cascading failure: '{}' → Failed (required '{}')",
                dep_name, failed_name
            );
        }
        // Recurse — the newly failed service may itself be required by others
        cascade_failure(dep_name);
    }
}

/// Reset a failed service so it can be restarted.
/// Clears the restart counter and sets state back to Stopped,
/// allowing a fresh `start_service()` attempt.
fn reset_service(idx: usize) -> bool {
    unsafe {
        if idx >= SERVICE_COUNT {
            return false;
        }
        let inst = &mut SERVICE_INSTANCES[idx];
        if inst.state != ServiceState::Failed {
            return false; // only reset from Failed state
        }
        inst.state = ServiceState::Stopped;
        inst.restart_count = 0;
        inst.next_restart_ms = 0;
        inst.pid = 0;
        if let Some(ref cfg) = SERVICE_CONFIGS[idx] {
            println!("[init] Service '{}' reset to Stopped", cfg.name_str());
        }
        true
    }
}

// ═══════════════════════════════════════════════════════════════
// Runlevel transitions
// ═══════════════════════════════════════════════════════════════

/// Transition to a new runlevel. This will:
/// 1. Stop services not needed in the new level
/// 2. Start services needed in the new level
/// Returns true if the transition completed successfully.
pub fn switch_runlevel(target: Runlevel) -> bool {
    let old;
    unsafe {
        old = CURRENT_RUNLEVEL;
        if target == old {
            println!("[init] Already at runlevel {}", target.name());
            return true;
        }
        println!(
            "[init] Runlevel transition: {} → {}",
            old.name(),
            target.name()
        );
    }

    // Phase 1: Stop services that should NOT run in the target level
    // Use reverse topological order (dependents before providers) to avoid
    // stopping a service that others still depend on.
    {
        let order = topological_order();
        // Iterate in reverse: dependents first, providers last
        for &i in order.iter().rev() {
            unsafe {
                if let Some(ref cfg) = SERVICE_CONFIGS[i] {
                    let inst = &SERVICE_INSTANCES[i];
                    if (inst.state == ServiceState::Running || inst.state == ServiceState::Crashed)
                        && !cfg.runlevels.contains(target)
                    {
                        stop_service(i);
                    }
                }
            }
        }
    }

    // Phase 2: Start services that SHOULD run in the target level
    // Use forward topological order (providers before dependents).
    {
        let order = topological_order();
        for &i in order.iter() {
            unsafe {
                if let Some(ref cfg) = SERVICE_CONFIGS[i] {
                    let inst = &SERVICE_INSTANCES[i];
                    // Not running but needed in target level?
                    if inst.state != ServiceState::Running
                        && inst.state != ServiceState::Starting
                        && cfg.runlevels.contains(target)
                    {
                        start_service(i);
                    }
                }
            }
        }
    }

    // Update current runlevel
    unsafe {
        CURRENT_RUNLEVEL = target;
    }

    // Handle special runlevels
    match target {
        Runlevel::Halt => {
            println!("[init] System halt requested");
            #[cfg(target_arch = "aarch64")]
            crate::drivers::power::shutdown();
        }
        Runlevel::Reboot => {
            println!("[init] System reboot requested");
            #[cfg(target_arch = "aarch64")]
            crate::drivers::power::reset();
        }
        _ => {}
    }

    println!("[init] Runlevel {} active", target.name());
    true
}

/// Get the current runlevel.
pub fn current_runlevel() -> Runlevel {
    unsafe { CURRENT_RUNLEVEL }
}

// ═══════════════════════════════════════════════════════════════
// Auto-restart and housekeeping
// ═══════════════════════════════════════════════════════════════

/// Called periodically from the scheduler tick to:
/// - Check if any running service PIDs have exited
/// - Restart crashed services with exponential backoff
/// - Transition pending restarts
pub fn tick() {
    let now_ms = crate::arch::ArchTimer::uptime_ms();

    unsafe {
        for i in 0..SERVICE_COUNT {
            let inst = &mut SERVICE_INSTANCES[i];

            // Check if a running service's PID is still alive
            if inst.state == ServiceState::Running && inst.pid != 0 {
                if let Some(idx) = sched::find_task_index(inst.pid) {
                    let task = sched::TASK_TABLE[idx].as_ref();
                    if let Some(t) = task {
                        if t.state == sched::TaskState::Zombie {
                            // Service process died
                            inst.state = ServiceState::Crashed;
                            inst.pid = 0;
                            println!("[init] Service idx {} crashed (PID was zombie)", i);
                        }
                    } else {
                        // Task slot empty — process gone
                        inst.state = ServiceState::Crashed;
                        inst.pid = 0;
                    }
                } else {
                    // PID not found — process gone
                    inst.state = ServiceState::Crashed;
                    inst.pid = 0;
                }
            }

            // Handle auto-restart for crashed services
            if inst.state == ServiceState::Crashed {
                let cfg = match SERVICE_CONFIGS[i].as_ref() {
                    Some(c) => c,
                    None => continue,
                };

                match cfg.restart {
                    RestartPolicy::Never => {
                        inst.state = ServiceState::Stopped;
                        continue;
                    }
                    RestartPolicy::OnFailure | RestartPolicy::Always => {
                        // Check if we've exceeded max restart attempts
                        if inst.restart_count >= MAX_RESTART_ATTEMPTS {
                            println!(
                                "[init] Service '{}' exceeded max restarts ({}), marking failed",
                                cfg.name_str(),
                                MAX_RESTART_ATTEMPTS
                            );
                            inst.state = ServiceState::Failed;
                            if cfg.critical {
                                println!(
                                    "[init] CRITICAL SERVICE '{}' HAS FAILED!",
                                    cfg.name_str()
                                );
                            }
                            // Cascade: fail all services that require this one
                            let failed_name = cfg.name_str();
                            drop(inst);
                            cascade_failure(failed_name);
                            continue;
                        }

                        // Schedule restart with exponential backoff
                        if inst.next_restart_ms == 0 {
                            let delay = RESTART_BASE_DELAY_MS * (1u64 << inst.restart_count.min(6));
                            inst.next_restart_ms = now_ms + delay;
                            println!(
                                "[init] Service '{}' will restart in {}ms (attempt {})",
                                cfg.name_str(),
                                delay,
                                inst.restart_count + 1
                            );
                        }

                        // Check if it's time to restart
                        if now_ms >= inst.next_restart_ms {
                            inst.next_restart_ms = 0;
                            inst.restart_count += 1;
                            // Drop the mutable borrow on inst before calling start_service
                            drop(inst);
                            start_service(i);
                            break; // restart loop in next tick
                        }
                    }
                }
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════
// Boot sequence
// ═══════════════════════════════════════════════════════════════

/// Perform the full init boot sequence.
/// Called from kernel_init after all subsystems are up.
/// Transitions through runlevels: Halt → Single → Network (or Graphical).
pub fn boot_sequence() {
    println!("[init] Starting boot sequence...");

    // Initialize the service manager
    init();

    // Phase 1: Single-user mode (filesystem, basic services)
    println!("[init] Phase 1: Single-user mode");
    switch_runlevel(Runlevel::Single);

    // Phase 2: Multi-user with networking
    println!("[init] Phase 2: Multi-user with networking");
    switch_runlevel(Runlevel::Network);

    // Phase 3: If display is available, go graphical
    // Check if the framebuffer was initialized
    if crate::display::fb::is_initialized() {
        println!("[init] Phase 3: Graphical mode");
        switch_runlevel(Runlevel::Graphical);
    }

    println!(
        "[init] Boot sequence complete — runlevel {}",
        current_runlevel().name()
    );
}

// ═══════════════════════════════════════════════════════════════
// Public query API (for shell commands)
// ═══════════════════════════════════════════════════════════════

/// Service info snapshot for external consumers.
pub struct ServiceInfo {
    pub name: [u8; MAX_SVC_NAME],
    pub name_len: usize,
    pub state: ServiceState,
    pub pid: Pid,
    pub restart_count: u32,
    pub runlevel_mask: u8,
    pub critical: bool,
}

impl ServiceInfo {
    pub fn name_str(&self) -> &str {
        core::str::from_utf8(&self.name[..self.name_len]).unwrap_or("?")
    }
}

/// Get info about all registered services.
pub fn list_services() -> alloc::vec::Vec<ServiceInfo> {
    let mut result = alloc::vec::Vec::new();
    unsafe {
        for i in 0..SERVICE_COUNT {
            if let Some(ref cfg) = SERVICE_CONFIGS[i] {
                let inst = &SERVICE_INSTANCES[i];
                let mut info = ServiceInfo {
                    name: [0u8; MAX_SVC_NAME],
                    name_len: cfg.name_len,
                    state: inst.state,
                    pid: inst.pid,
                    restart_count: inst.restart_count,
                    runlevel_mask: cfg.runlevels.0,
                    critical: cfg.critical,
                };
                info.name[..cfg.name_len].copy_from_slice(&cfg.name[..cfg.name_len]);
                result.push(info);
            }
        }
    }
    result
}

/// Start a service by name. Returns true on success.
pub fn start_by_name(name: &str) -> bool {
    if let Some(idx) = find_service(name) {
        start_service(idx)
    } else {
        println!("[init] Service '{}' not found", name);
        false
    }
}

/// Stop a service by name. Returns true on success.
pub fn stop_by_name(name: &str) -> bool {
    if let Some(idx) = find_service(name) {
        stop_service(idx)
    } else {
        println!("[init] Service '{}' not found", name);
        false
    }
}

/// Restart a service by name. Returns true on success.
pub fn restart_by_name(name: &str) -> bool {
    if let Some(idx) = find_service(name) {
        restart_service(idx)
    } else {
        println!("[init] Service '{}' not found", name);
        false
    }
}

/// Reset a failed service by name. Returns true on success.
pub fn reset_by_name(name: &str) -> bool {
    if let Some(idx) = find_service(name) {
        reset_service(idx)
    } else {
        println!("[init] Service '{}' not found", name);
        false
    }
}

/// Print dependency information for a service by name.
pub fn print_deps(name: &str) {
    if let Some(idx) = find_service(name) {
        unsafe {
            if let Some(ref cfg) = SERVICE_CONFIGS[idx] {
                println!("Service '{}':", cfg.name_str());
                println!("  Requires ({}):", cfg.requires_count);
                for d in 0..cfg.requires_count {
                    let dep = cfg.requires_str(d);
                    let state = find_service(dep)
                        .map(|i| SERVICE_INSTANCES[i].state.name())
                        .unwrap_or("unregistered");
                    println!("    {} [{}]", dep, state);
                }
                println!("  After ({}):", cfg.after_count);
                for d in 0..cfg.after_count {
                    let dep = cfg.after_str(d);
                    let state = find_service(dep)
                        .map(|i| SERVICE_INSTANCES[i].state.name())
                        .unwrap_or("unregistered");
                    println!("    {} [{}]", dep, state);
                }
                println!("  Runlevels: {:?}", cfg.runlevels.0);
                println!("  Restart: {:?}", cfg.restart);
                println!("  Critical: {}", cfg.critical);
                // Also show who depends on this service
                let mut dependents = alloc::vec::Vec::new();
                for i in 0..SERVICE_COUNT {
                    if let Some(ref c) = SERVICE_CONFIGS[i] {
                        for rd in 0..c.requires_count {
                            if c.requires_str(rd) == name {
                                dependents.push(c.name_str());
                                break;
                            }
                        }
                    }
                }
                if dependents.is_empty() {
                    println!("  Depended-on-by: (none)");
                } else {
                    println!("  Depended-on-by:");
                    for dep in &dependents {
                        println!("    {}", dep);
                    }
                }
            }
        }
    } else {
        println!("[init] Service '{}' not found", name);
    }
}

/// Get the state of a service by name.
pub fn get_state(name: &str) -> Option<ServiceState> {
    find_service(name).map(|idx| unsafe { SERVICE_INSTANCES[idx].state })
}
