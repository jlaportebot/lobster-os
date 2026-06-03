//! Ethernet II frame parser/builder
//! 
//! Provides functions to parse and build Ethernet frames.
//! No_std compatible - uses only core.

use core::mem::size_of;

/// Ethernet type constants
pub const ETHER_TYPE_IPV4: u16 = 0x0800;
pub const ETHER_TYPE_ARP: u16 = 0x0806;
pub const ETHER_TYPE_IPV6: u16 = 0x86DD;
pub const HEADER_LEN: usize = 14;

/// Ethernet frame header
#[derive(Clone, Copy, Debug)]
pub struct EthernetFrame {
    pub dst_mac: [u8; 6],
    pub src_mac: [u8; 6],
    pub ether_type: u16,
}

impl EthernetFrame {
    /// Parse an Ethernet frame from raw bytes
    /// Returns None if the data is too short or invalid
    pub fn parse(data: &[u8]) -> Option<EthernetFrame> {
        if data.len() < HEADER_LEN {
            crate::println!("eth: frame too short ({} < {})", data.len(), HEADER_LEN);
            return None;
        }

        let mut dst_mac = [0u8; 6];
        let mut src_mac = [0u8; 6];
        
        dst_mac.copy_from_slice(&data[0..6]);
        src_mac.copy_from_slice(&data[6..12]);
        
        // EtherType is big-endian
        let ether_type = u16::from_be_bytes([data[12], data[13]]);

        Some(EthernetFrame {
            dst_mac,
            src_mac,
            ether_type,
        })
    }

    /// Build an Ethernet frame into the output buffer
    /// Returns the total length of the frame (header + payload)
    pub fn build(dst: &[u8; 6], src: &[u8; 6], ether_type: u16, payload: &[u8], out: &mut [u8]) -> usize {
        let total_len = HEADER_LEN + payload.len();
        
        if out.len() < total_len {
            crate::println!("eth: output buffer too small ({} < {})", out.len(), total_len);
            return 0;
        }

        // Destination MAC
        out[0..6].copy_from_slice(dst);
        // Source MAC
        out[6..12].copy_from_slice(src);
        // EtherType (big-endian)
        out[12] = (ether_type >> 8) as u8;
        out[13] = (ether_type & 0xFF) as u8;
        // Payload
        out[14..total_len].copy_from_slice(payload);

        total_len
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_ethernet() {
        let mut frame = [0u8; 30];
        frame[0..6].copy_from_slice(&[0x00, 0x11, 0x22, 0x33, 0x44, 0x55]);
        frame[6..12].copy_from_slice(&[0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF]);
        frame[12] = 0x08;
        frame[13] = 0x00; // IPv4
        frame[14] = 0x42;

        let parsed = EthernetFrame::parse(&frame).unwrap();
        assert_eq!(parsed.dst_mac, [0x00, 0x11, 0x22, 0x33, 0x44, 0x55]);
        assert_eq!(parsed.src_mac, [0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF]);
        assert_eq!(parsed.ether_type, ETHER_TYPE_IPV4);
    }

    #[test]
    fn test_build_ethernet() {
        let dst = [0x00, 0x11, 0x22, 0x33, 0x44, 0x55];
        let src = [0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF];
        let payload = [0x42, 0x43, 0x44];
        let mut out = [0u8; 50];

        let len = EthernetFrame::build(&dst, &src, ETHER_TYPE_IPV4, &payload, &mut out);
        assert_eq!(len, HEADER_LEN + payload.len());
        assert_eq!(&out[0..6], &dst);
        assert_eq!(&out[6..12], &src);
        assert_eq!(u16::from_be_bytes([out[12], out[13]]), ETHER_TYPE_IPV4);
        assert_eq!(&out[14..17], &payload);
    }
}