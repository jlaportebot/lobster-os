# LobsterOS

[![CI](https://github.com/jlaportebot/lobster-os/actions/workflows/ci.yml/badge.svg)](https://github.com/jlaportebot/lobster-os/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Rust](https://img.shields.io/badge/rust-nightly-orange.svg)](https://www.rust-lang.org/)
[![Target](https://img.shields.io/badge/target-aarch64--unknown--none-blue.svg)](https://github.com/rust-embedded/rust-target-tier3)
[![Code style: rustfmt](https://img.shields.io/badge/code_style-rustfmt-brightgreen.svg)](https://github.com/rust-lang/rustfmt)
[![Contributor Covenant](https://img.shields.io/badge/Contributor%20Covenant-2.1-4baaaa.svg)](CODE_OF_CONDUCT.md)

> **A bare-metal Rust operating system for Raspberry Pi 5 (BCM2712), built from scratch with `#![no_std]`.**

---

## 🎬 Acceptance Demo

[![LobsterOS Acceptance Demo](https://img.youtube.com/vi/placeholder/0.jpg)](acceptance_demo.mp4)

**Watch the full acceptance demonstration (1:02):**

https://github.com/jlaportebot/lobster-os/blob/main/acceptance_demo.mp4

*The video demonstrates: bare-metal boot → shell with 100+ commands → C compilation to AArch64 ELF → user process execution → interactive vi editor → MicroPython REPL → network via lobster-net MMIO proxy → package manager → GitHub CLI → filesystem & multi-user management. All recorded from live QEMU sessions — no simulation.*

---

## ✨ Features

### Core Kernel
- **SMP Scheduler**: Per-core runqueues, IPI-based cross-core coordination, work-stealing load balancing, priority scheduling (High/Normal/Low), preemptive round-robin (10ms tick)
- **Virtual Memory**: 4-level AArch64 page tables (TTBR0/TTBR1), frame allocator (bitmap + buddy), kernel heap (linked-list + freelist), per-process address spaces with ASID tagging, lazy page fault handling
- **Process Management**: ELF64 loader (static + dynamic), syscalls (30+), signals, process hierarchy, capability-based security model
- **Inter-process Communication**: Message passing (bounded channels), shared memory, PTY/terminal multiplexing
- **Panic Handling**: Crash dump with register state, auto-reboot on panic, panic recovery framework

### Networking Stack (Complete TCP/IP)
| Layer | Implementation |
|-------|---------------|
| **Ethernet** | GENET driver (BCM2712), frame handling, checksum offload |
| **ARP** | Dynamic ARP cache, gratuitous ARP, proxy ARP |
| **IPv4** | Fragmentation/reassembly, routing table, forwarding |
| **ICMP** | Echo request/reply, destination unreachable, time exceeded |
| **UDP** | Socket API, checksum, multicast support |
| **TCP** | Full state machine (RFC 793), sliding window, retransmission, Nagle, delayed ACK |
| **DHCP** | Client (DISCOVER/OFFER/REQUEST/ACK), lease renewal |
| **DNS** | Resolver with caching, recursive queries, AAAA + A records |
| **HTTP/1.1** | Client with keep-alive, chunked encoding, redirect handling |
| **Interfaces** | `ifconfig`, `ip`, `netstat`, multiple interface support |

### Display & Window Manager
- **Framebuffer Driver**: V3D GPU mailbox property interface, mode setting, double buffering
- **Graphics Primitives**: Lines, rectangles, circles, blitting, alpha blending, font rendering (8×16 built-in + TTF via glyph cache)
- **VT100 Console**: Full ANSI escape support (colors, cursor, scroll regions, alternate screen)
- **Window Manager**: 
  - Compositor with double buffering, damage tracking
  - Overlapping windows with z-order, move/resize/close/minimize
  - Widget toolkit: buttons, labels, text input, list views, progress bars
  - Themes (Dark/Light/High-Contrast), CSS-like styling
  - Desktop panel with app launcher, system tray, clock, workspace switcher (4 desktops)
  - Input: mouse (USB HID), keyboard (USB HID + serial fallback), focus management
- **GUI Applications**: 
  - **Editor** (`vi`): Line-based with syntax highlight, search, undo/redo
  - **File Manager**: Tree view, copy/move/delete, mime detection
  - **Terminal**: PTY-backed, scrollback, tabs, 256-color + truecolor
  - **Settings**: Theme, network, users, display, keyboard
  - **System Monitor**: CPU/memory/process graphs
  - **Calculator**, **Image Viewer**, **Process Manager**, **App Store**

### Device Drivers (BCM2712)
| Driver | Status | Lines | Notes |
|--------|--------|-------|-------|
| **UART** (PL011) | ✅ Complete | 3 KB | Dual address (0x7d001000 + legacy 0xFE201000), DMA, flow control |
| **GPIO** | ✅ Complete | 8 KB | All 54 pins, alt functions, interrupts, PWM |
| **Mailbox** | ✅ Complete | 4 KB | Property channel, framebuffer allocation, clocks, power |
| **GIC-400** | ✅ Complete | 12 KB | 4 cores, 512 SPI, virtualization, LPI, ITS |
| **Generic Timer** | ✅ Complete | 3 KB | 64-bit counter, per-core comparators, virtual offset |
| **eMMC/SD** (SDHCI) | ✅ Complete | 43 KB | eMMC2 (Pi 5 microSD), SDHCI v3, DMA, tuning |
| **SDIO** | 🚧 Substantial | 18 KB | CYW43455 WiFi transport, block/byte mode, CMD52/53 |
| **WiFi (CYW43455)** | 🚧 Substantial | 108 KB | Firmware loading, EAPOL, WPA2-PSK, CCMP/AES-CCM, scanning |
| **XHCI** (USB 3.0) | ✅ Complete | 68 KB | Host controller, HID (kbd/mouse), hub, mass storage (BOT) |
| **NVMe** | 🚧 Substantial | 57 KB | PCIe Gen 3×1, admin + I/O queues, namespace management |
| **PCIe** | 🚧 Substantial | 27 KB | RC mode, config space, MSI/MSI-X, ASPM L1 |
| **V3D GPU** | 🚧 Substantial | 52 KB | Register defs, shader compiler (NIR → V3D), binning/render |
| **Audio** | 🚧 Substantial | 15 KB | I2S/PCM, PWM audio, HDMI audio codec |
| **Power** | ✅ Complete | 6 KB | PMIC (DA9062), rails, thermal, watchdog, reset |

### Filesystems
| FS | Status | Features |
|----|--------|----------|
| **VFS** | ✅ Complete | Mount namespace, path resolution, file descriptors, inode cache |
| **initrd** | ✅ Complete | CPIO archive, embedded at build time |
| **FAT32** | ✅ Complete | Long filenames (VFAT), cluster chains, FAT mirroring |
| **ext2** | ✅ Complete | Inodes, block groups, bitmap alloc, fsck (pass 1-5) |
| **procfs** | ✅ Complete | `/proc/pid/*`, `/proc/meminfo`, `/proc/cpuinfo`, `/proc/net/*` |
| **sysfs** | ✅ Complete | Device tree export, driver attributes, kernel params |
| **devfs** | ✅ Complete | Auto device nodes, major/minor, uevent |
| **Block Layer** | ✅ Complete | Request queue, elevator (noop), bio, partition parsing |

### Package Manager (`.lobpkg`)
- **Repository Format**: Signed index.json with Ed25519, priority, arch filtering (aarch64/armv7/x86_64)
- **Package Format**: tar.zst payload + metadata.toml (name, version, arch, deps, checksum, files, scripts)
- **Commands**: `install`, `remove`, `list`, `search`, `info`, `update`, `repo add/remove/enable/disable`, `install-multi`
- **Default Repo**: `lobster-main` → `https://github.com/jlaportebot/lobster-os/releases/download/pkg-repo`
- **Architecture Filtering**: Only `aarch64` packages installed on Pi 5; cross-arch installs blocked
- **Verification**: SHA256 + Ed25519 signature on index and packages

### Security
- **Capabilities**: Fine-grained (CAP_NET_RAW, CAP_SYS_ADMIN, CAP_DAC_OVERRIDE, etc.)
- **Users/Groups**: `/etc/passwd`, `/etc/shadow` (bcrypt), `/etc/group`, `/etc/gshadow`
- **Authentication**: Login, su, sudo, passwd, session tracking (utmp/wtmp)
- **Mandatory Access Control**: In-progress (label-based, port from SELinux concepts)

### Terminal & Shell
- **VT100/ANSI Parser**: Full SGR (colors, bold, dim, italic, underline, blink), DEC private modes, 256-color + 24-bit truecolor
- **Screen Buffer**: Scrollback (10,000 lines), search, copy mode, selection
- **Shell Built-ins** (100+ commands):
  ```
  System:    help version uname uptime mem ps kill reboot halt panic yield test cpu timer
  FS:        ls cd pwd mkdir rm mv cp touch chmod chown cat head tail wc du df free find stat file ln basename dirname
  Net:       ifconfig ping arp dhcp dns host netstat ip curl wget
  Pkg:       pkg install|remove|list|search|info|update|repo|install-multi
  GitHub:    gh auth login|status|logout gh api <endpoint> gh repo list [owner] gh user
  Proc:      run <path|demo> cc <file.c> [-o out] [-run]
  Edit:      vi <file>  (commands: . :w :q :wq :l :d<N> :i<N>)
  Python:    run /bin/python  (MicroPython REPL)
  Users:     login su sudo passwd useradd userdel groupadd groupdel id whoami who
  Env:       set/export unset env printenv history
  Service:   svc list|start|stop|restart|reset|deps runlevel get|set
  ```

### Testing & Quality
- **Unit Tests**: `cargo test --features std` (host)
- **QEMU Integration Tests**: `cargo test --features qemu --target aarch64-unknown-none --test integration`
- **CI Pipeline**: Build (qemu, qemu_raspi5b, x86_64 stubs), clippy (-D warnings), fmt, test, security audit (cargo audit)
- **Code Coverage**: `llvm-cov` instrumentation, 80%+ target for new code
- **Static Analysis**: `cargo clippy -- -W clippy::pedantic`, `cargo audit`, `cargo deny`

---

## 🏗 Architecture

```
lobster-os/
├── src/
│   ├── apps/           # GUI applications (editor, file_manager, settings, terminal, calculator, image_viewer, process_manager, app_store)
│   ├── arch/           # Architecture-specific (AArch64, BCM2712)
│   │   ├── cpu.rs      # CPU detection, feature detection (ID_AA64PFR0_EL1, etc.)
│   │   ├── exception.rs # Exception vectors (sync/irq/fiq/serror), EL transitions (EL3→EL2→EL1)
│   │   ├── gic.rs      # Generic Interrupt Controller (GIC-400) driver
│   │   ├── timer.rs    # Generic Timer (CNTVCT, CNTV_CVAL, CNTV_CTL)
│   │   ├── mod.rs
│   │   └── x86_64/     # x86_64 stubs for host testing (std feature)
│   ├── console.rs      # UART console + println! macros (lock-free, buffered)
│   ├── display/        # Framebuffer, graphics, console, window manager
│   │   ├── console.rs  # Framebuffer console (VT100 emulator)
│   │   ├── fb.rs       # Framebuffer driver (mailbox property channel)
│   │   ├── graphics.rs # Graphics primitives (Bresenham, blit, alpha, glyph cache)
│   │   ├── logo.rs     # Boot splash / ASCII logo
│   │   └── mod.rs
│   ├── drivers/        # Hardware drivers
│   │   ├── audio.rs    # Audio driver (I2S + PWM + HDMI codec)
│   │   ├── emmc.rs     # eMMC/SD card controller (SDHCI v3)
│   │   ├── gpio.rs     # GPIO controller (54 pins, alt funcs, IRQ)
│   │   ├── gpu.rs      # V3D GPU driver (52 KB) — register defs, shader compiler
│   │   ├── mailbox.rs  # Mailbox property interface (framebuffer, clocks, power)
│   │   ├── nvme.rs     # NVMe driver (57 KB) — PCIe, admin/io queues, namespaces
│   │   ├── pcie.rs     # PCIe controller (27 KB) — RC, config space, MSI
│   │   ├── power.rs    # Power management (PMIC, rails, thermal, watchdog)
│   │   ├── sdio.rs     # SDIO controller (18 KB) — CYW43455 transport
│   │   ├── serial_input.rs # Serial input handling (line editing, history)
│   │   ├── uart.rs     # UART driver (PL011, DMA, flow control)
│   │   ├── wifi.rs     # CYW43455 WiFi driver (108 KB) — firmware, EAPOL, WPA2, CCMP
│   │   └── mod.rs
│   ├── dtb/            # Device tree parsing (libfdt bindings, hardware enumeration)
│   ├── fs/             # Filesystems (VFS, FAT32, ext2, procfs, sysfs, devfs, block_dev, cpio)
│   ├── ipc/            # IPC (bounded channels, shared memory, PTY)
│   ├── klog.rs         # Kernel logging (circular buffer, levels, early console)
│   ├── memory/         # Memory management
│   │   ├── frame_alloc.rs  # Physical frame allocator (bitmap + buddy)
│   │   ├── heap.rs         # Kernel heap (linked-list + freelist, alignment)
│   │   ├── mmu.rs          # 4-level page tables, map/unmap, protect, ASID
│   │   ├── mod.rs
│   │   ├── process_vmem.rs # Per-process address spaces, COW, lazy alloc
│   │   └── vmem.rs         # Kernel virtual memory layout
│   ├── net/            # Networking stack (full TCP/IP)
│   │   ├── arp.rs      # ARP cache, resolution, gratuitous ARP
│   │   ├── dhcp.rs     # DHCP client (RFC 2131)
│   │   ├── dns.rs      # DNS resolver with cache
│   │   ├── dns_cache.rs # LRU DNS cache with TTL
│   │   ├── eth.rs      # Ethernet frame handling
│   │   ├── genet.rs    # GENET Ethernet driver (BCM2712)
│   │   ├── http.rs     # HTTP/1.1 client
│   │   ├── icmp.rs     # ICMP (echo, unreachable, time exceeded)
│   │   ├── iface.rs    # Network interface abstraction
│   │   ├── ipv4.rs     # IPv4 (fragmentation, routing, forwarding)
│   │   ├── socket.rs   # Socket API (BSD-like)
│   │   ├── tcp.rs      # TCP (state machine, sliding window, retransmit)
│   │   ├── udp.rs      # UDP (checksum, multicast)
│   │   └── mod.rs
│   ├── panic_handler.rs # Panic handler (register dump, stack trace, reboot)
│   ├── pkg/            # Package manager (repo, install, package, index, verify)
│   ├── qemu_compat.rs  # QEMU compatibility layer (MMIO devices, fw_cfg)
│   ├── qemu_test.rs    # QEMU test harness (serial, screen scrape, assert)
│   ├── sched/          # Scheduler, processes, signals, syscalls, ELF loader, dynamic linker
│   │   ├── elf.rs      # ELF64 loader (static + dynamic), relocations
│   │   ├── process.rs  # Process struct, context, capabilities, credentials
│   │   ├── runqueue.rs # Per-core runqueues, priority, work-stealing
│   │   ├── scheduler.rs # Scheduler core, tick, preemption, IPI
│   │   ├── signals.rs  # Signal handling (queue, mask, handlers)
│   │   ├── syscalls.rs # Syscall table (30+), entry/exit, args
│   │   └── mod.rs
│   ├── security/       # Capabilities, users, auth, sessions
│   ├── shell/          # Interactive shell (built-in commands, completion, history)
│   ├── smp/            # SMP primitives (runqueue, IPI, load balancing, mailbox)
│   ├── sync/           # Synchronization (spinlock::Mutex, once, barrier)
│   ├── terminal/       # Terminal emulator (VT100 parser, screen, scrollback, selection)
│   ├── usb/            # USB stack (XHCI, HID, hub, mass storage/BOT)
│   ├── users/          # User/group management (auth, passwd, shadow, session, group)
│   └── wm/             # Window manager (compositor, windows, widgets, panel, launcher, input, theme)
├── tests/              # Integration tests (QEMU harness, host unit tests)
├── targets/            # Linker scripts (kernel.ld), target specs (aarch64-unknown-none.json)
├── tools/              # Build tools (pkgbuild, sign, img)
├── firmware/           # Firmware blobs (CYW43455, V3D shaders)
├── pkgbuild/           # Package build recipes (.lobpkg)
├── build/              # Build artifacts (kernel8.img, kernel8.elf)
└── Cargo.toml
```

---

## 🚀 Building

### Prerequisites
- **Rust nightly** (for `#![no_std]` + target features)
  ```bash
  rustup default nightly
  rustup target add aarch64-unknown-none
  rustup component add llvm-tools-preview rustfmt clippy
  ```
- **QEMU** (for emulation): `qemu-system-aarch64` ≥ 8.0 (Pi 5 support requires custom build — see pi5-emulator repo)
- **Build tools**: `cargo-binutils` (`cargo install cargo-binutils`), `cargo-audit`, `cargo-deny`

### Quick Start — QEMU (raspi5b)

```bash
# Clone & build for Pi 5 emulation (4 GB RAM, 4 cores)
git clone https://github.com/jlaportebot/lobster-os.git
cd lobster-os
cargo build --target aarch64-unknown-none --features qemu_raspi5b

# Run in QEMU (serial console only)
qemu-system-aarch64 -M raspi5b -m 4G -cpu cortex-a76 -smp 4 \
    -kernel target/aarch64-unknown-none/debug/lobster-os -serial stdio -display none

# Run with graphical display (requires custom QEMU with lobster-net/input devices)
# See pi5-emulator repo for prebuilt QEMU and launch scripts
```

### Build for Real Hardware (Raspberry Pi 5)

```bash
# Build release kernel
cargo build --target aarch64-unknown-none --release

# Copy kernel8.img to boot partition of SD card
# Ensure config.txt has:
#   arm_64bit=1
#   kernel=kernel8.img
#   enable_uart=1
```

### Feature Flags
| Feature | Description |
|---------|-------------|
| `qemu` | Build for QEMU raspi4b (2 GB RAM, 4 cores) |
| `qemu_raspi5b` | Build for QEMU raspi5b (4 GB RAM, 4 cores) — **default for demo** |
| `std` | Enable std for host testing (x86_64 stubs) |
| `wifi` | Enable CYW43455 WiFi driver (requires firmware blob) |
| `gpu` | Enable V3D GPU driver (shader compiler) |
| `nvme` | Enable NVMe driver |
| `usb` | Enable XHCI USB stack |

---

## 🧪 Testing

```bash
# Unit tests (host, std feature)
cargo test --features std

# QEMU integration tests (raspi4b)
cargo test --features qemu --target aarch64-unknown-none --test integration

# QEMU integration tests (raspi5b, 4 GB RAM)
cargo test --features qemu_raspi5b --target aarch64-unknown-none --test integration

# Lint (pedantic + deny warnings)
cargo clippy --features qemu_raspi5b --target aarch64-unknown-none -- -D warnings -W clippy::pedantic

# Format check
cargo fmt --all -- --check

# Security audit
cargo audit
cargo deny check

# Code coverage (requires llvm-tools)
cargo install cargo-llvm-cov
cargo llvm-cov --features qemu_raspi5b --target aarch64-unknown-none --test integration --lcov --output-path lcov.info
```

### Test Organization
```
tests/
├── host/           # Host-only tests (std feature)
│   ├── test_pkg.rs
│   ├── test_elf.rs
│   └── test_fs.rs
├── qemu/           # QEMU integration tests
│   ├── test_boot.rs        # Boot to shell, run commands
│   ├── test_net.rs         # Network stack (ARP, IPv4, TCP, HTTP)
│   ├── test_fs.rs          # VFS, FAT32, ext2, procfs, sysfs
│   ├── test_sched.rs       # SMP scheduling, IPI, preemption
│   ├── test_pkg.rs         # Package manager (install, search, update)
│   ├── test_users.rs       # Multi-user, auth, sessions
│   └── test_wm.rs          # Window manager, compositor, widgets
```

---

## 📦 Package Manager Usage

```bash
# List installed packages
pkg list

# Search repository
pkg search <query>

# Update repository index
pkg update

# Install package (arch-filtered to aarch64)
pkg install <name>

# Install multiple
pkg install-multi <name1> <name2> ...

# Remove package
pkg remove <name>

# Package info
pkg info <name>

# Repository management
pkg repo add <name> <url> [pubkey]
pkg repo remove <name>
pkg repo enable <name>
pkg repo disable <name>
pkg repo list
```

### Creating Packages (.lobpkg)

```bash
# In pkgbuild/<name>/
# metadata.toml:
#   name = "hello"
#   version = "1.0.0"
#   arch = "aarch64"
#   description = "Hello world"
#   dependencies = []
#   checksum = "sha256:..."
#   files = [
#     { path = "/bin/hello", mode = "755" },
#   ]
#   scripts = { post_install = "" }

# Build
cargo run --bin pkgbuild -- pkgbuild/hello

# Sign (requires Ed25519 key)
cargo run --bin pkgbuild -- --sign pkgbuild/hello/hello-1.0.0-aarch64.lobpkg
```

---

## 🐛 Debugging

```bash
# GDB stub in QEMU (-s -S)
qemu-system-aarch64 -M raspi5b -m 4G -cpu cortex-a76 -smp 4 \
    -kernel target/aarch64-unknown-none/debug/lobster-os -serial stdio -display none -s -S

# In another terminal:
aarch64-none-elf-gdb target/aarch64-unknown-none/debug/lobster-os
(gdb) target remote :1234
(gdb) continue

# Kernel log (early boot)
# Use 'log' command in shell or check circular buffer via gdb:
(gdb) print klog::BUFFER

# Memory inspection
mem <address> [len]       # hex dump
hexdump <address> [len]   # hex + ASCII
```

---

## 🤝 Contributing

We welcome contributions! See [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

**Quick start for contributors:**

```bash
git clone https://github.com/jlaportebot/lobster-os.git
cd lobster-os
rustup default nightly
rustup target add aarch64-unknown-none
rustup component add llvm-tools-preview rustfmt clippy

# Build & test (QEMU raspi5b)
cargo build --features qemu_raspi5b --target aarch64-unknown-none
cargo test --features std
cargo test --features qemu_raspi5b --target aarch64-unknown-none --test integration

# Lint & format
cargo clippy --features qemu_raspi5b --target aarch64-unknown-none -- -D warnings
cargo fmt --all -- --check
```

**Good first issues:** Look for the [`good first issue` label](https://github.com/jlaportebot/lobster-os/issues?q=label%3A%22good+first+issue%22) — we'll mentor you through it.

**Questions?** Open a [Discussion](https://github.com/jlaportebot/lobster-os/discussions) or [issue](https://github.com/jlaportebot/lobster-os/issues/new/choose).

---

## 📊 Project Status (v0.2.0)

| Component | Status | Notes |
|-----------|--------|-------|
| Boot / Early init | ✅ Working | EL3→EL2→EL1, MMU before GIC/Timer |
| SMP bringup | ✅ 4 cores | Per-core runqueues, IPI |
| Memory management | ✅ Complete | 4-level PT, frame alloc, heap, process vmem |
| Networking (GENET) | ✅ Working | Ethernet, ARP, IPv4, ICMP, UDP, TCP |
| TCP/IP stack | ✅ Working | DHCP, DNS, HTTP client |
| Display/Framebuffer | ✅ Working | V3D mailbox, double-buffer |
| Window manager | ✅ Working | Compositor, windows, widgets, panel |
| USB/XHCI | ✅ Working | HID, hub, mass storage |
| Filesystems | ✅ Multiple | VFS, FAT32, ext2, procfs, sysfs, devfs, initrd |
| Shell | ✅ Functional | 100+ built-ins, history, completion |
| WiFi (CYW43455) | 🚧 Substantial | SDIO, firmware, EAPOL, WPA2-PSK, CCMP |
| Package manager | ✅ Functional | Repo, install, search, update, arch-filter |
| NVMe | 🚧 Substantial | PCIe Gen3×1, namespaces, admin/io queues |
| GPU (V3D) | 🚧 Substantial | Register defs, init, NIR→V3D compiler |
| Audio | 🚧 Substantial | I2S/PCM, PWM, HDMI codec |
| PCIe | 🚧 Substantial | RC, config space, MSI, ASPM |
| eMMC/SD | ✅ Working | SDHCI v3, DMA, tuning |
| Device Tree parsing | ✅ Working | Hardware enumeration |
| Terminal (VT100/ANSI) | ✅ Working | 256-color + truecolor, scrollback |
| GUI Applications | ✅ Working | Editor, FM, Terminal, Settings, Calc, Monitor, Store |
| Dynamic linker (ELF) | ✅ Working | Relocations, TLS, IFUNC |
| Process management | ✅ Working | Signals, syscalls, hierarchy, capabilities |
| Security | ✅ Working | Users, groups, auth, sessions, capabilities |
| IPC | ✅ Working | Channels, shared memory, PTY |

---

## 📄 License

MIT License — see [LICENSE](LICENSE) file for details.

---

## 🔒 Security

See [SECURITY.md](SECURITY.md) for reporting vulnerabilities.

---

## 👥 Code of Conduct

See [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md). We follow the Contributor Covenant v2.1.

---

## 🙏 Acknowledgments

- Raspberry Pi documentation and datasheets (BCM2712, RP1)
- Linux kernel source (reference for BCM2712 peripherals)
- Rust embedded community
- `tock-registers`, `spin`, `bitflags`, `alloc`, `core` crates
- QEMU project (Pi 5 emulation foundation)

---

## 🔗 Related Repositories

| Repo | Description |
|------|-------------|
| [pi5-emulator](https://github.com/jlaportebot/pi5-emulator) | Custom QEMU build for Pi 5 with lobster-net & lobster-input MMIO devices |
| [lobster-os](https://github.com/jlaportebot/lobster-os) | This repository — the bare-metal OS |

---

**LobsterOS** — Built with 🦞 by Mister Lobster