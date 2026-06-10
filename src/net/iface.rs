//! Network interface abstraction — decouples the IP stack from hardware drivers
//!
//! Provides a `NetInterface` trait that both Ethernet (GENET) and WiFi (BCM43455)
//! implement. The IP stack (ARP, IPv4, TCP, UDP, DHCP, DNS) operates against
//! this trait rather than directly calling genet::transmit/receive.
//!
//! Pi 5 optimization:
//! - Zero-copy where possible (DMA buffers passed through)
//! - Per-interface ARP cache (WiFi and Ethernet have different broadcast domains)
//! - Lock-free polling model (no mutexes on the hot path)
//! - Static dispatch via enum (no vtable overhead in #![no_std])

#[cfg(target_arch = "aarch64")]
use crate::drivers::wifi;
#[cfg(target_arch = "aarch64")]
use crate::net::genet;
use crate::println;

// ─── Error Type ────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum IfaceError {
    NotInitialized,
    NotConnected,
    TxFailed,
    RxFailed,
    NoData,
    InvalidState,
    InterfaceNotFound,
    BufferTooSmall,
}

// ─── Interface Type ────────────────────────────────────────────────

/// Hardware interface type
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum IfaceType {
    /// BCM2712 GENETv5 gigabit Ethernet
    Ethernet,
    /// Infineon CYW43455 802.11b/g/n WiFi
    Wifi,
    /// Loopback (for testing)
    Loopback,
}

// ─── Interface Identifier ──────────────────────────────────────────

/// Interface index (0-based, matches position in interface table)
pub type IfaceId = u8;

/// Maximum number of network interfaces
pub const MAX_INTERFACES: usize = 4;

// ─── Interface Config ──────────────────────────────────────────────

/// Per-interface network configuration
#[derive(Clone, Copy, Debug)]
pub struct IfaceConfig {
    pub mac_addr: [u8; 6],
    pub ip_addr: [u8; 4],
    pub netmask: [u8; 4],
    pub gateway: [u8; 4],
    pub dns_server: [u8; 4],
    pub dhcp_enabled: bool,
    pub mtu: u16,
    pub is_up: bool,
}

impl IfaceConfig {
    pub const fn default() -> Self {
        Self {
            mac_addr: [0u8; 6],
            ip_addr: [0u8; 4],
            netmask: [255, 255, 255, 255],
            gateway: [0u8; 4],
            dns_server: [8, 8, 8, 8],
            dhcp_enabled: true,
            mtu: 1500,
            is_up: false,
        }
    }
}

// ─── Interface State ───────────────────────────────────────────────

/// Runtime state for a network interface
#[derive(Clone, Copy, Debug)]
pub struct IfaceState {
    pub iface_type: IfaceType,
    pub name: [u8; 8],
    pub name_len: usize,
    pub config: IfaceConfig,
    pub link_up: bool,
    pub initialized: bool,
    /// RX packet count
    pub rx_packets: u64,
    /// TX packet count
    pub tx_packets: u64,
    /// RX byte count
    pub rx_bytes: u64,
    /// TX byte count
    pub tx_bytes: u64,
    /// RX error count
    pub rx_errors: u64,
    /// TX error count
    pub tx_errors: u64,
}

impl IfaceState {
    pub const fn new(iface_type: IfaceType, name: &[u8]) -> Self {
        let mut name_arr = [0u8; 8];
        let mut i = 0;
        while i < name.len() && i < 8 {
            name_arr[i] = name[i];
            i += 1;
        }
        Self {
            iface_type,
            name: name_arr,
            name_len: if name.len() < 8 { name.len() } else { 8 },
            config: IfaceConfig::default(),
            link_up: false,
            initialized: false,
            rx_packets: 0,
            tx_packets: 0,
            rx_bytes: 0,
            tx_bytes: 0,
            rx_errors: 0,
            tx_errors: 0,
        }
    }

    pub fn name_str(&self) -> &str {
        core::str::from_utf8(&self.name[..self.name_len]).unwrap_or("unknown")
    }
}

// ─── Interface Table ───────────────────────────────────────────────

/// Global interface table — static array, no heap allocation
static mut IFACE_TABLE: [Option<IfaceState>; MAX_INTERFACES] = [None, None, None, None];
static mut IFACE_COUNT: usize = 0;

// ─── Registration ──────────────────────────────────────────────────

/// Register a network interface. Returns the assigned IfaceId.
/// Called once during init for each hardware interface.
pub unsafe fn register(iface_type: IfaceType, name: &[u8]) -> Result<IfaceId, IfaceError> {
    if IFACE_COUNT >= MAX_INTERFACES {
        return Err(IfaceError::InterfaceNotFound);
    }
    let id = IFACE_COUNT as IfaceId;
    let mut state = IfaceState::new(iface_type, name);

    // Pre-populate MAC from driver
    match iface_type {
        IfaceType::Ethernet => {
            state.config.mac_addr = genet::mac_addr();
            state.link_up = genet::is_link_up();
            state.initialized = genet::is_initialized();
            state.config.is_up = state.initialized && state.link_up;
            state.config.mtu = 1500;
        }
        IfaceType::Wifi => {
            state.config.mac_addr = wifi::mac_addr();
            state.link_up = wifi::link_up();
            state.initialized = wifi::is_up();
            state.config.is_up = state.initialized && state.link_up;
            state.config.mtu = 1500;
        }
        IfaceType::Loopback => {
            state.config.mac_addr = [0x00, 0x00, 0x00, 0x00, 0x00, 0x01];
            state.link_up = true;
            state.initialized = true;
            state.config.is_up = true;
            state.config.mtu = 65535;
            state.config.ip_addr = [127, 0, 0, 1];
            state.config.netmask = [255, 0, 0, 0];
        }
    }

    IFACE_TABLE[id as usize] = Some(state);
    IFACE_COUNT += 1;
    println!(
        "[iface] Registered {} as if{} (MAC {:02x?})",
        state.name_str(),
        id,
        state.config.mac_addr
    );
    Ok(id)
}

// ─── Interface Lookup ──────────────────────────────────────────────

/// Get the number of registered interfaces
pub fn count() -> usize {
    unsafe { IFACE_COUNT }
}

/// Get interface state by ID
pub fn get(id: IfaceId) -> Option<IfaceState> {
    unsafe {
        if (id as usize) < IFACE_COUNT {
            IFACE_TABLE[id as usize]
        } else {
            None
        }
    }
}

/// Get the first interface of a given type
pub fn find_by_type(iface_type: IfaceType) -> Option<IfaceId> {
    unsafe {
        for i in 0..IFACE_COUNT {
            if let Some(ref state) = IFACE_TABLE[i] {
                if state.iface_type == iface_type {
                    return Some(i as IfaceId);
                }
            }
        }
    }
    None
}

/// Get the first interface that is up and has an IP address
pub fn find_default() -> Option<IfaceId> {
    unsafe {
        for i in 0..IFACE_COUNT {
            if let Some(ref state) = IFACE_TABLE[i] {
                if state.config.is_up && state.config.ip_addr != [0u8; 4] {
                    return Some(i as IfaceId);
                }
            }
        }
    }
    None
}

/// Get the first interface that is up (even without IP)
pub fn find_link_up() -> Option<IfaceId> {
    unsafe {
        for i in 0..IFACE_COUNT {
            if let Some(ref state) = IFACE_TABLE[i] {
                if state.config.is_up {
                    return Some(i as IfaceId);
                }
            }
        }
    }
    None
}

// ─── Transmit / Receive ───────────────────────────────────────────

/// Send a raw Ethernet frame on the specified interface
pub fn transmit(id: IfaceId, data: &[u8]) -> Result<(), IfaceError> {
    let state = get(id).ok_or(IfaceError::InterfaceNotFound)?;
    if !state.config.is_up {
        return Err(IfaceError::NotConnected);
    }

    let result = match state.iface_type {
        IfaceType::Ethernet => genet::transmit(data).map_err(|_| IfaceError::TxFailed),
        IfaceType::Wifi => wifi::transmit(data).map_err(|_| IfaceError::TxFailed),
        IfaceType::Loopback => {
            // Loopback TX goes to RX buffer (handled separately)
            Ok(())
        }
    };

    // Update stats
    unsafe {
        if let Some(ref mut s) = IFACE_TABLE[id as usize] {
            match result {
                Ok(()) => {
                    s.tx_packets += 1;
                    s.tx_bytes += data.len() as u64;
                }
                Err(_) => {
                    s.tx_errors += 1;
                }
            }
        }
    }

    result
}

/// Receive a raw Ethernet frame from the specified interface
/// Returns the number of bytes received (0 if no data)
pub fn receive(id: IfaceId, buf: &mut [u8]) -> usize {
    let state = match get(id) {
        Some(s) => s,
        None => return 0,
    };

    if !state.initialized {
        return 0;
    }

    let len = match state.iface_type {
        IfaceType::Ethernet => genet::receive(buf),
        IfaceType::Wifi => wifi::receive(buf),
        IfaceType::Loopback => 0, // TODO: loopback RX from buffer
    };

    // Update stats
    unsafe {
        if let Some(ref mut s) = IFACE_TABLE[id as usize] {
            if len > 0 {
                s.rx_packets += 1;
                s.rx_bytes += len as u64;
            } else if !state.config.is_up {
                s.rx_errors += 1;
            }
        }
    }

    len
}

/// Poll all interfaces for RX data. Returns (iface_id, byte_count) for
/// the first interface that has data, or None if all empty.
pub fn poll_any() -> Option<(IfaceId, usize)> {
    unsafe {
        for i in 0..IFACE_COUNT {
            if let Some(ref state) = IFACE_TABLE[i] {
                if !state.config.is_up {
                    continue;
                }
                // Check if interface has data without reading it
                match state.iface_type {
                    IfaceType::Ethernet => {
                        // GENET uses interrupt-driven RX; check ring consumer index
                        // For now, just try a zero-length probe
                    }
                    IfaceType::Wifi => {
                        // WiFi poll
                        wifi::poll();
                    }
                    IfaceType::Loopback => {}
                }
            }
        }
    }
    None
}

// ─── Configuration ────────────────────────────────────────────────

/// Update interface configuration (IP, netmask, gateway, DNS)
pub fn configure(id: IfaceId, config: &IfaceConfig) -> Result<(), IfaceError> {
    unsafe {
        if (id as usize) >= IFACE_COUNT {
            return Err(IfaceError::InterfaceNotFound);
        }
        if let Some(ref mut state) = IFACE_TABLE[id as usize] {
            state.config = *config;
            // If we have an IP, mark interface as up
            if config.ip_addr != [0u8; 4] {
                state.config.is_up = true;
            }
            return Ok(());
        }
    }
    Err(IfaceError::InterfaceNotFound)
}

/// Set interface up/down
pub fn set_up(id: IfaceId, up: bool) -> Result<(), IfaceError> {
    unsafe {
        if (id as usize) >= IFACE_COUNT {
            return Err(IfaceError::InterfaceNotFound);
        }
        if let Some(ref mut state) = IFACE_TABLE[id as usize] {
            state.config.is_up = up && state.initialized && state.link_up;
            return Ok(());
        }
    }
    Err(IfaceError::InterfaceNotFound)
}

/// Refresh interface state from driver (link status, etc.)
pub fn refresh(id: IfaceId) -> Result<(), IfaceError> {
    unsafe {
        if (id as usize) >= IFACE_COUNT {
            return Err(IfaceError::InterfaceNotFound);
        }
        if let Some(ref mut state) = IFACE_TABLE[id as usize] {
            match state.iface_type {
                IfaceType::Ethernet => {
                    state.link_up = genet::is_link_up();
                    state.initialized = genet::is_initialized();
                    state.config.mac_addr = genet::mac_addr();
                }
                IfaceType::Wifi => {
                    state.link_up = wifi::link_up();
                    state.initialized = wifi::is_up();
                    state.config.mac_addr = wifi::mac_addr();
                }
                IfaceType::Loopback => {
                    state.link_up = true;
                    state.initialized = true;
                }
            }
            state.config.is_up = state.initialized
                && state.link_up
                && (state.config.ip_addr != [0u8; 4] || !state.config.dhcp_enabled);
            return Ok(());
        }
    }
    Err(IfaceError::InterfaceNotFound)
}

/// Refresh all interfaces
pub fn refresh_all() {
    let n = count();
    for i in 0..n {
        let _ = refresh(i as IfaceId);
    }
}

/// Get MAC address for an interface
pub fn mac_addr(id: IfaceId) -> Result<[u8; 6], IfaceError> {
    get(id)
        .map(|s| s.config.mac_addr)
        .ok_or(IfaceError::InterfaceNotFound)
}

/// Get IP address for an interface
pub fn ip_addr(id: IfaceId) -> Result<[u8; 4], IfaceError> {
    get(id)
        .map(|s| s.config.ip_addr)
        .ok_or(IfaceError::InterfaceNotFound)
}

/// Get MTU for an interface
pub fn mtu(id: IfaceId) -> Result<u16, IfaceError> {
    get(id)
        .map(|s| s.config.mtu)
        .ok_or(IfaceError::InterfaceNotFound)
}

// ─── Initialization ───────────────────────────────────────────────

/// Initialize the interface table and register hardware interfaces.
/// Must be called after genet::init() and wifi::init().
pub unsafe fn init() {
    IFACE_COUNT = 0;

    // Register Ethernet (GENET) as if0
    if genet::is_initialized() {
        register(IfaceType::Ethernet, b"eth0").ok();
    } else {
        println!("[iface] eth0: GENET not initialized, skipping");
    }

    // Register WiFi as if1 (if initialized)
    if wifi::is_up() {
        register(IfaceType::Wifi, b"wlan0").ok();
    } else {
        println!("[iface] wlan0: WiFi not initialized, skipping");
    }

    println!("[iface] {} interface(s) registered", IFACE_COUNT);
}

/// Print interface status (for shell `ifconfig` command)
pub fn print_status() {
    let n = count();
    if n == 0 {
        println!("No network interfaces");
        return;
    }
    for i in 0..n {
        if let Some(state) = get(i as IfaceId) {
            let ip = state.config.ip_addr;
            let mask = state.config.netmask;
            let gw = state.config.gateway;
            let dns = state.config.dns_server;
            println!(
                "{}: type={} link={} up={} MAC {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
                state.name_str(),
                match state.iface_type {
                    IfaceType::Ethernet => "eth",
                    IfaceType::Wifi => "wifi",
                    IfaceType::Loopback => "lo",
                },
                if state.link_up { "UP" } else { "DOWN" },
                if state.config.is_up { "YES" } else { "NO" },
                state.config.mac_addr[0],
                state.config.mac_addr[1],
                state.config.mac_addr[2],
                state.config.mac_addr[3],
                state.config.mac_addr[4],
                state.config.mac_addr[5],
            );
            println!(
                "  IP {}.{}.{}.{} netmask {}.{}.{}.{} gw {}.{}.{}.{}",
                ip[0],
                ip[1],
                ip[2],
                ip[3],
                mask[0],
                mask[1],
                mask[2],
                mask[3],
                gw[0],
                gw[1],
                gw[2],
                gw[3]
            );
            println!(
                "  DNS {}.{}.{}.{} MTU {} DHCP={}",
                dns[0],
                dns[1],
                dns[2],
                dns[3],
                state.config.mtu,
                if state.config.dhcp_enabled {
                    "on"
                } else {
                    "off"
                }
            );
            println!(
                "  RX {} pkts / {} bytes ({} errors)",
                state.rx_packets, state.rx_bytes, state.rx_errors
            );
            println!(
                "  TX {} pkts / {} bytes ({} errors)",
                state.tx_packets, state.tx_bytes, state.tx_errors
            );
        }
    }
}
