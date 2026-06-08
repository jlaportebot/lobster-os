//! ARP (Address Resolution Protocol) implementation
//!
//! Handles ARP requests and replies, maintains an ARP cache.
//! No_std compatible - uses only core.

use crate::net::config;
use crate::net::eth::{EthernetFrame, ETHER_TYPE_ARP};
use crate::net::genet;
use core::sync::atomic::{AtomicU8, Ordering};

/// ARP hardware type: Ethernet
pub const ARP_HW_TYPE_ETH: u16 = 1;
/// ARP protocol type: IPv4
pub const ARP_PROTO_TYPE_IPV4: u16 = 0x0800;
/// ARP hardware length: 6 bytes (MAC)
pub const ARP_HW_LEN: u8 = 6;
/// ARP protocol length: 4 bytes (IPv4)
pub const ARP_PROTO_LEN: u8 = 4;

/// ARP operation codes
pub const ARP_OPER_REQUEST: u16 = 1;
pub const ARP_OPER_REPLY: u16 = 2;

/// ARP header structure
#[derive(Clone, Copy, Debug)]
pub struct ArpHeader {
    pub htype: u16,
    pub ptype: u16,
    pub hlen: u8,
    pub plen: u8,
    pub oper: u16,
    pub sha: [u8; 6],
    pub spa: [u8; 4],
    pub tha: [u8; 6],
    pub tpa: [u8; 4],
}

/// ARP cache entry
#[derive(Clone, Copy, Debug)]
pub struct ArpEntry {
    pub ip: [u8; 4],
    pub mac: [u8; 6],
    pub valid: bool,
}

/// ARP cache - static array of 16 entries
static mut ARP_CACHE: [ArpEntry; 16] = [ArpEntry {
    ip: [0, 0, 0, 0],
    mac: [0, 0, 0, 0, 0, 0],
    valid: false,
}; 16];
static ARP_CACHE_LOCK: AtomicU8 = AtomicU8::new(0);

/// Initialize ARP module
pub fn init() {
    unsafe {
        for entry in ARP_CACHE.iter_mut() {
            entry.valid = false;
        }
    }
    crate::println!("arp: initialized, {} entries", 16);
}

/// Acquire ARP cache lock (simple spinlock)
fn acquire_lock() {
    while ARP_CACHE_LOCK
        .compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        core::hint::spin_loop();
    }
}

/// Release ARP cache lock
fn release_lock() {
    ARP_CACHE_LOCK.store(0, Ordering::SeqCst);
}

/// Parse an ARP packet from raw bytes
fn parse_arp(data: &[u8]) -> Option<ArpHeader> {
    if data.len() < 28 {
        return None;
    }

    let htype = u16::from_be_bytes([data[0], data[1]]);
    let ptype = u16::from_be_bytes([data[2], data[3]]);
    let hlen = data[4];
    let plen = data[5];
    let oper = u16::from_be_bytes([data[6], data[7]]);

    let mut sha = [0u8; 6];
    let mut spa = [0u8; 4];
    let mut tha = [0u8; 6];
    let mut tpa = [0u8; 4];

    sha.copy_from_slice(&data[8..14]);
    spa.copy_from_slice(&data[14..18]);
    tha.copy_from_slice(&data[18..24]);
    tpa.copy_from_slice(&data[24..28]);

    Some(ArpHeader {
        htype,
        ptype,
        hlen,
        plen,
        oper,
        sha,
        spa,
        tha,
        tpa,
    })
}

/// Handle an ARP packet
pub fn handle_packet(data: &[u8]) {
    let arp = match parse_arp(data) {
        Some(a) => a,
        None => {
            crate::println!("arp: invalid packet");
            return;
        }
    };

    // Validate ARP header
    if arp.htype != ARP_HW_TYPE_ETH || arp.ptype != ARP_PROTO_TYPE_IPV4 {
        crate::println!("arp: unsupported hardware/protocol type");
        return;
    }
    if arp.hlen != ARP_HW_LEN || arp.plen != ARP_PROTO_LEN {
        crate::println!("arp: invalid lengths");
        return;
    }

    let cfg = config();
    let _our_mac = cfg.mac_addr;
    let our_ip = cfg.ip_addr;

    match arp.oper {
        ARP_OPER_REQUEST => {
            // Check if this request is for us
            if arp.tpa == our_ip {
                crate::println!("arp: request for us from {:?}, replying", arp.spa);
                send_reply(arp.sha, our_ip);
            }
        }
        ARP_OPER_REPLY => {
            // Cache the ARP reply
            crate::println!(
                "arp: received reply, caching {:?} -> {:?}",
                arp.spa,
                arp.sha
            );
            acquire_lock();
            for entry in unsafe { ARP_CACHE.iter_mut() } {
                if !entry.valid || entry.ip == [0, 0, 0, 0] {
                    entry.ip = arp.spa;
                    entry.mac = arp.sha;
                    entry.valid = true;
                    break;
                }
            }
            release_lock();
        }
        _ => {
            crate::println!("arp: unknown operation: {}", arp.oper);
        }
    }
}

/// Resolve an IP address to a MAC address
pub fn resolve(ip: [u8; 4]) -> Option<[u8; 6]> {
    // Check cache first
    acquire_lock();
    for entry in unsafe { ARP_CACHE.iter() } {
        if entry.valid && entry.ip == ip {
            let mac = entry.mac;
            release_lock();
            return Some(mac);
        }
    }
    release_lock();

    // Cache miss - send ARP request
    crate::println!("arp: cache miss for {:?}, sending request", ip);
    send_request(ip);
    None
}

/// Send an ARP request
pub fn send_request(target_ip: [u8; 4]) {
    let cfg = config();
    let our_mac = cfg.mac_addr;
    let our_ip = cfg.ip_addr;

    // Build ARP request
    let mut arp_data = [0u8; 28];
    arp_data[0..2].copy_from_slice(&ARP_HW_TYPE_ETH.to_be_bytes());
    arp_data[2..4].copy_from_slice(&ARP_PROTO_TYPE_IPV4.to_be_bytes());
    arp_data[4] = ARP_HW_LEN;
    arp_data[5] = ARP_PROTO_LEN;
    arp_data[6..8].copy_from_slice(&ARP_OPER_REQUEST.to_be_bytes());
    arp_data[8..14].copy_from_slice(&our_mac);
    arp_data[14..18].copy_from_slice(&our_ip);
    // Target hardware address (broadcast)
    arp_data[18..24].copy_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]);
    arp_data[24..28].copy_from_slice(&target_ip);

    // Build Ethernet frame with broadcast destination
    let broadcast_mac = [0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF];
    let mut eth_buf = [0u8; 60];
    let total_len = EthernetFrame::build(
        &broadcast_mac,
        &our_mac,
        ETHER_TYPE_ARP,
        &arp_data,
        &mut eth_buf,
    );

    crate::println!("arp: sending request for {:?}", target_ip);
    genet::transmit(&eth_buf[..total_len]);
}

/// Send an ARP reply
pub fn send_reply(target_mac: [u8; 6], target_ip: [u8; 4]) {
    let cfg = config();
    let our_mac = cfg.mac_addr;
    let our_ip = cfg.ip_addr;

    // Build ARP reply
    let mut arp_data = [0u8; 28];
    arp_data[0..2].copy_from_slice(&ARP_HW_TYPE_ETH.to_be_bytes());
    arp_data[2..4].copy_from_slice(&ARP_PROTO_TYPE_IPV4.to_be_bytes());
    arp_data[4] = ARP_HW_LEN;
    arp_data[5] = ARP_PROTO_LEN;
    arp_data[6..8].copy_from_slice(&ARP_OPER_REPLY.to_be_bytes());
    arp_data[8..14].copy_from_slice(&our_mac);
    arp_data[14..18].copy_from_slice(&our_ip);
    arp_data[18..24].copy_from_slice(&target_mac);
    arp_data[24..28].copy_from_slice(&target_ip);

    // Build Ethernet frame
    let mut eth_buf = [0u8; 60];
    let total_len = EthernetFrame::build(
        &target_mac,
        &our_mac,
        ETHER_TYPE_ARP,
        &arp_data,
        &mut eth_buf,
    );

    crate::println!("arp: sending reply to {:?}", target_ip);
    genet::transmit(&eth_buf[..total_len]);
}

/// Dump the ARP cache to the console
pub fn dump_cache() {
    acquire_lock();
    let mut count = 0;
    for entry in unsafe { ARP_CACHE.iter() } {
        if entry.valid {
            crate::println!(
                "  {}.{}.{}.{} -> {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
                entry.ip[0],
                entry.ip[1],
                entry.ip[2],
                entry.ip[3],
                entry.mac[0],
                entry.mac[1],
                entry.mac[2],
                entry.mac[3],
                entry.mac[4],
                entry.mac[5]
            );
            count += 1;
        }
    }
    release_lock();
    if count == 0 {
        crate::println!("  (cache empty)");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_arp() {
        let mut data = [0u8; 28];
        data[0..2].copy_from_slice(&ARP_HW_TYPE_ETH.to_be_bytes());
        data[2..4].copy_from_slice(&ARP_PROTO_TYPE_IPV4.to_be_bytes());
        data[4] = ARP_HW_LEN;
        data[5] = ARP_PROTO_LEN;
        data[6..8].copy_from_slice(&ARP_OPER_REQUEST.to_be_bytes());

        let arp = parse_arp(&data).unwrap();
        assert_eq!(arp.htype, ARP_HW_TYPE_ETH);
        assert_eq!(arp.oper, ARP_OPER_REQUEST);
    }
}
