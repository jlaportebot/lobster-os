//! Network stack module — TCP/IP for LobsterOS
//!
//! Layers:
//! - genet: BCM2712 GENETv5 Ethernet MAC driver (ARM64 only)
//! - eth: Ethernet II frame parser/builder
//! - arp: Address Resolution Protocol (IPv4 over Ethernet)
//! - ipv4: IPv4 packet parser/builder
//! - icmp: ICMPv4 (Echo Request/Reply = ping)
//! - udp: User Datagram Protocol
//! - tcp: Transmission Control Protocol (minimal)
//! - dhcp: DHCP client for auto-configuration
//! - socket: Socket API for userspace

#[cfg(target_arch = "aarch64")]
pub mod arp;
#[cfg(target_arch = "aarch64")]
pub mod dhcp;
#[cfg(target_arch = "aarch64")]
pub mod dns;
#[cfg(target_arch = "aarch64")]
pub mod dns_cache;
#[cfg(target_arch = "aarch64")]
pub mod eth;
#[cfg(target_arch = "aarch64")]
pub mod genet;
#[cfg(target_arch = "aarch64")]
pub mod http;
#[cfg(target_arch = "aarch64")]
pub mod icmp;
#[cfg(target_arch = "aarch64")]
pub mod iface;
#[cfg(target_arch = "aarch64")]
pub mod ipv4;
#[cfg(target_arch = "aarch64")]
pub mod socket;
#[cfg(target_arch = "aarch64")]
pub mod tcp;
#[cfg(target_arch = "aarch64")]
pub mod udp;

use crate::println;

/// Maximum packet buffer size
pub const MAX_PACKET_SIZE: usize = 1536;

/// Network interface configuration
pub struct InterfaceConfig {
    pub mac_addr: [u8; 6],
    pub ip_addr: [u8; 4],
    pub netmask: [u8; 4],
    pub gateway: [u8; 4],
    pub dns_server: [u8; 4],
    pub dhcp_enabled: bool,
}

static mut NET_CONFIG: InterfaceConfig = InterfaceConfig {
    mac_addr: [0u8; 6],
    ip_addr: [0u8, 0, 0, 0],
    netmask: [255u8, 255, 255, 255],
    gateway: [0u8, 0, 0, 0],
    dns_server: [8u8, 8, 8, 8],
    dhcp_enabled: true,
};

/// Initialize the network stack
pub unsafe fn init() {
    // 1. Initialize the GENET MAC
    if let Err(e) = genet::init() {
        println!("[net] GENET init failed: {}", e);
        return;
    }

    // 2. Read MAC address
    NET_CONFIG.mac_addr = genet::mac_addr();
    println!(
        "[net] MAC: {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        NET_CONFIG.mac_addr[0],
        NET_CONFIG.mac_addr[1],
        NET_CONFIG.mac_addr[2],
        NET_CONFIG.mac_addr[3],
        NET_CONFIG.mac_addr[4],
        NET_CONFIG.mac_addr[5]
    );

    // 3. Initialize ARP cache
    arp::init();

    // 4. Initialize socket layer
    socket::init();

    // 5. Try DHCP if enabled
    if NET_CONFIG.dhcp_enabled {
        println!("[net] Starting DHCP client...");
        if dhcp::discover() {
            println!(
                "[net] DHCP configured: {}.{}.{}.{}",
                NET_CONFIG.ip_addr[0],
                NET_CONFIG.ip_addr[1],
                NET_CONFIG.ip_addr[2],
                NET_CONFIG.ip_addr[3]
            );
        } else {
            println!("[net] DHCP failed, using link-local (169.254.x.x)");
            NET_CONFIG.ip_addr = [169, 254, 1, 1];
            NET_CONFIG.netmask = [255, 255, 0, 0];
        }
    }

    println!("[net] Stack initialized");
}

#[cfg(target_arch = "aarch64")]
/// Poll for incoming packets and process them
pub fn poll() {
    let mut pkt_buf = [0u8; MAX_PACKET_SIZE];
    let len = genet::receive(&mut pkt_buf);
    if len == 0 {
        return;
    }

    // Parse the Ethernet frame
    if let Some(frame) = eth::EthernetFrame::parse(&pkt_buf[..len]) {
        process_frame(frame, &pkt_buf[..len]);
    }
}

#[cfg(target_arch = "aarch64")]
/// Process a received Ethernet frame
pub fn process_frame(frame: eth::EthernetFrame, data: &[u8]) {
    match frame.ether_type {
        eth::ETHER_TYPE_ARP => {
            arp::handle_packet(&data[eth::HEADER_LEN..]);
        }
        eth::ETHER_TYPE_IPV4 => {
            if let Some(ip_hdr) = ipv4::parse(&data[eth::HEADER_LEN..]) {
                process_ipv4(ip_hdr, &data[eth::HEADER_LEN..]);
            }
        }
        eth::ETHER_TYPE_EAPOL => {
            // EAPOL frames for WPA 4-way handshake
            // Delegate to WiFi driver for handshake processing
            if iface::find_by_type(iface::IfaceType::Wifi).is_some() {
                crate::drivers::wifi::process_eapol_frame(&data[eth::HEADER_LEN..], frame.src_mac);
            }
        }
        eth::ETHER_TYPE_IPV6 => {
            // IPv6 not supported yet
        }
        _ => {}
    }
}

/// Process a received IPv4 packet
fn process_ipv4(ip_hdr: ipv4::Ipv4Header, data: &[u8]) {
    let ip_hdr_len = (ip_hdr.ihl as usize) * 4;
    let payload = &data[ip_hdr_len..];

    match ip_hdr.protocol {
        ipv4::IPPROTO_ICMP => {
            icmp::handle_packet(&ip_hdr, payload);
        }
        ipv4::IPPROTO_UDP => {
            if let Some(udp_hdr) = udp::parse(payload) {
                let udp_data = &payload[udp::HEADER_LEN..];
                socket::handle_udp(&ip_hdr, &udp_hdr, udp_data);
            }
        }
        ipv4::IPPROTO_TCP => {
            tcp::handle_packet(&ip_hdr, payload);
        }
        _ => {}
    }
}

#[cfg(target_arch = "aarch64")]
/// Get current interface configuration
pub fn config() -> &'static InterfaceConfig {
    unsafe { &NET_CONFIG }
}

#[cfg(target_arch = "aarch64")]
/// Convenience: return (MAC address, IP address) tuple
pub fn config_mac_ip() -> ([u8; 6], [u8; 4]) {
    unsafe { (NET_CONFIG.mac_addr, NET_CONFIG.ip_addr) }
}

#[cfg(target_arch = "aarch64")]
/// Convenience: return MAC address only
pub fn config_mac() -> [u8; 6] {
    unsafe { NET_CONFIG.mac_addr }
}

#[cfg(target_arch = "aarch64")]
/// Set IP address (called by DHCP client)
pub fn set_ip_addr(addr: [u8; 4]) {
    unsafe {
        NET_CONFIG.ip_addr = addr;
    }
}

#[cfg(target_arch = "aarch64")]
/// Set netmask (called by DHCP client)
pub fn set_netmask(mask: [u8; 4]) {
    unsafe {
        NET_CONFIG.netmask = mask;
    }
}

#[cfg(target_arch = "aarch64")]
/// Set gateway (called by DHCP client)
pub fn set_gateway(gw: [u8; 4]) {
    unsafe {
        NET_CONFIG.gateway = gw;
    }
}

#[cfg(target_arch = "aarch64")]
/// Set DNS server (called by DHCP client)
pub fn set_dns(dns: [u8; 4]) {
    unsafe {
        NET_CONFIG.dns_server = dns;
    }
}

#[cfg(target_arch = "aarch64")]
/// Handle GENET IRQ
pub fn handle_irq() {
    genet::handle_irq();
}

/// Unified URL fetch — picks the right backend for the platform.
///
/// On QEMU (qemu_raspi5b feature with lobster-net MMIO device detected),
/// uses the lobster-net HTTP proxy via the host's curl.
/// On real hardware, uses the full TCP/IP stack via net::http::http_get.
///
/// Returns Some((status_code, body_bytes)) on success, None on failure.
pub fn fetch_url(url: &str) -> Option<(u16, alloc::vec::Vec<u8>)> {
    // QEMU path: use lobster-net MMIO device if available
    #[cfg(feature = "qemu_raspi5b")]
    {
        if crate::drivers::lobster_net::is_available() {
            return crate::drivers::lobster_net::http_fetch(url).map(|body| (200u16, body));
        }
    }

    // Real hardware path: full TCP/IP stack
    #[cfg(target_arch = "aarch64")]
    {
        crate::net::http::http_get(url).map(|resp| {
            (
                resp.status_code,
                alloc::vec::Vec::from(&resp.body[..resp.body_len]),
            )
        })
    }

    // x86_64 testing stub
    #[cfg(not(target_arch = "aarch64"))]
    {
        let _ = url;
        None
    }
}

// x86_64 stub modules for cross-compilation testing
#[cfg(target_arch = "x86_64")]
pub mod genet {
    pub fn init() -> Result<(), &'static str> {
        Ok(())
    }
    pub fn mac_addr() -> [u8; 6] {
        [0x00, 0x11, 0x22, 0x33, 0x44, 0x55]
    }
    pub fn receive(_buf: &mut [u8]) -> usize {
        0
    }
    pub fn handle_irq() {}
}

#[cfg(target_arch = "x86_64")]
pub mod eth {
    pub const ETHER_TYPE_ARP: u16 = 0x0806;
    pub const ETHER_TYPE_IPV4: u16 = 0x0800;
    pub const ETHER_TYPE_IPV6: u16 = 0x86DD;
    pub const HEADER_LEN: usize = 14;
    #[derive(Clone, Copy)]
    pub struct EthernetFrame {
        pub dst_mac: [u8; 6],
        pub src_mac: [u8; 6],
        pub ether_type: u16,
    }
    impl EthernetFrame {
        pub fn parse(_data: &[u8]) -> Option<Self> {
            None
        }
    }
}

#[cfg(target_arch = "x86_64")]
pub mod arp {
    pub fn init() {}
    pub fn handle_packet(_data: &[u8]) {}
}

#[cfg(target_arch = "x86_64")]
pub mod ipv4 {
    pub const IPPROTO_ICMP: u8 = 1;
    pub const IPPROTO_UDP: u8 = 17;
    pub const IPPROTO_TCP: u8 = 6;
    #[derive(Clone, Copy)]
    pub struct Ipv4Header {
        pub ihl: u8,
        pub protocol: u8,
    }
    pub fn parse(_data: &[u8]) -> Option<Ipv4Header> {
        None
    }
}

#[cfg(target_arch = "x86_64")]
pub mod icmp {
    pub fn handle_packet(_ip_hdr: &crate::net::ipv4::Ipv4Header, _payload: &[u8]) {}
}

#[cfg(target_arch = "x86_64")]
pub mod udp {
    pub const HEADER_LEN: usize = 8;
    #[derive(Clone, Copy)]
    pub struct UdpHeader {
        pub src_port: u16,
        pub dst_port: u16,
    }
    pub fn parse(_data: &[u8]) -> Option<UdpHeader> {
        None
    }
}

#[cfg(target_arch = "x86_64")]
pub mod tcp {
    pub fn handle_packet(_ip_hdr: &crate::net::ipv4::Ipv4Header, _payload: &[u8]) {}
}

#[cfg(target_arch = "x86_64")]
pub mod dhcp {
    pub fn discover() -> bool {
        false
    }
}

#[cfg(target_arch = "x86_64")]
pub mod socket {
    pub fn init() {}
    pub fn handle_udp(
        _ip_hdr: &crate::net::ipv4::Ipv4Header,
        _udp_hdr: &crate::net::udp::UdpHeader,
        _data: &[u8],
    ) {
    }
}

#[cfg(target_arch = "x86_64")]
pub mod dns {
    pub fn init() {}
    pub fn resolve(_name: &str) -> Option<[u8; 4]> {
        None
    }
}

#[cfg(target_arch = "x86_64")]
pub mod http {
    pub fn init() {}
    pub fn http_get(_url: &str) -> Option<HttpResponse> {
        None
    }

    // Stub HttpResponse for x86_64
    pub struct HttpResponse {
        pub status_code: u16,
        pub status_reason: [u8; 32],
        pub reason_len: usize,
        pub body: [u8; 65536],
        pub body_len: usize,
        pub content_length: i64,
        pub chunked: bool,
    }
}
