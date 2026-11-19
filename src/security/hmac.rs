//! HMAC-SHA-256 (RFC 2104) and PBKDF2-HMAC-SHA-256 (RFC 8018), built on the
//! `sha2` crate so the kernel needs no further dependency. Used by the
//! CSPRNG (HMAC_DRBG) and by password hashing.

use sha2::{Digest, Sha256};

/// SHA-256 block size in bytes (HMAC key padding width).
const BLOCK_LEN: usize = 64;
/// SHA-256 digest size in bytes.
pub const DIGEST_LEN: usize = 32;
/// Inner padding byte from RFC 2104.
const IPAD: u8 = 0x36;
/// Outer padding byte from RFC 2104.
const OPAD: u8 = 0x5c;

/// A keyed HMAC-SHA-256 context. The padded-key hash states are computed
/// once so repeated MACs (PBKDF2 inner loop) only hash the message.
#[derive(Clone)]
pub struct HmacSha256 {
    inner: Sha256,
    outer: Sha256,
}

impl HmacSha256 {
    /// Key the context; keys longer than a block are hashed first.
    #[must_use]
    pub fn new(key: &[u8]) -> Self {
        let mut block = [0u8; BLOCK_LEN];
        if key.len() > BLOCK_LEN {
            block[..DIGEST_LEN].copy_from_slice(&Sha256::digest(key));
        } else {
            block[..key.len()].copy_from_slice(key);
        }
        let mut inner = Sha256::new();
        let mut outer = Sha256::new();
        inner.update(block.map(|b| b ^ IPAD));
        outer.update(block.map(|b| b ^ OPAD));
        Self { inner, outer }
    }

    /// MAC the concatenation of `parts`.
    #[must_use]
    pub fn mac(&self, parts: &[&[u8]]) -> [u8; DIGEST_LEN] {
        let mut inner = self.inner.clone();
        for part in parts {
            inner.update(part);
        }
        let mut outer = self.outer.clone();
        outer.update(inner.finalize());
        outer.finalize().into()
    }
}

/// One-shot HMAC-SHA-256 of the concatenation of `parts` under `key`.
#[must_use]
pub fn hmac_sha256(key: &[u8], parts: &[&[u8]]) -> [u8; DIGEST_LEN] {
    HmacSha256::new(key).mac(parts)
}

/// PBKDF2-HMAC-SHA-256: derive `out.len()` bytes from `password` and `salt`.
/// `iterations` must be at least 1 (0 is treated as 1).
pub fn pbkdf2_sha256(password: &[u8], salt: &[u8], iterations: u32, out: &mut [u8]) {
    let prf = HmacSha256::new(password);
    for (index, chunk) in out.chunks_mut(DIGEST_LEN).enumerate() {
        let block_no = (index as u32 + 1).to_be_bytes();
        let mut u = prf.mac(&[salt, &block_no]);
        let mut acc = u;
        for _ in 1..iterations {
            u = prf.mac(&[&u]);
            for (a, b) in acc.iter_mut().zip(u.iter()) {
                *a ^= b;
            }
        }
        chunk.copy_from_slice(&acc[..chunk.len()]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: &[u8]) -> std::string::String {
        bytes.iter().map(|b| std::format!("{b:02x}")).collect()
    }

    #[test]
    fn hmac_matches_rfc4231_case_1() {
        let mac = hmac_sha256(&[0x0b; 20], &[b"Hi There"]);
        assert_eq!(
            hex(&mac),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
    }

    #[test]
    fn hmac_hashes_keys_longer_than_a_block() {
        let msg = b"Test Using Larger Than Block-Size Key - Hash Key First";
        assert_eq!(
            hex(&hmac_sha256(&[0xaa; 131], &[msg])),
            "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
        );
    }

    #[test]
    fn hmac_parts_equal_concatenation() {
        assert_eq!(
            hmac_sha256(b"k", &[b"ab", b"cd"]),
            hmac_sha256(b"k", &[b"abcd"])
        );
    }

    #[test]
    fn pbkdf2_matches_python_hashlib_vectors() {
        let cases: [(&[u8], &[u8], u32, usize, &str); 4] = [
            (
                b"password",
                b"salt",
                1,
                32,
                "120fb6cffcf8b32c43e7225256c4f837a86548c92ccc35480805987cb70be17b",
            ),
            (
                b"password",
                b"salt",
                2,
                32,
                "ae4d0c95af6b46d32d0adff928f06dd02a303f8ef3c251dfd6e2d85a95474c43",
            ),
            (
                b"password",
                b"salt",
                4096,
                32,
                "c5e478d59288c841aa530db6845c4c8d962893a001ce4e11a4963873aa98134a",
            ),
            (
                b"passwordPASSWORDpassword",
                b"saltSALTsaltSALTsaltSALTsaltSALTsalt",
                4096,
                40,
                "348c89dbcbd32b2f32d814b8116e84cf2b17347ebc1800181c4e2a1fb8dd53e1c635518c7dac47e9",
            ),
        ];
        for (pw, salt, iters, len, want) in cases {
            let mut out = std::vec![0u8; len];
            pbkdf2_sha256(pw, salt, iters, &mut out);
            assert_eq!(hex(&out), want);
        }
    }
}
