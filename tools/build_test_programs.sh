#!/usr/bin/env bash
# Build the freestanding EL0 test programs (spin, spin_print, getrandom_test) into <outdir>
# (default: tools/build). Small page alignment keeps the ELF a few hundred
# bytes, like the other EL0 test binaries. Copy them onto the guest disk
# (e.g. /sd/bin/spin) with debugfs + `lobstervm disk import`.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
OUT="${1:-$HERE/build}"
CC="${CC:-aarch64-linux-gnu-gcc}"
mkdir -p "$OUT"
for prog in spin spin_print getrandom_test; do
    "$CC" -nostdlib -static -ffreestanding -O2 -Wall -Wextra \
        -Wl,-z,max-page-size=0x40 -Wl,--build-id=none \
        -T "$HERE/lobster_user.ld" -o "$OUT/$prog" "$HERE/$prog.c"
    echo "built $OUT/$prog ($(stat -c %s "$OUT/$prog") bytes)"
done
