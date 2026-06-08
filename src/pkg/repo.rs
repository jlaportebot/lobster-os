//! Repository handling for LobsterOS package manager

use crate::pkg::package::{PackageArch, RepoPackage, RepositoryIndex};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// Repository configuration
#[derive(Debug, Clone)]
pub struct RepositoryConfig {
    pub name: String,
    pub url: String,
    pub enabled: bool,
    pub priority: i32,
    pub verify_signatures: bool,
}

impl RepositoryConfig {
    pub fn new(name: &str, url: &str) -> Self {
        Self {
            name: name.to_string(),
            url: url.to_string(),
            enabled: true,
            priority: 0,
            verify_signatures: true,
        }
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

    /// Update repository index (fetch from remote)
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

        // Parse JSON index
        let json_data = core::str::from_utf8(&response.body[..response.body_len])
            .map_err(|_| RepoError::ParseError)?;

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
pub fn verify_checksum(data: &[u8], expected: &[u8; 32]) -> bool {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(data);
    let result = hasher.finalize();
    result.as_slice() == expected
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
