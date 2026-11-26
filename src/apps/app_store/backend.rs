//! The real side effects of store jobs, built on `crate::pkg`.

use super::job::{Backend, Failure};
use super::model::InstalledInfo;
use crate::pkg::install::{InstallError, Installer, PackageDatabase};
use crate::pkg::package::{Package, RepoPackage};
use crate::pkg::repo::{IndexFiles, RepoError};
use alloc::string::String;
use alloc::vec::Vec;

/// Download, verify and install state shared by the stages of one job.
pub struct PkgBackend {
    installer: Installer,
    index_files: Option<IndexFiles>,
    blob: Vec<u8>,
    package: Option<Package>,
}

static mut BACKEND: Option<PkgBackend> = None;

/// The shared backend, created on first use. `None` when the package
/// database cannot be opened.
pub fn get() -> Option<&'static mut PkgBackend> {
    // SAFETY: only the UI thread touches the backend, one call at a time.
    let slot = unsafe { &mut *core::ptr::addr_of_mut!(BACKEND) };
    if slot.is_none() {
        let mut installer = Installer::new().ok()?;
        if installer.repo_manager().ensure_default_repo() {
            let _ = installer.repo_manager().save_config();
        }
        *slot = Some(PkgBackend {
            installer,
            index_files: None,
            blob: Vec::new(),
            package: None,
        });
    }
    slot.as_mut()
}

fn repo_failure(err: RepoError) -> Failure {
    match err {
        RepoError::NetworkError | RepoError::NotImplemented => Failure::Network,
        RepoError::SignatureError => Failure::Signature,
        RepoError::HttpError(code) => Failure::Http(code),
        RepoError::NotFound => Failure::NotFound,
        RepoError::ParseError | RepoError::IoError => Failure::Invalid,
    }
}

fn install_failure(err: InstallError) -> Failure {
    match err {
        InstallError::ChecksumMismatch => Failure::Checksum,
        InstallError::DownloadFailed => Failure::Network,
        InstallError::HttpError(code) => Failure::Http(code),
        InstallError::NotFound => Failure::NotFound,
        InstallError::InsufficientSpace | InstallError::IoError => Failure::Storage,
        InstallError::ConflictDetected => Failure::Conflict,
        _ => Failure::Invalid,
    }
}

impl PkgBackend {
    fn repo_name(&mut self) -> Option<String> {
        let repos = self.installer.repo_manager().list_repos();
        repos.iter().find(|r| r.enabled).map(|r| r.name.clone())
    }

    fn refresh_database(&mut self) {
        if let Ok(db) = PackageDatabase::load() {
            *self.installer.database_mut() = db;
        }
    }

    /// Catalogue entries of the cached index (empty before a refresh).
    pub fn catalogue(&mut self) -> Vec<RepoPackage> {
        let Some(name) = self.repo_name() else {
            return Vec::new();
        };
        let manager = self.installer.repo_manager();
        manager
            .get_index(&name)
            .map(|index| index.packages.clone())
            .unwrap_or_default()
    }

    /// Packages installed on this machine, read fresh from the database.
    pub fn installed(&mut self) -> Vec<InstalledInfo> {
        self.refresh_database();
        self.installer
            .database()
            .list_installed()
            .into_iter()
            .map(|p| InstalledInfo {
                name: p.metadata.name.clone(),
                version: p.metadata.version.clone(),
                description: p.metadata.description.clone(),
                size: p.metadata.size,
                exec: p
                    .files
                    .iter()
                    .find(|f| crate::wm::launcher::is_program_path(&f.path) && !f.is_symlink)
                    .map(|f| f.path.clone())
                    .unwrap_or_default(),
            })
            .collect()
    }

    fn lookup(&mut self, name: &str) -> Result<RepoPackage, Failure> {
        let manager = self.installer.repo_manager();
        let (_, pkg) = manager.find_package(name).ok_or(Failure::NotFound)?;
        Ok(pkg.clone())
    }
}

impl Backend for PkgBackend {
    fn fetch_index(&mut self) -> Result<(), Failure> {
        let name = self.repo_name().ok_or(Failure::NotFound)?;
        let files = self
            .installer
            .repo_manager()
            .fetch_index_files(&name)
            .map_err(repo_failure)?;
        self.index_files = Some(files);
        Ok(())
    }

    fn check_index(&mut self) -> Result<(), Failure> {
        let name = self.repo_name().ok_or(Failure::NotFound)?;
        let files = self.index_files.take().ok_or(Failure::Invalid)?;
        let now = crate::arch::ArchTimer::uptime_ms();
        self.installer
            .repo_manager()
            .ingest_index(&name, &files.body, files.signature.as_deref(), now)
            .map_err(repo_failure)
    }

    fn download(&mut self, name: &str) -> Result<(), Failure> {
        let pkg = self.lookup(name)?;
        let (status, body) = crate::net::fetch_url(&pkg.download_url).ok_or(Failure::Network)?;
        if status != 200 {
            return Err(Failure::Http(status));
        }
        self.blob = body;
        Ok(())
    }

    fn verify(&mut self, name: &str) -> Result<(), Failure> {
        let checksum = self.lookup(name)?.checksum;
        let blob = core::mem::take(&mut self.blob);
        let package =
            crate::pkg::install::verify_blob(&blob, &checksum).map_err(install_failure)?;
        self.package = Some(package);
        Ok(())
    }

    fn install(&mut self, _name: &str) -> Result<(), Failure> {
        self.refresh_database();
        let package = self.package.take().ok_or(Failure::Invalid)?;
        for dep in &package.metadata.dependencies {
            if !self.installer.database().is_installed(dep) {
                return Err(Failure::MissingDependency(dep.clone()));
            }
        }
        self.installer
            .install_package(package)
            .map_err(install_failure)
    }

    fn remove(&mut self, name: &str) -> Result<(), Failure> {
        self.refresh_database();
        self.installer.remove(name).map_err(install_failure)
    }
}
