//! Minimal HTTP/1.1 GET client for LobsterOS
//!
//! Supports:
//! - HTTP GET requests over TCP
//! - Response status code parsing
//! - Header extraction (Content-Length, Transfer-Encoding)
//! - Chunked transfer encoding decoding
//! - Connection: close semantics

use crate::net::dns;
use crate::net::socket::{self, SocketType};
use crate::net::tcp;

/// Maximum HTTP response header size (4KB)
const MAX_HEADER_SIZE: usize = 4096;

/// Maximum HTTP response body size (64KB)
const MAX_BODY_SIZE: usize = 65536;

/// HTTP response
pub struct HttpResponse {
    /// HTTP status code (e.g. 200, 404)
    pub status_code: u16,
    /// Status reason phrase (e.g. "OK", "Not Found")
    pub status_reason: [u8; 32],
    pub reason_len: usize,
    /// Response body
    pub body: [u8; MAX_BODY_SIZE],
    pub body_len: usize,
    /// Content-Length header value (-1 if absent)
    pub content_length: i64,
    /// Whether Transfer-Encoding: chunked
    pub chunked: bool,
}

/// Parsed URL components
struct UrlParts {
    host: [u8; 128],
    host_len: usize,
    port: u16,
    path: [u8; 256],
    path_len: usize,
}

/// Parse a URL into host, port, and path
fn parse_url(url: &str) -> Option<UrlParts> {
    // Expect "http://host[:port]/path"
    let url_bytes = url.as_bytes();
    if url_bytes.len() < 7 {
        return None;
    }
    if &url_bytes[..7] != b"http://" {
        return None;
    }

    let rest = &url_bytes[7..];
    let mut host = [0u8; 128];
    let mut host_len = 0;
    let mut port: u16 = 80;
    let mut path = [0u8; 256];
    let mut path_len = 0;

    // Find end of host (either ':' for port, '/' for path, or end)
    let mut i = 0;
    while i < rest.len() && rest[i] != b'/' && rest[i] != b':' && host_len < 128 {
        host[host_len] = rest[i];
        host_len += 1;
        i += 1;
    }

    // Parse optional port
    if i < rest.len() && rest[i] == b':' {
        i += 1;
        let mut port_str = [0u8; 6];
        let mut port_len = 0;
        while i < rest.len() && rest[i] != b'/' && port_len < 6 {
            port_str[port_len] = rest[i];
            port_len += 1;
            i += 1;
        }
        // Parse port number
        let mut p: u16 = 0;
        for j in 0..port_len {
            if port_str[j] >= b'0' && port_str[j] <= b'9' {
                p = p * 10 + (port_str[j] - b'0') as u16;
            }
        }
        if p > 0 {
            port = p;
        }
    }

    // Remaining is the path (default "/")
    if i < rest.len() {
        while i < rest.len() && path_len < 256 {
            path[path_len] = rest[i];
            path_len += 1;
            i += 1;
        }
    }
    if path_len == 0 {
        path[0] = b'/';
        path_len = 1;
    }

    Some(UrlParts {
        host,
        host_len,
        port,
        path,
        path_len,
    })
}

/// Convert byte slice to &str (lossy)
fn bytes_to_str(b: &[u8]) -> &str {
    core::str::from_utf8(b).unwrap_or("<invalid>")
}

/// Perform an HTTP GET request
///
/// Usage: `http_get("http://example.com/path")`
/// Returns HttpResponse with status code and body on success, or None on failure.
pub fn http_get(url: &str) -> Option<HttpResponse> {
    let parts = parse_url(url)?;
    let host_str = bytes_to_str(&parts.host[..parts.host_len]);

    // Resolve host via DNS
    let ip = dns::resolve(host_str)?;
    crate::println!(
        "http: resolved {} to {}.{}.{}.{}",
        host_str,
        ip[0],
        ip[1],
        ip[2],
        ip[3]
    );

    // Create a TCP socket
    let fd = socket::socket(SocketType::Tcp)?;
    if !socket::bind(fd, 0) {
        crate::println!("http: bind failed");
        socket::close(fd);
        return None;
    }

    // Connect to server
    if !tcp::connect(fd, ip, parts.port) {
        crate::println!(
            "http: connect to {}.{}.{}.{}:{} failed",
            ip[0],
            ip[1],
            ip[2],
            ip[3],
            parts.port
        );
        socket::close(fd);
        return None;
    }

    // Build HTTP request
    let mut req = [0u8; 512];
    let mut req_len = 0;

    // "GET /path HTTP/1.1\r\n"
    let _get_line = b"GET ";
    let mut write = |buf: &[u8]| {
        let copy_len = buf.len().min(req.len() - req_len);
        req[req_len..req_len + copy_len].copy_from_slice(&buf[..copy_len]);
        req_len += copy_len;
    };

    write(b"GET ");
    write(&parts.path[..parts.path_len]);
    write(b" HTTP/1.1\r\nHost: ");
    write(&parts.host[..parts.host_len]);
    write(b"\r\nConnection: close\r\nUser-Agent: LobsterOS/1.0\r\nAccept: */*\r\n\r\n");

    // Send request
    match socket::sendto(fd, &req[..req_len], ip, parts.port) {
        Ok(_) => {}
        Err(e) => {
            crate::println!("http: send failed: {}", e);
            socket::close(fd);
            return None;
        }
    }

    // Read response
    let mut resp_buf = [0u8; MAX_HEADER_SIZE + MAX_BODY_SIZE];
    let mut resp_len = 0;

    // Read in a loop until we get the full response or hit limit
    let max_total = MAX_HEADER_SIZE + MAX_BODY_SIZE;
    while resp_len < max_total {
        let (n, _addr, _port) = socket::recvfrom(fd, &mut resp_buf[resp_len..]);
        if n == 0 {
            break;
        }
        resp_len += n;

        // Check if we've seen the end of headers + body
        // Simple heuristic: look for \r\n\r\n then check Content-Length
        if has_complete_response(&resp_buf[..resp_len]) {
            break;
        }
    }

    socket::close(fd);

    if resp_len == 0 {
        crate::println!("http: empty response");
        return None;
    }

    // Parse response
    parse_response(&resp_buf[..resp_len])
}

/// Check if we have a complete HTTP response
fn has_complete_response(data: &[u8]) -> bool {
    // Find header end
    let header_end = find_header_end(data);
    if header_end.is_none() {
        return false;
    }
    let hdr_end = header_end.unwrap();

    // Check for Content-Length
    if let Some(cl) = parse_content_length(data) {
        let body_len = data.len() - hdr_end;
        return body_len >= cl as usize;
    }

    // If chunked, we'd need to find "0\r\n\r\n" terminator
    // For Connection: close, just return true when we have headers + some body
    true
}

/// Find the end of HTTP headers (position after \r\n\r\n)
fn find_header_end(data: &[u8]) -> Option<usize> {
    for i in 0..data.len().saturating_sub(3) {
        if data[i] == b'\r' && data[i + 1] == b'\n' && data[i + 2] == b'\r' && data[i + 3] == b'\n'
        {
            return Some(i + 4);
        }
    }
    None
}

/// Parse Content-Length from headers
fn parse_content_length(data: &[u8]) -> Option<i64> {
    let needle = b"Content-Length:";
    for i in 0..data.len().saturating_sub(needle.len()) {
        if &data[i..i + needle.len()] == needle {
            let mut val: i64 = 0;
            let mut j = i + needle.len();
            // Skip whitespace
            while j < data.len() && (data[j] == b' ' || data[j] == b'\t') {
                j += 1;
            }
            while j < data.len() && data[j] >= b'0' && data[j] <= b'9' {
                val = val * 10 + (data[j] - b'0') as i64;
                j += 1;
            }
            return Some(val);
        }
    }
    None
}

/// Parse HTTP response
fn parse_response(data: &[u8]) -> Option<HttpResponse> {
    // Parse status line: "HTTP/1.1 200 OK\r\n"
    if data.len() < 12 {
        return None;
    }
    if &data[..5] != b"HTTP/" {
        return None;
    }

    // Find status code (after "HTTP/x.x ")
    let mut space_pos = 0;
    for i in 5..data.len().min(20) {
        if data[i] == b' ' {
            space_pos = i;
            break;
        }
    }
    if space_pos == 0 {
        return None;
    }

    // Parse 3-digit status code
    let mut status_code: u16 = 0;
    let mut digits = 0;
    let mut pos = space_pos + 1;
    while pos < data.len() && data[pos] >= b'0' && data[pos] <= b'9' && digits < 3 {
        status_code = status_code * 10 + (data[pos] - b'0') as u16;
        digits += 1;
        pos += 1;
    }
    if digits != 3 {
        return None;
    }

    // Parse reason phrase (until \r\n)
    let mut status_reason = [0u8; 32];
    let mut reason_len = 0;
    if pos < data.len() && data[pos] == b' ' {
        pos += 1;
    }
    while pos < data.len() && data[pos] != b'\r' && reason_len < 32 {
        status_reason[reason_len] = data[pos];
        reason_len += 1;
        pos += 1;
    }

    // Find body start
    let header_end = find_header_end(data)?;
    let body_data = &data[header_end..];
    let content_length = parse_content_length(data).unwrap_or(-1);
    let chunked = is_chunked(data);

    // Copy body
    let mut body = [0u8; MAX_BODY_SIZE];
    let body_len = if chunked {
        decode_chunked(body_data, &mut body)
    } else {
        let copy_len = body_data.len().min(MAX_BODY_SIZE);
        body[..copy_len].copy_from_slice(&body_data[..copy_len]);
        copy_len
    };

    crate::println!(
        "http: {} {} ({} bytes body)",
        status_code,
        bytes_to_str(&status_reason[..reason_len]),
        body_len
    );

    Some(HttpResponse {
        status_code,
        status_reason,
        reason_len,
        body,
        body_len,
        content_length,
        chunked,
    })
}

/// Check if Transfer-Encoding: chunked
fn is_chunked(data: &[u8]) -> bool {
    let needle = b"Transfer-Encoding: chunked";
    for i in 0..data.len().saturating_sub(needle.len()) {
        // Case-insensitive check for the header
        if &data[i..i + needle.len()].to_ascii_lowercase() == needle.to_ascii_lowercase().as_slice()
        {
            return true;
        }
    }
    false
}

/// Decode chunked transfer encoding
fn decode_chunked(data: &[u8], out: &mut [u8; MAX_BODY_SIZE]) -> usize {
    let mut read_pos = 0;
    let mut write_pos = 0;

    loop {
        // Read chunk size (hex until \r\n)
        let mut chunk_size: usize = 0;
        let mut hex_digits = 0;

        while read_pos < data.len().saturating_sub(1) {
            let b = data[read_pos];
            if b == b'\r' && data.get(read_pos + 1) == Some(&b'\n') {
                read_pos += 2;
                break;
            }
            if b >= b'0' && b <= b'9' {
                chunk_size = chunk_size * 16 + (b - b'0') as usize;
                hex_digits += 1;
            } else if b >= b'a' && b <= b'f' {
                chunk_size = chunk_size * 16 + (b - b'a' + 10) as usize;
                hex_digits += 1;
            } else if b >= b'A' && b <= b'F' {
                chunk_size = chunk_size * 16 + (b - b'A' + 10) as usize;
                hex_digits += 1;
            }
            read_pos += 1;
        }

        if chunk_size == 0 || hex_digits == 0 {
            break;
        }

        // Copy chunk data
        let copy_len = chunk_size
            .min(out.len() - write_pos)
            .min(data.len() - read_pos);
        out[write_pos..write_pos + copy_len].copy_from_slice(&data[read_pos..read_pos + copy_len]);
        write_pos += copy_len;
        read_pos += chunk_size;

        // Skip trailing \r\n
        if read_pos + 1 < data.len() && data[read_pos] == b'\r' && data[read_pos + 1] == b'\n' {
            read_pos += 2;
        }
    }

    write_pos
}

/// Read a header value by name from raw header bytes (case-insensitive)
/// Returns a slice of the value bytes, or None
pub fn get_header<'a>(headers: &'a [u8], name: &str) -> Option<&'a [u8]> {
    let _name_prefix = [b':', b' '];
    // Search for "Name: value\r\n"
    let name_lower = name.as_bytes();
    let mut i = 0;
    while i + name_lower.len() + 2 < headers.len() {
        // Check if this position matches the header name (case-insensitive)
        let mut matches = true;
        for j in 0..name_lower.len() {
            let a = headers[i + j].to_ascii_lowercase();
            let b = name_lower[j].to_ascii_lowercase();
            if a != b {
                matches = false;
                break;
            }
        }
        if matches && headers[i + name_lower.len()] == b':' {
            let val_start = i + name_lower.len() + 1;
            // Skip optional space
            let mut vs = val_start;
            while vs < headers.len() && headers[vs] == b' ' {
                vs += 1;
            }
            // Find end of value (\r\n)
            let mut ve = vs;
            while ve + 1 < headers.len() && !(headers[ve] == b'\r' && headers[ve + 1] == b'\n') {
                ve += 1;
            }
            return Some(&headers[vs..ve]);
        }
        // Advance to next line
        while i + 1 < headers.len() && !(headers[i] == b'\r' && headers[i + 1] == b'\n') {
            i += 1;
        }
        i += 2; // skip \r\n
    }
    None
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    #[test]
    fn test_parse_url() {
        let parts = parse_url("http://example.com/path").unwrap();
        assert_eq!(&parts.host[..parts.host_len], b"example.com");
        assert_eq!(parts.port, 80);
        assert_eq!(&parts.path[..parts.path_len], b"/path");
    }

    #[test]
    fn test_parse_url_with_port() {
        let parts = parse_url("http://example.com:8080/api/test").unwrap();
        assert_eq!(&parts.host[..parts.host_len], b"example.com");
        assert_eq!(parts.port, 8080);
        assert_eq!(&parts.path[..parts.path_len], b"/api/test");
    }

    #[test]
    fn test_parse_response() {
        let resp = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello";
        let result = parse_response(resp);
        assert!(result.is_some());
        let r = result.unwrap();
        assert_eq!(r.status_code, 200);
        assert_eq!(&r.body[..r.body_len], b"hello");
    }
}
