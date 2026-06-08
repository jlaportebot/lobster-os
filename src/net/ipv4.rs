//! IPv4 packet parser/builder
//!
//! Handles IPv4 header parsing and building.
//! No_std compatible - uses only core.

use crate::net::eth::{EthernetFrame, ETHER_TYPE_IPV4};
use crate::net::genet;

/// Protocol numbers
pub const IPPROTO_ICMP: u8 = 1;
pub const IPPROTO_TCP: u8 = 6;
pub const IPPROTO_UDP: u8 = 17;

/// Minimum IPv4 header length
pub const IPV4_HEADER_MIN_LEN: u8 = 20;

/// IPv4 header structure
#[derive(Clone, Copy, Debug)]
pub struct Ipv4Header {
    pub version: u8,
    pub ihl: u8,
    pub dscp: u8,
    pub total_len: u16,
    pub identification: u16,
    pub flags: u8,
    pub frag_offset: u16,
    pub ttl: u8,
    pub protocol: u8,
    pub checksum: u16,
    pub src_addr: [u8; 4],
    pub dst_addr: [u8; 4],
}

impl Ipv4Header {
    /// Get header length in bytes
    pub fn header_len(&self) -> usize {
        (self.ihl as usize) * 4
    }

    /// Verify/checksum the IPv4 header
    fn checksum_verify(&self) -> bool {
        let header_len = self.header_len();
        let mut sum: u32 = 0;
        let mut buf = [0u8; 40];

        // Copy header bytes, zeroing checksum field
        let mut pos = 0;
        for i in 0..header_len {
            if i == 10 || i == 11 {
                // Skip checksum field
                buf[pos] = 0;
                pos += 1;
                buf[pos] = 0;
            } else {
                buf[pos] = match i {
                    0 => (self.version << 4) | self.dscp >> 4,
                    1 => (self.dscp & 0x0F) << 4 | self.flags >> 4,
                    2 => (((self.flags & 0x0F) << 4) | (self.frag_offset >> 8) as u8) as u8,
                    3 => (self.frag_offset & 0xFF) as u8,
                    4 => (self.total_len >> 8) as u8,
                    5 => (self.total_len & 0xFF) as u8,
                    6 => (self.identification >> 8) as u8,
                    7 => (self.identification & 0xFF) as u8,
                    8 => self.ttl,
                    9 => self.protocol,
                    12..=15 => self.src_addr[i - 12],
                    16..=19 => self.dst_addr[i - 16],
                    _ => 0,
                };
            }
            pos += 1;
        }

        // Sum 16-bit words
        let len = header_len;
        let mut i = 0;
        while i < len {
            let word = ((buf[i] as u32) << 8) | (buf[i + 1] as u32);
            sum += word;
            i += 2;
        }

        // Fold carry
        while (sum >> 16) != 0 {
            sum = (sum & 0xFFFF) + (sum >> 16);
        }

        let result = !sum as u16;
        result == self.checksum
    }

    /// Compute checksum for IPv4 header
    fn compute_checksum(header: &Ipv4Header) -> u16 {
        let header_len = header.header_len();
        let mut sum: u32 = 0;
        let mut buf = [0u8; 40];

        // Build header bytes with zeroed checksum
        buf[0] = (header.version << 4) | (header.dscp >> 4);
        buf[1] = (header.dscp & 0x0F) << 4 | (header.flags >> 4);
        buf[2] = ((header.flags & 0x0F) << 4) | (header.frag_offset >> 8) as u8;
        buf[3] = (header.frag_offset & 0xFF) as u8;
        buf[4] = (header.total_len >> 8) as u8;
        buf[5] = (header.total_len & 0xFF) as u8;
        buf[6] = (header.identification >> 8) as u8;
        buf[7] = (header.identification & 0xFF) as u8;
        buf[8] = header.ttl;
        buf[9] = header.protocol;
        buf[10] = 0; // checksum will be computed
        buf[11] = 0;
        buf[12..16].copy_from_slice(&header.src_addr);
        buf[16..20].copy_from_slice(&header.dst_addr);

        // Sum 16-bit words
        let mut i = 0;
        while i < header_len {
            let word = ((buf[i] as u32) << 8) | (buf[i + 1] as u32);
            sum += word;
            i += 2;
        }

        // Fold carry
        while (sum >> 16) != 0 {
            sum = (sum & 0xFFFF) + (sum >> 16);
        }

        !sum as u16
    }
}

/// Parse an IPv4 header from raw bytes
pub fn parse(data: &[u8]) -> Option<Ipv4Header> {
    if data.len() < 20 {
        crate::println!("ipv4: packet too short ({} < 20)", data.len());
        return None;
    }

    let version_dscp = data[0];
    let version = version_dscp >> 4;
    let _dscp = version_dscp & 0x3F;

    if version != 4 {
        crate::println!("ipv4: not IPv4 (version={})", version);
        return None;
    }

    let flags_frag = ((data[2] as u16) << 8) | (data[3] as u16);
    let _flags = (flags_frag >> 8) as u8;
    let _frag_offset = flags_frag & 0x1FFF;

    let ihl = data[0] & 0x0F;
    if ihl < 5 {
        crate::println!("ipv4: invalid IHL ({})", ihl);
        return None;
    }

    let header_len = (ihl as usize) * 4;
    if data.len() < header_len {
        crate::println!(
            "ipv4: packet too short for header ({} < {})",
            data.len(),
            header_len
        );
        return None;
    }

    let total_len = u16::from_be_bytes([data[2], data[3]]);
    let identification = u16::from_be_bytes([data[4], data[5]]);
    let ttl = data[8];
    let protocol = data[9];
    let checksum = u16::from_be_bytes([data[10], data[11]]);

    let mut src_addr = [0u8; 4];
    let mut dst_addr = [0u8; 4];
    src_addr.copy_from_slice(&data[12..16]);
    dst_addr.copy_from_slice(&data[16..20]);

    let header = Ipv4Header {
        version,
        ihl,
        dscp: (version_dscp & 0x0F) << 2, // Adjust dscp
        total_len,
        identification,
        flags: (flags_frag >> 13) as u8,
        frag_offset: flags_frag & 0x1FFF,
        ttl,
        protocol,
        checksum,
        src_addr,
        dst_addr,
    };

    // Verify checksum
    if !header.checksum_verify() {
        crate::println!("ipv4: checksum verification failed");
        return None;
    }

    Some(header)
}

/// Build an IPv4 packet
pub fn build(
    src: [u8; 4],
    dst: [u8; 4],
    protocol: u8,
    ttl: u8,
    identification: u16,
    payload: &[u8],
    out: &mut [u8],
) -> usize {
    let header_len = 20; // minimum header, no options
    let total_len = (header_len + payload.len()) as u16;

    if out.len() < total_len as usize {
        crate::println!("ipv4: output buffer too small");
        return 0;
    }

    // Version and IHL
    out[0] = (4 << 4) | 5;
    // DSCP and ECN
    out[1] = 0;
    // Total length
    out[2] = (total_len >> 8) as u8;
    out[3] = (total_len & 0xFF) as u8;
    // Identification
    out[4] = (identification >> 8) as u8;
    out[5] = (identification & 0xFF) as u8;
    // Flags and fragment offset
    out[6] = 0x40; // Don't fragment
    out[7] = 0;
    // TTL
    out[8] = ttl;
    // Protocol
    out[9] = protocol;
    // Checksum (will be computed)
    out[10] = 0;
    out[11] = 0;
    // Source address
    out[12..16].copy_from_slice(&src);
    // Destination address
    out[16..20].copy_from_slice(&dst);
    // Payload
    if !payload.is_empty() {
        out[20..20 + payload.len()].copy_from_slice(payload);
    }

    // Compute checksum
    let checksum = Ipv4Header::compute_checksum(&Ipv4Header {
        version: 4,
        ihl: 5,
        dscp: 0,
        total_len,
        identification,
        flags: 0x4,
        frag_offset: 0,
        ttl,
        protocol,
        checksum: 0,
        src_addr: src,
        dst_addr: dst,
    });
    out[10] = (checksum >> 8) as u8;
    out[11] = (checksum & 0xFF) as u8;

    total_len as usize
}

/// Build and send an IPv4 packet
pub fn send_packet(
    src_mac: &[u8; 6],
    dst_mac: &[u8; 6],
    src: [u8; 4],
    dst: [u8; 4],
    protocol: u8,
    ttl: u8,
    identification: u16,
    payload: &[u8],
) {
    let mut buffer = [0u8; 1500];
    let total_len = build(
        src,
        dst,
        protocol,
        ttl,
        identification,
        payload,
        &mut buffer,
    );

    let mut eth_buf = [0u8; 1500];
    let eth_len = EthernetFrame::build(
        dst_mac,
        src_mac,
        ETHER_TYPE_IPV4,
        &buffer[..total_len],
        &mut eth_buf,
    );

    crate::println!("ipv4: sending {} bytes to {:?}", total_len, dst);
    genet::transmit(&eth_buf[..eth_len]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_ipv4() {
        let mut data = [0u8; 40];
        data[0] = 0x45; // Version 4, IHL 5
        data[1] = 0x00;
        data[2] = 0x00;
        data[3] = 0x28; // Total length 40
        data[8] = 0x40; // TTL 64
        data[9] = IPPROTO_ICMP;

        // Compute and set checksum
        let header = Ipv4Header {
            version: 4,
            ihl: 5,
            dscp: 0,
            total_len: 40,
            identification: 0,
            flags: 0x4,
            frag_offset: 0,
            ttl: 64,
            protocol: IPPROTO_ICMP,
            checksum: 0,
            src_addr: [192, 168, 1, 1],
            dst_addr: [192, 168, 1, 2],
        };
        let checksum = Ipv4Header::compute_checksum(&header);
        data[10] = (checksum >> 8) as u8;
        data[11] = (checksum & 0xFF) as u8;
        data[12..16].copy_from_slice(&[192, 168, 1, 1]);
        data[16..20].copy_from_slice(&[192, 168, 1, 2]);

        let parsed = parse(&data).unwrap();
        assert_eq!(parsed.version, 4);
        assert_eq!(parsed.protocol, IPPROTO_ICMP);
    }
}
