# 🦞 LobsterOS

A bare-metal `#![no_std]` Rust operating system for Raspberry Pi 5 (BCM2712 SoC). Built from scratch with ~65K lines of Rust across 120+ files.

## Features

| Subsystem | Status | Description |
|-----------|--------|-------------|
| Boot & MMU | ✅ Working | EL2 entry, 4-level page tables, frame allocator, kernel heap |
| SMP Scheduler | ✅ Working | Per-core runqueues, IPI-based coordination, work-stealing load balancing |
| Memory Management | ✅ Complete | Frame allocator, kernel heap, process address spaces, demand paging |
| Networking Stack | ✅ Working | GENET Ethernet, ARP, IPv4, ICMP, UDP, TCP, DHCP, DNS, HTTP client |
| USB Stack | ✅ Working | XHCI controller, HID (keyboard/mouse), hub, mass storage, device enumeration |
| Filesystem | ✅ Multiple | VFS layer, FAT32, ext2 (with fsck), procfs, sysfs, devfs, initrd, block layer |
| Window Manager | ✅ Working | Compositor, overlapping windows, widgets, themes, input handling, panel, launcher |
| Shell | ✅ Functional | Interactive shell with built-in commands: `ls`, `cat`, `wifi`, `pkg`, `uptime`, `ps`, `kill` |
| GUI Applications | ✅ Working | Text editor, file manager, settings app, terminal emulator, calculator, sysmonitor, paint, chat, music player, settings |
| WiFi (CYW43455) | 🚧 Substantial | SDIO driver, firmware loading, 802.11 stack (108 KB) |
| Audio | 🚧 Substantial | Driver skeleton, register map, DMA engine (15 KB) |
| Bluetooth | 🚧 Planned | Controller driver, HCI/L2CAP layers |
| GPIO | ✅ Working | GPIO controller driver, interrupt handling, sysfs interface |
| GPU (V3D) | 🚧 Substantial | Register definitions, initialization, command submission, shader compiler (52 KB) |
| NVMe | 🚧 Substantial | Controller driver, namespace management, I/O queues (57 KB) |
| PCIe | 🚧 Substantial | Root complex, config space enumeration, BAR translation (27 KB) |
| Audio | 🚧 Substantial | I2S driver, mixer, volume control (15 KB) |
| Device Tree | ✅ Working | DTB parsing, hardware enumeration, overlay support |
| Process Management | ✅ Working | ELF loader, dynamic linker, signals, syscalls, process hierarchy, orphan reaping |
| Security | ✅ Working | Capability-based security, users/groups, authentication, sessions |
| Terminal Emulator | ✅ Working | VT100/ANSI parser, screen buffer with scrollback, 256-color support |
| IPC | ✅ Working | Message passing, shared memory, PTY support, Unix domain sockets |
| Package Manager | 🚧 Scaffolding | Repository format, HTTP index download, installer scaffolding |
| Power Management | 🚧 Planned | CPU frequency scaling, standby/suspend, power button handling |
| Camera CSI | 🚧 Planned | CSI-2 receiver, ISP interface, V4L2 subdevice |
| Dual Display | 🚧 Planned | HDMI + DSI dual-display support, independent framebuffers |

## Build Instructions

### Prerequisites
- Rust nightly (for `#![no_std]` features)
- `aarch64-unknown-none` target: `rustup target add aarch64-unknown-none`
- QEMU (for emulation): `qemu-system-aarch64`
- `llvm-tools-preview` component for `cargo binutils`
- `make`, `git`, `curl`

### Quick Start
```bash
# Clone repository
git clone https://github.com/jlaportebot/lobster-os.git
cd lobster-os

# Build for QEMU (raspi4b emulation, 2 GB RAM)
cargo build --target aarch64-unknown-none --features qemu

# Run in QEMU (raspi4b)
qemu-system-aarch64 -M raspi4b -kernel target/aarch64-unknown-none/debug/lobster-os -serial stdio -display none

# Build release for Raspberry Pi 5
cargo build --target aarch64-unknown-none --release
# Produces: build/kernel8.img
```

### SD Card Image Creation
```bash
# Build release kernel
cargo build --target aarch64-unknown-none --release

# Create bootable SD card image (4GB)
./scripts/build_image.sh

# Flash to SD card (replace /dev/sdX with your device)
sudo dd if=build/lobsteros.img of=/dev/sdX bs=4M status=progress
sudo sync
```

### QEMU Testing Instructions
```bash
# Run unit tests (host)
cargo test

# Run QEMU integration tests (raspi4b)
cargo test --target aarch64-unknown-none --features qemu --test integration

# Run QEMU integration tests (raspi5b, 4GB RAM)
cargo test --target aarch64-unknown-none --features qemu_raspi5b --test integration

# Lint and format
cargo clippy --target aarch64-unknown-none -- -D warnings
cargo fmt --check
```

## Architecture Overview

```
lobster-os/
├── src/
│   ├── apps/           # GUI applications (editor, file_manager, settings, terminal, etc.)
│   ├── arch/           # Architecture-specific (AArch64, BCM2712)
│   │   ├── cpu.rs      # CPU detection, feature detection
│   │   ├── exception.rs# Exception vectors, EL transitions
│   │   ├── gic.rs      # Generic Interrupt Controller (GIC-400)
│   │   ├── timer.rs    # Generic Timer
│   │   └── mod.rs
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
│   │   ├── dns_cache.rs# DNS cache
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
│   ├── panic_handler.rs# Panic handler
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

## Shell Commands

| Command | Description |
|---------|-------------|
| `ls` | List directory contents |
| `cat <file>` | Display file contents |
| `wifi [scan\|connect <ssid> [password]]` | Scan for WiFi networks or connect to a network |
| `pkg [list\|install <pkg> [version]]` | List available packages or install a package |
| `uptime` | Show system uptime and load averages |
| `ps` | List running processes |
| `kill <pid>` | Send signal to terminate a process |
| `clear` | Clear the terminal screen |
| `help` | Display help for built-in commands |
| `shutdown` | Power off the system |
| `reboot` | Reboot the system |

## Keyboard Shortcuts

| Shortcut | Action |
|----------|--------|
| `Super + ←` | Move window left |
| `Super + →` | Move window right |
| `Super + ↑` | Maximize window |
| `Super + ↓` | Minimize window |
| `Alt + Tab` | Switch to next window |
| `Alt + Shift + Tab` | Switch to previous window |
| `Alt + F4` | Close focused window |
| `Alt + 1` → `Alt + 4` | Switch to workspace 1-4 |
| `Super + D` | Show desktop (hide all windows) |
| `Super + L` | Lock screen |
| `Super + T` | Open terminal emulator |
| `Super + E` | Open file manager |
| `Super + F` | Open web browser |
| `Super + S` | Open settings app |

## Pi 5 Hardware Support

| Component | Status | Details |
|-----------|--------|---------|
| RAM Detection | ✅ Working | 1GB-16GB LPDDR4X detection at boot |
| BCM2712 SoC | ✅ Working | Broadcom BCM2712 ARM Cortex-A76 (4x) + Cortex-A55 (4x) |
| GIC-400 | ✅ Working | Generic Interrupt Controller v4.0 for SMP |
| GENET Ethernet | ✅ Working | Gigabit Ethernet controller with DMA |
| xHCI USB | ✅ Working | eXtensible Host Controller Interface for USB 3.0/2.0 |
| CYW43455 WiFi/BT | 🚧 WiFi | SDIO interface, firmware loading, 802.11b/g/n/ac |
| Bluetooth | 🚧 Planned | UART interface, HCI protocol |
| GPIO | ✅ Working | 28x programmable GPIO pins with interrupt support |
| Display | ✅ Working | HDMI 2.0a, MIPI DSI display controller |
| Audio | 🚧 Substantial | I2S audio interface, PDM microphone input |
| Storage | ✅ Working | eMMC 5.1, SD/UHS-I, NVMe via PCIe |
| PCIe | 🚧 Substantial | PCIe Gen 3 x1 lane for NVMe/other peripherals |
| Camera CSI | 🚧 Planned | MIPI CSI-2 camera interface (2 lanes) |
| VideoCore VII | 🚧 Planned | VideoCore VII GPU (V3D 7.x) for 3D graphics |
| Power Management | 🚧 Planned | PMIC (Power Management Interface) control |

## Roadmap

| Milestone | Description | Target |
|-----------|-------------|--------|
| **Dual Display** | HDMI + DSI dual-display support with independent framebuffers | Q4 2026 |
| **V3D GPU 3D** | Hardware-accelerated 3D graphics via VideoCore VII | Q1 2027 |
| **Camera CSI** | MIPI CSI-2 camera interface with V4L2 subdevice support | Q2 2027 |
| **First-Run Wizard** | Initial setup wizard for WiFi, user creation, settings | Q3 2027 |
| **Bluetooth Stack** | Complete Bluetooth 5.2 controller and host stack | Q4 2027 |
| **Audio Stack** | Complete audio mixer, routing, and application support | Q1 2028 |
| **NVMe Hotplug** | Hot-plug support for NVMe drives via PCIe | Q2 2028 |
| **SMP Scaling** | Optimize scheduler for 8+ core systems | Q3 2028 |
| **File Encryption** | eCryptfs-style encrypted home directories | Q4 2028 |

## License

MIT License - see [LICENSE](LICENSE) file for details.

## Links

- **GitHub Repository**: [https://github.com/jlaportebot/lobster-os](https://github.com/jlaportebot/lobster-os)
- **Issue Tracker**: [https://github.com/jlaportebot/lobster-os/issues](https://github.com/jlaportebot/lobster-os/issues)
- **Discussions**: [https://github.com/jlaportebot/lobster-os/discussions](https://github.com/jlaportebot/lobster-os/discussions)
- **Documentation**: [https://github.com/jlaportebot/lobster-os/tree/main/docs](https://github.com/jlaportebot/lobster-os/tree/main/docs)

---
*Built with 🦞 by the LobsterOS community*