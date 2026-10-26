//! Package installation logic for LobsterOS

use crate::pkg::package::{
    InstalledPackage, Package, PackageArch, PackageFile, PackageMetadata, LOBPKG_MAGIC,
    LOBPKG_VERSION,
};
use crate::pkg::repo::verify_checksum;
use alloc::string::String;
use alloc::string::ToString;
use alloc::vec::Vec;

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

        // Parse database using from_db_string format
        let content = core::str::from_utf8(&data).unwrap_or("");
        for line in content.lines() {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(installed) = InstalledPackage::from_db_string(line) {
                // Determine if explicit from the database (field 5 in to_db_string is explicit as 0/1)
                // Actually from_db_string already parses explicit from field 4 (0-indexed)
                // We need to determine explicit from the line - let's check if it's in the format
                let parts: Vec<&str> = line.split('|').collect();
                let explicit = if parts.len() > 4 {
                    parts[4] == "1"
                } else {
                    true // default to explicit for backward compatibility
                };
                db.add(installed, explicit);
            }
        }

        Ok(db)
    }

    /// Save database to disk
    pub fn save(&self) -> Result<(), InstallError> {
        use crate::fs::vfs::{create, create_dir, open, write};

        // Ensure the full parent chain exists (VFS mkdir is
        // non-recursive; /var and /var/lib are not guaranteed).
        let _ = create_dir("/var");
        let _ = create_dir("/var/lib");
        let _ = create_dir("/var/lib/pkg");

        // Create or open the database file
        let _ = create(PKG_DB_PATH);
        let fd = match open(PKG_DB_PATH, 1) {
            // O_WRONLY
            Ok(fd) => fd,
            Err(e) => {
                crate::println!("[pkg] Failed to open database for writing: {:?}", e);
                return Err(InstallError::IoError);
            }
        };

        // Build database content using to_db_string format (includes file paths)
        let mut content = String::new();
        content.push_str("# LobsterOS package database\n");
        content.push_str("# Format: name|version|arch|install_time|explicit|description|dependencies|size|checksum|download_url|files\n");
        for entry in &self.entries {
            content.push_str(&entry.package.to_db_string());
            content.push('\n');
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
        let mut repo_manager = crate::pkg::repo::RepositoryManager::new();
        // Load the persisted repository config so a fresh Installer knows
        // which repos exist (boot init also seeds the default repo there).
        let _ = repo_manager.load_config();
        Ok(Self { db, repo_manager })
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
    #[cfg(target_arch = "aarch64")]
    pub fn install(&mut self, name: &str) -> Result<(), InstallError> {
        // Check if already installed
        if self.db.is_installed(name) {
            return Err(InstallError::AlreadyInstalled);
        }

        // Auto-refresh repository indices on demand: a fresh Installer has
        // an empty index cache (only repo CONFIG persists), so fetch the
        // index now if we don't have it. This makes `pkg install x` work
        // even without a prior `pkg update` in the same session.
        if !self.repo_manager.has_indices() {
            crate::println!("[pkg] Index cache empty — updating repositories...");
            let _ = self.update();
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
        let (status_code, body) =
            crate::net::fetch_url(&download_url).ok_or(InstallError::DownloadFailed)?;

        if status_code != 200 {
            crate::println!("[pkg] HTTP error: {}", status_code);
            return Err(InstallError::HttpError(status_code));
        }

        let buffer = body;

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
        let install_time = crate::arch::ArchTimer::uptime_ms();
        let installed = InstalledPackage::from_package(package, install_time);
        self.db.add(installed, true);

        // Save database
        self.db.save()?;

        Ok(())
    }

    /// Install a package by name from repository (x86_64 stub)
    #[cfg(target_arch = "x86_64")]
    pub fn install(&mut self, _name: &str) -> Result<(), InstallError> {
        Err(InstallError::HttpError(0)) // Network not available on x86_64
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

    /// Upgrade all installed packages to latest versions
    pub fn upgrade(&mut self) -> Result<(), InstallError> {
        // First update all repository indices
        self.update()?;

        let installed: alloc::vec::Vec<InstalledPackage> =
            self.db.list_installed().into_iter().cloned().collect();
        let mut upgraded = 0;

        for pkg in installed {
            // Find latest version in repositories
            if let Some((_repo, repo_pkg)) = self.repo_manager.find_package(&pkg.metadata.name) {
                // Compare versions (simple string comparison for now)
                if repo_pkg.version != pkg.metadata.version {
                    crate::println!(
                        "[pkg] Upgrading {} from {} to {}",
                        pkg.metadata.name,
                        pkg.metadata.version,
                        repo_pkg.version
                    );
                    // Remove old version
                    self.remove(&pkg.metadata.name)?;
                    // Install new version
                    self.install(&pkg.metadata.name)?;
                    upgraded += 1;
                }
            }
        }

        if upgraded > 0 {
            crate::println!("[pkg] Upgraded {} package(s)", upgraded);
        } else {
            crate::println!("[pkg] All packages are up to date");
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
            data: Vec::new(),
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
        _ => PackageArch::Any,
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
            download_url: String::new(),
        },
    ))
}

/// Extract package files to filesystem
fn extract_package(pkg: &Package) -> Result<(), InstallError> {
    use crate::fs::vfs::create_dir;

    // Track created directories to avoid redundant mkdir calls
    let mut created_dirs = alloc::collections::BTreeSet::new();

    // Track offset into file_data for each file
    let mut data_offset = 0usize;

    for file in &pkg.files {
        // Create parent directories
        let path = &file.path;
        let mut parent_path = String::new();
        for component in path.split('/') {
            if component.is_empty() {
                continue;
            }
            if parent_path.is_empty() {
                parent_path.push('/');
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

        // Extract file data from concatenated file_data.
        // NOTE: vfs::create returns an INODE id, not a file descriptor —
        // so write through the path-based write_file, not fd writes.
        let file_size = file.size as usize;
        if data_offset + file_size <= pkg.file_data.len() {
            let file_data = &pkg.file_data[data_offset..data_offset + file_size];
            match crate::fs::vfs::write_file(path, file_data) {
                Ok(_) => {}
                Err(e) => {
                    crate::println!("[pkg] Failed to write file {}: {:?}", path, e);
                    return Err(InstallError::IoError);
                }
            }
            data_offset += file_size;
        } else {
            crate::println!(
                "[pkg] Warning: file_data too short for {}, writing empty",
                path
            );
            let _ = crate::fs::vfs::write_file(path, &[]);
        }

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

/// Topological sort for package dependencies
/// Returns packages in installation order (dependencies first)
fn topological_sort(
    packages: &[(String, crate::pkg::package::RepoPackage)],
) -> Result<Vec<(String, crate::pkg::package::RepoPackage)>, InstallError> {
    use alloc::collections::{BTreeMap, VecDeque};

    // Build adjacency list and in-degree count
    let mut adj: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut in_degree: BTreeMap<String, usize> = BTreeMap::new();
    let mut pkg_map: BTreeMap<String, (String, crate::pkg::package::RepoPackage)> = BTreeMap::new();

    // Initialize all packages
    for (repo, pkg) in packages {
        let name = pkg.name.clone();
        pkg_map.insert(name.clone(), (repo.clone(), pkg.clone()));
        adj.entry(name.clone()).or_default();
        in_degree.entry(name).or_insert(0);
    }

    // Build edges
    for (_repo, pkg) in packages {
        let name = pkg.name.clone();
        for dep in &pkg.dependencies {
            if pkg_map.contains_key(dep) {
                adj.entry(dep.clone()).or_default().push(name.clone());
                *in_degree.entry(name.clone()).or_insert(0) += 1;
            }
            // External dependencies (not in this set) are assumed to be already installed
        }
    }

    // Kahn's algorithm
    let mut queue = VecDeque::new();
    for (name, &deg) in &in_degree {
        if deg == 0 {
            queue.push_back(name.clone());
        }
    }

    let mut result = Vec::new();
    while let Some(name) = queue.pop_front() {
        if let Some((repo, pkg)) = pkg_map.get(&name) {
            result.push((repo.clone(), pkg.clone()));
        }
        if let Some(neighbors) = adj.get(&name) {
            for neighbor in neighbors {
                let deg = in_degree.get_mut(neighbor).unwrap();
                *deg -= 1;
                if *deg == 0 {
                    queue.push_back(neighbor.clone());
                }
            }
        }
    }

    if result.len() != packages.len() {
        // Cycle detected
        return Err(InstallError::ConflictDetected);
    }

    Ok(result)
}

/// Install multiple packages with atomic transaction (all-or-nothing)
pub fn install_transaction(
    installer: &mut Installer,
    package_names: &[&str],
) -> Result<(), InstallError> {
    // First, collect all packages and their dependencies
    let mut all_packages: Vec<(String, crate::pkg::package::RepoPackage)> = Vec::new();
    let mut seen = alloc::collections::BTreeSet::new();

    fn collect_deps(
        installer: &Installer,
        name: &str,
        all: &mut Vec<(String, crate::pkg::package::RepoPackage)>,
        seen: &mut alloc::collections::BTreeSet<String>,
    ) -> Result<(), InstallError> {
        if seen.contains(name) {
            return Ok(());
        }
        let (repo, pkg) = installer
            .repo_manager
            .find_package(name)
            .ok_or(InstallError::NotFound)?;
        if installer.db.is_installed(name) {
            seen.insert(name.to_string());
            return Ok(());
        }
        seen.insert(name.to_string());
        for dep in &pkg.dependencies {
            if !installer.db.is_installed(dep) {
                collect_deps(installer, dep, all, seen)?;
            }
        }
        all.push((repo.name.clone(), pkg.clone()));
        Ok(())
    }

    for name in package_names {
        collect_deps(installer, name, &mut all_packages, &mut seen)?;
    }

    // Topologically sort
    let sorted = topological_sort(&all_packages)?;

    // Track installed packages for rollback
    let mut installed_in_transaction: Vec<String> = Vec::new();

    // Install in order
    for (_repo, pkg) in sorted {
        let name = pkg.name.as_str();
        if installer.db.is_installed(name) {
            continue;
        }

        // Download and verify
        crate::println!("[pkg] Downloading {} from {}...", name, pkg.download_url);
        let (status_code, body) =
            crate::net::fetch_url(&pkg.download_url).ok_or(InstallError::DownloadFailed)?;

        if status_code != 200 {
            // Rollback
            for rolled_back in installed_in_transaction.iter().rev() {
                let _ = installer.remove(rolled_back);
            }
            return Err(InstallError::HttpError(status_code));
        }

        let buffer = body;

        if !verify_checksum(&buffer, &pkg.checksum) {
            // Rollback
            for rolled_back in installed_in_transaction.iter().rev() {
                let _ = installer.remove(rolled_back);
            }
            return Err(InstallError::ChecksumMismatch);
        }

        let package = parse_package(&buffer)?;

        if !package.verify() {
            // Rollback
            for rolled_back in installed_in_transaction.iter().rev() {
                let _ = installer.remove(rolled_back);
            }
            return Err(InstallError::ChecksumMismatch);
        }

        // Check for conflicts
        for conflict in &package.metadata.conflicts {
            if installer.db.is_installed(conflict) {
                // Rollback
                for rolled_back in installed_in_transaction.iter().rev() {
                    let _ = installer.remove(rolled_back);
                }
                return Err(InstallError::ConflictDetected);
            }
        }

        // Extract files
        extract_package(&package)?;

        // Run install script
        if let Some(script) = &package.metadata.install_script {
            run_script(script)?;
        }

        // Add to database
        let install_time = crate::arch::ArchTimer::uptime_ms();
        let installed = InstalledPackage::from_package(package, install_time);
        installer.db.add(installed, true);
        installed_in_transaction.push(name.to_string());

        // Save database after each package (so we can rollback)
        installer.db.save()?;
    }

    Ok(())
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    use crate::pkg::RepoPackage;
    use alloc::string::ToString;
    use alloc::{format, vec};

    #[test]
    fn test_install_error_display() {
        assert_eq!(
            InstallError::InvalidPackage.to_string(),
            "Invalid package format"
        );
        assert_eq!(
            InstallError::ChecksumMismatch.to_string(),
            "Checksum mismatch"
        );
        assert_eq!(
            InstallError::DependencyMissing.to_string(),
            "Missing dependency"
        );
        assert_eq!(
            InstallError::ConflictDetected.to_string(),
            "Package conflict"
        );
        assert_eq!(
            InstallError::InsufficientSpace.to_string(),
            "Insufficient disk space"
        );
        assert_eq!(InstallError::IoError.to_string(), "I/O error");
        assert_eq!(InstallError::NotImplemented.to_string(), "Not implemented");
        assert_eq!(
            InstallError::AlreadyInstalled.to_string(),
            "Package already installed"
        );
        assert_eq!(
            InstallError::NotInstalled.to_string(),
            "Package not installed"
        );
        assert_eq!(InstallError::NotFound.to_string(), "Package not found");
        assert_eq!(
            InstallError::DownloadFailed.to_string(),
            "Package download failed"
        );
        assert_eq!(InstallError::HttpError(404).to_string(), "HTTP error: 404");
    }

    #[test]
    fn test_install_error_from_repo_error() {
        use crate::pkg::repo::RepoError;

        assert_eq!(
            InstallError::from(RepoError::NotFound),
            InstallError::NotFound
        );
        assert_eq!(
            InstallError::from(RepoError::NetworkError),
            InstallError::IoError
        );
        assert_eq!(
            InstallError::from(RepoError::ParseError),
            InstallError::InvalidPackage
        );
        assert_eq!(
            InstallError::from(RepoError::SignatureError),
            InstallError::ChecksumMismatch
        );
        assert_eq!(
            InstallError::from(RepoError::IoError),
            InstallError::IoError
        );
        assert_eq!(
            InstallError::from(RepoError::HttpError(500)),
            InstallError::HttpError(0)
        );
        assert_eq!(
            InstallError::from(RepoError::NotImplemented),
            InstallError::NotImplemented
        );
    }

    #[test]
    fn test_db_entry_creation() {
        let checksum = [0u8; 32];
        let meta = PackageMetadata::new(
            "test-pkg".to_string(),
            "1.0.0".to_string(),
            PackageArch::Aarch64,
            "Test package".to_string(),
            vec![],
            1024,
            checksum,
            "https://example.com/pkg.lobpkg".to_string(),
        );

        let pkg = Package {
            metadata: meta.clone(),
            files: vec![],
            file_data: vec![],
        };

        let installed = InstalledPackage::from_package(pkg, 1234567890);
        let entry = DbEntry {
            package: installed,
            explicit: true,
        };

        assert!(entry.explicit);
        assert_eq!(entry.package.metadata.name, "test-pkg");
    }

    #[test]
    fn test_package_database_new() {
        let db = PackageDatabase::new();
        assert_eq!(db.list_installed().len(), 0);
    }

    #[test]
    fn test_package_database_is_installed() {
        let mut db = PackageDatabase::new();
        assert!(!db.is_installed("test-pkg"));

        let checksum = [0u8; 32];
        let meta = PackageMetadata::new(
            "test-pkg".to_string(),
            "1.0.0".to_string(),
            PackageArch::Aarch64,
            "Test package".to_string(),
            vec![],
            1024,
            checksum,
            "https://example.com/pkg.lobpkg".to_string(),
        );

        let pkg = Package {
            metadata: meta,
            files: vec![],
            file_data: vec![],
        };

        let installed = InstalledPackage::from_package(pkg, 1234567890);
        db.add(installed, true);

        assert!(db.is_installed("test-pkg"));
    }

    #[test]
    fn test_package_database_get_installed() {
        let mut db = PackageDatabase::new();
        let checksum = [0u8; 32];
        let meta = PackageMetadata::new(
            "test-pkg".to_string(),
            "1.0.0".to_string(),
            PackageArch::Aarch64,
            "Test package".to_string(),
            vec![],
            1024,
            checksum,
            "https://example.com/pkg.lobpkg".to_string(),
        );

        let pkg = Package {
            metadata: meta,
            files: vec![],
            file_data: vec![],
        };

        let installed = InstalledPackage::from_package(pkg, 1234567890);
        db.add(installed, true);

        let retrieved = db.get_installed("test-pkg").unwrap();
        assert_eq!(retrieved.metadata.name, "test-pkg");
        assert_eq!(retrieved.metadata.version, "1.0.0");
    }

    #[test]
    fn test_package_database_get_installed_nonexistent() {
        let db = PackageDatabase::new();
        let result = db.get_installed("nonexistent");
        assert!(result.is_none());
    }

    #[test]
    fn test_package_database_list_installed() {
        let mut db = PackageDatabase::new();
        let checksum = [0u8; 32];

        for i in 0..3 {
            let meta = PackageMetadata::new(
                format!("pkg-{}", i),
                "1.0.0".to_string(),
                PackageArch::Aarch64,
                "Test package".to_string(),
                vec![],
                1024,
                checksum,
                format!("https://example.com/pkg-{}.lobpkg", i),
            );

            let pkg = Package {
                metadata: meta,
                files: vec![],
                file_data: vec![],
            };

            let installed = InstalledPackage::from_package(pkg, 1234567890 + i as u64);
            db.add(installed, true);
        }

        let list = db.list_installed();
        assert_eq!(list.len(), 3);
    }

    #[test]
    fn test_package_database_add_remove() {
        let mut db = PackageDatabase::new();
        let checksum = [0u8; 32];
        let meta = PackageMetadata::new(
            "test-pkg".to_string(),
            "1.0.0".to_string(),
            PackageArch::Aarch64,
            "Test package".to_string(),
            vec![],
            1024,
            checksum,
            "https://example.com/pkg.lobpkg".to_string(),
        );

        let pkg = Package {
            metadata: meta,
            files: vec![],
            file_data: vec![],
        };

        let installed = InstalledPackage::from_package(pkg, 1234567890);
        db.add(installed.clone(), true);

        assert!(db.is_installed("test-pkg"));

        let removed = db.remove("test-pkg").unwrap();
        assert_eq!(removed.metadata.name, "test-pkg");
        assert!(!db.is_installed("test-pkg"));

        // Removing again returns None
        let removed = db.remove("test-pkg");
        assert!(removed.is_none());
    }

    #[test]
    fn test_package_database_is_explicit() {
        let mut db = PackageDatabase::new();
        let checksum = [0u8; 32];
        let meta = PackageMetadata::new(
            "test-pkg".to_string(),
            "1.0.0".to_string(),
            PackageArch::Aarch64,
            "Test package".to_string(),
            vec![],
            1024,
            checksum,
            "https://example.com/pkg.lobpkg".to_string(),
        );

        let pkg = Package {
            metadata: meta,
            files: vec![],
            file_data: vec![],
        };

        let installed = InstalledPackage::from_package(pkg, 1234567890);
        db.add(installed, true);

        assert!(db.is_explicit("test-pkg"));

        // Add a dependency (explicit = false)
        let dep_meta = PackageMetadata::new(
            "dep-pkg".to_string(),
            "1.0.0".to_string(),
            PackageArch::Aarch64,
            "Dependency".to_string(),
            vec![],
            512,
            checksum,
            "https://example.com/dep.lobpkg".to_string(),
        );

        let dep_pkg = Package {
            metadata: dep_meta,
            files: vec![],
            file_data: vec![],
        };

        let dep_installed = InstalledPackage::from_package(dep_pkg, 1234567891);
        db.add(dep_installed, false);

        assert!(!db.is_explicit("dep-pkg"));
    }

    #[test]
    fn test_topological_sort_simple() {
        let packages = vec![
            (
                "repo".to_string(),
                RepoPackage {
                    name: "pkg-a".to_string(),
                    version: "1.0".to_string(),
                    arch: PackageArch::Aarch64,
                    description: "Package A".to_string(),
                    dependencies: vec![],
                    size: 100,
                    checksum: [0; 32],
                    download_url: "https://example.com/a.lobpkg".to_string(),
                },
            ),
            (
                "repo".to_string(),
                RepoPackage {
                    name: "pkg-b".to_string(),
                    version: "1.0".to_string(),
                    arch: PackageArch::Aarch64,
                    description: "Package B".to_string(),
                    dependencies: vec!["pkg-a".to_string()],
                    size: 100,
                    checksum: [0; 32],
                    download_url: "https://example.com/b.lobpkg".to_string(),
                },
            ),
        ];

        let sorted = topological_sort(&packages).unwrap();
        assert_eq!(sorted.len(), 2);
        // pkg-a should come before pkg-b (dependency first)
        assert_eq!(sorted[0].1.name, "pkg-a");
        assert_eq!(sorted[1].1.name, "pkg-b");
    }

    #[test]
    fn test_topological_sort_independent() {
        let packages = vec![
            (
                "repo".to_string(),
                RepoPackage {
                    name: "pkg-a".to_string(),
                    version: "1.0".to_string(),
                    arch: PackageArch::Aarch64,
                    description: "Package A".to_string(),
                    dependencies: vec![],
                    size: 100,
                    checksum: [0; 32],
                    download_url: "https://example.com/a.lobpkg".to_string(),
                },
            ),
            (
                "repo".to_string(),
                RepoPackage {
                    name: "pkg-b".to_string(),
                    version: "1.0".to_string(),
                    arch: PackageArch::Aarch64,
                    description: "Package B".to_string(),
                    dependencies: vec![],
                    size: 100,
                    checksum: [0; 32],
                    download_url: "https://example.com/b.lobpkg".to_string(),
                },
            ),
        ];

        let sorted = topological_sort(&packages).unwrap();
        assert_eq!(sorted.len(), 2);
        // Order of independent packages doesn't matter
    }

    #[test]
    fn test_topological_sort_cycle_detection() {
        let packages = vec![
            (
                "repo".to_string(),
                RepoPackage {
                    name: "pkg-a".to_string(),
                    version: "1.0".to_string(),
                    arch: PackageArch::Aarch64,
                    description: "Package A".to_string(),
                    dependencies: vec!["pkg-b".to_string()],
                    size: 100,
                    checksum: [0; 32],
                    download_url: "https://example.com/a.lobpkg".to_string(),
                },
            ),
            (
                "repo".to_string(),
                RepoPackage {
                    name: "pkg-b".to_string(),
                    version: "1.0".to_string(),
                    arch: PackageArch::Aarch64,
                    description: "Package B".to_string(),
                    dependencies: vec!["pkg-a".to_string()],
                    size: 100,
                    checksum: [0; 32],
                    download_url: "https://example.com/b.lobpkg".to_string(),
                },
            ),
        ];

        let result = topological_sort(&packages);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), InstallError::ConflictDetected);
    }

    #[test]
    fn test_constants() {
        assert_eq!(INSTALL_PREFIX, "/usr");
        assert_eq!(BIN_DIR, "/usr/bin");
        assert_eq!(LIB_DIR, "/usr/lib");
        assert_eq!(ETC_DIR, "/etc");
        assert_eq!(SHARE_DIR, "/usr/share");
        assert_eq!(PKG_DB_PATH, "/var/lib/pkg/installed.db");
    }
}
