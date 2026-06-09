//! Package builder for LobsterOS .lobpkg files
//!
//! This tool creates .lobpkg package files from a source directory and metadata.
//! Run with: cargo run --bin pkgbuild -- <package-spec.toml> <output.lobpkg>
//!
//! Package spec format (TOML):
//! ```
//! name = "coreutils"
//! version = "9.4"
//! arch = "aarch64"
//! description = "GNU core utilities"
//! maintainer = "LobsterOS Team <team@lobsteros.org>"
//! license = "GPL-3.0-or-later"
//! homepage = "https://www.gnu.org/software/coreutils/"
//! dependencies = []
//! provides = ["coreutils"]
//! conflicts = []
//! replaces = []
//! install_script = """
//! #!/bin/sh
//! # Post-install script
//! ```
//! remove_script = """
//! #!/bin/sh
//! # Pre-remove script
//! ```
//! config_files = ["/etc/coreutils.conf"]
//! ```
//!
//! The source directory should mirror the target filesystem layout:
//! ```
//! pkgbuild/
//!   usr/
//!     bin/
//!       ls
//!       cat
//!     lib/
//!     share/
//!   etc/
//!     coreutils.conf
//! ```
//!
//! Files under usr/bin, usr/lib, etc. will be installed to those paths.
//! Symlinks are preserved. File permissions are read from the filesystem.

use std::fs;
use std::io::{self, Read, Write};
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use sha2::{Digest, Sha256};

const LOBPKG_MAGIC: &[u8; 4] = b"LOB\0";
const LOBPKG_VERSION: u16 = 1;

#[derive(Debug, serde::Deserialize)]
struct PackageSpec {
    name: String,
    version: String,
    arch: String,
    description: String,
    maintainer: String,
    license: String,
    homepage: String,
    #[serde(default)]
    dependencies: Vec<String>,
    #[serde(default)]
    provides: Vec<String>,
    #[serde(default)]
    conflicts: Vec<String>,
    #[serde(default)]
    replaces: Vec<String>,
    #[serde(default)]
    install_script: Option<String>,
    #[serde(default)]
    remove_script: Option<String>,
    #[serde(default)]
    config_files: Vec<String>,
    #[serde(default)]
    download_url: String,
}

#[derive(Debug)]
struct PackageFile {
    path: String,
    mode: u32,
    uid: u32,
    gid: u32,
    size: u64,
    checksum: [u8; 32],
    data: Vec<u8>,
    is_config: bool,
    is_symlink: bool,
    symlink_target: Option<String>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("Usage: pkgbuild <package-spec.toml> <output.lobpkg>");
        std::process::exit(1);
    }

    let spec_path = &args[1];
    let output_path = &args[2];

    // Read and parse package spec
    let spec_content = fs::read_to_string(spec_path)?;
    let spec: PackageSpec = toml::from_str(&spec_content)?;

    // Determine source directory (same directory as spec, with same name as package)
    let spec_dir = Path::new(spec_path).parent().unwrap_or(Path::new("."));
    let source_dir = spec_dir.join(&spec.name);

    if !source_dir.exists() {
        eprintln!("Source directory not found: {}", source_dir.display());
        eprintln!(
            "Expected layout: <spec-dir>/{}/ (mirroring target filesystem)",
            spec.name
        );
        std::process::exit(1);
    }

    println!("Building package: {} {}", spec.name, spec.version);
    println!("Source: {}", source_dir.display());
    println!("Output: {}", output_path);

    // Collect all files from source directory
    let files = collect_files(&source_dir, &spec.config_files)?;

    // Build metadata
    let metadata = build_metadata(&spec, &files)?;

    // Write .lobpkg file
    write_package(output_path, &metadata, &files)?;

    println!("Package built successfully!");
    Ok(())
}

fn collect_files(
    source_dir: &Path,
    config_files: &[String],
) -> Result<Vec<PackageFile>, Box<dyn std::error::Error>> {
    let mut files = Vec::new();
    let config_set: std::collections::HashSet<_> = config_files.iter().cloned().collect();

    for entry in walkdir::WalkDir::new(source_dir)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if !path.is_file() && !is_symlink(path) {
            continue;
        }

        // Get relative path from source_dir
        let rel_path = path.strip_prefix(source_dir)?;
        let target_path = format!("/{}", rel_path.to_string_lossy().replace('\\', "/"));

        let metadata = fs::metadata(path)?;
        let is_config = config_set.contains(&target_path);

        let mut file_data = Vec::new();
        let (is_symlink, symlink_target) = if is_symlink(path) {
            let target = fs::read_link(path)?;
            (true, Some(target.to_string_lossy().to_string()))
        } else {
            fs::File::open(path)?.read_to_end(&mut file_data)?;
            (false, None)
        };

        // Calculate checksum of file data
        let mut hasher = Sha256::new();
        hasher.update(&file_data);
        let mut checksum = [0u8; 32];
        checksum.copy_from_slice(&hasher.finalize()[..]);

        files.push(PackageFile {
            path: target_path,
            mode: metadata.mode() & 0o7777,
            uid: metadata.uid(),
            gid: metadata.gid(),
            size: file_data.len() as u64,
            checksum,
            data: file_data,
            is_config,
            is_symlink,
            symlink_target,
        });
    }

    // Sort files by path for deterministic output
    files.sort_by(|a, b| a.path.cmp(&b.path));

    println!("Collected {} files", files.len());
    Ok(files)
}

fn is_symlink(path: &Path) -> bool {
    fs::read_link(path).is_ok()
}

fn build_metadata(
    spec: &PackageSpec,
    files: &[PackageFile],
) -> Result<PackageMetadata, Box<dyn std::error::Error>> {
    // Calculate total size and checksum of all file data concatenated
    let mut total_size = 0u64;
    let mut all_data = Vec::new();
    for file in files {
        total_size += file.size;
        all_data.extend(&file.data);
    }

    let mut hasher = Sha256::new();
    hasher.update(&all_data);
    let mut checksum = [0u8; 32];
    checksum.copy_from_slice(&hasher.finalize()[..]);

    let arch = match spec.arch.as_str() {
        "aarch64" => PackageArch::Aarch64,
        "x86_64" => PackageArch::X86_64,
        "armv7" => PackageArch::Armv7,
        "riscv64" => PackageArch::Riscv64,
        _ => PackageArch::Any,
    };

    Ok(PackageMetadata {
        name: spec.name.clone(),
        version: spec.version.clone(),
        arch,
        description: spec.description.clone(),
        maintainer: spec.maintainer.clone(),
        license: spec.license.clone(),
        homepage: spec.homepage.clone(),
        dependencies: spec.dependencies.clone(),
        provides: spec.provides.clone(),
        conflicts: spec.conflicts.clone(),
        replaces: spec.replaces.clone(),
        size: total_size,
        download_size: all_data.len() as u64,
        checksum,
        build_timestamp: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs(),
        install_script: spec.install_script.clone(),
        remove_script: spec.remove_script.clone(),
        config_files: spec.config_files.clone(),
        download_url: spec.download_url.clone(),
    })
}

fn write_package(
    output_path: &str,
    metadata: &PackageMetadata,
    files: &[PackageFile],
) -> Result<(), Box<dyn std::error::Error>> {
    let mut out = fs::File::create(output_path)?;

    // Magic + version
    out.write_all(LOBPKG_MAGIC)?;
    out.write_all(&LOBPKG_VERSION.to_le_bytes())?;

    // Write metadata
    write_metadata(&mut out, metadata)?;

    // Write file count
    out.write_all(&(files.len() as u32).to_le_bytes())?;

    // Write file entries
    for file in files {
        write_file_entry(&mut out, file)?;
    }

    // Write concatenated file data
    let total_data_len: u32 = files.iter().map(|f| f.data.len() as u32).sum();
    out.write_all(&total_data_len.to_le_bytes())?;

    for file in files {
        out.write_all(&file.data)?;
    }

    out.flush()?;
    Ok(())
}

fn write_metadata(
    out: &mut fs::File,
    meta: &PackageMetadata,
) -> Result<(), Box<dyn std::error::Error>> {
    let write_string = |out: &mut fs::File, s: &str| -> io::Result<()> {
        let bytes = s.as_bytes();
        out.write_all(&(bytes.len() as u32).to_le_bytes())?;
        out.write_all(bytes)
    };

    write_string(out, &meta.name)?;
    write_string(out, &meta.version)?;
    write_string(out, &meta.arch.to_string())?;
    write_string(out, &meta.description)?;
    write_string(out, &meta.maintainer)?;
    write_string(out, &meta.license)?;
    write_string(out, &meta.homepage)?;

    // dependencies
    out.write_all(&(meta.dependencies.len() as u32).to_le_bytes())?;
    for dep in &meta.dependencies {
        write_string(out, dep)?;
    }

    // provides
    out.write_all(&(meta.provides.len() as u32).to_le_bytes())?;
    for p in &meta.provides {
        write_string(out, p)?;
    }

    // conflicts
    out.write_all(&(meta.conflicts.len() as u32).to_le_bytes())?;
    for c in &meta.conflicts {
        write_string(out, c)?;
    }

    // replaces
    out.write_all(&(meta.replaces.len() as u32).to_le_bytes())?;
    for r in &meta.replaces {
        write_string(out, r)?;
    }

    // size, download_size
    out.write_all(&meta.size.to_le_bytes())?;
    out.write_all(&meta.download_size.to_le_bytes())?;

    // checksum
    out.write_all(&meta.checksum)?;

    // install_script
    if let Some(script) = &meta.install_script {
        write_string(out, script)?;
    } else {
        out.write_all(&0u32.to_le_bytes())?;
    }

    // remove_script
    if let Some(script) = &meta.remove_script {
        write_string(out, script)?;
    } else {
        out.write_all(&0u32.to_le_bytes())?;
    }

    // config_files
    out.write_all(&(meta.config_files.len() as u32).to_le_bytes())?;
    for cf in &meta.config_files {
        write_string(out, cf)?;
    }

    // build_timestamp
    out.write_all(&meta.build_timestamp.to_le_bytes())?;

    // download_url
    write_string(out, &meta.download_url)?;

    Ok(())
}

fn write_file_entry(
    out: &mut fs::File,
    file: &PackageFile,
) -> Result<(), Box<dyn std::error::Error>> {
    let write_string = |out: &mut fs::File, s: &str| -> io::Result<()> {
        let bytes = s.as_bytes();
        out.write_all(&(bytes.len() as u32).to_le_bytes())?;
        out.write_all(bytes)
    };

    // path
    write_string(out, &file.path)?;

    // mode, uid, gid, size
    out.write_all(&file.mode.to_le_bytes())?;
    out.write_all(&file.uid.to_le_bytes())?;
    out.write_all(&file.gid.to_le_bytes())?;
    out.write_all(&file.size.to_le_bytes())?;

    // checksum
    out.write_all(&file.checksum)?;

    // is_config, is_symlink
    out.write_all(&[if file.is_config { 1 } else { 0 }])?;
    out.write_all(&[if file.is_symlink { 1 } else { 0 }])?;

    // symlink_target (optional)
    if file.is_symlink {
        if let Some(target) = &file.symlink_target {
            write_string(out, target)?;
        } else {
            out.write_all(&0u32.to_le_bytes())?;
        }
    }

    Ok(())
}

#[derive(Debug)]
struct PackageMetadata {
    name: String,
    version: String,
    arch: PackageArch,
    description: String,
    maintainer: String,
    license: String,
    homepage: String,
    dependencies: Vec<String>,
    provides: Vec<String>,
    conflicts: Vec<String>,
    replaces: Vec<String>,
    size: u64,
    download_size: u64,
    checksum: [u8; 32],
    build_timestamp: u64,
    install_script: Option<String>,
    remove_script: Option<String>,
    config_files: Vec<String>,
    download_url: String,
}

#[derive(Debug, Clone, Copy)]
enum PackageArch {
    Aarch64,
    X86_64,
    Armv7,
    Riscv64,
    Any,
}

impl std::fmt::Display for PackageArch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PackageArch::Aarch64 => write!(f, "aarch64"),
            PackageArch::X86_64 => write!(f, "x86_64"),
            PackageArch::Armv7 => write!(f, "armv7"),
            PackageArch::Riscv64 => write!(f, "riscv64"),
            PackageArch::Any => write!(f, "any"),
        }
    }
}