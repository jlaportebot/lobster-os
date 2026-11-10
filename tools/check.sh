#!/usr/bin/env bash
# Local gate, same steps as CI plus the QEMU feature set. Run before every push.
# The QEMU-feature kernel is rebuilt and installed LAST: bare and qemu builds
# share one target directory, and leaving the bare ELF in place breaks boots.
set -euo pipefail
REPO="$(cd "$(dirname "$(readlink -f "$0")")/.." && pwd)"
HOST_TARGET="$(rustc -vV | sed -n 's/^host: //p')"
cd "$REPO"

step() { printf '\n== %s ==\n' "$1"; }

step "format"
cargo fmt --all -- --check
step "clippy (hardware feature set)"
cargo clippy --target aarch64-unknown-none --release -- -D warnings
step "clippy (qemu_raspi5b)"
cargo clippy --target aarch64-unknown-none --release --features qemu_raspi5b -- -D warnings
step "build (hardware feature set)"
cargo build --target aarch64-unknown-none --release
step "host unit tests ($HOST_TARGET)"
RUSTFLAGS="" cargo test --features std --target "$HOST_TARGET" --lib
step "build + install (qemu_raspi5b)"
tools/install-kernel.sh
echo
echo "CHECK OK"
