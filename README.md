# 🦞 LobsterOS

A bare-metal `#![no_std]` Rust operating system for Raspberry Pi 5 (BCM2712 SoC). Built from scratch with ~67K lines of Rust across 135 files. 174KB kernel image.

## Features

### Core System

| Subsystem | Status | Description |
|-----------|--------|-------------|
| Boot & MMU | ✅ Working | EL2 entry, 4-level page tables, frame allocator, kernel heap, 1-16GB RAM detection via DTB |
| SMP Scheduler | ✅ Working | Per-core runqueues, IPI-based coordination, work-stealing load balancing |
| Memory Management | ✅ Complete | Frame allocator, kernel heap, process address spaces, demand paging, OOM handler |
| Networking Stack | ✅ Working | GENET Ethernet, ARP, IPv4, ICMP, UDP, TCP, DHCP, DNS, HTTP client |
| USB Stack | ✅ Working | XHCI controller, HID (keyboard/mouse), hub, mass storage, device enumeration |
| Filesystem | ✅ Multiple | VFS layer, FAT32, ext2 (with fsck), procfs, sysfs, devfs, initrd, block layer |
| Shell | ✅ Functional | Interactive shell with built-in commands: `ls`, `cat`, `wifi`, `pkg`, `uptime`, `ps`, `kill` |
| Process Management | ✅ Working | ELF loader, dynamic linker, signals, syscalls, process hierarchy, orphan reaping |
| Security | ✅ Working | Capability-based security, users/groups, authentication, sessions |
| Terminal Emulator | ✅ Working | VT100/ANSI parser, 256-color, truecolor RGB, scrollback (10K lines), copy/paste clipboard |
| IPC | ✅ Working | Message passing, shared memory, PTY support, Unix domain sockets |
| Device Tree | ✅ Working | DTB parsing, hardware enumeration, memory node detection (16GB support) |
| Power Management | ✅ Working | DVFS, thermal monitoring, watchdog timer, suspend/resume |

### Desktop Environment

| Feature | Status | Description |
|---------|--------|-------------|
| Window Manager | ✅ Working | Compositor, overlapping windows, widgets, themes (dark/light), input handling, panel, launcher |
| Window Snapping | ✅ Working | Half-screen snap, maximize, edge-resize, keyboard shortcuts |
| Virtual Desktops | ✅ Working | 4 workspaces, keyboard shortcuts to switch |
| Context Menus | ✅ Working | Right-click context menu on desktop and windows |
| Themes | ✅ Working | Dark (default) + Light themes, 6 accent colors (Blue/Green/Orange/Purple/Red/Teal) |
| Font Size | ✅ Working | 4 sizes (Small/Medium/Large/ExtraLarge) with runtime adjustment |
| Color Support | ✅ Working | 16-color, 256-color palette, truecolor RGB in terminal |
| Boot Splash | ✅ Working | Graphical logo + progress bar, smooth transition to desktop |

### GUI Applications

| App | Description |
|-----|-------------|
| Terminal | VT100/ANSI emulator with scrollback, copy/paste, Ctrl+Shift+C/V |
| File Manager | Directory navigation, copy/move/delete/rename, file preview, bookmarks |
| Text Editor | Gap buffer, syntax highlighting, search/replace |
| Settings | Network (WiFi), Display (resolution, wallpaper), Users, System |
| Calculator | Basic calculator with GUI |
| System Monitor | CPU, memory, network, temperature monitoring |
| Image Viewer | Bitmap image display |
| Process Manager | Process list with kill capability |
| Network Manager | WiFi scan and connect GUI |
| App Store | Package manager GUI |
| Setup Wizard | First-boot configuration (language, WiFi, user, hostname) |

### Hardware Drivers

| Component | Status | Details |
|-----------|--------|---------|
| RAM Detection | ✅ Working | 1GB/2GB/4GB/8GB/16GB LPDDR4X detection via DTB |
| BCM2712 SoC | ✅ Working | Broadcom BCM2712 ARM Cortex-A76 (4x) @ 2.4GHz |
| GIC-400 | ✅ Working | Generic Interrupt Controller for SMP |
| GENET Ethernet | ✅ Working | Gigabit Ethernet controller with DMA |
| xHCI USB | ✅ Working | USB 3.0/2.0 host controller |
| CYW43455 WiFi/BT | ✅ Working | SDIO WiFi (802.11b/g/n/ac) + Bluetooth (HCI, L2CAP, GATT) |
| GPIO | ✅ Working | 28x programmable GPIO pins with interrupt + userspace API |
| Display | ✅ Working | HDMI 2.0a, dual display (Extended/Mirror), rotation (0/90/180/270), brightness control |
| Audio | ✅ Working | PWM audio, I2S, software mixer (multiple streams), 48kHz/16-bit stereo |
| Storage | ✅ Working | eMMC 5.1, SD/UHS-I, NVMe via PCIe |
| Power Management | ✅ Working | DVFS (governors: performance/powersave/ondemand), thermal management, watchdog |
| Bluetooth | ✅ Working | HCI over SDIO, L2CAP channels (8), GATT server (services, characteristics, read/write/notify) |

### Kernel Subsystems

| Subsystem | Status | Description |
|-----------|--------|-------------|
| Config System | ✅ Working | TOML-like config parser, /etc/lobsteros.conf, os-release |
| Syslog Daemon | ✅ Working | Kernel log ring buffer, /dev/kmsg, syslogd daemon |
| Panic Recovery | ✅ Working | Auto-reboot on crash, crash logging |
| OOM Handler | ✅ Working | Soft/hard severity, victim selection by runtime, SIGKILL delivery |
| Namespaces | ✅ Working | PID, Mount, Network, UTS, User, IPC — unshare()/setns() syscalls |
| Cgroups | ✅ Working | CPU, Memory, IO, PIDs controllers, hierarchy management |
| Container Support | ✅ Working | pivot_root(), create_container_ns() (full 6-namespace isolation) |
| Package Manager | 🚧 Scaffolding | Repository format, HTTP index download, installer, .lobpkg format |

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
# Produces: build/kernel8.img (174 KB)
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

### QEMU Testing
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

## Keyboard Shortcuts

| Shortcut | Action |
|----------|--------|
| `Super + ←/→` | Snap window left/right (half-screen) |
| `Super + ↑` | Maximize window |
| `Super + ↓` | Minimize window |
| `Alt + Tab` | Cycle window focus |
| `Alt + Shift + Tab` | Reverse cycle |
| `Alt + F4` | Close focused window |
| `Alt + 1` → `Alt + 4` | Switch virtual desktop |
| `Super + D` | Show desktop |
| `Super + L` | Lock screen |
| `Super + T` | Open terminal |
| `Super + E` | Open file manager |
| `Super + S` | Open settings |
| `Ctrl+Shift+C` | Copy (terminal) |
| `Ctrl+Shift+V` | Paste (terminal) |
| `Shift+PageUp` | Scrollback up (terminal) |
| `Shift+PageDown` | Scrollback down (terminal) |

## Architecture Overview

```
lobster-os/
├── src/
│   ├── apps/           # GUI applications (11 apps)
│   │   ├── calculator.rs, editor.rs, file_manager.rs
│   │   ├── image_viewer.rs, network_manager.rs
│   │   ├── process_manager.rs, settings.rs
│   │   ├── setup_wizard.rs, sysmon.rs, terminal.rs
│   │   └── app_store.rs
│   ├── arch/           # AArch64 architecture-specific code
│   ├── display/        # Framebuffer, graphics, console, logo, splash screen
│   ├── drivers/        # Hardware drivers (audio, bluetooth, emmc, gpio, gpu,
│   │                   #   mailbox, nvme, pcie, power_mgmt, sdio, uart, wifi)
│   ├── dtb/            # Device tree parsing (memory detection)
│   ├── fs/             # VFS, FAT32, ext2, block device, initrd
│   ├── ipc/            # IPC (message passing, shared memory, PTY, sockets)
│   ├── klog.rs         # Kernel logging
│   ├── memory/         # Frame allocator, heap, MMU, vmem, OOM handler
│   ├── net/            # Network stack (ARP, IPv4, TCP, UDP, DHCP, DNS, HTTP)
│   ├── pkg/            # Package manager
│   ├── sched/          # Scheduler, processes, signals, syscalls, ELF
│   ├── security/       # Capabilities, users, authentication
│   ├── shell/          # Interactive shell
│   ├── smp/            # SMP (runqueues, IPI, load balancing)
│   ├── sync/           # Synchronization (spinlock)
│   ├── terminal/       # VT100 terminal emulator (parser, screen, scrollback)
│   ├── usb/            # USB stack (XHCI, HID, hub, mass storage)
│   ├── users/          # User/group management
│   └── wm/             # Window manager (compositor, windows, widgets, panel,
│                       #   launcher, input, theme, workspace, context_menu)
├── .github/workflows/  # CI/CD pipeline
├── scripts/            # Build scripts (SD card image builder)
├── tests/              # Integration tests
└── build/              # Build artifacts (kernel8.img)
```

## CI/CD

GitHub Actions pipeline (`.github/workflows/ci.yml`):
- `cargo check --target aarch64-unknown-none`
- `cargo clippy --target aarch64-unknown-none -- -D warnings`
- `cargo fmt --check`
- Release build verification

## Boot Sequence

1. EL2 entry, BSS zeroing
2. DTB parsing (early — for RAM detection)
3. GIC-400 interrupt controller init
4. System timer init
5. IRQ enable
6. MMU init + frame allocator + kernel heap
7. **Display framebuffer + boot splash screen**
8. VFS init + user database + **first-run setup wizard check**
9. Scheduler + SMP (4 cores)
10. SDIO/WiFi init + network stack
11. eMMC/SD storage init
12. ext2 /etc mount + package manager config
13. USB stack init
14. Window manager init
15. Core apps init (11 apps)
16. Power management (DVFS + watchdog)
17. Audio mixer init
18. GPIO userspace API
19. System config + syslog daemon
20. Panic recovery init
21. **Bluetooth init + GATT server**
22. **Multi-display (dual HDMI) init**
23. Shell spawn
24. Main loop: WM tick + splash tick + watchdog pet

## Stats

- **Lines of code**: ~67,000 (Rust)
- **Source files**: 135 (.rs)
- **Kernel image**: 174 KB (kernel8.img)
- **Target**: aarch64-unknown-none (bare metal)
- **Warnings**: 0 (`cargo clippy -- -D warnings` clean)
- **Errors**: 0

## License

MIT License - see [LICENSE](LICENSE) file for details.

---

*Built with 🦞 by Mister Lobster*
