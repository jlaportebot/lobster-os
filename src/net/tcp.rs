//! Minimal TCP (Transmission Control Protocol) implementation
//!
//! Handles basic TCP state machine and packet processing.
//! No_std compatible - uses only core.

use crate::net::config;
use crate::net::eth::{EthernetFrame, ETHER_TYPE_IPV4};
use crate::net::genet;
use crate::net::ipv4::{self, Ipv4Header, IPPROTO_TCP};
use core::sync::atomic::{AtomicU8, Ordering};

/// Minimum TCP header length
pub const HEADER_LEN_MIN: usize = 20;

/// TCP flags
pub const FLAG_FIN: u8 = 0x01;
pub const FLAG_SYN: u8 = 0x02;
pub const FLAG_RST: u8 = 0x04;
pub const FLAG_PSH: u8 = 0x08;
pub const FLAG_ACK: u8 = 0x10;
pub const FLAG_URG: u8 = 0x20;

/// TCP state enum
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TcpState {
    Closed,
    Listen,
    SynSent,
    SynReceived,
    Established,
    FinWait1,
    FinWait2,
    CloseWait,
    Closing,
    LastAck,
    TimeWait,
}

impl Default for TcpState {
    fn default() -> Self {
        TcpState::Closed
    }
}

/// TCP header structure
#[derive(Clone, Copy, Debug)]
pub struct TcpHeader {
    pub src_port: u16,
    pub dst_port: u16,
    pub seq_num: u32,
    pub ack_num: u32,
    pub data_offset: u8,
    pub flags: u8,
    pub window: u16,
    pub checksum: u16,
    pub urgent_ptr: u16,
}

impl TcpHeader {
    /// Get header length in bytes
    pub fn header_len(&self) -> usize {
        (self.data_offset as usize) * 4
    }

    /// Check if SYN flag is set
    pub fn is_syn(&self) -> bool {
        self.flags & FLAG_SYN != 0
    }

    /// Check if ACK flag is set
    pub fn is_ack(&self) -> bool {
        self.flags & FLAG_ACK != 0
    }

    /// Check if FIN flag is set
    pub fn is_fin(&self) -> bool {
        self.flags & FLAG_FIN != 0
    }

    /// Check if RST flag is set
    pub fn is_rst(&self) -> bool {
        self.flags & FLAG_RST != 0
    }
}

/// TCP Control Block (connection state)
#[derive(Clone, Copy, Debug)]
pub struct TcpControlBlock {
    pub local_port: u16,
    pub remote_port: u16,
    pub remote_ip: [u8; 4],
    pub state: TcpState,
    pub snd_una: u32, // Send unacknowledged
    pub snd_nxt: u32, // Send next
    pub snd_wnd: u32, // Send window
    pub rcv_nxt: u32, // Receive next
    pub rcv_wnd: u32, // Receive window
}

impl TcpControlBlock {
    const ZERO: TcpControlBlock = TcpControlBlock {
        local_port: 0,
        remote_port: 0,
        remote_ip: [0, 0, 0, 0],
        state: TcpState::Closed,
        snd_una: 0,
        snd_nxt: 0,
        snd_wnd: 65535,
        rcv_nxt: 0,
        rcv_wnd: 65535,
    };
}

impl Default for TcpControlBlock {
    fn default() -> Self {
        Self::ZERO
    }
}

/// TCB table - static array of 8 entries
static mut TCB_TABLE: [TcpControlBlock; 8] = [TcpControlBlock::ZERO; 8];
static TCB_LOCK: AtomicU8 = AtomicU8::new(0);

/// Acquire TCB lock
fn acquire_tcb_lock() {
    while TCB_LOCK
        .compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        core::hint::spin_loop();
    }
}

/// Release TCB lock
fn release_tcb_lock() {
    TCB_LOCK.store(0, Ordering::SeqCst);
}

/// Initialize TCP module
pub fn init() {
    unsafe {
        for tcb in TCB_TABLE.iter_mut() {
            *tcb = TcpControlBlock::default();
        }
    }
    crate::println!("tcp: initialized, {} TCB slots", 8);
}

/// Parse a TCP header from raw bytes
fn parse_tcp(data: &[u8]) -> Option<TcpHeader> {
    if data.len() < HEADER_LEN_MIN {
        return None;
    }

    let src_port = u16::from_be_bytes([data[0], data[1]]);
    let dst_port = u16::from_be_bytes([data[2], data[3]]);
    let seq_num = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
    let ack_num = u32::from_be_bytes([data[8], data[9], data[10], data[11]]);

    let data_offset_flags = u16::from_be_bytes([data[12], data[13]]);
    let data_offset = ((data_offset_flags >> 12) & 0x0F) as u8;
    let flags = (data_offset_flags & 0x3F) as u8;

    let window = u16::from_be_bytes([data[14], data[15]]);
    let checksum = u16::from_be_bytes([data[16], data[17]]);
    let urgent_ptr = u16::from_be_bytes([data[18], data[19]]);

    if data_offset < 5 {
        return None;
    }

    Some(TcpHeader {
        src_port,
        dst_port,
        seq_num,
        ack_num,
        data_offset,
        flags,
        window,
        checksum,
        urgent_ptr,
    })
}

/// Find or create a TCB for the connection
fn find_or_create_tcb(tcp: &TcpHeader, ip: &Ipv4Header, is_server: bool) -> Option<usize> {
    unsafe {
        // First, look for existing TCB
        for (i, tcb) in TCB_TABLE.iter().enumerate() {
            if tcb.state == TcpState::Closed {
                continue;
            }

            let match_inbound = tcb.remote_port == tcp.src_port
                && tcb.local_port == tcp.dst_port
                && tcb.remote_ip == ip.src_addr;

            let match_outbound = tcb.remote_port == tcp.dst_port
                && tcb.local_port == tcp.src_port
                && tcb.remote_ip == ip.dst_addr;

            if match_inbound || match_outbound {
                return Some(i);
            }
        }

        // No existing TCB - create new if server mode and SYN received
        if is_server && tcp.is_syn() {
            for (i, tcb) in TCB_TABLE.iter_mut().enumerate() {
                if tcb.state == TcpState::Closed {
                    tcb.local_port = tcp.dst_port;
                    tcb.remote_port = tcp.src_port;
                    tcb.remote_ip = ip.src_addr;
                    tcb.state = TcpState::SynReceived;
                    tcb.rcv_nxt = tcp.seq_num + 1;
                    tcb.snd_nxt = 1; // Our ISN
                    return Some(i);
                }
            }
        }

        None
    }
}

/// Handle an incoming TCP packet
pub fn handle_packet(ip_hdr: &Ipv4Header, data: &[u8]) {
    let tcp = match parse_tcp(data) {
        Some(t) => t,
        None => {
            crate::println!("tcp: invalid packet");
            return;
        }
    };

    crate::println!(
        "tcp: received packet {:?}:{} -> {:?}:{} seq={} ack={} flags={:02X}",
        ip_hdr.src_addr,
        tcp.src_port,
        ip_hdr.dst_addr,
        tcp.dst_port,
        tcp.seq_num,
        tcp.ack_num,
        tcp.flags
    );

    // Try to find existing TCB
    let tcb_idx = find_or_create_tcb(&tcp, ip_hdr, true);

    match tcb_idx {
        Some(idx) => {
            acquire_tcb_lock();
            let tcb = unsafe { &mut TCB_TABLE[idx] };

            // Handle based on state and flags
            match tcb.state {
                TcpState::SynReceived => {
                    if tcp.is_ack() {
                        // Move to established
                        tcb.state = TcpState::Established;
                        crate::println!("tcp: connection established on port {}", tcb.local_port);
                    }
                }
                TcpState::Established => {
                    if tcp.is_fin() {
                        tcb.state = TcpState::FinWait1;
                        // Send FIN+ACK
                        send_tcp_packet(
                            ip_hdr.src_addr,
                            ip_hdr.dst_addr,
                            tcb.local_port,
                            tcb.remote_port,
                            tcb.snd_nxt,
                            tcb.rcv_nxt,
                            FLAG_FIN | FLAG_ACK,
                            &[],
                        );
                        tcb.snd_nxt += 1;
                    }
                }
                TcpState::FinWait1 => {
                    if tcp.is_fin() && tcp.is_ack() {
                        tcb.state = TcpState::TimeWait;
                        tcb.rcv_nxt += 1;
                    }
                }
                _ => {}
            }

            release_tcb_lock();
        }
        None => {
            // No TCB found - could be a new connection attempt
            if tcp.is_syn() {
                crate::println!("tcp: new SYN received, creating TCB");
                let idx = find_or_create_tcb(&tcp, ip_hdr, true);
                if let Some(idx) = idx {
                    acquire_tcb_lock();
                    let tcb = unsafe { &mut TCB_TABLE[idx] };

                    // Send SYN+ACK
                    send_tcp_packet(
                        ip_hdr.src_addr,
                        ip_hdr.dst_addr,
                        tcb.local_port,
                        tcb.remote_port,
                        tcb.snd_nxt,
                        tcp.seq_num + 1,
                        FLAG_SYN | FLAG_ACK,
                        &[],
                    );
                    tcb.snd_nxt += 1;

                    release_tcb_lock();
                }
            }
        }
    }
}

/// Build and send a TCP packet
fn send_tcp_packet(
    dst_ip: [u8; 4],
    src_ip: [u8; 4],
    src_port: u16,
    dst_port: u16,
    seq_num: u32,
    ack_num: u32,
    flags: u8,
    payload: &[u8],
) {
    let cfg = config();
    let our_mac = cfg.mac_addr;

    // Resolve MAC address
    let dst_mac = match crate::net::arp::resolve(dst_ip) {
        Some(mac) => mac,
        None => {
            crate::println!("tcp: cannot resolve MAC for {:?}", dst_ip);
            return;
        }
    };

    // Build TCP header
    let tcp_header_len = 20; // No options
    let mut tcp_buf = [0u8; 1500];

    // Source port
    tcp_buf[0] = (src_port >> 8) as u8;
    tcp_buf[1] = (src_port & 0xFF) as u8;
    // Destination port
    tcp_buf[2] = (dst_port >> 8) as u8;
    tcp_buf[3] = (dst_port & 0xFF) as u8;
    // Sequence number
    tcp_buf[4] = (seq_num >> 24) as u8;
    tcp_buf[5] = (seq_num >> 16) as u8;
    tcp_buf[6] = (seq_num >> 8) as u8;
    tcp_buf[7] = (seq_num & 0xFF) as u8;
    // Acknowledgment number
    tcp_buf[8] = (ack_num >> 24) as u8;
    tcp_buf[9] = (ack_num >> 16) as u8;
    tcp_buf[10] = (ack_num >> 8) as u8;
    tcp_buf[11] = (ack_num & 0xFF) as u8;
    // Data offset and flags
    tcp_buf[12] = ((tcp_header_len / 4) << 4) as u8;
    tcp_buf[13] = flags;
    // Window
    tcp_buf[14] = 0x7F; // 32767
    tcp_buf[15] = 0xFF;
    // Checksum (0 for now)
    tcp_buf[16] = 0;
    tcp_buf[17] = 0;
    // Urgent pointer
    tcp_buf[18] = 0;
    tcp_buf[19] = 0;

    // Payload
    let tcp_total = tcp_header_len + payload.len();
    if !payload.is_empty() {
        tcp_buf[tcp_header_len..tcp_total].copy_from_slice(payload);
    }

    // Build IPv4 packet
    let mut ipv4_buf = [0u8; 1500];
    let ipv4_len = ipv4::build(
        src_ip,
        dst_ip,
        IPPROTO_TCP,
        64,     // TTL
        0xDEAD, // Identification
        &tcp_buf[..tcp_total],
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

    crate::println!("tcp: sending packet flags={:02X}", flags);
    genet::transmit(&eth_buf[..eth_len]);
}

/// Find a TCB by socket fd (port-based lookup).
/// Returns a mutable reference to the TCB entry.
fn find_tcb_by_fd(fd: usize) -> Option<&'static mut TcpControlBlock> {
    // We use the fd as an index into the TCB table for simplicity.
    // In a full implementation, we'd have a proper fd→TCB mapping.
    let port = crate::net::socket::get_local_port(fd);
    if port == 0 {
        return None;
    }
    unsafe {
        for tcb in TCB_TABLE.iter_mut() {
            if tcb.local_port == port && tcb.state != TcpState::Closed {
                return Some(tcb);
            }
        }
        // If no match, allocate a free slot
        for tcb in TCB_TABLE.iter_mut() {
            if tcb.state == TcpState::Closed {
                tcb.local_port = port;
                return Some(tcb);
            }
        }
    }
    None
}

/// Connect a TCP socket to a remote address.
/// Initiates the 3-way handshake (SYN → SYN-ACK → ACK).
/// Returns true if the connection was initiated successfully.
pub fn connect(fd: usize, dst_ip: [u8; 4], dst_port: u16) -> bool {
    acquire_tcb_lock();
    let tcb = match find_tcb_by_fd(fd) {
        Some(t) => t,
        None => {
            release_tcb_lock();
            crate::println!("tcp: connect: no TCB for fd={}", fd);
            return false;
        }
    };

    // Only connect from Closed or Listen state
    if tcb.state != TcpState::Closed && tcb.state != TcpState::Listen {
        release_tcb_lock();
        crate::println!("tcp: connect: invalid state {:?}", tcb.state);
        return false;
    }

    let our_ip = crate::net::config().ip_addr;
    let our_port = tcb.local_port;
    let iss = 0x0000_1234u32; // Initial Sequence Number

    tcb.remote_ip = dst_ip;
    tcb.remote_port = dst_port;
    tcb.snd_nxt = iss;
    tcb.rcv_nxt = 0;
    tcb.state = TcpState::SynSent;

    release_tcb_lock();

    // Send SYN
    send_tcp_packet(our_ip, dst_ip, our_port, dst_port, iss, 0, FLAG_SYN, &[]);

    crate::println!(
        "tcp: SYN sent to {}.{}.{}.{}:{}",
        dst_ip[0],
        dst_ip[1],
        dst_ip[2],
        dst_ip[3],
        dst_port
    );

    // In a real implementation, we'd wait for SYN-ACK and send ACK.
    // For now, transition to Established after a brief delay.
    acquire_tcb_lock();
    if let Some(tcb) = find_tcb_by_fd(fd) {
        tcb.state = TcpState::Established;
        tcb.rcv_nxt = iss + 1;
    }
    release_tcb_lock();

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_tcp() {
        let data = [
            0x1F, 0x90, // src port 8080
            0x00, 0x50, // dst port 80
            0x00, 0x00, 0x00, 0x01, // seq 1
            0x00, 0x00, 0x00, 0x00, // ack 0
            0x50, 0x02, // data offset 5, flags SYN
            0x7F, 0xFF, // window
            0x00, 0x00, // checksum
            0x00, 0x00, // urgent
        ];

        let tcp = parse_tcp(&data).unwrap();
        assert_eq!(tcp.src_port, 8080);
        assert_eq!(tcp.dst_port, 80);
        assert!(tcp.is_syn());
    }
}
