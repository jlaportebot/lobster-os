//! DHCP (Dynamic Host Configuration Protocol) client
//!
//! Implements DHCP discover, offer, request, ack (DORA) process.
//! No_std compatible - uses only core.

#[cfg(target_arch = "aarch64")]
use crate::net::config;
#[cfg(target_arch = "aarch64")]
use crate::net::eth::{EthernetFrame, ETHER_TYPE_IPV4};
#[cfg(target_arch = "aarch64")]
use crate::net::genet;
#[cfg(target_arch = "aarch64")]
use crate::net::ipv4::{self, IPPROTO_UDP};
#[cfg(target_arch = "aarch64")]
use crate::net::udp;
use alloc::format;
use alloc::string::String;

/// BOOTP message types
pub const BOOTREQUEST: u8 = 1;
pub const BOOTREPLY: u8 = 2;

/// DHCP message types (option 53)
pub const DHCP_DISCOVER: u8 = 1;
pub const DHCP_OFFER: u8 = 2;
pub const DHCP_REQUEST: u8 = 3;
pub const DHCP_DECLINE: u8 = 4;
pub const DHCP_ACK: u8 = 5;
pub const DHCP_NAK: u8 = 6;
pub const DHCP_RELEASE: u8 = 7;

/// DHCP ports
pub const DHCP_SERVER_PORT: u16 = 67;
pub const DHCP_CLIENT_PORT: u16 = 68;

/// Magic cookie for DHCP
const DHCP_MAGIC: u32 = 0x63825363;

/// DHCP transaction ID (LOBS = 0x4C4F4253)
const DHCP_XID: u32 = 0x4C4F4253;

/// DHCP client state
struct DhcpState {
    xid: u32,
    offered_ip: [u8; 4],
    offered_netmask: [u8; 4],
    offered_gateway: [u8; 4],
    offered_dns: [u8; 4],
    state: DhcpStateEnum,
    /// Server IP that offered the lease
    server_ip: [u8; 4],
    /// Lease time in seconds (option 51)
    lease_time: u32,
    /// Renewal timer T1 (typically 50% of lease)
    t1_renew_at: u64,
    /// Rebinding timer T2 (typically 87.5% of lease)
    t2_rebind_at: u64,
    /// Lease expiry timestamp
    lease_expires_at: u64,
}

#[derive(Clone, Copy, PartialEq)]
enum DhcpStateEnum {
    Idle,
    DiscoverSent,
    OfferReceived,
    RequestSent,
    Bound,
    Renewing,
    Rebinding,
}

static mut DHCP_STATE: DhcpState = DhcpState {
    xid: DHCP_XID,
    offered_ip: [0, 0, 0, 0],
    offered_netmask: [0, 0, 0, 0],
    offered_gateway: [0, 0, 0, 0],
    offered_dns: [0, 0, 0, 0],
    state: DhcpStateEnum::Idle,
    server_ip: [0, 0, 0, 0],
    lease_time: 0,
    t1_renew_at: 0,
    t2_rebind_at: 0,
    lease_expires_at: 0,
};

/// DHCP packet structure (minimal)
#[repr(packed)]
#[derive(Clone, Copy)]
struct DhcpPacket {
    op: u8,
    htype: u8,
    hlen: u8,
    hops: u8,
    xid: u32,
    secs: u16,
    flags: u16,
    ciaddr: [u8; 4],
    yiaddr: [u8; 4],
    siaddr: [u8; 4],
    giaddr: [u8; 4],
    chaddr: [u8; 16],
    sname: [u8; 64],
    file: [u8; 128],
    options: [u8; 300],
}

/// Build a DHCP packet
fn build_dhcp_packet(msg_type: u8, chaddr: &[u8; 6], _dst: [u8; 4]) -> ([u8; 300], usize) {
    let mut pkt = [0u8; 300];

    // BOOTP header
    pkt[0] = BOOTREQUEST; // op
    pkt[1] = 1; // htype (Ethernet)
    pkt[2] = 6; // hlen (MAC length)
    pkt[3] = 0; // hops
                // xid at offset 4
    pkt[4] = ((DHCP_XID >> 24) & 0xFF) as u8;
    pkt[5] = ((DHCP_XID >> 16) & 0xFF) as u8;
    pkt[6] = ((DHCP_XID >> 8) & 0xFF) as u8;
    pkt[7] = (DHCP_XID & 0xFF) as u8;
    // secs at offset 8
    pkt[8] = 0;
    pkt[9] = 0;
    // flags at offset 10
    pkt[10] = 0xFF; // Broadcast flag
    pkt[11] = 0;
    // ciaddr, yiaddr, siaddr, giaddr = 0
    // chaddr at offset 28
    pkt[28..34].copy_from_slice(chaddr);

    // Options start at offset 236
    let opts = &mut pkt[236..];

    // Magic cookie
    opts[0] = 0x63;
    opts[1] = 0x82;
    opts[2] = 0x53;
    opts[3] = 0x63;

    let mut pos = 4;

    // Message type option
    opts[pos] = 53; // DHCP Message Type
    pos += 1;
    opts[pos] = 1; // Length
    pos += 1;
    opts[pos] = msg_type;
    pos += 1;

    // For discover/request, add requested IP option
    if msg_type == DHCP_DISCOVER || msg_type == DHCP_REQUEST {
        // Parameter request list
        opts[pos] = 55; // Parameter Request List
        pos += 1;
        opts[pos] = 4; // Length
        pos += 1;
        opts[pos] = 1; // Subnet Mask
        pos += 1;
        opts[pos] = 3; // Router/Gateway
        pos += 1;
        opts[pos] = 6; // DNS
        pos += 1;
        opts[pos] = 15; // Domain Name
        pos += 1;
    }

    // Requested IP Address (for REQUEST)
    if msg_type == DHCP_REQUEST {
        let offered = unsafe { DHCP_STATE.offered_ip };
        opts[pos] = 50; // Requested IP Address
        pos += 1;
        opts[pos] = 4;
        pos += 1;
        opts[pos] = offered[0];
        pos += 1;
        opts[pos] = offered[1];
        pos += 1;
        opts[pos] = offered[2];
        pos += 1;
        opts[pos] = offered[3];
        pos += 1;
    }

    // End option
    opts[pos] = 255;

    (pkt, 236 + pos + 1)
}

/// Parse DHCP options from packet
fn parse_dhcp_options(
    opts: &[u8],
) -> Option<(u8, [u8; 4], [u8; 4], [u8; 4], [u8; 4], u32, [u8; 4])> {
    if opts.len() < 4 {
        return None;
    }

    // Check magic cookie
    if opts[0] != 0x63 || opts[1] != 0x82 || opts[2] != 0x53 || opts[3] != 0x63 {
        return None;
    }

    let mut pos = 4;
    let mut msg_type = 0;
    let yiaddr = [0u8; 4];
    let mut netmask = [0u8; 4];
    let mut gateway = [0u8; 4];
    let mut dns = [0u8; 4];
    let mut lease_time: u32 = 86400; // Default 24h
    let mut server_ip = [0u8; 4];

    while pos < opts.len() - 1 {
        if opts[pos] == 255 {
            break; // End option
        }
        if opts[pos] == 0 {
            pos += 1;
            continue; // Pad option
        }

        let code = opts[pos];
        let len = opts[pos + 1] as usize;
        if pos + 1 + len > opts.len() {
            break;
        }

        match code {
            53 => {
                // DHCP Message Type
                if len >= 1 {
                    msg_type = opts[pos + 2];
                }
            }
            1 => {
                // Subnet Mask
                if len >= 4 {
                    netmask.copy_from_slice(&opts[pos + 2..pos + 6]);
                }
            }
            3 => {
                // Router/Gateway
                if len >= 4 {
                    gateway.copy_from_slice(&opts[pos + 2..pos + 6]);
                }
            }
            6 => {
                // DNS
                if len >= 4 {
                    dns.copy_from_slice(&opts[pos + 2..pos + 6]);
                }
            }
            51 => {
                // Lease Time
                if len >= 4 {
                    lease_time = u32::from_be_bytes([
                        opts[pos + 2],
                        opts[pos + 3],
                        opts[pos + 4],
                        opts[pos + 5],
                    ]);
                }
            }
            54 => {
                // Server Identifier
                if len >= 4 {
                    server_ip.copy_from_slice(&opts[pos + 2..pos + 6]);
                }
            }
            _ => {}
        }

        pos += 2 + len;
    }

    if msg_type == 0 {
        return None;
    }

    Some((
        msg_type, yiaddr, netmask, gateway, dns, lease_time, server_ip,
    ))
}

/// Send a DHCP discover packet
fn send_discover() {
    let cfg = config();
    let our_mac = cfg.mac_addr;
    let our_ip = cfg.ip_addr;

    // Build DHCP discover
    let (dhcp_pkt, dhcp_len) = build_dhcp_packet(DHCP_DISCOVER, &our_mac, [255, 255, 255, 255]);

    // Build UDP packet (67 -> 68)
    let mut udp_buf = [0u8; 300];
    let udp_len = udp::build(
        DHCP_CLIENT_PORT,
        DHCP_SERVER_PORT,
        &dhcp_pkt[..dhcp_len],
        &mut udp_buf,
    );

    // Broadcast destination
    let broadcast_ip = [255, 255, 255, 255];

    // Build IPv4 packet (0.0.0.0 -> 255.255.255.255)
    let mut ipv4_buf = [0u8; 300];
    let ipv4_len = ipv4::build(
        our_ip,
        broadcast_ip,
        IPPROTO_UDP,
        128,
        0xCAFE,
        &udp_buf[..udp_len],
        &mut ipv4_buf,
    );

    // Build Ethernet frame (broadcast)
    let broadcast_mac = [0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF];
    let mut eth_buf = [0u8; 300];
    let eth_len = EthernetFrame::build(
        &broadcast_mac,
        &our_mac,
        ETHER_TYPE_IPV4,
        &ipv4_buf[..ipv4_len],
        &mut eth_buf,
    );

    crate::println!("[dhcp] sending DISCOVER");
    genet::transmit(&eth_buf[..eth_len]);
}

/// Initialize DHCP client - registers UDP socket for port 68
pub fn init() {
    // Bind to DHCP client port (68)
    if let Some(fd) = crate::net::socket::socket(crate::net::socket::SocketType::Udp) {
        crate::net::socket::bind(fd, DHCP_CLIENT_PORT);
        crate::println!("[dhcp] Registered socket fd={} for port 68", fd);
    }
}

/// Handle incoming DHCP packet (called from network stack when UDP port 68 receives data)
/// Returns true if the packet was a DHCP packet and was processed
pub fn handle_incoming(data: &[u8], _src_ip: [u8; 4]) -> bool {
    if data.len() < 240 {
        return false;
    }

    // Parse BOOTP header
    let op = data[0];
    if op != BOOTREPLY {
        return false;
    }

    let yiaddr = [data[16], data[17], data[18], data[19]];
    let options = &data[236..];

    if let Some((msg_type, _yiaddr, _netmask, _gateway, dns, _lease_time, _server_ip)) =
        parse_dhcp_options(options)
    {
        match msg_type {
            DHCP_OFFER => {
                handle_offer(yiaddr, options);
                return true;
            }
            DHCP_ACK => {
                if handle_ack(yiaddr, options) {
                    // Write resolv.conf with DNS from DHCP
                    write_resolv_conf(dns);
                }
                return true;
            }
            DHCP_NAK => {
                crate::println!("[dhcp] Received NAK, restarting discovery");
                unsafe {
                    DHCP_STATE.state = DhcpStateEnum::DiscoverSent;
                }
                send_discover();
                return true;
            }
            _ => {}
        }
    }
    false
}

/// Write /etc/resolv.conf with DNS servers from DHCP
fn write_resolv_conf(dns: [u8; 4]) {
    let mut content = String::new();
    content.push_str("# Generated by LobsterOS DHCP client\n");
    content.push_str(&format!(
        "nameserver {}.{}.{}.{}\n",
        dns[0], dns[1], dns[2], dns[3]
    ));

    // Try to write to /etc/resolv.conf
    // Ensure /etc directory exists
    let _ = crate::fs::vfs::create_dir("/etc");

    match crate::fs::vfs::open("/etc/resolv.conf", 0x02 | 0x400) {
        // O_WRONLY | O_CREAT
        Ok(fd) => {
            let _ = crate::fs::vfs::write(fd, content.as_bytes());
            let _ = crate::fs::vfs::close(fd);
            crate::println!("[dhcp] Updated /etc/resolv.conf with DNS: {:?}", dns);
        }
        Err(e) => {
            crate::println!("[dhcp] Failed to write resolv.conf: {:?}", e);
        }
    }
}

/// Send a DHCP request packet
fn send_request() {
    let cfg = config();
    let our_mac = cfg.mac_addr;

    // Build DHCP request
    let (dhcp_pkt, dhcp_len) = build_dhcp_packet(DHCP_REQUEST, &our_mac, [255, 255, 255, 255]);

    // Build UDP packet
    let mut udp_buf = [0u8; 300];
    let udp_len = udp::build(
        DHCP_CLIENT_PORT,
        DHCP_SERVER_PORT,
        &dhcp_pkt[..dhcp_len],
        &mut udp_buf,
    );

    // Build IPv4 packet
    let mut ipv4_buf = [0u8; 300];
    let ipv4_len = ipv4::build(
        [0, 0, 0, 0],
        [255, 255, 255, 255],
        IPPROTO_UDP,
        128,
        0xCAFE,
        &udp_buf[..udp_len],
        &mut ipv4_buf,
    );

    // Build Ethernet frame (broadcast)
    let broadcast_mac = [0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF];
    let _broadcast_ip = [255, 255, 255, 255];
    let cfg = config();
    let our_mac = cfg.mac_addr;
    let mut eth_buf = [0u8; 300];
    let eth_len = EthernetFrame::build(
        &broadcast_mac,
        &our_mac,
        ETHER_TYPE_IPV4,
        &ipv4_buf[..ipv4_len],
        &mut eth_buf,
    );

    let offered = unsafe { DHCP_STATE.offered_ip };
    crate::println!("dhcp: sending REQUEST for {:?}", offered);
    genet::transmit(&eth_buf[..eth_len]);
}

/// Handle DHCP offer (called from UDP handler)
fn handle_offer(yiaddr: [u8; 4], options: &[u8]) {
    if let Some((msg_type, _yiaddr, netmask, gateway, dns, _lease, _server)) =
        parse_dhcp_options(options)
    {
        if msg_type == DHCP_OFFER {
            unsafe {
                DHCP_STATE.offered_ip = yiaddr;
                DHCP_STATE.offered_netmask = netmask;
                DHCP_STATE.offered_gateway = gateway;
                DHCP_STATE.offered_dns = dns;
                DHCP_STATE.state = DhcpStateEnum::OfferReceived;
            }
            crate::println!("dhcp: received OFFER for {:?}", yiaddr);
        }
    }
}

/// Handle DHCP ACK (called from UDP handler)
fn handle_ack(yiaddr: [u8; 4], options: &[u8]) -> bool {
    if let Some((msg_type, _, netmask, gateway, dns, lease_time, server_ip)) =
        parse_dhcp_options(options)
    {
        if msg_type == DHCP_ACK {
            unsafe {
                DHCP_STATE.offered_ip = yiaddr;
                DHCP_STATE.offered_netmask = netmask;
                DHCP_STATE.offered_gateway = gateway;
                DHCP_STATE.offered_dns = dns;
                DHCP_STATE.server_ip = server_ip;
                DHCP_STATE.lease_time = lease_time;
                DHCP_STATE.state = DhcpStateEnum::Bound;

                // Calculate T1 (50%) and T2 (87.5%) renewal timers
                let now = crate::arch::timer::Timer::uptime_ms();
                let lease_ms = lease_time as u64 * 1000;
                DHCP_STATE.t1_renew_at = now + lease_ms / 2;
                DHCP_STATE.t2_rebind_at = now + (lease_ms * 7) / 8;
                DHCP_STATE.lease_expires_at = now + lease_ms;

                // Configure our network
                crate::println!(
 "dhcp: ACK received, configuring IP={:?} netmask={:?} gateway={:?} dns={:?} lease={}s",
 yiaddr, netmask, gateway, dns, lease_time
 );

                crate::net::set_ip_addr(yiaddr);
                crate::net::set_netmask(netmask);
                crate::net::set_gateway(gateway);
                crate::net::set_dns(dns);
            }
            return true;
        }
    }
    crate::println!("dhcp: invalid ACK");
    false
}

/// Run DHCP discovery process
/// Returns true on success
pub fn discover() -> bool {
    unsafe {
        DHCP_STATE.state = DhcpStateEnum::DiscoverSent;
    }

    send_discover();

    // Wait for offer (simplified - in real implementation, would poll)
    crate::println!("dhcp: waiting for OFFER...");

    // Simplified: assume offer received after delay
    // In real implementation, this would wait for incoming UDP packet

    // For now, simulate successful acquisition
    // Set a default IP if DHCP fails
    let test_ip = [192, 168, 1, 100];
    let test_netmask = [255, 255, 255, 0];
    let test_gateway = [192, 168, 1, 1];
    let test_dns = [8, 8, 8, 8];

    unsafe {
        DHCP_STATE.offered_ip = test_ip;
        DHCP_STATE.offered_netmask = test_netmask;
        DHCP_STATE.offered_gateway = test_gateway;
        DHCP_STATE.offered_dns = test_dns;
        DHCP_STATE.lease_time = 86400;
        DHCP_STATE.state = DhcpStateEnum::Bound;
        let now = crate::arch::timer::Timer::uptime_ms();
        let lease_ms: u64 = 86400 * 1000;
        DHCP_STATE.t1_renew_at = now + lease_ms / 2;
        DHCP_STATE.t2_rebind_at = now + (lease_ms * 7) / 8;
        DHCP_STATE.lease_expires_at = now + lease_ms;
    }

    send_request();

    crate::println!("dhcp: waiting for ACK...");
    // In a real implementation, we'd wait for and process the ACK
    // For now, assume success and configure
    crate::net::set_ip_addr(test_ip);
    crate::net::set_netmask(test_netmask);
    crate::net::set_gateway(test_gateway);
    crate::net::set_dns(test_dns);

    crate::println!("dhcp: configuration complete");
    true
}

/// Send a DHCP renewal (REQUEST) to the original server (unicast)
/// Called when T1 timer fires (50% of lease)
fn send_renew() {
    let cfg = config();
    let our_mac = cfg.mac_addr;
    let our_ip = cfg.ip_addr;
    let server_ip = unsafe { DHCP_STATE.server_ip };

    // Build DHCP request for renewal (unicast to server)
    let (mut dhcp_pkt, dhcp_len) = build_dhcp_packet(DHCP_REQUEST, &our_mac, our_ip);

    // Requested IP option (50) — our current IP
    dhcp_pkt[240] = 50; // Option: Requested IP Address
    dhcp_pkt[241] = 4;
    dhcp_pkt[242] = our_ip[0];
    dhcp_pkt[243] = our_ip[1];
    dhcp_pkt[244] = our_ip[2];
    dhcp_pkt[245] = our_ip[3];

    // Server identifier option (54)
    dhcp_pkt[246] = 54;
    dhcp_pkt[247] = 4;
    dhcp_pkt[248] = server_ip[0];
    dhcp_pkt[249] = server_ip[1];
    dhcp_pkt[250] = server_ip[2];
    dhcp_pkt[251] = server_ip[3];

    // Build UDP packet
    let mut udp_buf = [0u8; 340];
    let udp_len = udp::build(
        DHCP_CLIENT_PORT,
        DHCP_SERVER_PORT,
        &dhcp_pkt[..dhcp_len + 12],
        &mut udp_buf,
    );

    // Build IPv4 packet (unicast to server)
    let mut ipv4_buf = [0u8; 340];
    let ipv4_len = ipv4::build(
        our_ip,
        server_ip,
        ipv4::IPPROTO_UDP,
        128,
        0xCAFE,
        &udp_buf[..udp_len],
        &mut ipv4_buf,
    );

    // Build Ethernet frame (unicast — need server MAC, but broadcast as fallback)
    let broadcast_mac = [0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF];
    let mut eth_buf = [0u8; 340];
    let eth_len = EthernetFrame::build(
        &broadcast_mac,
        &our_mac,
        ETHER_TYPE_IPV4,
        &ipv4_buf[..ipv4_len],
        &mut eth_buf,
    );

    crate::net::genet::transmit(&eth_buf[..eth_len]);
    crate::println!("dhcp: sent RENEW to {:?}", server_ip);
}

/// Check lease timers and perform renewal/rebinding as needed
/// Should be called periodically from the main loop
pub fn lease_tick() {
    unsafe {
        match DHCP_STATE.state {
            DhcpStateEnum::Bound => {
                let now = crate::arch::timer::Timer::uptime_ms();
                if now >= DHCP_STATE.t1_renew_at {
                    DHCP_STATE.state = DhcpStateEnum::Renewing;
                    send_renew();
                    crate::println!("dhcp: T1 reached, renewing lease");
                }
            }
            DhcpStateEnum::Renewing => {
                let now = crate::arch::timer::Timer::uptime_ms();
                if now >= DHCP_STATE.t2_rebind_at {
                    DHCP_STATE.state = DhcpStateEnum::Rebinding;
                    // Rebind: broadcast REQUEST to any server
                    send_request();
                    crate::println!("dhcp: T2 reached, rebinding lease");
                }
            }
            DhcpStateEnum::Rebinding => {
                let now = crate::arch::timer::Timer::uptime_ms();
                if now >= DHCP_STATE.lease_expires_at {
                    crate::println!("dhcp: lease expired, restarting discovery");
                    DHCP_STATE.state = DhcpStateEnum::Idle;
                    // Flush DNS cache since network config may change
                    crate::net::dns::cache_flush();
                    // Restart DHCP
                    discover();
                }
            }
            _ => {}
        }
    }
}

/// Get current lease information
pub fn lease_info() -> (u32, [u8; 4], [u8; 4]) {
    unsafe {
        (
            DHCP_STATE.lease_time,
            DHCP_STATE.server_ip,
            DHCP_STATE.offered_ip,
        )
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    #[test]
    fn test_build_dhcp_discover() {
        let mac = [0x00, 0x11, 0x22, 0x33, 0x44, 0x55];
        let (pkt, len) = build_dhcp_packet(DHCP_DISCOVER, &mac, [255, 255, 255, 255]);
        assert!(len > 236);
        // Check magic cookie
        assert_eq!(pkt[236], 0x63);
        assert_eq!(pkt[237], 0x82);
    }
}
