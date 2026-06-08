//! Package format definitions for LobsterOS

use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// Magic bytes for .lobpkg file format
pub const LOBPKG_MAGIC: &[u8; 4] = b"LOB\x00";

/// Package format version
pub const LOBPKG_VERSION: u16 = 1;

/// Maximum package name length
pub const MAX_PKG_NAME: usize = 64;

/// Maximum package version length
pub const MAX_PKG_VERSION: usize = 32;

/// Package architecture
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageArch {
    Aarch64,
    Armv7,
    X86_64,
    All,
}

impl PackageArch {
    pub fn as_str(&self) -> &'static str {
        match self {
            PackageArch::Aarch64 => "aarch64",
            PackageArch::Armv7 => "armv7",
            PackageArch::X86_64 => "x86_64",
            PackageArch::All => "all",
        }
    }
}

/// Package metadata
#[derive(Debug, Clone)]
pub struct PackageMetadata {
    pub name: String,
    pub version: String,
    pub arch: PackageArch,
    pub description: String,
    pub maintainer: String,
    pub license: String,
    pub homepage: String,
    pub dependencies: Vec<String>,
    pub provides: Vec<String>,
    pub conflicts: Vec<String>,
    pub replaces: Vec<String>,
    pub size: u64,          // Installed size in bytes
    pub download_size: u64, // Download size in bytes
    pub checksum: [u8; 32], // SHA256 of package contents
    pub build_timestamp: u64,
    pub install_script: Option<String>,
    pub remove_script: Option<String>,
    pub config_files: Vec<String>,
}

/// Package file entry
#[derive(Debug, Clone)]
pub struct PackageFile {
    pub path: String,
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
    pub size: u64,
    pub checksum: [u8; 32],
    pub is_config: bool,
    pub is_symlink: bool,
    pub symlink_target: Option<String>,
}

/// Complete package structure
#[derive(Debug, Clone)]
pub struct Package {
    pub metadata: PackageMetadata,
    pub files: Vec<PackageFile>,
    pub file_data: Vec<u8>, // Concatenated file contents
}

impl PackageMetadata {
    /// Create a new package metadata with defaults
    pub fn new(name: &str, version: &str) -> Self {
        Self {
            name: name.to_string(),
            version: version.to_string(),
            arch: PackageArch::Aarch64,
            description: String::new(),
            maintainer: String::new(),
            license: String::new(),
            homepage: String::new(),
            dependencies: Vec::new(),
            provides: Vec::new(),
            conflicts: Vec::new(),
            replaces: Vec::new(),
            size: 0,
            download_size: 0,
            checksum: [0; 32],
            build_timestamp: 0,
            install_script: None,
            remove_script: None,
            config_files: Vec::new(),
        }
    }
}

impl Package {
    /// Create a new empty package
    pub fn new(name: &str, version: &str) -> Self {
        Self {
            metadata: PackageMetadata::new(name, version),
            files: Vec::new(),
            file_data: Vec::new(),
        }
    }

    /// Calculate SHA256 checksum of package contents
    pub fn calculate_checksum(&self) -> [u8; 32] {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();

        // Hash metadata fields
        hasher.update(self.metadata.name.as_bytes());
        hasher.update(b"\0");
        hasher.update(self.metadata.version.as_bytes());
        hasher.update(b"\0");
        hasher.update(self.metadata.description.as_bytes());
        hasher.update(b"\0");

        // Hash dependencies
        for dep in &self.metadata.dependencies {
            hasher.update(dep.as_bytes());
            hasher.update(b",");
        }

        // Hash file entries
        for file in &self.files {
            hasher.update(file.path.as_bytes());
            hasher.update(b"\0");
            hasher.update(file.mode.to_le_bytes());
            hasher.update(file.size.to_le_bytes());
            hasher.update(file.checksum);
        }

        // Hash file data
        hasher.update(&self.file_data);

        let result = hasher.finalize();
        let mut checksum = [0u8; 32];
        checksum.copy_from_slice(&result);
        checksum
    }

    /// Verify package integrity
    pub fn verify(&self) -> bool {
        let calc = self.calculate_checksum();
        calc == self.metadata.checksum
    }

    /// Get total installed size
    pub fn installed_size(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }
}

/// Repository index entry
#[derive(Debug, Clone)]
pub struct RepoPackage {
    pub name: String,
    pub version: String,
    pub arch: PackageArch,
    pub description: String,
    pub dependencies: Vec<String>,
    pub size: u64,
    pub checksum: [u8; 32],
    pub download_url: String,
}

/// Repository index
#[derive(Debug, Clone)]
pub struct RepositoryIndex {
    pub name: String,
    pub url: String,
    pub packages: Vec<RepoPackage>,
    pub timestamp: u64,
    pub public_key: [u8; 32],
}

impl RepositoryIndex {
    pub fn new(name: &str, url: &str) -> Self {
        Self {
            name: name.to_string(),
            url: url.to_string(),
            packages: Vec::new(),
            timestamp: 0,
            public_key: [0; 32],
        }
    }

    pub fn find_package(&self, name: &str) -> Option<&RepoPackage> {
        self.packages.iter().find(|p| p.name == name)
    }

    pub fn search(&self, query: &str) -> Vec<&RepoPackage> {
        let query = query.to_lowercase();
        self.packages
            .iter()
            .filter(|p| {
                p.name.to_lowercase().contains(&query)
                    || p.description.to_lowercase().contains(&query)
            })
            .collect()
    }
}

/// Installed package database entry
#[derive(Debug, Clone)]
pub struct InstalledPackage {
    pub metadata: PackageMetadata,
    pub files: Vec<PackageFile>,
    pub install_time: u64,
}

impl InstalledPackage {
    pub fn from_package(pkg: Package, install_time: u64) -> Self {
        Self {
            metadata: pkg.metadata,
            files: pkg.files,
            install_time,
        }
    }
}
