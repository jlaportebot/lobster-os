#!/bin/bash
# Build .lobpkg package files for LobsterOS
# Usage: ./build_lobpkg.sh <package_dir> <output.lobpkg>

set -e

PKG_DIR="$1"
OUTPUT="$2"

if [ -z "$PKG_DIR" ] || [ -z "$OUTPUT" ]; then
    echo "Usage: $0 <package_dir> <output.lobpkg>"
    exit 1
fi

python3 /mnt/data/sd-overflow/LobsterOS/lobster-os/tools/build_lobpkg.py "$PKG_DIR" "$OUTPUT"
echo "Built $OUTPUT"