#!/usr/bin/env python3
"""LobsterOS C Compile Server.

Accepts requests like:
  GET /compile/<filename>.c

Reads the C file from SOURCE_DIR, compiles it with aarch64-linux-gnu-gcc
using the LobsterOS linker script, strips it, and returns the ELF binary.

Also supports:
  GET /hello_lobster  — serves the pre-compiled binary directly

Usage:
  python3 compile_server.py [--port 8890] [--source-dir /path/to/sources]
"""

import http.server
import subprocess
import os
import sys
import tempfile
import time
import argparse
import urllib.parse

GCC = "aarch64-linux-gnu-gcc"
STRIP = "aarch64-linux-gnu-strip"
OBJCOPY = "aarch64-linux-gnu-objcopy"
LINKER_SCRIPT = None  # Set in main()
DEFAULT_BINARY_DIR = "/mnt/data/sd-overflow/LobsterOS/lobster-os/tools"

class CompileHandler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        parsed = urllib.parse.urlparse(self.path)
        path = parsed.path

        # Serve pre-compiled binaries directly
        if path.startswith("/hello_lobster"):
            self.serve_file(os.path.join(DEFAULT_BINARY_DIR, "hello_lobster"))
            return

        # Compile endpoint: /compile/<filename>
        if path.startswith("/compile/"):
            source_name = path[len("/compile/"):]
            self.handle_compile(source_name)
            return

        # Health check
        if path == "/" or path == "/health":
            self.send_response(200)
            self.send_header("Content-Type", "text/plain")
            self.end_headers()
            self.wfile.write(b"LobsterOS Compile Server ready\n")
            return

        self.send_error(404, f"Not found: {path}")

    def serve_file(self, filepath):
        if not os.path.exists(filepath):
            self.send_error(404, f"File not found: {filepath}")
            return
        with open(filepath, "rb") as f:
            data = f.read()
        self.send_response(200)
        self.send_header("Content-Type", "application/octet-stream")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)
        print(f"  Served {filepath} ({len(data)} bytes)")

    def handle_compile(self, source_name):
        source_name = urllib.parse.unquote(source_name)
        # Look for the source file in source dirs
        source_path = None
        for d in [self.server.source_dir, DEFAULT_BINARY_DIR, os.getcwd()]:
            candidate = os.path.join(d, source_name)
            if os.path.exists(candidate):
                source_path = candidate
                break

        if not source_path:
            # Could be inline source (base64 encoded)
            self.send_error(404, f"Source not found: {source_name}")
            return

        print(f"\n[compile] {source_path}")

        # Compile in a temp directory
        with tempfile.NamedTemporaryFile(suffix=".elf", delete=False) as tmp:
            output_path = tmp.name

        try:
            cmd = [
                GCC, "-nostdlib", "-static", "-ffreestanding", "-O2",
                "-Wl,--build-id=none", "-Wl,-z,max-page-size=0x40",
                "-T", LINKER_SCRIPT,
                "-o", output_path, source_path
            ]
            print(f"  Running: {' '.join(cmd)}")
            result = subprocess.run(cmd, capture_output=True, text=True, timeout=30)

            if result.returncode != 0:
                print(f"  Compile FAILED: {result.stderr}")
                self.send_response(500)
                self.send_header("Content-Type", "text/plain")
                self.end_headers()
                self.wfile.write(f"Compile error:\n{result.stderr}\n".encode())
                return

            # Strip debug symbols and remove eh_frame
            subprocess.run([STRIP, "--strip-all", output_path],
                         capture_output=True, timeout=10)
            subprocess.run([OBJCOPY, "--remove-section", ".eh_frame",
                          "--remove-section", ".eh_frame_hdr",
                          "--remove-section", ".comment", output_path],
                         capture_output=True, timeout=10)

            # Read the final binary
            with open(output_path, "rb") as f:
                binary_data = f.read()

            print(f"  Compiled OK: {len(binary_data)} bytes")

            self.send_response(200)
            self.send_header("Content-Type", "application/octet-stream")
            self.send_header("Content-Length", str(len(binary_data)))
            self.end_headers()
            self.wfile.write(binary_data)

        except Exception as e:
            print(f"  Error: {e}")
            self.send_error(500, str(e))
        finally:
            os.unlink(output_path)

    def log_message(self, format, *args):
        print(f"[{time.strftime('%H:%M:%S')}] {format % args}")


def main():
    global LINKER_SCRIPT
    parser = argparse.ArgumentParser(description="LobsterOS C Compile Server")
    parser.add_argument("--port", type=int, default=8890)
    parser.add_argument("--source-dir", default="/mnt/data/sd-overflow/LobsterOS/lobster-os/tools")
    parser.add_argument("--linker-script", default="/mnt/data/sd-overflow/LobsterOS/lobster-os/tools/lobster_user.ld")
    args = parser.parse_args()

    LINKER_SCRIPT = args.linker_script

    server = http.server.HTTPServer(("0.0.0.0", args.port), CompileHandler)
    server.source_dir = args.source_dir

    print(f"LobsterOS C Compile Server")
    print(f"  Port: {args.port}")
    print(f"  Source dir: {args.source_dir}")
    print(f"  Linker script: {LINKER_SCRIPT}")
    print(f"  Endpoints:")
    print(f"    GET /compile/<file.c>  — Compile C source → ELF binary")
    print(f"    GET /hello_lobster      — Serve pre-compiled binary")
    print(f"    GET /health             — Health check")
    print(f"  Waiting for requests...\n")

    server.serve_forever()


if __name__ == "__main__":
    main()
