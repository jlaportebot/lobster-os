//! ICMPv4 (Internet Control Message Protocol) implementation
//!
//! Handles ICMP echo requests/replies.
//! No_std compatible - uses only core.

use crate::net::config;
use crate::net::eth::{EthernetFrame, ETHER_TYPE_IPV4};
use crate::net::genet;
use crate::net::ipv4::{self, Ipv4Header, IPPROTO_ICMP};

/// ICMP Type constants
pub const ICMP_TYPE_ECHO_REPLY: u8 = 0;
pub const ICMP_TYPE_ECHO_REQUEST: u8 = 8;

/// ICMP header structure
#[derive(Clone, Copy, Debug)]
pub struct IcmpHeader {
    pub r#type: u8,
    pub code: u8,
    pub checksum: u16,
    // For echo: identifier and sequence number
    pub identifier: u16,
    pub sequence: u16,
}

/// Compute ICMP checksum
fn compute_checksum(data: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    let mut i = 0;

    while i < data.len() - 1 {
        let word = ((data[i] as u32) << 8) | (data[i + 1] as u32);
        sum += word;
        i += 2;
    }

    // Handle odd-length data
    if data.len() % 2 == 1 {
        sum += (data[data.len() - 1] as u32) << 8;
    }

    // Fold carry
    while (sum >> 16) != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }

    !sum as u16
}

/// Handle an incoming ICMP packet
pub fn handle_packet(ip_hdr: &Ipv4Header, data: &[u8]) {
    if data.len() < 8 {
        crate::println!("icmp: packet too short");
        return;
    }

    let icmp_type = data[0];
    let _code = data[1];
    let checksum = u16::from_be_bytes([data[2], data[3]]);

    // Verify checksum
    let calc_checksum = compute_checksum(data);
    if checksum != calc_checksum {
        crate::println!("icmp: checksum verification failed");
        return;
    }

    if icmp_type == ICMP_TYPE_ECHO_REQUEST {
        // Extract identifier and sequence
        let identifier = u16::from_be_bytes([data[4], data[5]]);
        let sequence = u16::from_be_bytes([data[6], data[7]]);

        crate::println!(
            "icmp: received echo request id={} seq={}",
            identifier,
            sequence
        );

        // Send echo reply
        send_echo_reply(ip_hdr.src_addr, identifier, sequence, &data[8..]);
    } else if icmp_type == ICMP_TYPE_ECHO_REPLY {
        crate::println!("icmp: received echo reply");
    } else {
        crate::println!("icmp: unknown type: {}", icmp_type);
    }
}

/// Send an ICMP echo request
pub fn send_echo_request(dst: [u8; 4], seq: u16) {
    let cfg = config();
    let our_mac = cfg.mac_addr;
    let our_ip = cfg.ip_addr;

    // Resolve MAC address for destination
    let dst_mac = match crate::net::arp::resolve(dst) {
        Some(mac) => mac,
        None => {
            crate::println!("icmp: cannot resolve MAC for {:?}", dst);
            return;
        }
    };

    // Build ICMP payload (can include data)
    let mut icmp_data = [0u8; 64];
    icmp_data[0] = ICMP_TYPE_ECHO_REQUEST;
    icmp_data[1] = 0; // code
    icmp_data[2] = 0; // checksum (will be computed)
    icmp_data[3] = 0;
    icmp_data[4] = 0; // identifier
    icmp_data[5] = 0;
    icmp_data[6] = (seq >> 8) as u8;
    icmp_data[7] = (seq & 0xFF) as u8;

    // Add some payload data
    for i in 8..64 {
        icmp_data[i] = i as u8;
    }

    // Compute checksum
    let checksum = compute_checksum(&icmp_data);
    icmp_data[2] = (checksum >> 8) as u8;
    icmp_data[3] = (checksum & 0xFF) as u8;

    // Build IPv4 packet
    let mut ipv4_buf = [0u8; 1500];
    let ipv4_len = ipv4::build(
        our_ip,
        dst,
        IPPROTO_ICMP,
        64,     // TTL
        0x1234, // Identification
        &icmp_data,
        &mut ipv4_buf,
    );

    // Build Ethernet frame
    let mut eth_buf = [0u8; 1500];
    let eth_len = EthernetFrame::build(
        &dst_mac,
        &our_mac,
        ETHER_TYPE_IPV4,
        &ipv4_buf[..ipv4_len],
        &mut eth_buf,
    );

    crate::println!("icmp: sending echo request to {:?}", dst);
    genet::transmit(&eth_buf[..eth_len]);
}

/// Send an ICMP echo reply
fn send_echo_reply(dst: [u8; 4], identifier: u16, sequence: u16, payload: &[u8]) {
    let cfg = config();
    let our_mac = cfg.mac_addr;
    let our_ip = cfg.ip_addr;

    // Resolve MAC address for destination
    let dst_mac = match crate::net::arp::resolve(dst) {
        Some(mac) => mac,
        None => {
            crate::println!("icmp: cannot resolve MAC for {:?}", dst);
            return;
        }
    };

    // Build ICMP reply
    let mut icmp_data = [0u8; 64];
    icmp_data[0] = ICMP_TYPE_ECHO_REPLY;
    icmp_data[1] = 0; // code
    icmp_data[2] = 0; // checksum (will be computed)
    icmp_data[3] = 0;
    icmp_data[4] = (identifier >> 8) as u8;
    icmp_data[5] = (identifier & 0xFF) as u8;
    icmp_data[6] = (sequence >> 8) as u8;
    icmp_data[7] = (sequence & 0xFF) as u8;

    // Copy payload
    let payload_len = payload.min(&icmp_data[8..]).len();
    icmp_data[8..8 + payload_len].copy_from_slice(&payload[..payload_len]);

    // Compute checksum
    let checksum = compute_checksum(&icmp_data[..8 + payload_len]);
    icmp_data[2] = (checksum >> 8) as u8;
    icmp_data[3] = (checksum & 0xFF) as u8;

    // Build IPv4 packet
    let mut ipv4_buf = [0u8; 1500];
    let ipv4_len = ipv4::build(
        our_ip,
        dst,
        IPPROTO_ICMP,
        64,     // TTL
        0x5678, // Identification
        &icmp_data[..8 + payload_len],
        &mut ipv4_buf,
    );

    // Build Ethernet frame
    let mut eth_buf = [0u8; 1500];
    let eth_len = EthernetFrame::build(
        &dst_mac,
        &our_mac,
        ETHER_TYPE_IPV4,
        &ipv4_buf[..ipv4_len],
        &mut eth_buf,
    );

    crate::println!("icmp: sending echo reply to {:?}", dst);
    genet::transmit(&eth_buf[..eth_len]);
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    #[test]
    fn test_icmp_checksum() {
        let mut data = [0u8; 8];
        data[0] = ICMP_TYPE_ECHO_REQUEST;
        data[1] = 0;
        data[4] = 0;
        data[5] = 0;
        data[6] = 0;
        data[7] = 1;

        let checksum = compute_checksum(&data);
        assert_ne!(checksum, 0);
    }
}
