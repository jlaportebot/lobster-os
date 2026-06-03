//! Socket API for LobsterOS networking stack
//!
//! Provides a simple socket interface for UDP and TCP.
//! No_std compatible - uses only core.

use core::sync::atomic::{AtomicU8, Ordering};
use crate::net::ipv4::Ipv4Header;
use crate::net::udp::UdpHeader;

/// Socket types
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SocketType {
    Udp,
    Tcp,
    Raw,
}

/// Socket structure
#[derive(Clone, Copy, Debug)]
pub struct Socket {
    pub fd: usize,
    pub sock_type: SocketType,
    pub local_port: u16,
    pub remote_port: u16,
    pub remote_ip: [u8; 4],
    pub rx_buf: [u8; 2048],
    pub rx_len: usize,
    pub bound: bool,
}

impl Socket {
    const ZERO: Socket = Socket {
        fd: 0,
        sock_type: SocketType::Udp,
        local_port: 0,
        remote_port: 0,
        remote_ip: [0, 0, 0, 0],
        rx_buf: [0; 2048],
        rx_len: 0,
        bound: false,
    };
}

impl Default for Socket {
    fn default() -> Self {
        Self::ZERO
    }
}

/// Socket table - 16 entries
static mut SOCKET_TABLE: [Socket; 16] = [Socket::ZERO; 16];
static SOCKET_LOCK: AtomicU8 = AtomicU8::new(0);

/// Acquire socket lock
fn acquire_lock() {
    while SOCKET_LOCK
        .compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        core::hint::spin_loop();
    }
}

/// Release socket lock
fn release_lock() {
    SOCKET_LOCK.store(0, Ordering::SeqCst);
}

/// Initialize socket module
pub fn init() {
    unsafe {
        for sock in SOCKET_TABLE.iter_mut() {
            *sock = Socket::default();
        }
    }
    crate::println!("socket: initialized, {} entries", 16);
}

/// Allocate a new socket
pub fn socket(sock_type: SocketType) -> Option<usize> {
    acquire_lock();
    for (i, sock) in unsafe { SOCKET_TABLE.iter_mut() }.enumerate() {
        if !sock.bound && sock.local_port == 0 {
            sock.fd = i;
            sock.sock_type = sock_type;
            sock.bound = true;
            sock.rx_len = 0;
            release_lock();
            crate::println!("socket: allocated fd={} type={:?}", i, sock_type);
            return Some(i);
        }
    }
    release_lock();
    crate::println!("socket: no free sockets");
    None
}

/// Bind a socket to a local port
pub fn bind(fd: usize, port: u16) -> bool {
    if fd >= 16 {
        return false;
    }

    acquire_lock();
    let sock = unsafe { &mut SOCKET_TABLE[fd] };
    if sock.bound && sock.local_port == 0 {
        // Check if port is already in use
        for s in unsafe { SOCKET_TABLE.iter() } {
            if s.bound && s.local_port == port && s.local_port != 0 {
                release_lock();
                return false;
            }
        }
        sock.local_port = port;
        release_lock();
        crate::println!("socket: fd={} bound to port {}", fd, port);
        return true;
    }
    release_lock();
    false
}

/// Send data to a remote address
pub fn sendto(fd: usize, data: &[u8], addr: [u8; 4], port: u16) -> Result<usize, &'static str> {
    if fd >= 16 {
        return Err("invalid fd");
    }

    let sock = unsafe { &SOCKET_TABLE[fd] };
    if !sock.bound {
        return Err("socket not bound");
    }

    match sock.sock_type {
        SocketType::Udp => {
            // Send UDP packet
            crate::net::udp::send_packet(
                crate::net::config().ip_addr, // our IP
                addr,
                sock.local_port,
                port,
                data,
            );
            Ok(data.len())
        }
        SocketType::Tcp => {
            // Simplified TCP send - in real implementation, would use TCB
            crate::println!("socket: TCP send not fully implemented");
            Err("TCP send not implemented")
        }
        SocketType::Raw => {
            crate::println!("socket: raw socket send not implemented");
            Err("raw socket not implemented")
        }
    }
}

/// Receive data from a socket
pub fn recvfrom(fd: usize, buf: &mut [u8]) -> (usize, [u8; 4], u16) {
    if fd >= 16 {
        return (0, [0, 0, 0, 0], 0);
    }

    acquire_lock();
    let sock = unsafe { &mut SOCKET_TABLE[fd] };
    if sock.rx_len == 0 {
        release_lock();
        return (0, [0, 0, 0, 0], 0);
    }

    let len = sock.rx_len.min(buf.len());
    buf[..len].copy_from_slice(&sock.rx_buf[..len]);
    let src_ip = sock.remote_ip;
    let src_port = sock.remote_port;
    sock.rx_len = 0;
    release_lock();

    (len, src_ip, src_port)
}

/// Close a socket
pub fn close(fd: usize) {
    if fd >= 16 {
        return;
    }

    acquire_lock();
    unsafe {
        SOCKET_TABLE[fd] = Socket::default();
    }
    release_lock();
    crate::println!("socket: fd={} closed", fd);
}

/// Handle incoming UDP packet - deliver to matching socket
pub fn handle_udp(
    ip_hdr: &Ipv4Header,
    udp_hdr: &UdpHeader,
    data: &[u8],
) {
    let dst_port = udp_hdr.dst_port;

    acquire_lock();
    for sock in unsafe { SOCKET_TABLE.iter_mut() } {
        if sock.bound && sock.local_port == dst_port {
            // Copy data to socket receive buffer
            let len = data.len().min(2048);
            sock.rx_buf[..len].copy_from_slice(&data[..len]);
            sock.rx_len = len;
            sock.remote_ip = ip_hdr.src_addr;
            sock.remote_port = udp_hdr.src_port;
            release_lock();
            crate::println!(
                "socket: UDP packet delivered to fd={} len={}",
                sock.fd,
                len
            );
            return;
        }
    }
    release_lock();
    crate::println!("socket: no socket bound to port {}", dst_port);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_socket_allocate() {
        init();
        let fd1 = socket(SocketType::Udp);
        assert!(fd1.is_some());
        let fd2 = socket(SocketType::Udp);
        assert!(fd2.is_some());
        assert_ne!(fd1, fd2);
    }

    #[test]
    fn test_socket_bind() {
        init();
        let fd = socket(SocketType::Udp).unwrap();
        assert!(bind(fd, 8080));
        assert!(!bind(fd, 8081)); // Already bound
    }
}