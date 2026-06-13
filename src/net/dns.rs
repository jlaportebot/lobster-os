//! DNS resolver module for LobsterOS
//!
//! Implements a minimal DNS client for resolving domain names to IPv4 addresses.
//! No_std compatible - uses only core.

use crate::net::config;
use crate::net::dns_cache::{cache_insert, cache_lookup};
use crate::net::socket::{self, SocketType};
use core::sync::atomic::{AtomicU16, Ordering};

/// DNS port
pub const DNS_PORT: u16 = 53;

/// DNS query type: A record (IPv4 address)
pub const QTYPE_A: u16 = 1;

/// DNS query class: IN (Internet)
pub const QCLASS_IN: u16 = 1;

/// Maximum domain name length
const MAX_DOMAIN_LEN: usize = 253;

/// Maximum DNS packet size
const MAX_DNS_PACKET: usize = 256;

/// DNS header (12 bytes)
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct DnsHeader {
    pub id: u16,
    pub flags: u16,
    pub qdcount: u16,
    pub ancount: u16,
    pub nscount: u16,
    pub arcount: u16,
}

/// DNS question structure
pub struct DnsQuestion {
    pub name: [u8; 256],
    pub name_len: usize,
    pub qtype: u16,
    pub qclass: u16,
}

/// DNS answer structure
pub struct DnsAnswer {
    pub name: [u8; 256],
    pub qtype: u16,
    pub qclass: u16,
    pub ttl: u32,
    pub rd_length: u16,
    pub ip_addr: [u8; 4],
}

/// Transaction ID counter
static QUERY_ID_COUNTER: AtomicU16 = AtomicU16::new(0x1234);

/// Initialize DNS module
pub fn init() {
    crate::println!("dns: initialized");
}

/// Get next query ID
fn next_query_id() -> u16 {
    QUERY_ID_COUNTER.fetch_add(1, Ordering::Relaxed)
}

/// Resolve a domain name to an IPv4 address (with cache)
/// Returns the IP address or None if resolution fails
pub fn resolve(domain: &str) -> Option<[u8; 4]> {
    // Check cache first
    if let Some(ip) = cache_lookup(domain) {
        crate::println!("dns: cache hit for '{}'", domain);
        return Some(ip);
    }

    // Validate domain length
    if domain.is_empty() || domain.len() > MAX_DOMAIN_LEN {
        crate::println!("dns: domain name too long or empty");
        return None;
    }

    let query_id = next_query_id();
    let mut query_buf = [0u8; MAX_DNS_PACKET];
    let mut response_buf = [0u8; MAX_DNS_PACKET];

    // Build the query packet
    let query_len = build_query(domain, query_id, &mut query_buf);
    if query_len == 0 {
        crate::println!("dns: failed to build query");
        return None;
    }

    crate::println!("dns: resolving '{}' (query_len={})", domain, query_len);

    // Allocate a socket
    let fd = match socket::socket(SocketType::Udp) {
        Some(fd) => fd,
        None => {
            crate::println!("dns: no free sockets");
            return None;
        }
    };

    // Bind to a random high port
    let local_port = 1024 + (query_id as u16);
    if !socket::bind(fd, local_port) {
        crate::println!("dns: failed to bind socket");
        socket::close(fd);
        return None;
    }

    // Send the query to the DNS server
    let dns_server = config().dns_server;
    match socket::sendto(fd, &query_buf[..query_len], dns_server, DNS_PORT) {
        Ok(sent) => crate::println!("dns: sent {} bytes to DNS server", sent),
        Err(e) => {
            crate::println!("dns: sendto failed: {}", e);
            socket::close(fd);
            return None;
        }
    }

    // Poll for response with timeout (simplified - just poll a few times)
    // In real implementation, would use proper timeout mechanism
    const POLL_ITERATIONS: usize = 50;
    for i in 0..POLL_ITERATIONS {
        // Poll network stack
        crate::net::poll();

        // Try to receive response
        let (len, _src_ip, _src_port) = socket::recvfrom(fd, &mut response_buf);
        if len > 0 {
            crate::println!("dns: received {} byte response", len);
            let result = parse_response(&response_buf[..len], query_id);
            socket::close(fd);
            // Cache the result on success
            if let Some(ref r) = result {
                cache_insert(domain, r.ip, r.ttl);
                return Some(r.ip);
            }
            return None;
        }

        // Small delay between polls
        if i > 0 && i % 10 == 0 {
            crate::arch::cpu::wfe();
        }
    }

    crate::println!("dns: timeout waiting for response");
    socket::close(fd);
    None
}

/// Build a DNS query packet for a domain name
/// Returns the length of the built packet, or 0 on failure
fn build_query(domain: &str, query_id: u16, out: &mut [u8]) -> usize {
    if out.len() < 12 {
        return 0;
    }

    // Build header
    let header = DnsHeader {
        id: query_id,
        flags: 0x0100, // Standard query, recursion desired
        qdcount: 1,
        ancount: 0,
        nscount: 0,
        arcount: 0,
    };

    // Pack header into buffer (network byte order = big endian)
    out[0] = (header.id >> 8) as u8;
    out[1] = (header.id & 0xFF) as u8;
    out[2] = (header.flags >> 8) as u8;
    out[3] = (header.flags & 0xFF) as u8;
    out[4] = (header.qdcount >> 8) as u8;
    out[5] = (header.qdcount & 0xFF) as u8;
    out[6] = (header.ancount >> 8) as u8;
    out[7] = (header.ancount & 0xFF) as u8;
    out[8] = (header.nscount >> 8) as u8;
    out[9] = (header.nscount & 0xFF) as u8;
    out[10] = (header.arcount >> 8) as u8;
    out[11] = (header.arcount & 0xFF) as u8;

    // Encode domain name
    let mut offset = 12;
    offset += encode_name(domain, &mut out[offset..]);

    // Add question type (A = 1) and class (IN = 1)
    if out.len() < offset + 4 {
        return 0;
    }
    out[offset] = 0; // QTYPE high byte
    out[offset + 1] = 1; // QTYPE low byte = A record
    out[offset + 2] = 0; // QCLASS high byte
    out[offset + 3] = 1; // QCLASS low byte = IN
    offset += 4;

    offset
}

/// Result of a successful DNS resolution
struct DnsResult {
    ip: [u8; 4],
    ttl: u32,
}

/// Parse a DNS response and extract A record answers
/// Returns the IP address and TTL if found, None otherwise
fn parse_response(data: &[u8], expected_id: u16) -> Option<DnsResult> {
    if data.len() < 12 {
        crate::println!("dns: response too short");
        return None;
    }

    // Parse header
    let id = u16::from_be_bytes([data[0], data[1]]);
    let flags = u16::from_be_bytes([data[2], data[3]]);
    let qdcount = u16::from_be_bytes([data[4], data[5]]);
    let ancount = u16::from_be_bytes([data[6], data[7]]);

    // Verify transaction ID
    if id != expected_id {
        crate::println!("dns: ID mismatch (expected {}, got {})", expected_id, id);
        return None;
    }

    // Check for error code (RCODE is lower 4 bits of flags)
    let rcode = flags & 0x000F;
    if rcode != 0 {
        crate::println!("dns: DNS error RCODE={}", rcode);
        return None;
    }

    // Must have at least one question and one answer
    if qdcount < 1 || ancount < 1 {
        crate::println!(
            "dns: no answers in response (qdcount={}, ancount={})",
            qdcount,
            ancount
        );
        return None;
    }

    // Parse question section (to find where answer section starts)
    let mut offset = 12;

    // Skip question(s)
    for _ in 0..qdcount {
        let result = decode_name(data, offset, &mut [0u8; 256]);
        offset = result.0;

        // Skip QTYPE (2) + QCLASS (2)
        offset += 4;
    }

    // Parse answer section
    for _ in 0..ancount {
        // Skip answer name
        let result = decode_name(data, offset, &mut [0u8; 256]);
        offset = result.0;

        if data.len() < offset + 10 {
            crate::println!("dns: truncated answer");
            return None;
        }

        // Parse answer type and class
        let qtype = u16::from_be_bytes([data[offset], data[offset + 1]]);
        let qclass = u16::from_be_bytes([data[offset + 2], data[offset + 3]]);
        let ttl = u32::from_be_bytes([
            data[offset + 4],
            data[offset + 5],
            data[offset + 6],
            data[offset + 7],
        ]);
        let rd_length = u16::from_be_bytes([data[offset + 8], data[offset + 9]]);

        offset += 10;

        crate::println!(
            "dns: answer type={}, class={}, ttl={}, rd_length={}",
            qtype,
            qclass,
            ttl,
            rd_length
        );

        // Check if this is an A record (type 1, class 1)
        if qtype == QTYPE_A && qclass == QCLASS_IN && rd_length == 4 {
            if data.len() < offset + 4 {
                crate::println!("dns: truncated A record data");
                return None;
            }

            let ip_addr = [
                data[offset],
                data[offset + 1],
                data[offset + 2],
                data[offset + 3],
            ];

            crate::println!(
                "dns: resolved to {}.{}.{}.{}",
                ip_addr[0],
                ip_addr[1],
                ip_addr[2],
                ip_addr[3]
            );
            return Some(DnsResult { ip: ip_addr, ttl });
        }

        // Skip RDATA
        offset += rd_length as usize;
    }

    crate::println!("dns: no A record found in response");
    None
}

/// Encode a domain name into DNS wire format (length-prefixed labels)
/// Returns the number of bytes written
fn encode_name(domain: &str, out: &mut [u8]) -> usize {
    if out.is_empty() {
        return 0;
    }

    let mut offset = 0;

    // Iterate over dot-separated labels without allocating Vec
    for label in domain.split('.') {
        if label.is_empty() {
            continue;
        }

        let len = label.len();
        if len > 63 {
            crate::println!("dns: label too long ({})", len);
            return 0;
        }

        if offset + 1 + len > out.len() {
            crate::println!("dns: output buffer too small");
            return 0;
        }

        // Write label length
        out[offset] = len as u8;
        offset += 1;

        // Write label
        for (i, c) in label.bytes().enumerate() {
            out[offset + i] = c;
        }
        offset += len;
    }

    // Write root label (zero byte)
    if offset < out.len() {
        out[offset] = 0;
        offset += 1;
    }

    offset
}

/// Decode a domain name from DNS wire format (handling compression pointers)
/// Returns (new_offset, name_length). The name is written to `out`.
fn decode_name(data: &[u8], offset: usize, out: &mut [u8]) -> (usize, usize) {
    if offset >= data.len() {
        return (offset, 0);
    }

    let mut out_offset = 0;
    let mut current_offset = offset;
    let mut jumped = false;
    let mut first_jump_offset = 0;

    loop {
        if current_offset >= data.len() {
            break;
        }

        let len = data[current_offset] as usize;

        // Check for compression pointer (top 2 bits set)
        if (len & 0xC0) == 0xC0 {
            // Compression pointer
            if current_offset + 1 >= data.len() {
                break;
            }

            let pointer = ((len & 0x3F) << 8) | (data[current_offset + 1] as usize);

            if !jumped {
                first_jump_offset = current_offset + 2;
                jumped = true;
            }

            current_offset = pointer;
            continue;
        }

        // End of name
        if len == 0 {
            if !jumped {
                return (current_offset + 1, out_offset);
            }
            if jumped && out_offset > 0 {
                // Add terminating null if we built a name
                if out_offset < out.len() {
                    out[out_offset] = 0;
                    out_offset += 1;
                }
            }
            break;
        }

        // Regular label
        if current_offset + 1 + len > data.len() {
            break;
        }

        // Add label to output
        if out_offset > 0 && out_offset < out.len() {
            out[out_offset] = b'.';
            out_offset += 1;
        }

        let copy_len = len.min(out.len() - out_offset);
        if copy_len > 0 {
            out[out_offset..out_offset + copy_len]
                .copy_from_slice(&data[current_offset + 1..current_offset + 1 + copy_len]);
            out_offset += copy_len;
        }

        current_offset += 1 + len;
    }

    if jumped {
        (first_jump_offset, out_offset)
    } else {
        (current_offset + 1, out_offset)
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    #[test]
    fn test_encode_name() {
        let mut out = [0u8; 256];
        let len = encode_name("example.com", &mut out);

        // e x a m p l e . c o m -> 7 example 3 com 0
        assert_eq!(len, 13); // 1 + 7 + 1 + 3 + 1
        assert_eq!(out[0], 7);
        assert_eq!(&out[1..8], b"example");
        assert_eq!(out[8], 3);
        assert_eq!(&out[9..12], b"com");
        assert_eq!(out[12], 0);
    }

    #[test]
    fn test_encode_name_single_label() {
        let mut out = [0u8; 256];
        let len = encode_name("localhost", &mut out);

        assert_eq!(len, 11); // 1 + 9 + 1
        assert_eq!(out[0], 9);
        assert_eq!(&out[1..10], b"localhost");
        assert_eq!(out[10], 0);
    }

    #[test]
    fn test_build_query() {
        let mut buf = [0u8; 512];
        let id = next_query_id();
        let len = build_query("example.com", id, &mut buf);

        assert!(len > 0);

        // Check header
        assert_eq!(u16::from_be_bytes([buf[0], buf[1]]), id);
        assert_eq!(u16::from_be_bytes([buf[2], buf[3]]), 0x0100); // flags
        assert_eq!(u16::from_be_bytes([buf[4], buf[5]]), 1); // qdcount
        assert_eq!(u16::from_be_bytes([buf[6], buf[7]]), 0); // ancount
    }
}
