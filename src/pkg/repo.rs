//! Repository handling for LobsterOS package manager
//!
//! Features:
//! - Repository configuration and management
//! - HTTP index download with JSON parsing
//! - Ed25519 signature verification for repository authenticity

use crate::pkg::package::{PackageArch, RepoPackage, RepositoryIndex};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};

/// Maximum public key size (Ed25519 = 32 bytes)
const ED25519_PUBKEY_LEN: usize = 32;
/// Maximum signature size (Ed25519 = 64 bytes)
const ED25519_SIG_LEN: usize = 64;

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
        use crate::fs::vfs::{create, create_dir, open, write};

        // Ensure config directory exists
        let _ = create_dir("/etc/pkg");

        // Create or open the config file
        let _ = create("/etc/pkg/repos.conf");
        let fd = match open("/etc/pkg/repos.conf", 1) {
            Ok(fd) => fd,
            Err(_) => return Err(RepoError::IoError),
        };

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

        // Write to file
        write(fd, content.as_bytes());
        Ok(())
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

        // HTTP GET the index file from repo.url/index.json
        let index_url = format!("{}/index.json", repo.url);
        crate::println!("[pkg] Updating repository index from {}", index_url);

        let response = crate::net::http::http_get(&index_url).ok_or(RepoError::NetworkError)?;

        if response.status_code != 200 {
            crate::println!("[pkg] HTTP error: {}", response.status_code);
            return Err(RepoError::HttpError(response.status_code));
        }

        let body = &response.body[..response.body_len];

        // Verify signature if public key is configured
        if repo.verify_signatures {
            if let Some(pubkey) = repo.public_key {
                let json_str = core::str::from_utf8(body).map_err(|_| RepoError::ParseError)?;
                verify_repository_signature(json_str, &pubkey)?;
                crate::println!("[pkg] Repository signature verified");
            } else {
                crate::println!(
                    "[pkg] WARNING: verify_signatures=true but no public key configured"
                );
            }
        }

        // Parse JSON index
        let json_data = core::str::from_utf8(body).map_err(|_| RepoError::ParseError)?;

        // Parse JSON - simple manual parsing for no_std
        let mut index =
            parse_repository_index(json_data, &repo.name.to_string(), &repo.url.to_string())?;

        index.timestamp = crate::arch::timer::Timer::uptime_ms();

        // Replace or add index
        if let Some(pos) = self.indices.iter().position(|i| i.name == name) {
            self.indices[pos] = index;
        } else {
            self.indices.push(index);
        }

        Ok(())
    }

    /// Update repository index (x86_64 stub)
    #[cfg(target_arch = "x86_64")]
    pub fn update_index(&mut self, _name: &str) -> Result<(), RepoError> {
        Err(RepoError::NotImplemented)
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

/// Verify Ed25519 signature of repository index JSON
/// {
///   "name": "repo-name",
///   "url": "https://...",
///   "packages": [...],
///   "signature": "hex-encoded-64-byte-signature"
/// }
/// The signature is computed over the canonical JSON without the signature field.
fn verify_repository_signature(
    json: &str,
    pubkey: &[u8; ED25519_PUBKEY_LEN],
) -> Result<(), RepoError> {
    // Extract signature field from JSON
    let sig_field = "\"signature\"";
    let sig_start = json.find(sig_field).ok_or(RepoError::SignatureError)?;
    let colon_pos = json[sig_start..]
        .find(':')
        .ok_or(RepoError::SignatureError)?
        + sig_start;
    let value_start = json[colon_pos + 1..].trim_start();

    if !value_start.starts_with('"') {
        return Err(RepoError::SignatureError);
    }

    let sig_end = value_start[1..]
        .find('"')
        .ok_or(RepoError::SignatureError)?
        + 1;
    let sig_hex = &value_start[1..sig_end];

    if sig_hex.len() != 128 {
        return Err(RepoError::SignatureError); // 64 bytes = 128 hex chars
    }

    // Parse signature hex
    let mut sig_bytes = [0u8; ED25519_SIG_LEN];
    for i in 0..ED25519_SIG_LEN {
        let byte_str = &sig_hex[i * 2..i * 2 + 2];
        sig_bytes[i] = u8::from_str_radix(byte_str, 16).map_err(|_| RepoError::SignatureError)?;
    }

    // Reconstruct the signed data (JSON without signature field)
    // Find the signature field position in the original JSON and remove it
    let signed_data = remove_signature_field(json).ok_or(RepoError::SignatureError)?;

    // Verify signature
    let verifying_key = VerifyingKey::from_bytes(pubkey).map_err(|_| RepoError::SignatureError)?;
    let signature = Signature::from_bytes(&sig_bytes);
    verifying_key
        .verify(signed_data.as_bytes(), &signature)
        .map_err(|_| RepoError::SignatureError)?;

    Ok(())
}

/// Remove the "signature" field from JSON for verification
/// Returns the JSON string without the signature field
fn remove_signature_field(json: &str) -> Option<String> {
    let sig_field = "\"signature\"";
    let sig_start = json.find(sig_field)?;
    // Find the start of the field (including potential whitespace/comma before)
    let field_start = sig_start;
    // Find the end of the value (after the closing quote of the signature value)
    let colon_pos = json[field_start..].find(':')? + field_start;
    let value_start = json[colon_pos + 1..].trim_start();
    if !value_start.starts_with('"') {
        return None;
    }
    let sig_end_rel = value_start[1..].find('"')? + 1;
    let _value_end =
        colon_pos + 1 + (value_start.as_ptr() as usize - json.as_ptr() as usize) + sig_end_rel;
    // Actually, let's do this more carefully with byte offsets
    let value_part = &json[colon_pos + 1..];
    let value_trimmed = value_part.trim_start();
    if !value_trimmed.starts_with('"') {
        return None;
    }
    let quote_end = value_trimmed[1..].find('"')? + 1;
    let field_end = colon_pos
        + 1
        + (value_trimmed.as_ptr() as usize - value_part.as_ptr() as usize)
        + quote_end
        + 1;

    // Now we need to find the comma before or after
    let mut result = String::with_capacity(json.len());
    result.push_str(&json[..field_start]);
    // Skip the field, check for trailing comma
    let after_field = &json[field_end..].trim_start();
    let after_field = if after_field.starts_with(',') {
        &after_field[1..]
    } else {
        after_field
    };
    result.push_str(after_field);
    Some(result)
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

/// Parse a single RepoPackage from JSON object
fn parse_repo_package(json: &str) -> Option<RepoPackage> {
    // Simple JSON field extractor for no_std
    let get_field = |json: &str, field: &str| -> Option<String> {
        let search = alloc::format!("\"{}\"", field);
        let start = json.find(&search)?;
        let colon = json[start..].find(':')? + start;
        let value_start = colon + 1;
        let value_part = &json[value_start..].trim_start();

        if value_part.starts_with('"') {
            // String value
            let end = value_part[1..].find('"')? + 1;
            Some(value_part[1..end].to_string())
        } else if value_part.starts_with('[') {
            // Array - for dependencies, return empty for now
            Some(String::new())
        } else {
            // Number or boolean
            let end = value_part
                .find(|c: char| [',', '}', ']'].contains(&c))
                .unwrap_or(value_part.len());
            Some(value_part[..end].trim().to_string())
        }
    };

    let name = get_field(json, "name")?;
    let version = get_field(json, "version")?;
    let arch_str = get_field(json, "arch").unwrap_or_else(|| "aarch64".to_string());
    let arch = match arch_str.as_str() {
        "aarch64" => PackageArch::Aarch64,
        "x86_64" => PackageArch::X86_64,
        "armv7" => PackageArch::Armv7,
        "riscv64" => PackageArch::Riscv64,
        _ => PackageArch::Any,
    };
    let description = get_field(json, "description").unwrap_or_default();
    let dependencies = get_field(json, "dependencies")
        .map(|s| {
            s.split(',')
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect()
        })
        .unwrap_or_default();
    let size = get_field(json, "size")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    // Parse checksum hex string
    let checksum_str = get_field(json, "checksum").unwrap_or_default();
    let mut checksum = [0u8; 32];
    if checksum_str.len() == 64 {
        for i in 0..32 {
            let byte_str = &checksum_str[i * 2..i * 2 + 2];
            checksum[i] = u8::from_str_radix(byte_str, 16).ok()?;
        }
    }

    let download_url = get_field(json, "download_url").unwrap_or_default();

    Some(RepoPackage {
        name,
        version,
        arch,
        description,
        dependencies,
        size,
        checksum,
        download_url,
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

    let response = crate::net::http::http_get(url).ok_or(RepoError::NetworkError)?;

    if response.status_code != 200 {
        crate::println!("[pkg] HTTP error: {}", response.status_code);
        return Err(RepoError::HttpError(response.status_code));
    }

    let data = &response.body[..response.body_len];
    let len = data.len().min(buffer.len());
    buffer[..len].copy_from_slice(&data[..len]);

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
}
