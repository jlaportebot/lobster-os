#!/usr/bin/env bash
# Build the QEMU-feature kernel and install it where the emulator looks for it.
#   tools/install-kernel.sh            build + install
#   EMULATOR_REPO=/path tools/install-kernel.sh
set -euo pipefail
REPO="$(cd "$(dirname "$(readlink -f "$0")")/.." && pwd)"
EMULATOR_REPO="${EMULATOR_REPO:-$REPO/../../pi5-emulator-repo}"
TARGET_ELF="$REPO/target/aarch64-unknown-none/release/lobster-os"

cd "$REPO"
cargo build --release --target aarch64-unknown-none --features qemu_raspi5b
mkdir -p build
cp "$TARGET_ELF" build/kernel8.elf
strip --strip-unneeded build/kernel8.elf
objcopy -O binary build/kernel8.elf build/kernel8.img
if [[ -d "$EMULATOR_REPO" ]]; then
    cp build/kernel8.img "$EMULATOR_REPO/kernel8.img"
    echo "installed $(stat -c %s build/kernel8.img) bytes -> $EMULATOR_REPO/kernel8.img"
else
    echo "built build/kernel8.img (emulator repo not found at $EMULATOR_REPO)"
fi
