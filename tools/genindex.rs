//! Repository index generator for LobsterOS package manager
//!
//! This tool creates an index.json file from a directory of .lobpkg files.
//! Run with: cargo run --bin genindex -- <packages-dir> <repo-name> <repo-url> <output.json>

use std::fs;
use std::io::Read;
use std::path::Path;

const LOBPKG_MAGIC: &[u8; 4] = b"LOB\0";
const LOBPKG_VERSION: u16 = 1;

#[derive(serde::Serialize)]
struct RepoPackage {
    name: String,
    version: String,
    arch: String,
    description: String,
    dependencies: Vec<String>,
    size: u64,
    checksum: String, // hex
    download_url: String,
}

#[derive(serde::Serialize)]
struct RepositoryIndex {
    name: String,
    url: String,
    packages: Vec<RepoPackage>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 5 {
        eprintln!("Usage: genindex <packages-dir> <repo-name> <repo-url> <output.json>");
        std::process::exit(1);
    }

    let packages_dir = &args[1];
    let repo_name = &args[2];
    let repo_url = &args[3];
    let output_path = &args[4];

    let mut packages = Vec::new();

    for entry in fs::read_dir(packages_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().map_or(false, |ext| ext == "lobpkg") {
            if let Ok(pkg) = parse_package(&path) {
                packages.push(pkg);
            }
        }
    }

    packages.sort_by(|a, b| a.name.cmp(&b.name));

    let index = RepositoryIndex {
        name: repo_name.clone(),
        url: repo_url.clone(),
        packages,
    };

    let json = serde_json::to_string_pretty(&index)?;
    fs::write(output_path, json)?;

    println!("Generated index with {} packages at {}", index.packages.len(), output_path);
    Ok(())
}

fn parse_package(path: &Path) -> Result<RepoPackage, Box<dyn std::error::Error>> {
    let mut file = fs::File::open(path)?;
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer)?;

    // Verify magic
    if buffer.len() < 6 || &buffer[0..4] != LOBPKG_MAGIC {
        return Err("Invalid magic".into());
    }

    let version = u16::from_le_bytes([buffer[4], buffer[5]]);
    if version != LOBPKG_VERSION {
        return Err("Unsupported version".into());
    }

    let mut offset = 6;

    // Read metadata strings
    let read_string = |buf: &[u8], offset: &mut usize| -> Result<String, Box<dyn std::error::Error>> {
        if *offset + 4 > buf.len() {
            return Err("Buffer too short for string length".into());
        }
        let len = u32::from_le_bytes([
            buf[*offset],
            buf[*offset + 1],
            buf[*offset + 2],
            buf[*offset + 3],
        ]) as usize;
        *offset += 4;
        if *offset + len > buf.len() {
            return Err("Buffer too short for string data".into());
        }
        let s = std::str::from_utf8(&buf[*offset..*offset + len])?.to_string();
        *offset += len;
        Ok(s)
    };

    let name = read_string(&buffer, &mut offset)?;
    let version_str = read_string(&buffer, &mut offset)?;
    let arch = read_string(&buffer, &mut offset)?;
    let description = read_string(&buffer, &mut offset)?;
    let _maintainer = read_string(&buffer, &mut offset)?;
    let _license = read_string(&buffer, &mut offset)?;
    let _homepage = read_string(&buffer, &mut offset)?;

    // dependencies
    let dep_count = read_u32(&buffer, &mut offset)? as usize;
    let mut dependencies = Vec::new();
    for _ in 0..dep_count {
        dependencies.push(read_string(&buffer, &mut offset)?);
    }

    // provides (skip)
    let provide_count = read_u32(&buffer, &mut offset)? as usize;
    for _ in 0..provide_count {
        read_string(&buffer, &mut offset)?;
    }

    // conflicts (skip)
    let conflict_count = read_u32(&buffer, &mut offset)? as usize;
    for _ in 0..conflict_count {
        read_string(&buffer, &mut offset)?;
    }

    // replaces (skip)
    let replace_count = read_u32(&buffer, &mut offset)? as usize;
    for _ in 0..replace_count {
        read_string(&buffer, &mut offset)?;
    }

    // size, download_size
    let size = read_u64(&buffer, &mut offset)?;
    let _download_size = read_u64(&buffer, &mut offset)?;

    // checksum
    if offset + 32 > buffer.len() {
        return Err("Buffer too short for checksum".into());
    }
    let mut checksum = [0u8; 32];
    checksum.copy_from_slice(&buffer[offset..offset + 32]);
    offset += 32;
    let checksum_hex = checksum.iter().map(|b| format!("{:02x}", b)).collect::<String>();

    // install_script (skip)
    let script_len = read_u32(&buffer, &mut offset)? as usize;
    offset += script_len;

    // remove_script (skip)
    let script_len = read_u32(&buffer, &mut offset)? as usize;
    offset += script_len;

    // config_files (skip)
    let config_count = read_u32(&buffer, &mut offset)? as usize;
    for _ in 0..config_count {
        read_string(&buffer, &mut offset)?;
    }

    // build_timestamp (skip)
    offset += 8;

    // download_url
    let download_url = read_string(&buffer, &mut offset)?;

    Ok(RepoPackage {
        name,
        version: version_str,
        arch,
        description,
        dependencies,
        size,
        checksum: checksum_hex,
        download_url,
    })
}

fn read_u32(buf: &[u8], offset: &mut usize) -> Result<u32, Box<dyn std::error::Error>> {
    if *offset + 4 > buf.len() {
        return Err("Buffer too short for u32".into());
    }
    let val = u32::from_le_bytes([
        buf[*offset],
        buf[*offset + 1],
        buf[*offset + 2],
        buf[*offset + 3],
    ]);
    *offset += 4;
    Ok(val)
}

fn read_u64(buf: &[u8], offset: &mut usize) -> Result<u64, Box<dyn std::error::Error>> {
    if *offset + 8 > buf.len() {
        return Err("Buffer too short for u64".into());
    }
    let val = u64::from_le_bytes([
        buf[*offset],
        buf[*offset + 1],
        buf[*offset + 2],
        buf[*offset + 3],
        buf[*offset + 4],
        buf[*offset + 5],
        buf[*offset + 6],
        buf[*offset + 7],
    ]);
    *offset += 8;
    Ok(val)
}