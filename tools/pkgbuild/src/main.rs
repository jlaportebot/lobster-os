//! LobsterOS Package Builder
//!
//! Creates .lobpkg package files from a source directory.
//!
//! Package format:
//! - Magic: "LOB\0" (4 bytes)
//! - Version: u16 (1)
//! - Metadata: name, version, arch, description, maintainer, license, homepage,
//!   dependencies, provides, conflicts, replaces, size, download_size, checksum,
//!   build_timestamp, install_script, remove_script, config_files, download_url
//! - File count: u32
//! - File entries: path, mode, uid, gid, size, checksum, is_config, is_symlink, symlink_target
//! - File data: concatenated file contents

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use sha2::{Digest, Sha256};

const LOBPKG_MAGIC: &[u8; 4] = b"LOB\0";
const LOBPKG_VERSION: u16 = 1;

#[derive(Debug, Clone)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PackageArch {
    Aarch64,
    X86_64,
    Armv7,
    Riscv64,
    Any,
}

impl PackageArch {
    fn to_str(&self) -> &'static str {
        match self {
            PackageArch::Aarch64 => "aarch64",
            PackageArch::X86_64 => "x86_64",
            PackageArch::Armv7 => "armv7",
            PackageArch::Riscv64 => "riscv64",
            PackageArch::Any => "any",
        }
    }
}

#[derive(Debug, Clone)]
struct PackageFile {
    path: String,
    mode: u32,
    uid: u32,
    gid: u32,
    size: u64,
    checksum: [u8; 32],
    is_config: bool,
    is_symlink: bool,
    symlink_target: Option<String>,
}

fn write_string<W: Write>(writer: &mut W, s: &str) -> std::io::Result<()> {
    let bytes = s.as_bytes();
    writer.write_all(&(bytes.len() as u32).to_le_bytes())?;
    writer.write_all(bytes)?;
    Ok(())
}

fn write_bytes<W: Write>(writer: &mut W, bytes: &[u8]) -> std::io::Result<()> {
    writer.write_all(&(bytes.len() as u32).to_le_bytes())?;
    writer.write_all(bytes)?;
    Ok(())
}

fn compute_checksum(data: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let result = hasher.finalize();
    let mut checksum = [0u8; 32];
    checksum.copy_from_slice(&result);
    checksum
}

fn build_package(source_dir: &Path, output_path: &Path, metadata: PackageMetadata) -> std::io::Result<()> {
    println!("Building package from {}", source_dir.display());

    // Collect all files
    let mut files = Vec::new();
    let mut file_data = Vec::new();

    for entry in walkdir::WalkDir::new(source_dir).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        let rel_path = path.strip_prefix(source_dir).unwrap();
        let rel_path_str = rel_path.to_string_lossy().replace('\\', "/");

        let data = fs::read(path)?;
        let checksum = compute_checksum(&data);
        let file_metadata = fs::metadata(path)?;

        let is_config = metadata.config_files.contains(&rel_path_str);

        let pkg_file = PackageFile {
            path: rel_path_str,
            mode: 0o644,
            uid: 0,
            gid: 0,
            size: data.len() as u64,
            checksum,
            is_config,
            is_symlink: false,
            symlink_target: None,
        };

        file_data.extend_from_slice(&data);
        files.push(pkg_file);
    }

    // Sort files by path for deterministic output
    files.sort_by(|a, b| a.path.cmp(&b.path));

    // Compute total size
    let total_size: u64 = files.iter().map(|f| f.size).sum();
    let package_checksum = compute_checksum(&file_data);

    // Build package file
    let output_file = File::create(output_path)?;
    let mut writer = BufWriter::new(output_file);

    // Magic
    writer.write_all(LOBPKG_MAGIC)?;

    // Version
    writer.write_all(&LOBPKG_VERSION.to_le_bytes())?;

    // Metadata
    write_string(&mut writer, &metadata.name)?;
    write_string(&mut writer, &metadata.version)?;
    write_string(&mut writer, metadata.arch.to_str())?;
    write_string(&mut writer, &metadata.description)?;
    write_string(&mut writer, &metadata.maintainer)?;
    write_string(&mut writer, &metadata.license)?;
    write_string(&mut writer, &metadata.homepage)?;

    // Dependencies
    writer.write_all(&(metadata.dependencies.len() as u32).to_le_bytes())?;
    for dep in &metadata.dependencies {
        write_string(&mut writer, dep)?;
    }

    // Provides
    writer.write_all(&(metadata.provides.len() as u32).to_le_bytes())?;
    for prov in &metadata.provides {
        write_string(&mut writer, prov)?;
    }

    // Conflicts
    writer.write_all(&(metadata.conflicts.len() as u32).to_le_bytes())?;
    for conf in &metadata.conflicts {
        write_string(&mut writer, conf)?;
    }

    // Replaces
    writer.write_all(&(metadata.replaces.len() as u32).to_le_bytes())?;
    for repl in &metadata.replaces {
        write_string(&mut writer, repl)?;
    }

    // Size, download_size (use computed values)
    writer.write_all(&total_size.to_le_bytes())?;
    writer.write_all(&file_data.len().to_le_bytes())?;

    // Checksum (computed from file_data)
    writer.write_all(&package_checksum)?;

    // Install script
    if let Some(script) = &metadata.install_script {
        write_bytes(&mut writer, script.as_bytes())?;
    } else {
        writer.write_all(&0u32.to_le_bytes())?;
    }

    // Remove script
    if let Some(script) = &metadata.remove_script {
        write_bytes(&mut writer, script.as_bytes())?;
    } else {
        writer.write_all(&0u32.to_le_bytes())?;
    }

    // Config files
    writer.write_all(&(metadata.config_files.len() as u32).to_le_bytes())?;
    for cf in &metadata.config_files {
        write_string(&mut writer, cf)?;
    }

    // Build timestamp
    writer.write_all(&metadata.build_timestamp.to_le_bytes())?;

    // Download URL
    write_string(&mut writer, &metadata.download_url)?;

    // File count
    writer.write_all(&(files.len() as u32).to_le_bytes())?;

    // File entries
    for file in &files {
        write_string(&mut writer, &file.path)?;
        writer.write_all(&file.mode.to_le_bytes())?;
        writer.write_all(&file.uid.to_le_bytes())?;
        writer.write_all(&file.gid.to_le_bytes())?;
        writer.write_all(&file.size.to_le_bytes())?;
        writer.write_all(&file.checksum)?;
        writer.write_all(&[file.is_config as u8])?;
        writer.write_all(&[file.is_symlink as u8])?;

        if file.is_symlink {
            if let Some(target) = &file.symlink_target {
                write_string(&mut writer, target)?;
            } else {
                writer.write_all(&0u32.to_le_bytes())?;
            }
        }
    }

    // File data length
    writer.write_all(&(file_data.len() as u32).to_le_bytes())?;

    // File data
    writer.write_all(&file_data)?;

    writer.flush()?;

    println!("Package written to {}", output_path.display());
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("Usage: pkgbuild <source_dir> <output.lobpkg> [metadata...]");
        eprintln!("Metadata options:");
        eprintln!("  --name <name>");
        eprintln!("  --version <version>");
        eprintln!("  --arch <aarch64|x86_64|armv7|riscv64|any>");
        eprintln!("  --description <desc>");
        eprintln!("  --maintainer <name>");
        eprintln!("  --license <license>");
        eprintln!("  --homepage <url>");
        eprintln!("  --dep <dependency> (can repeat)");
        eprintln!("  --install-script <file>");
        eprintln!("  --remove-script <file>");
        eprintln!("  --config <file> (can repeat)");
        eprintln!("  --download-url <url>");
        return;
    }

    let source_dir = PathBuf::from(&args[1]);
    let output_path = PathBuf::from(&args[2]);

    // Parse metadata from args (simplified)
    let mut metadata = PackageMetadata {
        name: "test-package".to_string(),
        version: "1.0.0".to_string(),
        arch: PackageArch::Aarch64,
        description: "Test package".to_string(),
        maintainer: "LobsterOS Team".to_string(),
        license: "MIT".to_string(),
        homepage: "https://lobsteros.org".to_string(),
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
        download_url: String::new(),
    };

    // Parse remaining args
    let mut i = 3;
    while i < args.len() {
        match args[i].as_str() {
            "--name" => { i += 1; metadata.name = args[i].clone(); }
            "--version" => { i += 1; metadata.version = args[i].clone(); }
            "--arch" => {
                i += 1;
                metadata.arch = match args[i].as_str() {
                    "aarch64" => PackageArch::Aarch64,
                    "x86_64" => PackageArch::X86_64,
                    "armv7" => PackageArch::Armv7,
                    "riscv64" => PackageArch::Riscv64,
                    _ => PackageArch::Any,
                };
            }
            "--description" => { i += 1; metadata.description = args[i].clone(); }
            "--maintainer" => { i += 1; metadata.maintainer = args[i].clone(); }
            "--license" => { i += 1; metadata.license = args[i].clone(); }
            "--homepage" => { i += 1; metadata.homepage = args[i].clone(); }
            "--dep" => { i += 1; metadata.dependencies.push(args[i].clone()); }
            "--install-script" => { i += 1; metadata.install_script = Some(fs::read_to_string(&args[i]).unwrap()); }
            "--remove-script" => { i += 1; metadata.remove_script = Some(fs::read_to_string(&args[i]).unwrap()); }
            "--config" => { i += 1; metadata.config_files.push(args[i].clone()); }
            "--download-url" => { i += 1; metadata.download_url = args[i].clone(); }
            _ => {}
        }
        i += 1;
    }

    // Compute checksum of file data (placeholder)
    metadata.checksum = [0; 32];
    metadata.size = 0;
    metadata.download_size = 0;
    metadata.build_timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    if let Err(e) = build_package(&source_dir, &output_path, metadata) {
        eprintln!("Error building package: {}", e);
        std::process::exit(1);
    }
}