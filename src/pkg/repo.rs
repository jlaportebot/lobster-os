//! Repository handling for LobsterOS package manager
//!
//! Features:
//! - Repository configuration and management
//! - HTTP index download with JSON parsing
//! - Ed25519 signature verification for repository authenticity

use crate::pkg::package::{AppKind, PackageArch, RepoPackage, RepositoryIndex, StoreMeta};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};

/// Maximum public key size (Ed25519 = 32 bytes)
const ED25519_PUBKEY_LEN: usize = 32;
/// Maximum signature size (Ed25519 = 64 bytes)
const ED25519_SIG_LEN: usize = 64;

/// Name of the repository every install starts with.
pub const DEFAULT_REPO_NAME: &str = "lobster-main";
/// Where the CI workflow publishes the catalogue (GitHub release `pkg-repo`).
pub const DEFAULT_REPO_URL: &str =
    "https://github.com/jlaportebot/lobster-os/releases/download/pkg-repo";
/// Ed25519 public key (hex) the catalogue index is signed with; the private
/// half is the `PKG_SIGNING_KEY` secret of the pkg-repo workflow.
pub const DEFAULT_REPO_KEY_HEX: &str =
    "fe09e6d0b0b94b7dd72b5ceaf358fb27c28e7981b4193f833602e888e2bd57bc";
/// File next to `index.json` holding the hex signature over its exact bytes.
pub const INDEX_SIGNATURE_FILE: &str = "index.json.sig";
/// Priority of the default repository.
const DEFAULT_REPO_PRIORITY: i32 = 10;

/// Repository configuration
#[derive(Debug, Clone)]
pub struct RepositoryConfig {
    pub name: String,
    pub url: String,
    pub enabled: bool,
    pub priority: i32,
    pub verify_signatures: bool,
    /// Ed25519 public key for signature verification (32 bytes, hex-encoded when stored)
    pub public_key: Option<[u8; ED25519_PUBKEY_LEN]>,
}

impl RepositoryConfig {
    pub fn new(name: &str, url: &str) -> Self {
        Self {
            name: name.to_string(),
            url: url.to_string(),
            enabled: true,
            priority: 0,
            verify_signatures: true,
            public_key: None,
        }
    }

    /// Set the Ed25519 public key for signature verification
    /// Key should be 32 bytes (raw) or 64 hex characters
    pub fn set_public_key(&mut self, key_hex: &str) -> Result<(), RepoError> {
        if key_hex.len() != 64 {
            return Err(RepoError::SignatureError);
        }
        let mut key = [0u8; ED25519_PUBKEY_LEN];
        for i in 0..ED25519_PUBKEY_LEN {
            let byte_str = &key_hex[i * 2..i * 2 + 2];
            key[i] = u8::from_str_radix(byte_str, 16).map_err(|_| RepoError::SignatureError)?;
        }
        self.public_key = Some(key);
        Ok(())
    }

    /// Get the public key as hex string
    pub fn public_key_hex(&self) -> Option<String> {
        self.public_key.map(|k| {
            let mut s = String::with_capacity(64);
            for &b in &k {
                use core::fmt::Write;
                write!(&mut s, "{:02x}", b).ok();
            }
            s
        })
    }
}

/// Repository manager
pub struct RepositoryManager {
    repos: Vec<RepositoryConfig>,
    indices: Vec<RepositoryIndex>,
}

impl RepositoryManager {
    pub fn new() -> Self {
        Self {
            repos: Vec::new(),
            indices: Vec::new(),
        }
    }

    /// Add a repository
    pub fn add_repo(&mut self, config: RepositoryConfig) {
        self.repos.push(config);
    }

    /// Remove a repository by name
    pub fn remove_repo(&mut self, name: &str) -> bool {
        if let Some(pos) = self.repos.iter().position(|r| r.name == name) {
            self.repos.remove(pos);
            self.indices.retain(|i| i.name != name);
            true
        } else {
            false
        }
    }

    /// Get repository config by name
    pub fn get_repo(&self, name: &str) -> Option<&RepositoryConfig> {
        self.repos.iter().find(|r| r.name == name)
    }

    /// Get mutable repository config by name
    pub fn get_repo_mut(&mut self, name: &str) -> Option<&mut RepositoryConfig> {
        self.repos.iter_mut().find(|r| r.name == name)
    }

    /// List all repositories
    pub fn list_repos(&self) -> &[RepositoryConfig] {
        &self.repos
    }

    /// Save repository configuration to VFS
    pub fn save_config(&self) -> Result<(), RepoError> {
        let _ = crate::fs::vfs::create_dir("/etc/pkg");

        // Build config content
        let mut content = String::new();
        content.push_str("# LobsterOS repository configuration\n");
        content.push_str("# Format: name|url|enabled|priority|verify_signatures|public_key_hex\n");
        for repo in &self.repos {
            let pubkey_hex = repo.public_key_hex().unwrap_or_default();
            content.push_str(&alloc::format!(
                "{}|{}|{}|{}|{}|{}\n",
                repo.name,
                repo.url,
                if repo.enabled { "1" } else { "0" },
                repo.priority,
                if repo.verify_signatures { "1" } else { "0" },
                pubkey_hex
            ));
        }

        // write_file truncates and writes through to the disk.
        crate::fs::vfs::write_file("/etc/pkg/repos.conf", content.as_bytes())
            .map(|_| ())
            .map_err(|_| RepoError::IoError)
    }

    /// Load repository configuration from VFS
    pub fn load_config(&mut self) -> Result<(), RepoError> {
        use crate::fs::vfs::read_file;

        let data = match read_file("/etc/pkg/repos.conf") {
            Ok(d) => d,
            Err(_) => return Ok(()), // No config file yet, return empty
        };

        let content = core::str::from_utf8(&data).unwrap_or("");
        for line in content.lines() {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let parts: Vec<&str> = line.split('|').collect();
            if parts.len() < 5 {
                continue;
            }
            let mut config = RepositoryConfig::new(parts[0], parts[1]);
            config.enabled = parts[2] == "1";
            config.priority = parts[3].parse().unwrap_or(0);
            config.verify_signatures = parts[4] == "1";
            if parts.len() >= 6 && !parts[5].is_empty() {
                let _ = config.set_public_key(parts[5]);
            }
            self.add_repo(config);
        }
        Ok(())
    }

    /// Update repository index (fetch from remote)
    #[cfg(target_arch = "aarch64")]
    pub fn update_index(&mut self, name: &str) -> Result<(), RepoError> {
        let repo = self
            .repos
            .iter()
            .find(|r| r.name == name && r.enabled)
            .ok_or(RepoError::NotFound)?;
        let index_url = format!("{}/index.json", repo.url);
        let verify = repo.verify_signatures;
        let signature_url = format!("{}/{}", repo.url, INDEX_SIGNATURE_FILE);
        crate::println!("[pkg] Updating repository index from {}", index_url);

        let body = fetch_ok(&index_url)?;
        let signature = if verify {
            Some(fetch_ok(&signature_url).map_err(|_| RepoError::SignatureError)?)
        } else {
            None
        };
        self.ingest_index(
            name,
            &body,
            signature.as_deref(),
            crate::arch::timer::Timer::uptime_ms(),
        )
    }

    /// Update repository index (x86_64 stub)
    #[cfg(target_arch = "x86_64")]
    pub fn update_index(&mut self, _name: &str) -> Result<(), RepoError> {
        Err(RepoError::NotImplemented)
    }

    /// Make sure the default repository exists and carries the catalogue
    /// signing key. A repository configured before signing existed (same
    /// name and URL, no key) is upgraded in place. Returns true when the
    /// configuration changed and should be saved.
    pub fn ensure_default_repo(&mut self) -> bool {
        if self.repos.is_empty() {
            let mut repo = RepositoryConfig::new(DEFAULT_REPO_NAME, DEFAULT_REPO_URL);
            repo.priority = DEFAULT_REPO_PRIORITY;
            let _ = repo.set_public_key(DEFAULT_REPO_KEY_HEX);
            self.repos.push(repo);
            return true;
        }
        let Some(repo) = self.repos.iter_mut().find(|r| {
            r.name == DEFAULT_REPO_NAME && r.url.trim_end_matches('/') == DEFAULT_REPO_URL
        }) else {
            return false;
        };
        if repo.public_key.is_some() {
            return false;
        }
        repo.verify_signatures = true;
        repo.set_public_key(DEFAULT_REPO_KEY_HEX).is_ok()
    }

    /// Check `body` against its detached `signature` (when the repository
    /// demands one), parse it and store it as the cached index of `name`.
    pub fn ingest_index(
        &mut self,
        name: &str,
        body: &[u8],
        signature: Option<&[u8]>,
        now_ms: u64,
    ) -> Result<(), RepoError> {
        let repo = self
            .repos
            .iter()
            .find(|r| r.name == name && r.enabled)
            .ok_or(RepoError::NotFound)?;
        if repo.verify_signatures {
            let key = repo.public_key.ok_or(RepoError::SignatureError)?;
            verify_detached_signature(body, signature.ok_or(RepoError::SignatureError)?, &key)?;
        }
        let json = core::str::from_utf8(body).map_err(|_| RepoError::ParseError)?;
        let mut index = parse_repository_index(json, &repo.name, &repo.url)?;
        index.timestamp = now_ms;
        match self.indices.iter().position(|i| i.name == name) {
            Some(pos) => self.indices[pos] = index,
            None => self.indices.push(index),
        }
        Ok(())
    }

    /// Get index for a repository
    pub fn get_index(&self, name: &str) -> Option<&RepositoryIndex> {
        self.indices.iter().find(|i| i.name == name)
    }

    /// Search across all repositories
    pub fn search(&self, query: &str) -> Vec<(&str, &RepoPackage)> {
        let mut results = Vec::new();
        for index in &self.indices {
            for pkg in index.search(query) {
                results.push((index.name.as_str(), pkg));
            }
        }
        results
    }

    /// Find package across all repositories (returns latest version)
    /// True if any repository index is cached (populated by update_index).
    pub fn has_indices(&self) -> bool {
        !self.indices.is_empty()
    }

    pub fn find_package(&self, name: &str) -> Option<(&RepositoryIndex, &RepoPackage)> {
        for index in &self.indices {
            if let Some(pkg) = index.find_package(name) {
                return Some((index, pkg));
            }
        }
        None
    }
}

impl Default for RepositoryManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Repository errors
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepoError {
    NotFound,
    NetworkError,
    ParseError,
    SignatureError,
    IoError,
    HttpError(u16),
    NotImplemented,
}

impl core::fmt::Display for RepoError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            RepoError::NotFound => write!(f, "Repository not found"),
            RepoError::NetworkError => write!(f, "Network error"),
            RepoError::ParseError => write!(f, "Parse error"),
            RepoError::SignatureError => write!(f, "Signature verification failed"),
            RepoError::IoError => write!(f, "I/O error"),
            RepoError::HttpError(code) => write!(f, "HTTP error: {}", code),
            RepoError::NotImplemented => write!(f, "Not implemented"),
        }
    }
}

/// Verify package checksum
pub fn verify_checksum(data: &[u8], expected: &[u8]) -> bool {
    if expected.len() != 32 {
        return false;
    }
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(data);
    let result = hasher.finalize();
    result.as_slice() == expected
}

/// Verify the detached Ed25519 signature of a repository index.
///
/// `signature` is the content of `index.json.sig`: 128 hex characters (the
/// 64-byte signature) optionally followed by whitespace. It signs the exact
/// bytes of `index.json`, so nothing is re-serialised before checking.
pub fn verify_detached_signature(
    data: &[u8],
    signature: &[u8],
    pubkey: &[u8; ED25519_PUBKEY_LEN],
) -> Result<(), RepoError> {
    let text = core::str::from_utf8(signature)
        .map_err(|_| RepoError::SignatureError)?
        .trim();
    if text.len() != ED25519_SIG_LEN * 2 || !text.is_ascii() {
        return Err(RepoError::SignatureError);
    }
    let mut sig_bytes = [0u8; ED25519_SIG_LEN];
    for (i, byte) in sig_bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[i * 2..i * 2 + 2], 16)
            .map_err(|_| RepoError::SignatureError)?;
    }
    let verifying_key = VerifyingKey::from_bytes(pubkey).map_err(|_| RepoError::SignatureError)?;
    verifying_key
        .verify(data, &Signature::from_bytes(&sig_bytes))
        .map_err(|_| RepoError::SignatureError)
}

/// GET `url`, accepting only HTTP 200 and a complete body.
#[cfg(target_arch = "aarch64")]
fn fetch_ok(url: &str) -> Result<Vec<u8>, RepoError> {
    let response = crate::net::fetch(url).map_err(|_| RepoError::NetworkError)?;
    if response.status != 200 {
        crate::println!("[pkg] HTTP error: {} for {}", response.status, url);
        return Err(RepoError::HttpError(response.status));
    }
    if response.truncated {
        return Err(RepoError::ParseError);
    }
    Ok(response.body)
}

/// Simple JSON parser for repository index
/// Expected format:
/// {
///   "name": "repo-name",
///   "url": "https://...",
///   "packages": [
///     {
///       "name": "pkg",
///       "version": "1.0",
///       "arch": "aarch64",
///       "description": "...",
///       "dependencies": ["dep1"],
///       "size": 12345,
///       "checksum": "hex...",
///       "download_url": "https://..."
///     }
///   ]
/// }
fn parse_repository_index(json: &str, name: &str, url: &str) -> Result<RepositoryIndex, RepoError> {
    let mut index = RepositoryIndex::new(name.to_string(), url.to_string());

    // Very simple JSON parser for the specific format we expect
    // Look for "packages": [ ... ]
    let packages_start = json.find("\"packages\"").ok_or(RepoError::ParseError)?;
    let array_start = json[packages_start..]
        .find('[')
        .ok_or(RepoError::ParseError)?
        + packages_start;
    let array_end =
        find_matching_bracket(&json[array_start..]).ok_or(RepoError::ParseError)? + array_start;

    let packages_json = &json[array_start + 1..array_end];

    // Parse each object in the array
    let mut pos = 0;
    while pos < packages_json.len() {
        // Find next {
        let obj_start = packages_json[pos..].find('{');
        if obj_start.is_none() {
            break;
        }
        let obj_start = obj_start.unwrap() + pos;
        let obj_end = find_matching_brace(&packages_json[obj_start..])
            .ok_or(RepoError::ParseError)?
            + obj_start;

        let obj_json = &packages_json[obj_start..obj_end + 1];
        if let Some(pkg) = parse_repo_package(obj_json) {
            index.packages.push(pkg);
        }

        pos = obj_end + 1;
    }

    Ok(index)
}

/// Value of `"key": ...` in a flat JSON object: the text of a string, or the
/// raw token of a number. Arrays give an empty string (see [`string_array`]).
fn json_field(json: &str, key: &str) -> Option<String> {
    let search = format!("\"{}\"", key);
    let start = json.find(&search)?;
    let colon = json[start..].find(':')? + start;
    let value = json[colon + 1..].trim_start();
    if let Some(rest) = value.strip_prefix('"') {
        return Some(rest[..rest.find('"')?].to_string());
    }
    if value.starts_with('[') {
        return Some(String::new());
    }
    let end = value
        .find(|c: char| [',', '}', ']'].contains(&c))
        .unwrap_or(value.len());
    Some(value[..end].trim().to_string())
}

/// Elements of `"key": ["a", "b"]`; empty when the key or array is absent.
fn string_array(json: &str, key: &str) -> Vec<String> {
    let search = format!("\"{}\"", key);
    let Some(start) = json.find(&search) else {
        return Vec::new();
    };
    let Some(open) = json[start..].find('[').map(|i| i + start) else {
        return Vec::new();
    };
    let Some(close) = json[open..].find(']').map(|i| i + open) else {
        return Vec::new();
    };
    json[open + 1..close]
        .split(',')
        .map(|item| item.trim().trim_matches('"'))
        .filter(|item| !item.is_empty())
        .map(String::from)
        .collect()
}

fn parse_hex_checksum(hex: &str) -> Option<[u8; 32]> {
    let mut checksum = [0u8; 32];
    if hex.len() != 64 || !hex.is_ascii() {
        return Some(checksum);
    }
    for (i, byte) in checksum.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(checksum)
}

fn parse_arch(name: &str) -> PackageArch {
    match name {
        "aarch64" => PackageArch::Aarch64,
        "x86_64" => PackageArch::X86_64,
        "armv7" => PackageArch::Armv7,
        "riscv64" => PackageArch::Riscv64,
        _ => PackageArch::Any,
    }
}

/// The optional catalogue fields of an index entry.
fn parse_store_meta(json: &str) -> StoreMeta {
    let text = |key: &str| json_field(json, key).unwrap_or_default();
    StoreMeta {
        details: text("details"),
        category: text("category"),
        icon: text("icon"),
        kind: if text("kind") == "cli" {
            AppKind::Cli
        } else {
            AppKind::Gui
        },
        exec: text("exec"),
        installed_size: text("installed_size").parse().unwrap_or(0),
    }
}

/// Parse a single RepoPackage from JSON object
fn parse_repo_package(json: &str) -> Option<RepoPackage> {
    let text = |key: &str| json_field(json, key);
    Some(RepoPackage {
        name: text("name")?,
        version: text("version")?,
        arch: parse_arch(&text("arch").unwrap_or_else(|| "aarch64".to_string())),
        description: text("description").unwrap_or_default(),
        dependencies: string_array(json, "dependencies"),
        size: text("size").and_then(|s| s.parse().ok()).unwrap_or(0),
        checksum: parse_hex_checksum(&text("checksum").unwrap_or_default())?,
        download_url: text("download_url").unwrap_or_default(),
        store: parse_store_meta(json),
    })
}

/// Find matching closing bracket for array
fn find_matching_bracket(s: &str) -> Option<usize> {
    let mut depth = 0;
    for (i, c) in s.chars().enumerate() {
        match c {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Find matching closing brace for object
fn find_matching_brace(s: &str) -> Option<usize> {
    let mut depth = 0;
    for (i, c) in s.chars().enumerate() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Download a package file
#[cfg(target_arch = "aarch64")]
pub fn download_package(url: &str, buffer: &mut [u8]) -> Result<usize, RepoError> {
    crate::println!("[pkg] Downloading package from {}", url);

    let (status_code, body) = crate::net::fetch_url(url).ok_or(RepoError::NetworkError)?;

    if status_code != 200 {
        crate::println!("[pkg] HTTP error: {}", status_code);
        return Err(RepoError::HttpError(status_code));
    }

    let len = body.len().min(buffer.len());
    buffer[..len].copy_from_slice(&body[..len]);

    Ok(len)
}

/// Download a package file (x86_64 stub)
#[cfg(target_arch = "x86_64")]
pub fn download_package(_url: &str, _buffer: &mut [u8]) -> Result<usize, RepoError> {
    Err(RepoError::NotImplemented)
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    use alloc::string::ToString;
    use alloc::{format, vec};

    #[test]
    fn test_repository_config_new() {
        let config = RepositoryConfig::new("test-repo", "https://example.com/pkg");
        assert_eq!(config.name, "test-repo");
        assert_eq!(config.url, "https://example.com/pkg");
        assert!(config.enabled);
        assert_eq!(config.priority, 0);
        assert!(config.verify_signatures);
        assert!(config.public_key.is_none());
    }

    #[test]
    fn test_repository_config_set_public_key_valid() {
        let mut config = RepositoryConfig::new("test-repo", "https://example.com/pkg");
        let key_hex = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        assert!(config.set_public_key(key_hex).is_ok());
        assert!(config.public_key.is_some());
        assert_eq!(config.public_key_hex(), Some(key_hex.to_string()));
    }

    #[test]
    fn test_repository_config_set_public_key_invalid_length() {
        let mut config = RepositoryConfig::new("test-repo", "https://example.com/pkg");
        let key_hex = "0123456789abcdef"; // Too short
        assert!(config.set_public_key(key_hex).is_err());
    }

    #[test]
    fn test_repository_config_set_public_key_invalid_hex() {
        let mut config = RepositoryConfig::new("test-repo", "https://example.com/pkg");
        let key_hex = "0123456789abcdefghijklmnopqrstuvwxyz0123456789abcdef0123456789abcdef"; // Invalid hex
        assert!(config.set_public_key(key_hex).is_err());
    }

    #[test]
    fn test_repository_manager_new() {
        let manager = RepositoryManager::new();
        assert_eq!(manager.list_repos().len(), 0);
    }

    #[test]
    fn test_repository_manager_add_remove_repo() {
        let mut manager = RepositoryManager::new();
        let config = RepositoryConfig::new("test-repo", "https://example.com/pkg");
        manager.add_repo(config);
        assert_eq!(manager.list_repos().len(), 1);
        assert!(manager.get_repo("test-repo").is_some());

        let removed = manager.remove_repo("test-repo");
        assert!(removed);
        assert_eq!(manager.list_repos().len(), 0);
        assert!(manager.get_repo("test-repo").is_none());

        // Removing non-existent repo returns false
        let removed = manager.remove_repo("nonexistent");
        assert!(!removed);
    }

    #[test]
    fn test_repository_manager_get_repo_mut() {
        let mut manager = RepositoryManager::new();
        let config = RepositoryConfig::new("test-repo", "https://example.com/pkg");
        manager.add_repo(config);

        if let Some(repo) = manager.get_repo_mut("test-repo") {
            repo.enabled = false;
        }

        assert!(!manager.get_repo("test-repo").unwrap().enabled);
    }

    #[test]
    fn test_repository_manager_search_empty() {
        let manager = RepositoryManager::new();
        let results = manager.search("vim");
        assert!(results.is_empty());
    }

    #[test]
    fn test_repository_manager_find_package_empty() {
        let manager = RepositoryManager::new();
        let result = manager.find_package("vim");
        assert!(result.is_none());
    }

    #[test]
    fn test_parse_repository_index_valid() {
        let json = r#"{
            "packages": [
                {
                    "name": "vim",
                    "version": "9.0",
                    "arch": "aarch64",
                    "description": "Vi IMproved text editor",
                    "dependencies": [],
                    "size": 1024,
                    "checksum": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
                    "download_url": "https://example.com/vim.lobpkg"
                }
            ]
        }"#;

        let index = parse_repository_index(json, "test-repo", "https://example.com/pkg").unwrap();
        assert_eq!(index.name, "test-repo");
        assert_eq!(index.url, "https://example.com/pkg");
        assert_eq!(index.packages.len(), 1);
        assert_eq!(index.packages[0].name, "vim");
        assert_eq!(index.packages[0].version, "9.0");
        assert_eq!(index.packages[0].arch, PackageArch::Aarch64);
    }

    #[test]
    fn test_parse_repository_index_multiple_packages() {
        let json = r#"{
            "packages": [
                {
                    "name": "vim",
                    "version": "9.0",
                    "arch": "aarch64",
                    "description": "Editor",
                    "dependencies": [],
                    "size": 1024,
                    "checksum": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
                    "download_url": "https://example.com/vim.lobpkg"
                },
                {
                    "name": "coreutils",
                    "version": "9.4",
                    "arch": "aarch64",
                    "description": "Core utilities",
                    "dependencies": [],
                    "size": 2048,
                    "checksum": "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210",
                    "download_url": "https://example.com/coreutils.lobpkg"
                }
            ]
        }"#;

        let index = parse_repository_index(json, "test-repo", "https://example.com/pkg").unwrap();
        assert_eq!(index.packages.len(), 2);
    }

    #[test]
    fn test_parse_repository_index_missing_packages_field() {
        let json = r#"{"name": "test", "url": "https://example.com"}"#;
        let result = parse_repository_index(json, "test-repo", "https://example.com/pkg");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_repo_package_valid() {
        let json = r#"{
            "name": "vim",
            "version": "9.0",
            "arch": "aarch64",
            "description": "Vi IMproved",
            "dependencies": [],
            "size": 1024,
            "checksum": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "download_url": "https://example.com/vim.lobpkg"
        }"#;

        let pkg = parse_repo_package(json).unwrap();
        assert_eq!(pkg.name, "vim");
        assert_eq!(pkg.version, "9.0");
        assert_eq!(pkg.arch, PackageArch::Aarch64);
        assert_eq!(pkg.size, 1024);
    }

    #[test]
    fn test_parse_repo_package_with_dependencies() {
        let json = r#"{
            "name": "vim",
            "version": "9.0",
            "arch": "aarch64",
            "description": "Editor",
            "dependencies": ["libc", "ncurses"],
            "size": 1024,
            "checksum": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "download_url": "https://example.com/vim.lobpkg"
        }"#;

        let pkg = parse_repo_package(json).unwrap();
        // Note: current parser returns empty for arrays, so dependencies will be empty
        // This test documents current behavior
        assert_eq!(pkg.name, "vim");
    }

    #[test]
    fn test_parse_repo_package_missing_name() {
        let json = r#"{"version": "9.0"}"#;
        let pkg = parse_repo_package(json);
        assert!(pkg.is_none());
    }

    #[test]
    fn test_find_matching_bracket() {
        assert_eq!(find_matching_bracket("[]"), Some(1));
        assert_eq!(find_matching_bracket("[[]]"), Some(3));
        assert_eq!(find_matching_bracket("[{}, []]"), Some(7));
        assert_eq!(find_matching_bracket("["), None);
    }

    #[test]
    fn test_find_matching_brace() {
        assert_eq!(find_matching_brace("{}"), Some(1));
        assert_eq!(find_matching_brace("{{}}"), Some(3));
        assert_eq!(find_matching_brace("{[]}"), Some(3));
        assert_eq!(find_matching_brace("{"), None);
    }

    #[test]
    fn test_verify_checksum() {
        use sha2::{Digest, Sha256};
        let data = b"test data";
        let mut hasher = Sha256::new();
        hasher.update(data);
        let checksum: [u8; 32] = hasher.finalize().into();
        assert!(verify_checksum(data, &checksum));

        let wrong_checksum = [0u8; 32];
        assert!(!verify_checksum(data, &wrong_checksum));
    }

    #[test]
    fn test_repo_error_display() {
        assert_eq!(RepoError::NotFound.to_string(), "Repository not found");
        assert_eq!(RepoError::NetworkError.to_string(), "Network error");
        assert_eq!(RepoError::ParseError.to_string(), "Parse error");
        assert_eq!(
            RepoError::SignatureError.to_string(),
            "Signature verification failed"
        );
        assert_eq!(RepoError::IoError.to_string(), "I/O error");
        assert_eq!(RepoError::HttpError(404).to_string(), "HTTP error: 404");
        assert_eq!(RepoError::NotImplemented.to_string(), "Not implemented");
    }

    const FIXTURE_INDEX: &[u8] = include_bytes!("../../tests/fixtures/pkgrepo/index.json");
    const FIXTURE_SIG: &[u8] = include_bytes!("../../tests/fixtures/pkgrepo/index.json.sig");
    const FIXTURE_KEY: &str = include_str!("../../tests/fixtures/pkgrepo/public-key.hex");

    fn fixture_manager(verify: bool, key: Option<&str>) -> RepositoryManager {
        let mut repo = RepositoryConfig::new("lobster-main", "http://127.0.0.1:8000");
        repo.verify_signatures = verify;
        if let Some(key) = key {
            repo.set_public_key(key.trim()).expect("fixture key");
        }
        let mut manager = RepositoryManager::new();
        manager.add_repo(repo);
        manager
    }

    #[test]
    fn signed_index_from_pkgrepo_is_accepted_and_parsed() {
        let mut manager = fixture_manager(true, Some(FIXTURE_KEY));
        manager
            .ingest_index("lobster-main", FIXTURE_INDEX, Some(FIXTURE_SIG), 7)
            .expect("good signature");
        let index = manager.get_index("lobster-main").expect("cached");
        assert_eq!(index.packages.len(), 2);
        assert_eq!(index.timestamp, 7);
        let beta = index.find_package("beta").expect("beta");
        assert_eq!(beta.dependencies, vec!["alpha".to_string()]);
        assert_eq!(beta.store.kind, AppKind::Cli);
        assert_eq!(beta.store.exec, "/apps/beta");
        assert_eq!(beta.store.installed_size, beta.size * 2);
        let alpha = index.find_package("alpha").expect("alpha");
        assert_eq!(alpha.version, "1.10.0");
        assert_eq!(alpha.store.category, "Games");
        assert_eq!(alpha.store.icon, "AppGeneric");
        assert_eq!(alpha.store.kind, AppKind::Gui);
        assert!(alpha.dependencies.is_empty());
    }

    #[test]
    fn flipped_index_byte_is_rejected_and_not_cached() {
        let mut manager = fixture_manager(true, Some(FIXTURE_KEY));
        let mut tampered = FIXTURE_INDEX.to_vec();
        tampered[20] ^= 0x01;
        let result = manager.ingest_index("lobster-main", &tampered, Some(FIXTURE_SIG), 1);
        assert_eq!(result, Err(RepoError::SignatureError));
        assert!(manager.get_index("lobster-main").is_none());
    }

    #[test]
    fn missing_signature_is_rejected_when_verification_is_on() {
        let mut manager = fixture_manager(true, Some(FIXTURE_KEY));
        let result = manager.ingest_index("lobster-main", FIXTURE_INDEX, None, 1);
        assert_eq!(result, Err(RepoError::SignatureError));
    }

    #[test]
    fn verification_without_a_key_is_rejected() {
        let mut manager = fixture_manager(true, None);
        let result = manager.ingest_index("lobster-main", FIXTURE_INDEX, Some(FIXTURE_SIG), 1);
        assert_eq!(result, Err(RepoError::SignatureError));
    }

    #[test]
    fn signature_by_another_key_is_rejected() {
        let other = "fe09e6d0b0b94b7dd72b5ceaf358fb27c28e7981b4193f833602e888e2bd57bc";
        let mut manager = fixture_manager(true, Some(other));
        let result = manager.ingest_index("lobster-main", FIXTURE_INDEX, Some(FIXTURE_SIG), 1);
        assert_eq!(result, Err(RepoError::SignatureError));
    }

    #[test]
    fn malformed_signature_text_is_rejected() {
        let mut manager = fixture_manager(true, Some(FIXTURE_KEY));
        for bad in [&b"abcd"[..], &b""[..], &[0xFF; 128][..], &[b'z'; 128][..]] {
            let result = manager.ingest_index("lobster-main", FIXTURE_INDEX, Some(bad), 1);
            assert_eq!(result, Err(RepoError::SignatureError));
        }
    }

    #[test]
    fn unsigned_index_is_accepted_when_verification_is_off() {
        let mut manager = fixture_manager(false, None);
        manager
            .ingest_index("lobster-main", FIXTURE_INDEX, None, 1)
            .expect("verify off");
        assert!(manager.has_indices());
    }

    #[test]
    fn default_repo_is_added_to_an_empty_config_with_the_key() {
        let mut manager = RepositoryManager::new();
        assert!(manager.ensure_default_repo());
        let repo = manager.get_repo(DEFAULT_REPO_NAME).expect("default repo");
        assert_eq!(repo.url, DEFAULT_REPO_URL);
        assert!(repo.verify_signatures);
        assert_eq!(repo.public_key_hex().as_deref(), Some(DEFAULT_REPO_KEY_HEX));
    }

    #[test]
    fn old_default_repo_without_a_key_gets_the_key() {
        let mut manager = RepositoryManager::new();
        manager.add_repo(RepositoryConfig::new(DEFAULT_REPO_NAME, DEFAULT_REPO_URL));
        assert!(manager.ensure_default_repo());
        let repo = manager.get_repo(DEFAULT_REPO_NAME).expect("repo");
        assert_eq!(repo.public_key_hex().as_deref(), Some(DEFAULT_REPO_KEY_HEX));
        assert!(
            !manager.ensure_default_repo(),
            "second call changes nothing"
        );
    }

    #[test]
    fn custom_repositories_are_left_alone() {
        let mut manager = RepositoryManager::new();
        manager.add_repo(RepositoryConfig::new(
            DEFAULT_REPO_NAME,
            "http://127.0.0.1:8000",
        ));
        assert!(!manager.ensure_default_repo());
        assert!(manager
            .get_repo(DEFAULT_REPO_NAME)
            .expect("repo")
            .public_key
            .is_none());
    }

    #[test]
    fn dependencies_array_parses_to_names() {
        let json = r#"{"name":"x","version":"1","dependencies": ["a", "b"],"size":3}"#;
        let pkg = parse_repo_package(json).expect("parses");
        assert_eq!(pkg.dependencies, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(pkg.size, 3);
    }
}
