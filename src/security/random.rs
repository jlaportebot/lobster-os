//! Kernel CSPRNG: HMAC_DRBG over SHA-256 (NIST SP 800-90A, section 10.1.2)
//! seeded and periodically reseeded from every entropy source the platform
//! offers. `/dev/urandom` and `getrandom(2)` both draw from here.
//!
//! The DRBG core ([`HmacDrbg`]) is pure and host-testable; the global
//! instance, entropy gathering and status reporting sit below it.

use crate::security::hmac::{HmacSha256, DIGEST_LEN};
use crate::sync::spinlock::Spinlock;
use core::sync::atomic::{AtomicU32, Ordering};

/// SP 800-90A caps one generate request at 2^19 bits.
const MAX_REQUEST_BYTES: usize = 1 << 16;
/// Requests served before the global DRBG pulls fresh entropy again.
const RESEED_INTERVAL: u64 = 256;
/// CNTVCT jitter samples folded into every seed; each sample times a short
/// busy loop, so the low bits depend on cache, bus and scheduler noise.
const JITTER_SAMPLES: usize = 128;
/// Iterations of the busy loop timed per jitter sample.
const JITTER_SPIN: u32 = 61;
/// 32-bit words requested from the lobster-sys entropy register per seed.
const HOST_ENTROPY_WORDS: usize = 16;
/// Largest seed buffer: host words + jitter + fixed-size identity words.
const SEED_CAP: usize = HOST_ENTROPY_WORDS * 4 + JITTER_SAMPLES * 8 + 64;

/// Entropy source bit: lobster-sys host random register.
pub const SRC_HOST: u32 = 1 << 0;
/// Entropy source bit: counter jitter samples.
pub const SRC_JITTER: u32 = 1 << 1;
/// Entropy source bit: RTC wall clock.
pub const SRC_RTC: u32 = 1 << 2;
/// Entropy source bit: CPU identity registers (MIDR/MPIDR).
pub const SRC_CPUID: u32 = 1 << 3;
/// Entropy source bit: boot-time counters (uptime, system time).
pub const SRC_COUNTERS: u32 = 1 << 4;

/// HMAC_DRBG state (key, value, request counter).
pub struct HmacDrbg {
    key: [u8; DIGEST_LEN],
    value: [u8; DIGEST_LEN],
    requests: u64,
}

impl HmacDrbg {
    /// Instantiate from seed material (entropy || nonce || personalisation).
    #[must_use]
    pub fn new(seed: &[u8]) -> Self {
        let mut drbg = Self {
            key: [0x00; DIGEST_LEN],
            value: [0x01; DIGEST_LEN],
            requests: 0,
        };
        drbg.update(seed);
        drbg
    }

    /// HMAC_DRBG_Update: fold `data` into key and value.
    fn update(&mut self, data: &[u8]) {
        self.step(0x00, data);
        if !data.is_empty() {
            self.step(0x01, data);
        }
    }

    fn step(&mut self, tag: u8, data: &[u8]) {
        let mac = HmacSha256::new(&self.key);
        self.key = mac.mac(&[&self.value, &[tag], data]);
        self.value = HmacSha256::new(&self.key).mac(&[&self.value]);
    }

    /// Mix fresh entropy into the state.
    pub fn reseed(&mut self, material: &[u8]) {
        self.update(material);
        self.requests = 0;
    }

    /// Requests served since instantiation or the last reseed.
    #[must_use]
    pub fn requests(&self) -> u64 {
        self.requests
    }

    /// Fill `out` with pseudo-random bytes, splitting oversize requests.
    pub fn generate(&mut self, out: &mut [u8]) {
        for piece in out.chunks_mut(MAX_REQUEST_BYTES) {
            self.generate_request(piece);
        }
    }

    fn generate_request(&mut self, out: &mut [u8]) {
        for chunk in out.chunks_mut(DIGEST_LEN) {
            self.value = HmacSha256::new(&self.key).mac(&[&self.value]);
            chunk.copy_from_slice(&self.value[..chunk.len()]);
        }
        self.update(&[]);
        self.requests += 1;
    }
}

static DRBG: Spinlock<Option<HmacDrbg>> = Spinlock::new(None);
static SOURCES: AtomicU32 = AtomicU32::new(0);

/// Fixed-capacity byte buffer so seeding never allocates.
struct SeedBuf {
    bytes: [u8; SEED_CAP],
    len: usize,
}

impl SeedBuf {
    fn new() -> Self {
        Self {
            bytes: [0; SEED_CAP],
            len: 0,
        }
    }

    fn push(&mut self, data: &[u8]) {
        let room = SEED_CAP - self.len;
        let take = data.len().min(room);
        self.bytes[self.len..self.len + take].copy_from_slice(&data[..take]);
        self.len += take;
    }

    fn as_slice(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}

/// Fill `out` with cryptographically secure random bytes.
pub fn fill(out: &mut [u8]) {
    let mut guard = DRBG.lock();
    let needs_seed = guard
        .as_ref()
        .is_none_or(|drbg| drbg.requests() >= RESEED_INTERVAL);
    if needs_seed {
        let seed = gather_seed();
        match guard.as_mut() {
            Some(drbg) => drbg.reseed(seed.as_slice()),
            None => *guard = Some(HmacDrbg::new(seed.as_slice())),
        }
    }
    if let Some(drbg) = guard.as_mut() {
        drbg.generate(out);
    }
}

/// Mix caller-supplied bytes (a write to /dev/urandom) into the generator.
/// This only adds to the state, so it can never make output more predictable.
pub fn add_entropy(material: &[u8]) {
    fill(&mut []);
    if let Some(drbg) = DRBG.lock().as_mut() {
        drbg.reseed(material);
    }
}

/// A random `u64`.
#[must_use]
pub fn u64() -> u64 {
    let mut bytes = [0u8; 8];
    fill(&mut bytes);
    u64::from_le_bytes(bytes)
}

/// Bitmask of the `SRC_*` sources that contributed to any seed so far.
#[must_use]
pub fn sources_used() -> u32 {
    SOURCES.load(Ordering::Relaxed)
}

/// Human-readable status for the `random status` diagnostic.
#[must_use]
pub fn status() -> alloc::string::String {
    const NAMES: [(u32, &str); 5] = [
        (SRC_HOST, "lobster-sys-entropy"),
        (SRC_JITTER, "cntvct-jitter"),
        (SRC_RTC, "rtc"),
        (SRC_CPUID, "midr/mpidr"),
        (SRC_COUNTERS, "boot-counters"),
    ];
    let used = sources_used();
    let mut line = alloc::string::String::from("random: hmac-drbg-sha256, sources:");
    for (bit, name) in NAMES {
        if used & bit != 0 {
            line.push(' ');
            line.push_str(name);
        }
    }
    if used == 0 {
        line.push_str(" (not yet seeded)");
    }
    line
}

fn gather_seed() -> SeedBuf {
    let mut seed = SeedBuf::new();
    let mut used = 0;
    used |= push_host_entropy(&mut seed);
    used |= push_jitter(&mut seed);
    used |= push_platform_words(&mut seed);
    SOURCES.fetch_or(used, Ordering::Relaxed);
    seed
}

#[cfg(not(feature = "std"))]
fn push_host_entropy(seed: &mut SeedBuf) -> u32 {
    let mut any = false;
    for _ in 0..HOST_ENTROPY_WORDS {
        let Some(word) = crate::drivers::lobster_sys::host_entropy() else {
            break;
        };
        seed.push(&word.to_le_bytes());
        any = true;
    }
    if any {
        SRC_HOST
    } else {
        0
    }
}

#[cfg(not(feature = "std"))]
fn push_jitter(seed: &mut SeedBuf) -> u32 {
    let mut previous = read_counter();
    for _ in 0..JITTER_SAMPLES {
        let mut spin = 0u32;
        for step in 0..JITTER_SPIN {
            spin = core::hint::black_box(spin.wrapping_add(step));
        }
        let now = read_counter();
        seed.push(&(now.wrapping_sub(previous) ^ u64::from(spin)).to_le_bytes());
        previous = now;
    }
    SRC_JITTER
}

#[cfg(not(feature = "std"))]
fn read_counter() -> u64 {
    let count: u64;
    // SAFETY: CNTVCT_EL0 is a read-only counter register accessible at EL1.
    unsafe { core::arch::asm!("mrs {}, cntvct_el0", out(reg) count) };
    count
}

#[cfg(not(feature = "std"))]
fn push_platform_words(seed: &mut SeedBuf) -> u32 {
    let mut used = SRC_COUNTERS;
    let midr: u64;
    // SAFETY: MIDR_EL1 is a read-only identification register readable at EL1.
    unsafe { core::arch::asm!("mrs {}, midr_el1", out(reg) midr) };
    seed.push(&midr.to_le_bytes());
    seed.push(&crate::arch::cpu::Cpu::mpidr().to_le_bytes());
    used |= SRC_CPUID;
    if let Some((epoch, offset)) = crate::drivers::lobster_sys::wall_clock() {
        seed.push(&epoch.to_le_bytes());
        seed.push(&offset.to_le_bytes());
        used |= SRC_RTC;
    }
    seed.push(&read_counter().to_le_bytes());
    seed.push(&crate::arch::timer::Timer::system_time_us().to_le_bytes());
    seed.push(&crate::arch::timer::Timer::uptime_ms().to_le_bytes());
    used
}

/// Host test builds have no platform registers: the seed comes from the
/// process clock and address-space layout so instances still differ.
#[cfg(feature = "std")]
fn push_host_entropy(_seed: &mut SeedBuf) -> u32 {
    0
}

#[cfg(feature = "std")]
fn push_jitter(seed: &mut SeedBuf) -> u32 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    seed.push(&nanos.to_le_bytes());
    SRC_JITTER
}

#[cfg(feature = "std")]
fn push_platform_words(seed: &mut SeedBuf) -> u32 {
    let local = 0u8;
    seed.push(&(&local as *const u8 as usize).to_le_bytes());
    SRC_COUNTERS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: &[u8]) -> std::string::String {
        bytes.iter().map(|b| std::format!("{b:02x}")).collect()
    }

    fn seed48() -> [u8; 48] {
        core::array::from_fn(|i| i as u8)
    }

    // Reference values come from /mnt/data/hermes/cache/scratch/lock/vectors.py:
    // an independent SP 800-90A HMAC_DRBG written on top of Python's hmac/hashlib.
    #[test]
    fn drbg_matches_independent_reference_output() {
        let mut drbg = HmacDrbg::new(&seed48());
        let mut out = [0u8; 40];
        drbg.generate(&mut out);
        assert_eq!(
            hex(&out),
            "0ffb80875a3e9022a4941a3fa1b0d3611df14e1cf651a73ce9229b9f3ad56887680428845710288e"
        );
        drbg.generate(&mut out);
        assert_eq!(
            hex(&out),
            "cac8490ba9b23ffc16f14f9b05d42adbabc2f9b96b2abe2561240450cdd38b52b99c232018196a00"
        );
    }

    #[test]
    fn reseed_changes_the_following_output_to_the_reference() {
        let mut drbg = HmacDrbg::new(&seed48());
        let mut out = [0u8; 40];
        drbg.generate(&mut out);
        drbg.generate(&mut out);
        drbg.reseed(b"reseed-material");
        assert_eq!(drbg.requests(), 0);
        drbg.generate(&mut out);
        assert_eq!(
            hex(&out),
            "913b999f119e07e4bed9848375cb8ccccd431138c746156f1c5704a3643b47ca91b808c4beb4bb3f"
        );
    }

    #[test]
    fn differently_seeded_instances_differ() {
        let (mut a, mut b) = (HmacDrbg::new(b"seed-a"), HmacDrbg::new(b"seed-b"));
        let (mut out_a, mut out_b) = ([0u8; 32], [0u8; 32]);
        a.generate(&mut out_a);
        b.generate(&mut out_b);
        assert_ne!(out_a, out_b);
    }

    #[test]
    fn consecutive_requests_differ() {
        let mut drbg = HmacDrbg::new(b"seed");
        let (mut first, mut second) = ([0u8; 32], [0u8; 32]);
        drbg.generate(&mut first);
        drbg.generate(&mut second);
        assert_ne!(first, second);
    }

    #[test]
    fn odd_lengths_are_filled_completely() {
        let mut drbg = HmacDrbg::new(b"seed");
        let mut out = [0u8; 70];
        drbg.generate(&mut out);
        assert!(out[64..].iter().any(|&b| b != 0));
    }

    #[test]
    fn global_fill_returns_distinct_blocks_and_records_sources() {
        let _guard = crate::test_support::serial_guard();
        let (mut a, mut b) = ([0u8; 32], [0u8; 32]);
        fill(&mut a);
        fill(&mut b);
        assert_ne!(a, b);
        assert_ne!(sources_used(), 0);
        assert_ne!(u64(), u64());
    }

    #[test]
    fn status_names_the_sources() {
        let _guard = crate::test_support::serial_guard();
        fill(&mut [0u8; 1]);
        assert!(status().contains("hmac-drbg-sha256"));
    }
}
