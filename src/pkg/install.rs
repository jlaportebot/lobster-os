//! Package installation logic for LobsterOS

use crate::net::http::http_get;
use crate::pkg::package::{
    InstalledPackage, Package, PackageArch, PackageFile, PackageMetadata, LOBPKG_MAGIC,
    LOBPKG_VERSION,
};
use crate::pkg::repo::verify_checksum;
use alloc::string::String;
use alloc::string::ToString;
use alloc::vec::Vec;
use alloc::format;

/// Installation target paths
pub const INSTALL_PREFIX: &str = "/usr";
pub const BIN_DIR: &str = "/usr/bin";
pub const LIB_DIR: &str = "/usr/lib";
pub const ETC_DIR: &str = "/etc";
pub const SHARE_DIR: &str = "/usr/share";

/// Package database path
pub const PKG_DB_PATH: &str = "/var/lib/pkg/installed.db";

/// Installation errors
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallError {
    InvalidPackage,
    ChecksumMismatch,
    DependencyMissing,
    ConflictDetected,
    InsufficientSpace,
    IoError,
    NotImplemented,
    AlreadyInstalled,
    NotInstalled,
    NotFound,
    DownloadFailed,
    HttpError(u16),
}

impl core::fmt::Display for InstallError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            InstallError::InvalidPackage => write!(f, "Invalid package format"),
            InstallError::ChecksumMismatch => write!(f, "Checksum mismatch"),
            InstallError::DependencyMissing => write!(f, "Missing dependency"),
            InstallError::ConflictDetected => write!(f, "Package conflict"),
            InstallError::InsufficientSpace => write!(f, "Insufficient disk space"),
            InstallError::IoError => write!(f, "I/O error"),
            InstallError::NotImplemented => write!(f, "Not implemented"),
            InstallError::AlreadyInstalled => write!(f, "Package already installed"),
            InstallError::NotInstalled => write!(f, "Package not installed"),
            InstallError::NotFound => write!(f, "Package not found"),
            InstallError::DownloadFailed => write!(f, "Package download failed"),
            InstallError::HttpError(code) => write!(f, "HTTP error: {}", code),
        }
    }
}

impl From<crate::pkg::repo::RepoError> for InstallError {
    fn from(err: crate::pkg::repo::RepoError) -> Self {
        match err {
            crate::pkg::repo::RepoError::NotFound => InstallError::NotFound,
            crate::pkg::repo::RepoError::NetworkError => InstallError::IoError,
            crate::pkg::repo::RepoError::ParseError => InstallError::InvalidPackage,
            crate::pkg::repo::RepoError::SignatureError => InstallError::ChecksumMismatch,
            crate::pkg::repo::RepoError::IoError => InstallError::IoError,
            crate::pkg::repo::RepoError::HttpError(_) => InstallError::HttpError(0),
            crate::pkg::repo::RepoError::NotImplemented => InstallError::NotImplemented,
        }
    }
}

/// Package database entry
#[derive(Debug, Clone)]
pub struct DbEntry {
    pub package: InstalledPackage,
    pub explicit: bool, // true if explicitly installed, false if dependency
}

/// Package database
pub struct PackageDatabase {
    entries: Vec<DbEntry>,
}

impl PackageDatabase {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Load database from disk
    pub fn load() -> Result<Self, InstallError> {
        use crate::fs::vfs::read_file;

        let mut db = Self::new();

        // Try to read the database file
        let data = match read_file(PKG_DB_PATH) {
            Ok(d) => d,
            Err(_) => return Ok(db), // No database file yet, return empty
        };

        // Parse JSON database
        // For simplicity, we'll use a simple format:
        // Each line: name|version|arch|install_time|explicit|path1|path2|...
        // In a real implementation, use proper JSON
        let content = core::str::from_utf8(&data).unwrap_or("");
        for line in content.lines() {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let parts: Vec<&str> = line.split('|').collect();
            if parts.len() >= 5 {
                let name = parts[0];
                let version = parts[1];
                let _arch = parts[2];
                let install_time = parts[3].parse().unwrap_or(0);
                let explicit = parts[4] == "1";

                // Create minimal installed package entry
                use crate::pkg::package::{InstalledPackage, Package, PackageMetadata, PackageArch};
                let meta = PackageMetadata {
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
                };
                let pkg = Package {
                    metadata: meta,
                    files: Vec::new(),
                    file_data: Vec::new(),
                };
                let installed = InstalledPackage::from_package(pkg, install_time);
                db.add(installed, explicit);
            }
        }

        Ok(db)
    }

    /// Save database to disk
    pub fn save(&self) -> Result<(), InstallError> {
        use crate::fs::vfs::{create_dir, create, open, write};

        // Ensure parent directory exists
        let _ = create_dir("/var/lib/pkg");

        // Create or open the database file
        let _ = create(PKG_DB_PATH);
        let fd = match open(PKG_DB_PATH, 1) { // O_WRONLY
            Ok(fd) => fd,
            Err(e) => {
                crate::println!("[pkg] Failed to open database for writing: {:?}", e);
                return Err(InstallError::IoError);
            }
        };

        // Build database content (simple text format)
        let mut content = String::new();
        content.push_str("# LobsterOS package database\n");
        content.push_str("# Format: name|version|arch|install_time|explicit\n");
        for entry in &self.entries {
            content.push_str(&format!(
                "{}|{}|aarch64|{}|{}\n",
                entry.package.metadata.name,
                entry.package.metadata.version,
                entry.package.install_time,
                if entry.explicit { "1" } else { "0" }
            ));
        }

        // Write to file
        write(fd, content.as_bytes());
        Ok(())
    }

    /// Check if package is installed
    pub fn is_installed(&self, name: &str) -> bool {
        self.entries.iter().any(|e| e.package.metadata.name == name)
    }

    /// Get installed package
    pub fn get_installed(&self, name: &str) -> Option<&InstalledPackage> {
        self.entries
            .iter()
            .find(|e| e.package.metadata.name == name)
            .map(|e| &e.package)
    }

    /// Get all installed packages
    pub fn list_installed(&self) -> Vec<&InstalledPackage> {
        self.entries.iter().map(|e| &e.package).collect()
    }

    /// Add installed package
    pub fn add(&mut self, pkg: InstalledPackage, explicit: bool) {
        self.entries.push(DbEntry {
            package: pkg,
            explicit,
        });
    }

    /// Remove installed package
    pub fn remove(&mut self, name: &str) -> Option<InstalledPackage> {
        if let Some(pos) = self
            .entries
            .iter()
            .position(|e| e.package.metadata.name == name)
        {
            Some(self.entries.remove(pos).package)
        } else {
            None
        }
    }

    /// Check if package is explicitly installed
    pub fn is_explicit(&self, name: &str) -> bool {
        self.entries
            .iter()
            .find(|e| e.package.metadata.name == name)
            .map(|e| e.explicit)
            .unwrap_or(false)
    }
}

impl Default for PackageDatabase {
    fn default() -> Self {
        Self::new()
    }
}

/// Package installer
pub struct Installer {
    db: PackageDatabase,
    repo_manager: crate::pkg::repo::RepositoryManager,
}

impl Installer {
    pub fn new() -> Result<Self, InstallError> {
        let db = PackageDatabase::load()?;
        Ok(Self {
            db,
            repo_manager: crate::pkg::repo::RepositoryManager::new(),
        })
    }

    /// Get reference to repository manager
    pub fn repo_manager(&mut self) -> &mut crate::pkg::repo::RepositoryManager {
        &mut self.repo_manager
    }

    /// Get reference to package database
    pub fn database(&self) -> &PackageDatabase {
        &self.db
    }

    /// Get mutable reference to package database
    pub fn database_mut(&mut self) -> &mut PackageDatabase {
        &mut self.db
    }

    /// Install a package by name from repository
    pub fn install(&mut self, name: &str) -> Result<(), InstallError> {
        // Check if already installed
        if self.db.is_installed(name) {
            return Err(InstallError::AlreadyInstalled);
        }

        // Find package in repositories
        let (_, repo_pkg) = self
            .repo_manager
            .find_package(name)
            .ok_or(InstallError::NotFound)?;

        // Clone all needed fields before potential recursive calls
        let dependencies = repo_pkg.dependencies.clone();
        let download_url = repo_pkg.download_url.clone();
        let repo_checksum = repo_pkg.checksum;
        let _pkg_size = repo_pkg.size;

        // Check dependencies
        for dep in &dependencies {
            if !self.db.is_installed(dep) {
                // Try to install dependency first
                self.install(dep)?;
            }
        }

        // Download package
        crate::println!("[pkg] Downloading {} from {}...", name, download_url);
        let response = http_get(&download_url).ok_or(InstallError::DownloadFailed)?;

        if response.status_code != 200 {
            crate::println!("[pkg] HTTP error: {}", response.status_code);
            return Err(InstallError::HttpError(response.status_code));
        }

        let buffer = response.body[..response.body_len].to_vec();

        // Verify checksum
        if !verify_checksum(&buffer, &repo_checksum) {
            return Err(InstallError::ChecksumMismatch);
        }

        // Parse package
        let package = parse_package(&buffer)?;

        // Verify package integrity
        if !package.verify() {
            return Err(InstallError::ChecksumMismatch);
        }

        // Check for conflicts
        for conflict in &package.metadata.conflicts {
            if self.db.is_installed(conflict) {
                return Err(InstallError::ConflictDetected);
            }
        }

        // Extract files
        extract_package(&package)?;

        // Run install script if present
        if let Some(script) = &package.metadata.install_script {
            run_script(script)?;
        }

        // Add to database
        let install_time = crate::arch::timer::Timer::uptime_ms();
        let installed = InstalledPackage::from_package(package, install_time);
        self.db.add(installed, true);

        // Save database
        self.db.save()?;

        Ok(())
    }

    /// Remove a package
    pub fn remove(&mut self, name: &str) -> Result<(), InstallError> {
        // Check if installed
        let pkg = self
            .db
            .get_installed(name)
            .ok_or(InstallError::NotInstalled)?
            .clone();

        // Check if other packages depend on this
        for entry in &self.db.entries {
            if entry.package.metadata.name != name {
                for dep in &entry.package.metadata.dependencies {
                    if dep == name && entry.explicit {
                        return Err(InstallError::DependencyMissing);
                    }
                }
            }
        }

        // Run remove script if present
        if let Some(script) = &pkg.metadata.remove_script {
            run_script(script)?;
        }

        // Remove files
        remove_package_files(&pkg)?;

        // Remove from database
        self.db.remove(name);

        // Save database
        self.db.save()?;

        Ok(())
    }

    /// List installed packages
    pub fn list(&self) -> Vec<&InstalledPackage> {
        self.db.list_installed()
    }

    /// Show package info
    pub fn info(&self, name: &str) -> Option<&InstalledPackage> {
        self.db.get_installed(name)
    }

    /// Update package index
    pub fn update(&mut self) -> Result<(), InstallError> {
        let repo_names: alloc::vec::Vec<String> = self
            .repo_manager
            .list_repos()
            .iter()
            .filter(|r| r.enabled)
            .map(|r| r.name.clone())
            .collect();

        for name in repo_names {
            self.repo_manager.update_index(&name)?;
        }
        Ok(())
    }

    /// Search packages
    pub fn search(&self, query: &str) -> Vec<(&str, &crate::pkg::package::RepoPackage)> {
        self.repo_manager.search(query)
    }
}

impl Default for Installer {
    fn default() -> Self {
        Self::new().unwrap()
    }
}

/// Parse package from raw bytes
fn parse_package(data: &[u8]) -> Result<Package, InstallError> {
    // Verify magic
    if data.len() < 8 || &data[0..4] != LOBPKG_MAGIC {
        return Err(InstallError::InvalidPackage);
    }

    // Verify version
    let version = u16::from_le_bytes([data[4], data[5]]);
    if version != LOBPKG_VERSION {
        return Err(InstallError::InvalidPackage);
    }

    let mut offset = 6;

    // Read metadata using custom binary format
    let metadata = read_metadata(&data[offset..])?;
    offset += metadata.0; // bytes consumed

    // Read file count
    if offset + 4 > data.len() {
        return Err(InstallError::InvalidPackage);
    }
    let file_count = u32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ]) as usize;
    offset += 4;

    let mut files = Vec::with_capacity(file_count);

    // Parse each file entry
    for _ in 0..file_count {
        // path_len
        if offset + 4 > data.len() {
            return Err(InstallError::InvalidPackage);
        }
        let path_len = u32::from_le_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ]) as usize;
        offset += 4;

        if offset + path_len > data.len() {
            return Err(InstallError::InvalidPackage);
        }
        let path = core::str::from_utf8(&data[offset..offset + path_len])
            .map_err(|_| InstallError::InvalidPackage)?
            .to_string();
        offset += path_len;

        // mode, uid, gid, size
        if offset + 4 * 4 > data.len() {
            return Err(InstallError::InvalidPackage);
        }
        let mode = u32::from_le_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ]);
        offset += 4;

        let uid = u32::from_le_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ]);
        offset += 4;

        let gid = u32::from_le_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ]);
        offset += 4;

        let size = u64::from_le_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
            data[offset + 4],
            data[offset + 5],
            data[offset + 6],
            data[offset + 7],
        ]);
        offset += 8;

        // checksum (32 bytes)
        if offset + 32 > data.len() {
            return Err(InstallError::InvalidPackage);
        }
        let mut checksum = [0u8; 32];
        checksum.copy_from_slice(&data[offset..offset + 32]);
        offset += 32;

        // is_config, is_symlink
        if offset + 2 > data.len() {
            return Err(InstallError::InvalidPackage);
        }
        let is_config = data[offset] != 0;
        offset += 1;
        let is_symlink = data[offset] != 0;
        offset += 1;

        // symlink_target (optional)
        let symlink_target = if is_symlink {
            if offset + 4 > data.len() {
                return Err(InstallError::InvalidPackage);
            }
            let target_len = u32::from_le_bytes([
                data[offset],
                data[offset + 1],
                data[offset + 2],
                data[offset + 3],
            ]) as usize;
            offset += 4;

            if offset + target_len > data.len() {
                return Err(InstallError::InvalidPackage);
            }
            let target = core::str::from_utf8(&data[offset..offset + target_len])
                .map_err(|_| InstallError::InvalidPackage)?
                .to_string();
            offset += target_len;
            Some(target)
        } else {
            None
        };

        files.push(PackageFile {
            path,
            mode,
            uid,
            gid,
            size,
            checksum,
            is_config,
            is_symlink,
            symlink_target,
        });
    }

    // Read file data length
    if offset + 4 > data.len() {
        return Err(InstallError::InvalidPackage);
    }
    let file_data_len = u32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ]) as usize;
    offset += 4;

    if offset + file_data_len > data.len() {
        return Err(InstallError::InvalidPackage);
    }
    let file_data = data[offset..offset + file_data_len].to_vec();

    Ok(Package {
        metadata: metadata.1,
        files,
        file_data,
    })
}

/// Read metadata from binary format
/// Returns (bytes_consumed, PackageMetadata)
fn read_metadata(data: &[u8]) -> Result<(usize, PackageMetadata), InstallError> {
    let mut offset = 0;

    // Read string helper
    let read_string = |data: &[u8], offset: &mut usize| -> Result<String, InstallError> {
        if *offset + 4 > data.len() {
            return Err(InstallError::InvalidPackage);
        }
        let len = u32::from_le_bytes([
            data[*offset],
            data[*offset + 1],
            data[*offset + 2],
            data[*offset + 3],
        ]) as usize;
        *offset += 4;
        if *offset + len > data.len() {
            return Err(InstallError::InvalidPackage);
        }
        let s = core::str::from_utf8(&data[*offset..*offset + len])
            .map_err(|_| InstallError::InvalidPackage)?
            .to_string();
        *offset += len;
        Ok(s)
    };

    let name = read_string(data, &mut offset)?;
    let version = read_string(data, &mut offset)?;
    let arch = match read_string(data, &mut offset)?.as_str() {
        "aarch64" => PackageArch::Aarch64,
        "armv7" => PackageArch::Armv7,
        "x86_64" => PackageArch::X86_64,
        _ => PackageArch::All,
    };
    let description = read_string(data, &mut offset)?;
    let maintainer = read_string(data, &mut offset)?;
    let license = read_string(data, &mut offset)?;
    let homepage = read_string(data, &mut offset)?;

    // dependencies count + strings
    if offset + 4 > data.len() {
        return Err(InstallError::InvalidPackage);
    }
    let dep_count = u32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ]) as usize;
    offset += 4;
    let mut dependencies = Vec::with_capacity(dep_count);
    for _ in 0..dep_count {
        dependencies.push(read_string(data, &mut offset)?);
    }

    // provides
    if offset + 4 > data.len() {
        return Err(InstallError::InvalidPackage);
    }
    let provide_count = u32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ]) as usize;
    offset += 4;
    let mut provides = Vec::with_capacity(provide_count);
    for _ in 0..provide_count {
        provides.push(read_string(data, &mut offset)?);
    }

    // conflicts
    if offset + 4 > data.len() {
        return Err(InstallError::InvalidPackage);
    }
    let conflict_count = u32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ]) as usize;
    offset += 4;
    let mut conflicts = Vec::with_capacity(conflict_count);
    for _ in 0..conflict_count {
        conflicts.push(read_string(data, &mut offset)?);
    }

    // replaces
    if offset + 4 > data.len() {
        return Err(InstallError::InvalidPackage);
    }
    let replace_count = u32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ]) as usize;
    offset += 4;
    let mut replaces = Vec::with_capacity(replace_count);
    for _ in 0..replace_count {
        replaces.push(read_string(data, &mut offset)?);
    }

    // size, download_size
    if offset + 8 + 8 > data.len() {
        return Err(InstallError::InvalidPackage);
    }
    let size = u64::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
        data[offset + 4],
        data[offset + 5],
        data[offset + 6],
        data[offset + 7],
    ]);
    offset += 8;

    let download_size = u64::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
        data[offset + 4],
        data[offset + 5],
        data[offset + 6],
        data[offset + 7],
    ]);
    offset += 8;

    // checksum
    if offset + 32 > data.len() {
        return Err(InstallError::InvalidPackage);
    }
    let mut checksum = [0u8; 32];
    checksum.copy_from_slice(&data[offset..offset + 32]);
    offset += 32;

    // install_script (optional)
    let install_script = {
        if offset + 4 > data.len() {
            return Err(InstallError::InvalidPackage);
        }
        let len = u32::from_le_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ]) as usize;
        offset += 4;
        if len > 0 {
            if offset + len > data.len() {
                return Err(InstallError::InvalidPackage);
            }
            let s = core::str::from_utf8(&data[offset..offset + len])
                .map_err(|_| InstallError::InvalidPackage)?
                .to_string();
            offset += len;
            Some(s)
        } else {
            None
        }
    };

    // remove_script (optional)
    let remove_script = {
        if offset + 4 > data.len() {
            return Err(InstallError::InvalidPackage);
        }
        let len = u32::from_le_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ]) as usize;
        offset += 4;
        if len > 0 {
            if offset + len > data.len() {
                return Err(InstallError::InvalidPackage);
            }
            let s = core::str::from_utf8(&data[offset..offset + len])
                .map_err(|_| InstallError::InvalidPackage)?
                .to_string();
            offset += len;
            Some(s)
        } else {
            None
        }
    };

    // config_files
    if offset + 4 > data.len() {
        return Err(InstallError::InvalidPackage);
    }
    let config_count = u32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ]) as usize;
    offset += 4;
    let mut config_files = Vec::with_capacity(config_count);
    for _ in 0..config_count {
        config_files.push(read_string(data, &mut offset)?);
    }

    // build_timestamp
    if offset + 8 > data.len() {
        return Err(InstallError::InvalidPackage);
    }
    let build_timestamp = u64::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
        data[offset + 4],
        data[offset + 5],
        data[offset + 6],
        data[offset + 7],
    ]);
    offset += 8;

    Ok((
        offset,
        PackageMetadata {
            name,
            version,
            arch,
            description,
            maintainer,
            license,
            homepage,
            dependencies,
            provides,
            conflicts,
            replaces,
            size,
            download_size,
            checksum,
            build_timestamp,
            install_script,
            remove_script,
            config_files,
        },
    ))
}

/// Extract package files to filesystem
fn extract_package(pkg: &Package) -> Result<(), InstallError> {
    use crate::fs::vfs::{create_dir, create, write};

    // Track created directories to avoid redundant mkdir calls
    let mut created_dirs = alloc::collections::BTreeSet::new();

    for file in &pkg.files {
        // Create parent directories
        let path = &file.path;
        let mut parent_path = String::new();
        for component in path.split('/') {
            if component.is_empty() {
                continue;
            }
            if parent_path.is_empty() {
                parent_path.push_str("/");
            } else {
                parent_path.push('/');
            }
            parent_path.push_str(component);

            // Check if this is the file itself (last component)
            if parent_path == *path {
                break;
            }

            // Create directory if not already created
            if created_dirs.insert(parent_path.clone()) {
                let _ = create_dir(&parent_path);
            }
        }

        // Create the file
        let fd = match create(path) {
            Ok(fd) => fd,
            Err(e) => {
                crate::println!("[pkg] Failed to create file {}: {:?}", path, e);
                return Err(InstallError::IoError);
            }
        };

        // For now, we don't have per-file data separation in file_data
        // In a real implementation, file_data would contain concatenated file contents
        // with known offsets/sizes. For this stub, we write empty files.
        // The actual file data extraction would need the package format to support
        // individual file data sections.
        let _ = write(fd.try_into().unwrap(), &[]);

        // Set permissions (would use chmod syscall when available)
        // let _ = crate::fs::vfs::chmod(path, file.mode);
    }

    Ok(())
}

/// Remove package files from filesystem
fn remove_package_files(pkg: &InstalledPackage) -> Result<(), InstallError> {
    use crate::fs::vfs::remove;

    // Remove files listed in package
    for file in &pkg.files {
        let _ = remove(&file.path);
    }

    // Note: We don't remove empty directories to avoid removing
    // directories that might be shared with other packages.
    // A more sophisticated implementation would track directory reference counts.

    Ok(())
}

/// Run installation/removal script
/// Executes shell commands in the script
fn run_script(script: &str) -> Result<(), InstallError> {
    crate::println!("[pkg] Running script ({} chars):", script.len());

    // Split script into lines and execute each as a shell command
    for line in script.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        crate::println!("[pkg]   $ {}", line);

        // Execute via shell command processor
        // We need to parse and execute the command
        let mut args: [&str; 32] = [&""; 32];
        let mut argc = 0;
        for part in line.split_whitespace() {
            if argc < 32 {
                args[argc] = part;
                argc += 1;
            }
        }

        if argc > 0 {
            // Try to dispatch to built-in commands
            match args[0] {
                "mkdir" => {
                    if argc >= 2 {
                        let _ = crate::fs::vfs::create_dir(args[1]);
                    }
                }
                "rm" => {
                    if argc >= 2 {
                        let _ = crate::fs::vfs::remove(args[1]);
                    }
                }
                "chmod" | "chown" => {
                    // Would need chmod/chown syscalls
                }
                "echo" => {
                    if argc >= 2 {
                        let output = args[1..argc].join(" ");
                        crate::println!("{}", output);
                    }
                }
                _ => {
                    // For external commands, we'd need to spawn a process
                    // For now, just log
                    crate::println!("[pkg]   [external command not yet supported: {}]", args[0]);
                }
            }
        }
    }

    Ok(())
}
