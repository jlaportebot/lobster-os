#!/usr/bin/env python3
"""Build .lobpkg package files for LobsterOS"""
import os
import sys
import struct
import hashlib

def write_string(f, s):
    b = s.encode('utf-8')
    f.write(struct.pack('<I', len(b)))
    f.write(b)

def main():
    if len(sys.argv) != 3:
        print("Usage: build_lobpkg.py <package_dir> <output.lobpkg>")
        sys.exit(1)
    
    PKG_DIR = sys.argv[1]
    OUTPUT = sys.argv[2]
    
    # Read package metadata from package.config
    config_path = os.path.join(PKG_DIR, "package.config")
    if not os.path.exists(config_path):
        print(f"Error: {config_path} not found")
        sys.exit(1)
    
    # Parse package.config
    meta = {}
    with open(config_path) as f:
        for line in f:
            line = line.strip()
            if line and not line.startswith('#') and '=' in line:
                k, v = line.split('=', 1)
                meta[k.strip()] = v.strip().strip('"')
    
    NAME = meta.get('name', 'unknown')
    VERSION = meta.get('version', '0.0.0')
    ARCH = meta.get('arch', 'aarch64')
    DESCRIPTION = meta.get('description', '')
    MAINTAINER = meta.get('maintainer', '')
    LICENSE = meta.get('license', '')
    HOMEPAGE = meta.get('homepage', '')
    DEPENDENCIES = meta.get('dependencies', '')
    SIZE = int(meta.get('size', '0'))
    DOWNLOAD_SIZE = int(meta.get('download_size', '0'))
    BUILD_TIMESTAMP = int(meta.get('build_timestamp', '0'))
    DOWNLOAD_URL = meta.get('download_url', '')
    
    # Collect all files
    files = []
    file_data = b''
    files_dir = os.path.join(PKG_DIR, "files")
    if os.path.exists(files_dir):
        for root, dirs, filenames in os.walk(files_dir):
            for fname in filenames:
                fpath = os.path.join(root, fname)
                rel_path = os.path.relpath(fpath, files_dir)
                stat = os.stat(fpath)
                with open(fpath, "rb") as f:
                    data = f.read()
                checksum = hashlib.sha256(data).digest()
                files.append({
                    "path": rel_path,
                    "mode": stat.st_mode & 0o777,
                    "uid": stat.st_uid,
                    "gid": stat.st_gid,
                    "size": len(data),
                    "checksum": checksum,
                    "is_config": False,
                    "is_symlink": False,
                    "symlink_target": None,
                    "data": data
                })
                file_data += data
    
    def write_string(f, s):
        b = s.encode('utf-8')
        f.write(struct.pack('<I', len(b)))
        f.write(b)
    
    # Build output
    with open(OUTPUT, "wb") as out:
        # Magic + version
        out.write(b'LOB\0')
        out.write(struct.pack('<H', 1))
        
        # Metadata strings
        write_string(out, NAME)
        write_string(out, VERSION)
        write_string(out, ARCH)
        write_string(out, DESCRIPTION)
        write_string(out, MAINTAINER)
        write_string(out, LICENSE)
        write_string(out, HOMEPAGE)
        
        # Dependencies
        deps = DEPENDENCIES.split(',') if DEPENDENCIES else []
        out.write(struct.pack('<I', len(deps)))
        for d in deps:
            write_string(out, d)
        
        # Size fields
        out.write(struct.pack('<Q', SIZE))
        out.write(struct.pack('<Q', DOWNLOAD_SIZE))
        
        # Checksum (placeholder)
        out.write(b'\0' * 32)
        
        out.write(struct.pack('<Q', BUILD_TIMESTAMP))
        
        # Install/remove scripts (none)
        out.write(struct.pack('<B', 0))
        out.write(struct.pack('<B', 0))
        
        # Config files
        out.write(struct.pack('<I', 0))
        
        # Download URL
        write_string(out, DOWNLOAD_URL)
        
        # File count
        out.write(struct.pack('<I', len(files)))
        
        # File entries
        for f in files:
            write_string(out, f["path"])
            out.write(struct.pack('<I', f["mode"]))
            out.write(struct.pack('<I', f["uid"]))
            out.write(struct.pack('<I', f["gid"]))
            out.write(struct.pack('<Q', f["size"]))
            out.write(f["checksum"])
            out.write(struct.pack('<B', 1 if f["is_config"] else 0))
            out.write(struct.pack('<B', 1 if f["is_symlink"] else 0))
            if f["is_symlink"]:
                write_string(out, f["symlink_target"] or "")
        
        # File data
        out.write(struct.pack('<I', len(file_data)))
        out.write(file_data)
    
    print(f"Package built: {OUTPUT}")

if __name__ == "__main__":
    main()