# LobsterOS

[![CI](https://github.com/jlaportebot/lobster-os/actions/workflows/ci.yml/badge.svg)](https://github.com/jlaportebot/lobster-os/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Rust](https://img.shields.io/badge/rust-nightly-orange.svg)](https://www.rust-lang.org/)
[![Target](https://img.shields.io/badge/target-aarch64--unknown--none-blue.svg)](https://github.com/rust-embedded/rust-target-tier3)
[![Code style: rustfmt](https://img.shields.io/badge/code_style-rustfmt-brightgreen.svg)](https://github.com/rust-lang/rustfmt)
[![Contributor Covenant](https://img.shields.io/badge/Contributor%20Covenant-2.1-4baaaa.svg)](CODE_OF_CONDUCT.md)

A bare-metal Rust operating system for Raspberry Pi 5 (BCM2712), built from scratch with `#![no_std]`.

## Features

- **SMP Scheduler**: Per-core runqueues, IPI-based cross-core coordination, work-stealing load balancing
- **Virtual Memory**: 4-level page tables, frame allocator, kernel heap, process address spaces
- **Networking Stack**: Ethernet (GENET), ARP, IPv4, ICMP, UDP, TCP, DHCP, DNS, HTTP client
- **Display & Graphics**: Framebuffer driver, graphics primitives, VT100 console, window manager with compositor
- **USB Stack**: XHCI controller driver, HID (keyboard/mouse), device enumeration
- **Filesystems**: VFS layer, initrd, FAT32, ext2 (with fsck), procfs, sysfs, devfs, block device layer
- **Shell**: Interactive shell with built-in commands (wifi, net, fs, pkg, proc, wm, help)
- **Window Manager**: Compositor, overlapping windows, widgets, themes, input handling
- **Device Drivers**: UART, GPIO, mailbox, timer, GIC, power, EMMC/SD, SDIO, PCIe, NVMe, GPU, WiFi (CYW43455), audio
- **Package Manager**: Repository format, HTTP index download, installer scaffolding
- **Process Management**: ELF loader, dynamic linker, signals, syscalls, process hierarchy
- **Security**: Capability-based security, users/groups, authentication, sessions
- **Terminal Emulator**: VT100/ANSI parser, screen buffer with scrollback
- **IPC**: Message passing, shared memory, PTY support
- **Testing**: QEMU integration tests, hardware validation scripts

## Architecture

```
lobster-os/
├── src/
│   ├── arch/           # Architecture-specific (AArch64, BCM2712)
│   │   ├── cpu.rs      # CPU detection, feature detection
│   │   ├── exception.rs # Exception vectors, EL transitions
│   │   ├── gic.rs      # Generic Interrupt Controller
│   │   ├── mmu.rs      # Memory Management Unit
│   │   └── timer.rs    # Generic Timer
│   ├── display/        # Framebuffer, graphics, console, window manager
│   ├── drivers/        # Hardware drivers
│   ├── fs/             # Filesystems (VFS, FAT32, ext2, procfs, sysfs, devfs)
│   ├── ipc/            # Inter-process communication
│   ├── memory/         # Memory management (frame alloc, heap, vmem, process_vmem)
│   ├── net/            # Networking stack
│   ├── pkg/            # Package manager
│   ├── sched/          # Scheduler, processes, signals, syscalls, ELF loader
│   ├── security/       # Capabilities, users, auth
│   ├── shell/          # Interactive shell
│   ├── smp/            # SMP primitives (runqueue, IPI, load balancing)
│   ├── sync/           # Synchronization (spinlock, mutex)
│   ├── terminal/       # Terminal emulator, VT100 parser
│   ├── usb/            # USB stack (XHCI, HID)
│   ├── users/          # User/group management
│   └── wm/             # Window manager
├── tests/              # Integration tests
├── targets/            # Linker scripts, target specs
└── build/              # Build artifacts
```

## Building

### Prerequisites

- Rust nightly (for `#![no_std]` features)
- `aarch64-unknown-none` target: `rustup target add aarch64-unknown-none`
- QEMU (for emulation): `qemu-system-aarch64`
- `llvm-tools-preview` component for `cargo binutils`

### Quick Start

```bash
# Build for QEMU (raspi4b emulation)
cargo build --target aarch64-unknown-none --features qemu

# Run in QEMU
qemu-system-aarch64 -M raspi4b -kernel target/aarch64-unknown-none/debug/lobster-os -serial stdio -display none

# Build release
cargo build --target aarch64-unknown-none --features qemu --release
```

### Hardware (Raspberry Pi 5)

```bash
# Build for Pi 5
cargo build --target aarch64-unknown-none --release

# Copy kernel8.img to boot partition of SD card
# Ensure config.txt has: arm_64bit=1 kernel=kernel8.img
```

## Testing

```bash
# Run unit tests (host)
cargo test

# Run QEMU integration tests
cargo test --target aarch64-unknown-none --features qemu --test integration

# Lint
cargo clippy --target aarch64-unknown-none -- -D warnings
cargo fmt --check
```

## Project Structure

| Module | Description |
|--------|-------------|
| `arch` | AArch64/BCM2712 specific code |
| `display` | Framebuffer, graphics, window manager |
| `drivers` | Hardware device drivers |
| `fs` | Filesystem implementations |
| `ipc` | Message passing, shared memory |
| `memory` | Physical/virtual memory management |
| `net` | Full TCP/IP stack |
| `pkg` | Package manager |
| `sched` | Scheduler, processes, syscalls |
| `security` | Capabilities, authentication |
| `shell` | User shell |
| `smp` | Multi-core coordination |
| `sync` | Locking primitives |
| `terminal` | VT100 terminal emulator |
| `usb` | USB host stack |
| `users` | User/group database |
| `wm` | Window manager |

## Status

| Component | Status |
|-----------|--------|
| Boot / Early init | ✅ Working |
| SMP bringup | ✅ 4 cores |
| Memory management | ✅ Complete |
| Networking (GENET) | ✅ Working |
| TCP/IP stack | ✅ Working |
| DHCP/DNS/HTTP | ✅ Working |
| Display/Framebuffer | ✅ Working |
| Window manager | ✅ Working |
| USB/XHCI | ✅ Working |
| Filesystems | ✅ Multiple |
| Shell | ✅ Functional |
| WiFi (CYW43455) | 🚧 In progress |
| Package manager | 🚧 Scaffolding |
| NVMe | 🚧 In progress |
| GPU (V3D) | 🚧 Planned |
| Audio | 🚧 Planned |

## Contributing

We welcome contributions! See [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

**Quick start for contributors:**

```bash
git clone https://github.com/jlaportebot/lobster-os.git
cd lobster-os
rustup default nightly
rustup target add aarch64-unknown-none
rustup component add llvm-tools-preview rustfmt clippy

# Build & test
cargo build --features qemu --target aarch64-unknown-none
cargo test --features std
cargo test --features qemu --target aarch64-unknown-none --test integration

# Lint & format
cargo clippy --features qemu --target aarch64-unknown-none -- -D warnings
cargo fmt --all -- --check
```

**Good first issues:** Look for the [`good first issue` label](https://github.com/jlaportebot/lobster-os/issues?q=label%3A%22good+first+issue%22) — we'll mentor you through it.

**Questions?** Open a [Discussion](https://github.com/jlaportebot/lobster-os/discussions) or [issue](https://github.com/jlaportebot/lobster-os/issues/new/choose).

## License

MIT License — see [LICENSE](LICENSE) file for details.

## Security

See [SECURITY.md](SECURITY.md) for reporting vulnerabilities.

## Code of Conduct

See [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md). We follow the Contributor Covenant.

## Acknowledgments

- Raspberry Pi documentation and datasheets
- Linux kernel source (reference for BCM2712 peripherals)
- Rust embedded community
- `tock-registers`, `spin`, `bitflags` crates

---

**LobsterOS** — Built with 🦞 by Mister Lobster