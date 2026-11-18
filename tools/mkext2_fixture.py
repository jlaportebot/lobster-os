#!/usr/bin/env python3
"""Build sparse ext2 test fixtures for src/fs/ext2_tests.rs.

Usage: mkext2_fixture.py <block_size 1024|4096> <size_mib> <out.blocks>

Runs the host mke2fs with the same feature set as the LobsterOS system volume,
then stores only the non-zero 1 KiB chunks:
  magic 'EXT2FX01' | u64 total_bytes | u32 chunk_size | u32 count
  then count * (u32 chunk_index | chunk_size bytes), all little-endian.
"""
import os, struct, subprocess, sys, tempfile

CHUNK = 1024
FEATURES = "none,ext_attr,resize_inode,dir_index,filetype,sparse_super,large_file"

def main():
    block_size, size_mib, out = int(sys.argv[1]), int(sys.argv[2]), sys.argv[3]
    total = size_mib * 1024 * 1024
    group_blocks = block_size * 8
    with tempfile.TemporaryDirectory() as tmp:
        img = os.path.join(tmp, "img")
        subprocess.run(["mke2fs", "-q", "-t", "ext2", "-o", "Linux", "-r", "1",
                        "-b", str(block_size), "-I", "256", "-g", str(group_blocks),
                        "-N", "1024", "-m", "0", "-L", "TEST", "-O", FEATURES,
                        "-E", "root_owner=0:0", img, str(total // block_size)],
                       check=True)
        chunks = []
        with open(img, "rb") as f:
            index = 0
            while True:
                data = f.read(CHUNK)
                if not data:
                    break
                if data.strip(b"\0"):
                    chunks.append((index, data))
                index += 1
    with open(out, "wb") as f:
        f.write(b"EXT2FX01" + struct.pack("<QII", total, CHUNK, len(chunks)))
        for index, data in chunks:
            f.write(struct.pack("<I", index) + data)
    print(f"{out}: {len(chunks)} chunks, {os.path.getsize(out)} bytes")

if __name__ == "__main__":
    main()
