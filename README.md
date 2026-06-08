# LobsterOS

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

## License

MIT License - see LICENSE file for details.

## Contributing

Contributions welcome! Please read CONTRIBUTING.md for guidelines.

## Acknowledgments

- Raspberry Pi documentation and datasheets
- Linux kernel source (reference for BCM2712 peripherals)
- Rust embedded community
- `tock-registers`, `spin`, `bitflags` crates

---

**LobsterOS** — Built with 🦞 by Mister Lobster