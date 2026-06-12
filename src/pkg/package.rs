//! Package metadata and structures for LobsterOS package manager

use alloc::string::String;
use alloc::vec::Vec;
use core::str::FromStr;

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

impl core::str::FromStr for PackageArch {
    type Err = &'static str;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "aarch64" => Ok(PackageArch::Aarch64),
            "x86_64" => Ok(PackageArch::X86_64),
            "armv7" => Ok(PackageArch::Armv7),
            "riscv64" => Ok(PackageArch::Riscv64),
            _ => Ok(PackageArch::Any),
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
                arch: PackageArch::from_str(parts[2]).unwrap_or(PackageArch::Any),
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

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;
    use alloc::{format, vec};

    #[test]
    fn test_package_arch_from_str() {
        assert_eq!(PackageArch::from_str("aarch64"), PackageArch::Aarch64);
        assert_eq!(PackageArch::from_str("x86_64"), PackageArch::X86_64);
        assert_eq!(PackageArch::from_str("armv7"), PackageArch::Armv7);
        assert_eq!(PackageArch::from_str("riscv64"), PackageArch::Riscv64);
        assert_eq!(PackageArch::from_str("unknown"), PackageArch::Any);
    }

    #[test]
    fn test_package_arch_display() {
        assert_eq!(PackageArch::Aarch64.to_string(), "aarch64");
        assert_eq!(PackageArch::X86_64.to_string(), "x86_64");
        assert_eq!(PackageArch::Armv7.to_string(), "armv7");
        assert_eq!(PackageArch::Riscv64.to_string(), "riscv64");
        assert_eq!(PackageArch::Any.to_string(), "any");
    }

    #[test]
    fn test_package_metadata_new() {
        let deps = vec!["dep1".to_string(), "dep2".to_string()];
        let checksum = [0u8; 32];
        let meta = PackageMetadata::new(
            "test-pkg".to_string(),
            "1.0.0".to_string(),
            PackageArch::Aarch64,
            "Test package".to_string(),
            deps.clone(),
            1024,
            checksum,
            "https://example.com/pkg.lobpkg".to_string(),
        );
        assert_eq!(meta.name, "test-pkg");
        assert_eq!(meta.version, "1.0.0");
        assert_eq!(meta.arch, PackageArch::Aarch64);
        assert_eq!(meta.description, "Test package");
        assert_eq!(meta.dependencies, deps);
        assert_eq!(meta.size, 1024);
        assert_eq!(meta.checksum, checksum);
    }

    #[test]
    fn test_installed_package_to_from_db_string() {
        let mut checksum = [0u8; 32];
        checksum[0] = 0xde;
        checksum[1] = 0xad;
        checksum[2] = 0xbe;
        checksum[3] = 0xef;

        let meta = PackageMetadata::new(
            "test-pkg".to_string(),
            "1.0.0".to_string(),
            PackageArch::Aarch64,
            "Test package".to_string(),
            vec!["dep1".to_string()],
            1024,
            checksum,
            "https://example.com/pkg.lobpkg".to_string(),
        );

        let pkg = Package {
            metadata: meta.clone(),
            files: vec![
                PackageFile {
                    path: "/usr/bin/test".to_string(),
                    mode: 0o755,
                    uid: 0,
                    gid: 0,
                    size: 100,
                    checksum: [0; 32],
                    data: Vec::new(),
                    is_config: false,
                    is_symlink: false,
                    symlink_target: None,
                },
                PackageFile {
                    path: "/etc/test.conf".to_string(),
                    mode: 0o644,
                    uid: 0,
                    gid: 0,
                    size: 50,
                    checksum: [0; 32],
                    data: Vec::new(),
                    is_config: true,
                    is_symlink: false,
                    symlink_target: None,
                },
            ],
            file_data: Vec::new(),
        };

        let installed = InstalledPackage::from_package(pkg, 1234567890);
        let db_string = installed.to_db_string();

        // Parse it back
        let parsed =
            InstalledPackage::from_db_string(&db_string).expect("Failed to parse db string");

        assert_eq!(parsed.metadata.name, "test-pkg");
        assert_eq!(parsed.metadata.version, "1.0.0");
        assert_eq!(parsed.metadata.arch, PackageArch::Aarch64);
        assert_eq!(parsed.metadata.description, "Test package");
        assert_eq!(parsed.metadata.dependencies.len(), 1);
        assert_eq!(parsed.metadata.dependencies[0], "dep1");
        assert_eq!(parsed.install_time, 1234567890);
        assert!(parsed.explicit);
        assert_eq!(parsed.files.len(), 2);
        assert_eq!(parsed.files[0].path, "/usr/bin/test");
        assert_eq!(parsed.files[1].path, "/etc/test.conf");
        assert!(parsed.files[1].is_config);
    }

    #[test]
    fn test_repository_index_search() {
        let mut index =
            RepositoryIndex::new("test-repo".to_string(), "https://example.com".to_string());
        index.add_package(RepoPackage {
            name: "vim".to_string(),
            version: "9.0".to_string(),
            arch: PackageArch::Aarch64,
            description: "Vi IMproved text editor".to_string(),
            dependencies: vec![],
            size: 1024,
            checksum: [0; 32],
            download_url: "https://example.com/vim.lobpkg".to_string(),
        });
        index.add_package(RepoPackage {
            name: "coreutils".to_string(),
            version: "9.4".to_string(),
            arch: PackageArch::Aarch64,
            description: "GNU core utilities".to_string(),
            dependencies: vec![],
            size: 2048,
            checksum: [0; 32],
            download_url: "https://example.com/coreutils.lobpkg".to_string(),
        });

        let results = index.search("vim");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "vim");

        let results = index.search("editor");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "vim");

        let results = index.search("gnu");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "coreutils");

        let results = index.search("nonexistent");
        assert_eq!(results.len(), 0);
    }

    #[test]
    fn test_package_verify() {
        use sha2::{Digest, Sha256};

        let mut hasher = Sha256::new();
        let file_data = b"test file data";
        hasher.update(file_data);
        let checksum: [u8; 32] = hasher.finalize().into();

        let meta = PackageMetadata::new(
            "test-pkg".to_string(),
            "1.0.0".to_string(),
            PackageArch::Aarch64,
            "Test".to_string(),
            vec![],
            file_data.len() as u64,
            checksum,
            "https://example.com/pkg.lobpkg".to_string(),
        );

        let pkg = Package {
            metadata: meta,
            files: vec![],
            file_data: file_data.to_vec(),
        };

        assert!(pkg.verify());

        // Wrong data should fail
        let pkg_bad = Package {
            metadata: pkg.metadata.clone(),
            files: vec![],
            file_data: b"wrong data".to_vec(),
        };
        assert!(!pkg_bad.verify());
    }
}
