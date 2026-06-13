//! DNS Cache module for LobsterOS
//!
//! Provides a thread-safe DNS cache for resolved A records with TTL expiration.
//! Uses a simple array-based LRU eviction policy suitable for no_std environments.

use spin::Mutex;

/// Maximum cached DNS entries
const DNS_CACHE_SIZE: usize = 16;

/// Maximum domain name length stored in cache
const DNS_CACHE_NAME_LEN: usize = 128;

/// A single DNS cache entry
#[derive(Clone, Copy, Debug)]
pub struct DnsCacheEntry {
    /// Domain name (zero-padded)
    name: [u8; DNS_CACHE_NAME_LEN],
    /// Resolved IPv4 address
    ip: [u8; 4],
    /// TTL in seconds from the DNS response
    ttl: u32,
    /// Timestamp when cached (system ticks in ms)
    cached_at: u64,
    /// Whether this entry is valid
    valid: bool,
}

impl DnsCacheEntry {
    /// Create an empty/invalid entry
    const fn empty() -> Self {
        DnsCacheEntry {
            name: [0u8; DNS_CACHE_NAME_LEN],
            ip: [0u8; 4],
            ttl: 0,
            cached_at: 0,
            valid: false,
        }
    }

    /// Check if this entry matches the given domain and is not expired
    fn matches(&self, domain: &str, now_ms: u64) -> bool {
        if !self.valid {
            return false;
        }
        // Compare domain name
        let name_len = self
            .name
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(DNS_CACHE_NAME_LEN);
        if name_len != domain.len() {
            return false;
        }
        if &self.name[..name_len] != domain.as_bytes() {
            return false;
        }
        // Check TTL expiry
        let elapsed_secs = ((now_ms - self.cached_at) / 1000) as u32;
        elapsed_secs <= self.ttl
    }

    /// Invalidate this entry
    fn invalidate(&mut self) {
        self.valid = false;
    }
}

/// Global DNS cache protected by a spinlock for SMP safety
static DNS_CACHE: Mutex<[DnsCacheEntry; DNS_CACHE_SIZE]> =
    Mutex::new([DnsCacheEntry::empty(); DNS_CACHE_SIZE]);

/// System time provider - uses the architecture timer
fn now_ms() -> u64 {
    crate::arch::timer::Timer::uptime_ms()
}

/// Look up a domain in the DNS cache
/// Returns `Some(ip)` if found and not expired, `None` otherwise
pub fn cache_lookup(domain: &str) -> Option<[u8; 4]> {
    let now = now_ms();
    let cache = DNS_CACHE.lock();
    for entry in cache.iter() {
        if entry.matches(domain, now) {
            return Some(entry.ip);
        }
    }
    None
}

/// Insert a resolved record into the DNS cache
/// Uses LRU eviction when the cache is full
pub fn cache_insert(domain: &str, ip: [u8; 4], ttl: u32) {
    let now = now_ms();
    let mut cache = DNS_CACHE.lock();

    // Find an empty slot, or the least recently used entry
    let mut lru_idx = 0;
    let mut lru_time = u64::MAX;
    let mut empty_idx: Option<usize> = None;

    for (i, entry) in cache.iter().enumerate() {
        if !entry.valid {
            empty_idx = Some(i);
            break;
        }
        if entry.cached_at < lru_time {
            lru_time = entry.cached_at;
            lru_idx = i;
        }
    }

    let idx = empty_idx.unwrap_or(lru_idx);
    let entry = &mut cache[idx];
    entry.name = [0u8; DNS_CACHE_NAME_LEN];
    let copy_len = domain.len().min(DNS_CACHE_NAME_LEN);
    entry.name[..copy_len].copy_from_slice(&domain.as_bytes()[..copy_len]);
    entry.ip = ip;
    entry.ttl = ttl;
    entry.cached_at = now;
    entry.valid = true;
}

/// Invalidate the entire DNS cache (e.g. on DHCP reconfigure or network change)
pub fn cache_flush() {
    let mut cache = DNS_CACHE.lock();
    for entry in cache.iter_mut() {
        entry.invalidate();
    }
}

/// Get cache statistics for debugging
pub fn cache_stats() -> (usize, usize) {
    let cache = DNS_CACHE.lock();
    let valid = cache.iter().filter(|e| e.valid).count();
    (valid, DNS_CACHE_SIZE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_entry_matches() {
        let mut entry = DnsCacheEntry::empty();
        let now = 10000;
        entry.name[..11].copy_from_slice(b"example.com");
        entry.ip = [93, 184, 216, 34];
        entry.ttl = 300;
        entry.cached_at = now;
        entry.valid = true;

        // Should match at same time
        assert!(entry.matches("example.com", now));

        // Should match within TTL
        assert!(entry.matches("example.com", now + 100_000)); // 100s later

        // Should not match after TTL expiry
        assert!(!entry.matches("example.com", now + 400_000)); // 400s later

        // Should not match different domain
        assert!(!entry.matches("other.com", now));

        // Should not match if invalid
        entry.invalidate();
        assert!(!entry.matches("example.com", now));
    }

    #[test]
    fn test_cache_insert_and_lookup() {
        cache_flush();

        // Insert a record
        cache_insert("example.com", [93, 184, 216, 34], 300);

        // Lookup should succeed
        let ip = cache_lookup("example.com");
        assert_eq!(ip, Some([93, 184, 216, 34]));

        // Different domain should not be found
        assert_eq!(cache_lookup("other.com"), None);

        // Flush should clear the cache
        cache_flush();
        assert_eq!(cache_lookup("example.com"), None);
    }

    #[test]
    fn test_cache_lru_eviction() {
        cache_flush();

        // Fill the cache completely
        for i in 0..DNS_CACHE_SIZE {
            let domain = format!("host{}.example.com", i);
            let ip = [192, 168, 1, i as u8];
            cache_insert(&domain, ip, 300);
        }

        // All entries should be present
        for i in 0..DNS_CACHE_SIZE {
            let domain = format!("host{}.example.com", i);
            let ip = cache_lookup(&domain);
            assert_eq!(ip, Some([192, 168, 1, i as u8]), "Entry {} should exist", i);
        }

        // Insert one more - should evict LRU (host0)
        cache_insert("new.example.com", [10, 0, 0, 1], 300);

        // New entry should exist
        assert_eq!(cache_lookup("new.example.com"), Some([10, 0, 0, 1]));

        // LRU (host0) should be evicted
        assert_eq!(cache_lookup("host0.example.com"), None);

        // Others should still exist
        for i in 1..DNS_CACHE_SIZE {
            let domain = format!("host{}.example.com", i);
            let ip = cache_lookup(&domain);
            assert_eq!(
                ip,
                Some([192, 168, 1, i as u8]),
                "Entry {} should still exist",
                i
            );
        }
    }

    #[test]
    fn test_cache_stats() {
        cache_flush();
        let (valid, total) = cache_stats();
        assert_eq!(valid, 0);
        assert_eq!(total, DNS_CACHE_SIZE);

        cache_insert("test.example.com", [1, 2, 3, 4], 300);
        let (valid, total) = cache_stats();
        assert_eq!(valid, 1);
        assert_eq!(total, DNS_CACHE_SIZE);
    }
}
