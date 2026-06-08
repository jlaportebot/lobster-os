//! UDP (User Datagram Protocol) implementation
//!
//! Handles UDP packet parsing and building.
//! No_std compatible - uses only core.

use crate::net::config;
use crate::net::eth::{EthernetFrame, ETHER_TYPE_IPV4};
use crate::net::genet;
use crate::net::ipv4::{self, IPPROTO_UDP};

/// UDP header length
pub const HEADER_LEN: usize = 8;

/// UDP header structure
#[derive(Clone, Copy, Debug)]
pub struct UdpHeader {
    pub src_port: u16,
    pub dst_port: u16,
    pub length: u16,
    pub checksum: u16,
}

impl UdpHeader {
    /// Get header weight (for pseudo header checksum)
    pub fn header_weight(&self) -> u32 {
        (((self.src_port as u32) << 8) | (self.src_port as u32 >> 8))
            + (((self.dst_port as u32) << 8) | (self.dst_port as u32 >> 8))
            + (self.length as u32)
            + (((self.checksum as u32) << 8) | (self.checksum as u32 >> 8))
    }
}

/// Parse a UDP header from raw bytes
pub fn parse(data: &[u8]) -> Option<UdpHeader> {
    if data.len() < HEADER_LEN {
        crate::println!("udp: packet too short ({} < {})", data.len(), HEADER_LEN);
        return None;
    }

    let src_port = u16::from_be_bytes([data[0], data[1]]);
    let dst_port = u16::from_be_bytes([data[2], data[3]]);
    let length = u16::from_be_bytes([data[4], data[5]]);
    let checksum = u16::from_be_bytes([data[6], data[7]]);

    Some(UdpHeader {
        src_port,
        dst_port,
        length,
        checksum,
    })
}

/// Build a UDP header and packet
pub fn build(src_port: u16, dst_port: u16, payload: &[u8], out: &mut [u8]) -> usize {
    let udp_len = (HEADER_LEN + payload.len()) as u16;

    if out.len() < udp_len as usize {
        crate::println!("udp: output buffer too small");
        return 0;
    }

    // Source port
    out[0] = (src_port >> 8) as u8;
    out[1] = (src_port & 0xFF) as u8;
    // Destination port
    out[2] = (dst_port >> 8) as u8;
    out[3] = (dst_port & 0xFF) as u8;
    // Length
    out[4] = (udp_len >> 8) as u8;
    out[5] = (udp_len & 0xFF) as u8;
    // Checksum (set to 0 for now - optional in IPv4)
    out[6] = 0;
    out[7] = 0;
    // Payload
    if !payload.is_empty() {
        out[8..8 + payload.len()].copy_from_slice(payload);
    }

    udp_len as usize
}

/// Compute UDP checksum (with pseudo-header)
pub fn compute_checksum(ip_src: [u8; 4], ip_dst: [u8; 4], udp_len: u16, data: &[u8]) -> u16 {
    // For now, return 0 (checksum is optional in IPv4 UDP)
    // Full implementation would compute the pseudo-header checksum
    let mut sum: u32 = 0;

    // Pseudo-header
    for i in 0..4 {
        sum += (ip_src[i] as u32) << 8;
        sum += (ip_src[i] as u32) >> 8;
        sum += (ip_dst[i] as u32) << 8;
        sum += (ip_dst[i] as u32) >> 8;
    }

    // Protocol
    sum += IPPROTO_UDP as u32;
    // UDPLength
    sum += udp_len as u32;

    // UDP header and data
    let mut i = 0;
    while i < data.len() - 1 {
        let word = ((data[i] as u32) << 8) | (data[i + 1] as u32);
        sum += word;
        i += 2;
    }

    // Handle odd length
    if data.len() % 2 == 1 {
        sum += (data[data.len() - 1] as u32) << 8;
    }

    // Fold carry
    while (sum >> 16) != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }

    !sum as u16
}

/// Build and send a UDP packet
pub fn send_packet(src_ip: [u8; 4], dst_ip: [u8; 4], src_port: u16, dst_port: u16, payload: &[u8]) {
    let cfg = config();
    let our_mac = cfg.mac_addr;

    // Resolve MAC address for destination
    let dst_mac = match crate::net::arp::resolve(dst_ip) {
        Some(mac) => mac,
        None => {
            crate::println!("udp: cannot resolve MAC for {:?}", dst_ip);
            return;
        }
    };

    // Build UDP packet
    let mut udp_buf = [0u8; 1500];
    let udp_len = build(src_port, dst_port, payload, &mut udp_buf);

    // Compute and set checksum
    let checksum = compute_checksum(src_ip, dst_ip, udp_len as u16, &udp_buf[..udp_len]);
    udp_buf[6] = (checksum >> 8) as u8;
    udp_buf[7] = (checksum & 0xFF) as u8;

    // Build IPv4 packet
    let mut ipv4_buf = [0u8; 1500];
    let ipv4_len = ipv4::build(
        src_ip,
        dst_ip,
        IPPROTO_UDP,
        64,     // TTL
        0x9ABC, // Identification
        &udp_buf[..udp_len],
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

    crate::println!(
        "udp: sending {} bytes to {:?}:{}",
        udp_len,
        dst_ip,
        dst_port
    );
    genet::transmit(&eth_buf[..eth_len]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_udp() {
        let data = [0x12, 0x34, 0x56, 0x78, 0x00, 0x0C, 0x00, 0x00];
        let udp = parse(&data).unwrap();
        assert_eq!(udp.src_port, 0x1234);
        assert_eq!(udp.dst_port, 0x5678);
        assert_eq!(udp.length, 12);
    }

    #[test]
    fn test_build_udp() {
        let payload = [0x41, 0x42, 0x43];
        let mut out = [0u8; 50];
        let len = build(1234, 5678, &payload, &mut out);
        assert_eq!(len, 11); // 8 bytes header + 3 bytes payload
        assert_eq!(u16::from_be_bytes([out[0], out[1]]), 1234);
        assert_eq!(u16::from_be_bytes([out[2], out[3]]), 5678);
        assert_eq!(out[8], 0x41);
    }
}
