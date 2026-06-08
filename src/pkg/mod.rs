//! Package manager for LobsterOS

pub mod install;
pub mod package;
pub mod repo;

#[allow(unused_imports)]
pub use package::{
    InstalledPackage, Package, PackageArch, PackageFile, PackageMetadata, RepoPackage,
    RepositoryIndex, LOBPKG_MAGIC, LOBPKG_VERSION, MAX_PKG_NAME, MAX_PKG_VERSION,
};

#[allow(unused_imports)]
pub use repo::{download_package, verify_checksum, RepoError, RepositoryConfig, RepositoryManager};

#[allow(unused_imports)]
pub use install::{
    DbEntry, InstallError, Installer, PackageDatabase, BIN_DIR, ETC_DIR, INSTALL_PREFIX, LIB_DIR,
    PKG_DB_PATH, SHARE_DIR,
};
