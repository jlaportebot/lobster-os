#!/usr/bin/env bash
# Build the freestanding EL0 test programs (spin, spin_print, getrandom_test, efault,
# two_apps_a, two_apps_b) into <outdir>
# (default: tools/build). Small page alignment keeps the ELF a few hundred
# bytes, like the other EL0 test binaries. Copy them onto the guest disk
# (e.g. /sd/bin/spin) with debugfs + `lobstervm disk import`.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
OUT="${1:-$HERE/build}"
CC="${CC:-aarch64-linux-gnu-gcc}"
mkdir -p "$OUT"
for prog in spin spin_print getrandom_test efault; do
    "$CC" -nostdlib -static -ffreestanding -O2 -Wall -Wextra \
        -Wl,-z,max-page-size=0x40 -Wl,--build-id=none \
        -T "$HERE/lobster_user.ld" -o "$OUT/$prog" "$HERE/$prog.c"
    echo "built $OUT/$prog ($(stat -c %s "$OUT/$prog") bytes)"
done
for role in a b; do
    upper="$(echo "$role" | tr a-z A-Z)"
    "$CC" -nostdlib -static -ffreestanding -O2 -Wall -Wextra -DROLE_"$upper" \
        -Wl,-z,max-page-size=0x40 -Wl,--build-id=none \
        -T "$HERE/lobster_user.ld" -o "$OUT/two_apps_$role" "$HERE/two_apps.c"
    echo "built $OUT/two_apps_$role ($(stat -c %s "$OUT/two_apps_$role") bytes)"
done
