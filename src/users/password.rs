//! Password hash format and verification: PBKDF2-HMAC-SHA-256 with a random
//! 16-byte salt, stored passlib-style as
//! `$pbkdf2-sha256$<iterations>$<salt-b64>$<hash-b64>` (adapted base64, `.`
//! instead of `+`, no padding). Legacy unsalted `$sha256$<hex>` hashes still
//! verify so existing accounts keep working, and report that they should be
//! re-hashed.

use crate::security::hmac::{pbkdf2_sha256, DIGEST_LEN};
use alloc::string::String;
use alloc::vec::Vec;
use sha2::{Digest, Sha256};

/// PBKDF2 iteration count for new hashes. Measured with `authbench 12000`
/// under the TCG-emulated Pi 5 on the build host: 360 and 378 ms per hash
/// (about 30 us per iteration), the 0.3-0.5 s a login should take. A real
/// Pi 5 is far faster, so this is the slow-emulator floor, not the target
/// for hardware; raise it when the emulator gets faster.
pub const PBKDF2_ITERATIONS: u32 = 12_000;
/// Refuse stored hashes demanding more than this, so a corrupted or hostile
/// shadow file cannot hang the login screen.
const MAX_ITERATIONS: u32 = 5_000_000;
/// Salt length for new hashes, in bytes.
pub const SALT_LEN: usize = 16;
/// Shortest and longest stored salt accepted when verifying.
const SALT_RANGE: core::ops::RangeInclusive<usize> = 8..=64;
/// Identifier prefix of the current format.
const PBKDF2_PREFIX: &str = "$pbkdf2-sha256$";
/// Identifier prefix of the legacy unsalted format.
const LEGACY_PREFIX: &str = "$sha256$";
/// Base64 alphabet with `.` in place of `+` (passlib "ab64").
const B64_ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789./";
/// Bits carried by one base64 character.
const B64_BITS: u32 = 6;

/// Outcome of checking a password against a stored hash.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Correct and the hash is current.
    Match,
    /// Correct, but stored in the legacy or a weaker format: re-hash it.
    MatchNeedsRehash,
    /// Wrong password, or the stored string is not a usable hash.
    Mismatch,
}

fn b64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len() * 4 / 3 + 2);
    let (mut acc, mut bits) = (0u32, 0u32);
    for &byte in data {
        acc = (acc << 8) | u32::from(byte);
        bits += 8;
        while bits >= B64_BITS {
            bits -= B64_BITS;
            out.push(char::from(B64_ALPHABET[((acc >> bits) & 0x3F) as usize]));
        }
    }
    if bits > 0 {
        out.push(char::from(
            B64_ALPHABET[((acc << (B64_BITS - bits)) & 0x3F) as usize],
        ));
    }
    out
}

fn b64_decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for ch in text.bytes() {
        let value = B64_ALPHABET.iter().position(|&c| c == ch)? as u32;
        acc = (acc << B64_BITS) | value;
        bits += B64_BITS;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// Compare two byte strings without an early exit on the first difference.
#[must_use]
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let diff = a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y));
    core::hint::black_box(diff) == 0
}

/// Hash `password` with an explicit salt and iteration count.
#[must_use]
pub fn hash_with(password: &str, iterations: u32, salt: &[u8]) -> String {
    let mut derived = [0u8; DIGEST_LEN];
    pbkdf2_sha256(password.as_bytes(), salt, iterations, &mut derived);
    alloc::format!(
        "{PBKDF2_PREFIX}{iterations}${}${}",
        b64_encode(salt),
        b64_encode(&derived)
    )
}

/// Hash `password` with a fresh random salt at the current cost.
#[must_use]
pub fn hash(password: &str) -> String {
    let mut salt = [0u8; SALT_LEN];
    crate::security::random::fill(&mut salt);
    hash_with(password, PBKDF2_ITERATIONS, &salt)
}

struct Parsed {
    iterations: u32,
    salt: Vec<u8>,
    digest: Vec<u8>,
}

fn parse(stored: &str) -> Option<Parsed> {
    let mut fields = stored.strip_prefix(PBKDF2_PREFIX)?.split('$');
    let iterations: u32 = fields.next()?.parse().ok()?;
    let salt = b64_decode(fields.next()?)?;
    let digest = b64_decode(fields.next()?)?;
    let well_formed = fields.next().is_none()
        && (1..=MAX_ITERATIONS).contains(&iterations)
        && SALT_RANGE.contains(&salt.len())
        && digest.len() == DIGEST_LEN;
    well_formed.then_some(Parsed {
        iterations,
        salt,
        digest,
    })
}

fn verify_legacy(password: &str, stored: &str) -> bool {
    let Some(hex) = stored.strip_prefix(LEGACY_PREFIX) else {
        return false;
    };
    let digest = Sha256::digest(password.as_bytes());
    let mut expected = String::with_capacity(DIGEST_LEN * 2);
    for byte in digest.iter() {
        expected.push(char::from(b"0123456789abcdef"[usize::from(byte >> 4)]));
        expected.push(char::from(b"0123456789abcdef"[usize::from(byte & 0xF)]));
    }
    ct_eq(expected.as_bytes(), hex.as_bytes())
}

/// Check `password` against a stored hash string. Never panics on malformed
/// input; unusable strings are a mismatch.
#[must_use]
pub fn verify(password: &str, stored: &str) -> Verdict {
    if let Some(p) = parse(stored) {
        let mut derived = [0u8; DIGEST_LEN];
        pbkdf2_sha256(password.as_bytes(), &p.salt, p.iterations, &mut derived);
        return match (ct_eq(&derived, &p.digest), p.iterations < PBKDF2_ITERATIONS) {
            (false, _) => Verdict::Mismatch,
            (true, true) => Verdict::MatchNeedsRehash,
            (true, false) => Verdict::Match,
        };
    }
    if verify_legacy(password, stored) {
        Verdict::MatchNeedsRehash
    } else {
        Verdict::Mismatch
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Reference from Python:
    //   import hashlib,base64
    //   dk=hashlib.pbkdf2_hmac("sha256",b"correct horse",bytes(range(16)),12000)
    //   b64 = lambda x: base64.b64encode(x).decode().rstrip("=").replace("+", ".")
    //   "$pbkdf2-sha256$12000$" + b64(salt) + "$" + b64(dk)
    const REFERENCE: &str =
        "$pbkdf2-sha256$12000$AAECAwQFBgcICQoLDA0ODw$K2IO3tVkvR2aRAZdGN1sn.xFgNsFtXA66lhQefleiOs";

    fn salt() -> [u8; SALT_LEN] {
        core::array::from_fn(|i| i as u8)
    }

    #[test]
    fn hash_matches_python_reference() {
        let hash = hash_with("correct horse", PBKDF2_ITERATIONS, &salt());
        assert_eq!(hash, REFERENCE);
    }

    #[test]
    fn roundtrip_verifies_and_wrong_password_fails() {
        let stored = hash("hunter2");
        assert_eq!(verify("hunter2", &stored), Verdict::Match);
        assert_eq!(verify("hunter3", &stored), Verdict::Mismatch);
        assert_eq!(verify("", &stored), Verdict::Mismatch);
    }

    #[test]
    fn salts_are_unique_per_hash() {
        let _guard = crate::test_support::serial_guard();
        assert_ne!(hash("same"), hash("same"));
    }

    #[test]
    fn legacy_sha256_hash_verifies_and_asks_for_rehash() {
        let legacy = "$sha256$5e884898da28047151d0e56f8dc6292773603d0d6aabbdd62a11ef721d1542d8";
        assert_eq!(verify("password", legacy), Verdict::MatchNeedsRehash);
        assert_eq!(verify("Password", legacy), Verdict::Mismatch);
    }

    #[test]
    fn lower_cost_hash_asks_for_rehash() {
        let weak = hash_with("pw", 1_000, &salt());
        assert_eq!(verify("pw", &weak), Verdict::MatchNeedsRehash);
    }

    #[test]
    fn malformed_hashes_are_rejected_without_panicking() {
        let bad = [
            "",
            "$",
            "$pbkdf2-sha256$",
            "$pbkdf2-sha256$12000",
            "$pbkdf2-sha256$12000$AAECAwQFBgcICQoLDA0ODw",
            "$pbkdf2-sha256$0$AAECAwQFBgcICQoLDA0ODw$AAECAwQFBgcICQoLDA0ODw",
            "$pbkdf2-sha256$99999999999$AAECAwQFBgcICQoLDA0ODw$x",
            "$pbkdf2-sha256$abc$AAECAwQFBgcICQoLDA0ODw$x",
            "$pbkdf2-sha256$12000$!!!!$????",
            "$pbkdf2-sha256$12000$AA$AA",
            "$pbkdf2-sha256$12000$AAECAwQFBgcICQoLDA0ODw$AAECAwQFBgcICQoLDA0ODw$extra",
            "$sha256$zz",
            "$argon2id$v=19$m=65536,t=3,p=4$salt$hash",
            "!",
            "*",
            "plaintext",
        ];
        for stored in bad {
            assert_eq!(verify("pw", stored), Verdict::Mismatch, "{stored}");
        }
    }

    #[test]
    fn base64_roundtrips_all_lengths() {
        for len in 0..40usize {
            let data: Vec<u8> = (0..len).map(|i| (i * 7 + 3) as u8).collect();
            assert_eq!(b64_decode(&b64_encode(&data)), Some(data));
        }
    }

    #[test]
    fn constant_time_compare_rejects_length_and_content_changes() {
        assert!(ct_eq(b"abc", b"abc"));
        assert!(!ct_eq(b"abc", b"abd"));
        assert!(!ct_eq(b"abc", b"ab"));
    }
}
