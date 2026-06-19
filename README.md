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
- **Networking Stack**: Ethernet (GENET), ARP, IPv4, ICMP, UDP, TCP, DHCP, DNS, HTTP client, network interfaces
- **Display & Graphics**: Framebuffer driver, graphics primitives, VT100 console, window manager with compositor
- **USB Stack**: XHCI controller driver, HID (keyboard/mouse), hub, mass storage, device enumeration
- **Filesystems**: VFS layer, initrd, FAT32, ext2 (with fsck), procfs, sysfs, devfs, block device layer
- **Shell**: Interactive shell with built-in commands (wifi, net, fs, pkg, proc, wm, help)
- **Window Manager**: Compositor, overlapping windows, widgets, themes, input handling, panel, launcher
- **Device Drivers**: UART, GPIO, mailbox, timer, GIC, power, EMMC/SD, SDIO, PCIe, NVMe, GPU (V3D), WiFi (CYW43455), audio
- **Package Manager**: Repository format, HTTP index download, installer scaffolding
- **Process Management**: ELF loader, dynamic linker, signals, syscalls, process hierarchy
- **Security**: Capability-based security, users/groups, authentication, sessions
- **Terminal Emulator**: VT100/ANSI parser, screen buffer with scrollback
- **IPC**: Message passing, shared memory, PTY support
- **GUI Applications**: Text editor, file manager, settings app, terminal emulator
- **Device Tree Parsing**: Hardware enumeration via DTB
- **Testing**: QEMU integration tests (raspi4b, raspi5b), hardware validation scripts

## Architecture

```
lobster-os/
├── src/
│   ├── apps/           # GUI applications (editor, file_manager, settings, terminal)
│   ├── arch/           # Architecture-specific (AArch64, BCM2712)
│   │   ├── cpu.rs      # CPU detection, feature detection
│   │   ├── exception.rs # Exception vectors, EL transitions
│   │   ├── gic.rs      # Generic Interrupt Controller (GIC-400)
│   │   ├── timer.rs    # Generic Timer
│   │   ├── mod.rs
│   │   └── x86_64/     # x86_64 stubs for host testing
│   ├── console.rs      # UART console + println! macros
│   ├── display/        # Framebuffer, graphics, console, window manager
│   │   ├── console.rs  # Framebuffer console (VT100)
│   │   ├── fb.rs       # Framebuffer driver
│   │   ├── graphics.rs # Graphics primitives
│   │   ├── logo.rs     # Boot splash / ASCII logo
│   │   └── mod.rs
│   ├── drivers/        # Hardware drivers
│   │   ├── audio.rs    # Audio driver (planned)
│   │   ├── emmc.rs     # eMMC/SD card controller
│   │   ├── gpio.rs     # GPIO controller
│   │   ├── gpu.rs      # V3D GPU driver (52 KB)
│   │   ├── mailbox.rs  # Mailbox property interface
│   │   ├── nvme.rs     # NVMe driver (57 KB)
│   │   ├── pcie.rs     # PCIe controller
│   │   ├── power.rs    # Power management
│   │   ├── sdio.rs     # SDIO controller (for WiFi)
│   │   ├── serial_input.rs # Serial input handling
│   │   ├── uart.rs     # UART driver
│   │   ├── wifi.rs     # CYW43455 WiFi driver (108 KB)
│   │   └── mod.rs
│   ├── dtb/            # Device tree parsing
│   ├── fs/             # Filesystems (VFS, FAT32, ext2, procfs, sysfs, devfs)
│   ├── ipc/            # Inter-process communication (message passing, shared memory, PTY)
│   ├── klog.rs         # Kernel logging
│   ├── memory/         # Memory management (frame alloc, heap, vmem, process_vmem)
│   ├── net/            # Networking stack
│   │   ├── arp.rs      # ARP
│   │   ├── dhcp.rs     # DHCP client
│   │   ├── dns.rs      # DNS resolver
│   │   ├── dns_cache.rs # DNS cache
│   │   ├── eth.rs      # Ethernet frame handling
│   │   ├── genet.rs    # GENET Ethernet driver
│   │   ├── http.rs     # HTTP client
│   │   ├── icmp.rs     # ICMP
│   │   ├── iface.rs    # Network interface abstraction
│   │   ├── ipv4.rs     # IPv4
│   │   ├── socket.rs   # Socket API
│   │   ├── tcp.rs      # TCP
│   │   ├── udp.rs      # UDP
│   │   └── mod.rs
│   ├── panic_handler.rs # Panic handler
│   ├── pkg/            # Package manager (repo, install, package)
│   ├── qemu_compat.rs  # QEMU compatibility layer
│   ├── qemu_test.rs    # QEMU test harness
│   ├── sched/          # Scheduler, processes, signals, syscalls, ELF loader
│   ├── security/       # Capabilities, users, auth
│   ├── shell/          # Interactive shell (built-in commands)
│   ├── smp/            # SMP primitives (runqueue, IPI, load balancing)
│   ├── sync/           # Synchronization (spinlock)
│   ├── terminal/       # Terminal emulator, VT100 parser
│   ├── usb/            # USB stack (XHCI, HID, hub, mass storage)
│   ├── users/          # User/group management (auth, passwd, shadow, session)
│   └── wm/             # Window manager (compositor, windows, widgets, panel, launcher, input)
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
# Build for QEMU (raspi4b emulation, 2 GB RAM)
cargo build --target aarch64-unknown-none --features qemu

# Build for QEMU (raspi5b emulation, 4 GB RAM)
cargo build --target aarch64-unknown-none --features qemu_raspi5b

# Run in QEMU (raspi4b)
qemu-system-aarch64 -M raspi4b -kernel target/aarch64-unknown-none/debug/lobster-os -serial stdio -display none

# Run in QEMU (raspi5b)
qemu-system-aarch64 -M raspi5b -kernel target/aarch64-unknown-none/debug/lobster-os -serial stdio -display none

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
| `apps` | GUI applications (editor, file manager, settings, terminal) |
| `arch` | AArch64/BCM2712 specific code (cpu, exception, gic, timer, x86_64 stubs) |
| `display` | Framebuffer, graphics, window manager |
| `drivers` | Hardware device drivers (gpu, nvme, wifi, pcie, emmc, audio, etc.) |
| `dtb` | Device tree parsing |
| `fs` | Filesystem implementations (VFS, FAT32, ext2, procfs, sysfs, devfs, block_dev) |
| `ipc` | Message passing, shared memory, PTY |
| `memory` | Physical/virtual memory management (mmu, frame_alloc, heap, vmem, process_vmem) |
| `net` | Full TCP/IP stack (arp, dhcp, dns, http, icmp, ipv4, tcp, udp, socket, genet, iface) |
| `pkg` | Package manager (repo, install, package) |
| `sched` | Scheduler, processes, signals, syscalls, ELF loader, dynamic linker |
| `security` | Capabilities, authentication, users |
| `shell` | User shell (built-in commands) |
| `smp` | Multi-core coordination (runqueue, IPI, load balancing) |
| `sync` | Locking primitives (spinlock) |
| `terminal` | VT100 terminal emulator |
| `usb` | USB host stack (XHCI, HID, hub, mass storage) |
| `users` | User/group database (auth, passwd, shadow, session, group) |
| `wm` | Window manager (compositor, windows, widgets, panel, launcher, input, theme) |

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
| Filesystems (VFS, FAT32, ext2, procfs, sysfs, devfs) | ✅ Multiple |
| Shell | ✅ Functional |
| WiFi (CYW43455) | 🚧 Substantial impl (108 KB) — SDIO + firmware loading |
| Package manager | 🚧 Scaffolding (repo, install, package) |
| NVMe | 🚧 Substantial impl (57 KB) — controller + namespace |
| GPU (V3D) | 🚧 Substantial impl (52 KB) — register defs, init, shader compiler |
| Audio | 🚧 Substantial impl (15 KB) — driver skeleton + register map |
| PCIe | 🚧 Substantial impl (27 KB) — root complex, config space |
| eMMC/SD | ✅ Working (43 KB) |
| Device Tree parsing | ✅ Working |
| Terminal emulator (VT100/ANSI) | ✅ Working |
| GUI Applications (editor, file manager, settings, terminal) | ✅ Working |
| Dynamic linker (ELF) | ✅ Working |
| Process management (signals, syscalls, hierarchy) | ✅ Working |
| Security (capabilities, users, auth, sessions) | ✅ Working |
| IPC (message passing, shared memory, PTY) | ✅ Working |

## Contributing

We welcome contributions! See [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

**Quick start for contributors:**

```bash
git clone https://github.com/jlaportebot/lobster-os.git
cd lobster-os
rustup default nightly
rustup target add aarch64-unknown-none
rustup component add llvm-tools-preview rustfmt clippy

# Build & test (QEMU raspi4b)
cargo build --features qemu --target aarch64-unknown-none
cargo test --features std
cargo test --features qemu --target aarch64-unknown-none --test integration

# Build & test (QEMU raspi5b, 4 GB RAM)
cargo build --features qemu_raspi5b --target aarch64-unknown-none
cargo test --features qemu_raspi5b --target aarch64-unknown-none --test integration

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