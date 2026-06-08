//! Package metadata and structures for LobsterOS package manager

use alloc::string::String;
use alloc::vec::Vec;

/// Package architecture
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageArch {
    Aarch64,
    X86_64,
    Armv7,
    Riscv64,
    Any,
}

impl core::fmt::Display for PackageArch {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            PackageArch::Aarch64 => write!(f, "aarch64"),
            PackageArch::X86_64 => write!(f, "x86_64"),
            PackageArch::Armv7 => write!(f, "armv7"),
            PackageArch::Riscv64 => write!(f, "riscv64"),
            PackageArch::Any => write!(f, "any"),
        }
    }
}

impl PackageArch {
    pub fn from_str(s: &str) -> Self {
        match s {
            "aarch64" => PackageArch::Aarch64,
            "x86_64" => PackageArch::X86_64,
            "armv7" => PackageArch::Armv7,
            "riscv64" => PackageArch::Riscv64,
            _ => PackageArch::Any,
        }
    }
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
    pub data: Vec<u8>, // File contents (for small files)
    pub is_config: bool,
    pub is_symlink: bool,
    pub symlink_target: Option<String>,
}

/// Package magic bytes and version
pub const LOBPKG_MAGIC: &[u8; 4] = b"LOB\0";
pub const LOBPKG_VERSION: u16 = 1;
pub const MAX_PKG_NAME: usize = 64;
pub const MAX_PKG_VERSION: usize = 32;

/// Package metadata from repository index
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
    pub size: u64,
    pub download_size: u64,
    pub checksum: [u8; 32], // SHA-256
    pub build_timestamp: u64,
    pub install_script: Option<String>,
    pub remove_script: Option<String>,
    pub config_files: Vec<String>,
    pub download_url: String,
}

/// Installed package information (stored in local database)
#[derive(Debug, Clone)]
pub struct InstalledPackage {
    pub metadata: PackageMetadata,
    pub install_time: u64,       // Unix timestamp
    pub explicit: bool,          // true = user requested, false = dependency
    pub files: Vec<PackageFile>, // Installed file list for removal
}

/// Repository index entry
#[derive(Debug, Clone)]
pub struct RepositoryIndex {
    pub name: String,
    pub url: String,
    pub enabled: bool,
    pub packages: Vec<RepoPackage>,
    pub timestamp: u64, // Last update time
}

/// Package from repository index (lightweight, no file data)
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

/// Full package (with file data) — parsed from .lobpkg file
#[derive(Debug, Clone)]
pub struct Package {
    pub metadata: PackageMetadata,
    pub files: Vec<PackageFile>,
    pub file_data: Vec<u8>, // All file contents concatenated
}

impl Package {
    /// Verify package integrity (checksum of file_data)
    pub fn verify(&self) -> bool {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(&self.file_data);
        let result = hasher.finalize();
        result.as_slice() == self.metadata.checksum
    }
}

impl PackageMetadata {
    pub fn new(
        name: String,
        version: String,
        arch: PackageArch,
        description: String,
        dependencies: Vec<String>,
        size: u64,
        checksum: [u8; 32],
        download_url: String,
    ) -> Self {
        Self {
            name,
            version,
            arch,
            description,
            maintainer: String::new(),
            license: String::new(),
            homepage: String::new(),
            dependencies,
            provides: Vec::new(),
            conflicts: Vec::new(),
            replaces: Vec::new(),
            size,
            download_size: 0,
            checksum,
            build_timestamp: 0,
            install_script: None,
            remove_script: None,
            config_files: Vec::new(),
            download_url,
        }
    }
}

impl InstalledPackage {
    pub fn new(metadata: PackageMetadata, install_time: u64, explicit: bool) -> Self {
        Self {
            metadata,
            install_time,
            explicit,
            files: Vec::new(),
        }
    }

    /// Create from a full Package (after installation)
    pub fn from_package(pkg: Package, install_time: u64) -> Self {
        Self {
            metadata: pkg.metadata,
            install_time,
            explicit: true,
            files: pkg.files,
        }
    }

    /// Serialize to database format: name|version|arch|install_time|explicit|description|dependencies|size|checksum|download_url|files
    pub fn to_db_string(&self) -> String {
        let deps = self.metadata.dependencies.join(",");
        let checksum_hex = self
            .metadata
            .checksum
            .iter()
            .map(|b| alloc::format!("{:02x}", b))
            .collect::<String>();

        // Serialize files as comma-separated paths
        let files_str = self
            .files
            .iter()
            .map(|f| f.path.as_str())
            .collect::<alloc::vec::Vec<&str>>()
            .join(",");

        alloc::format!(
            "{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
            self.metadata.name,
            self.metadata.version,
            self.metadata.arch,
            self.install_time,
            if self.explicit { 1 } else { 0 },
            self.metadata.description,
            deps,
            self.metadata.size,
            checksum_hex,
            self.metadata.download_url,
            files_str
        )
    }

    /// Parse from database format
    pub fn from_db_string(s: &str) -> Option<Self> {
        let parts: Vec<&str> = s.split('|').collect();
        if parts.len() < 10 {
            return None;
        }

        let explicit = parts[4] == "1";
        let install_time = parts[3].parse().ok()?;
        let size = parts[7].parse().ok()?;

        // Parse checksum hex
        let checksum_str = parts[8];
        let mut checksum = [0u8; 32];
        if checksum_str.len() == 64 {
            for i in 0..32 {
                let byte_str = &checksum_str[i * 2..i * 2 + 2];
                checksum[i] = u8::from_str_radix(byte_str, 16).ok()?;
            }
        } else {
            return None;
        }

        // Parse dependencies
        let deps = if parts[6].is_empty() {
            Vec::new()
        } else {
            parts[6].split(',').map(String::from).collect()
        };

        // Parse files (field 10, 0-indexed = 10)
        let files_str = parts.get(10).unwrap_or(&"");
        let mut files = Vec::new();
        if !files_str.is_empty() {
            for path in files_str.split(',') {
                if !path.is_empty() {
                    files.push(PackageFile {
                        path: String::from(path),
                        mode: 0o644,
                        uid: 0,
                        gid: 0,
                        size: 0,
                        checksum: [0; 32],
                        data: Vec::new(),
                        is_config: false,
                        is_symlink: false,
                        symlink_target: None,
                    });
                }
            }
        }

        Some(Self {
            metadata: PackageMetadata {
                name: String::from(parts[0]),
                version: String::from(parts[1]),
                arch: PackageArch::from_str(parts[2]),
                description: String::from(parts[5]),
                maintainer: String::new(),
                license: String::new(),
                homepage: String::new(),
                dependencies: deps,
                provides: Vec::new(),
                conflicts: Vec::new(),
                replaces: Vec::new(),
                size,
                download_size: 0,
                checksum,
                build_timestamp: 0,
                install_script: None,
                remove_script: None,
                config_files: Vec::new(),
                download_url: String::from(parts[9]),
            },
            install_time,
            explicit,
            files,
        })
    }
}

impl RepositoryIndex {
    pub fn new(name: String, url: String) -> Self {
        Self {
            name,
            url,
            enabled: true,
            packages: Vec::new(),
            timestamp: 0,
        }
    }

    pub fn add_package(&mut self, pkg: RepoPackage) {
        self.packages.push(pkg);
    }

    pub fn find_package(&self, name: &str) -> Option<&RepoPackage> {
        self.packages.iter().find(|p| p.name == name)
    }

    pub fn search(&self, query: &str) -> Vec<&RepoPackage> {
        self.packages
            .iter()
            .filter(|p| p.name.contains(query) || p.description.contains(query))
            .collect()
    }
}
