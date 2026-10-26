## Host test suite (Q2 groundwork) — 2026-10-25

`cargo test --features std --target aarch64-unknown-linux-gnu --lib` now passes
133/133 on the Pi 5 host (previously did not compile at all).

Fixed to make host tests possible (all verified against the real target build —
`cargo build --release --target aarch64-unknown-none --features qemu_raspi5b`
stays clean, kernel boots to desktop in QEMU with 0 panics):

### Real kernel bugs found & fixed
- `wifi.rs pbkdf2_hmac_sha1`: block index serialized as usize 8-byte to_be_bytes
  instead of RFC 2898's u32 — every derived WPA2 PMK was silently wrong.
- `gpu.rs parse_detailed_timing`: all EDID 18-byte timing fields decoded with
  from_le_bytes nibble masks — h_active 1920 read as 128 for every real mode.
- `net/ipv4.rs parse()`: read flags/frag from the TOTAL LENGTH field and DSCP
  from the IHL nibble; both checksum serializers used the same wrong offsets
  (ver/ihl/dscp/id/flags all misplaced) — every inbound packet failed checksum
  verification. Also fixed the pos double-advance in the csum skip branch.
- `pkg/package.rs from_db_string`: is_config lost in DB roundtrip (now derived
  from /etc/ prefix); search() made case-insensitive (package-manager convention).
- `sched/mod.rs`: `.Lcoop_resume` was a temporary symbol that fails to link in
  multi-CGU host builds — renamed to the global `COOP_RESUME_ADDR`.
- `apps/calculator.rs`: `5 % 0` panicked the whole kernel (audit Q1a) — guarded,
  plus pure `apply_op()` extracted and unit-tested (3 tests).

### Fabricated test constants corrected (tests lied, code was right)
- lib.rs RFC 2202 HMAC case 1 expected …4edc, real value …be00.
- wifi.rs `hmac_sha1("key","data")` and WPA2 PBKDF2 vectors were invented numbers.

### Test-fixture bugs
- gpu.rs EDID 1920x1080 fixture: byte 4 packed 0x07 instead of 0x71 (4-bit shift).
- ipv4 test built a packet whose bytes didn't match the checksummed header.
- pkg search test expected a package it never added.

### Build fixes (host only, no target impact)
- x86_64/aarch64-linux host builds: gated qemu-only render hooks, naked-asm
  trampolines, tlbi/MMIO/UART touches, global_allocator (std tests use the
  system allocator), duplicate cmd_ifconfig/cmd_ping legacy fns renamed.
- Terminal tests: 10 MB scrollback must be heap-constructed (new_boxed with
  in-place Scrollback::new_in_place) — stack construction overflows test threads.
