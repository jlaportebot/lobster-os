<!-- SUPERSEDED 2026-10-25: see docs/DESKTOP_PLAN.md (living) and docs/AUDIT_2026-10-25.md (queue). Do not add work here. -->

# LobsterOS Implementation Plan

## Current State Analysis

LobsterOS is a ~28K-line `#![no_std]` Rust bare-metal OS for Raspberry Pi 5 (BCM2712). Major subsystems already implemented:

### ✅ Completed Subsystems
- **Boot**: EL2→EL1 transition, DTB parsing, timer initialization
- **Memory**: MMU (4-level page tables), frame allocator, kernel heap, per-process VMEM
- **Display**: Framebuffer, graphics primitives, console, logo renderer
- **Network**: Full TCP/IP stack (ARP, DHCP, DNS, HTTP, ICMP, IPv4, TCP, UDP, sockets)
- **USB**: XHCI controller, HID (keyboard/mouse)
- **Window Manager**: Compositor (double-buffered), windows, widgets, themes, input handling
- **Filesystem**: VFS, FAT32, ext2, initrd, procfs, sysfs, devfs, block devices
- **Scheduler**: Preemptive round-robin, EL1/EL0 tasks, priority, signals, syscalls
- **SMP**: Per-core runqueues, IPI, load balancing
- **Security**: Credentials (uid/gid), POSIX capabilities, access control
- **Init System**: systemd-like service manager, runlevels, dependency ordering
- **Shell**: Full-featured with commands (ls, cat, cd, mkdir, rm, run, svc, runlevel, net, dns, dhcp, wm, audio, gpu, env, history)
- **Drivers**: UART, GIC, timer, mailbox, GPIO, eMMC, NVMe, PCIe, GPU, audio, WiFi (CYW43439), serial input, power

---

## Implementation Plan

### Phase 1: WiFi Connectivity (Priority: HIGH)
**Goal**: Connect to WPA2/WPA3 networks from shell

#### 1.1 WiFi Shell Command Enhancement
- [ ] Add `wifi scan` - trigger scan and display results (SSID, BSSID, signal, security)
- [ ] Add `wifi connect <ssid> <password>` - WPA2-PSK association + 4-way handshake
- [ ] Add `wifi status` - show current connection state, IP, signal
- [ ] Add `wifi disconnect` - deauthenticate
- [ ] Add `wifi save <ssid>` - persist credentials to `/etc/wifi/known_networks`

#### 1.2 WiFi Driver Completion (drivers/wifi.rs)
- [ ] Complete WPA supplicant state machine (association, authentication, 4-way handshake)
- [ ] Implement CCMP/GCMP decryption for data frames
- [ ] Add roaming support (background scan, BSS transition)
- [ ] Add power save mode (PS-Poll)

#### 1.3 DHCP Integration
- [ ] Auto-trigger DHCP on successful association
- [ ] Handle DHCP renew/rebind on lease expiry
- [ ] Update `/etc/resolv.conf` with DNS from DHCP

---

### Phase 2: Package Manager (Priority: HIGH)
**Goal**: Download, verify, install ELF binaries from repositories

#### 2.1 Package Format
- [ ] Define `.lob` package format (ELF + metadata: name, version, deps, checksum, install script)
- [ ] Define repository index format (JSON: packages, versions, SHA256)

#### 2.2 Package Manager Implementation (`/sbin/pkg`)
- [ ] `pkg update` - fetch repository index via HTTP
- [ ] `pkg search <query>` - search local index
- [ ] `pkg install <name>` - download, verify checksum, extract to `/usr/bin`, `/usr/lib`, `/etc`
- [ ] `pkg remove <name>` - remove files, run uninstall script
- [ ] `pkg list` - show installed packages
- [ ] `pkg info <name>` - show package details

#### 2.3 Repository Infrastructure
- [ ] Create sample repository with base packages (coreutils, editor, etc.)
- [ ] Sign packages with Ed25519, verify on install
- [ ] Support multiple repositories with priorities

---

### Phase 3: Multi-User System (Priority: HIGH)
**Goal**: Real user accounts, login, privilege separation

#### 3.1 User Database
- [ ] `/etc/passwd` parser (username:password:uid:gid:gecos:home:shell)
- [ ] `/etc/shadow` parser (hashed passwords, aging)
- [ ] `/etc/group` parser (group:gid:members)
- [ ] `/etc/sudoers` - basic sudo support

#### 3.2 Authentication
- [ ] Password hashing (Argon2id)
- [ ] `login` command - username/password prompt, spawn shell with credentials
- [ ] `su` - switch user (requires target user's password or root)
- [ ] `sudo` - run command as another user (check sudoers)
- [ ] `passwd` - change password

#### 3.3 Session Management
- [ ] TTY allocation per login
- [ ] Login shell environment (HOME, USER, PATH, SHELL)
- [ ] Session tracking (utmp/wtmp)

#### 3.4 Integration Points
- [ ] VFS permission checks use real uid/gid
- [ ] Process credentials initialized from user database
- [ ] Service manager can run services as specific users

---

### Phase 4: Desktop Environment (Priority: MEDIUM)
**Goal**: Usable GUI with wallpaper, panel, app launcher

#### 4.1 Compositor Enhancements
- [ ] Background layer (wallpaper image, solid color, gradient)
- [ ] `wm wallpaper <path|color>` shell command
- [ ] Wallpaper scaling modes: fill, fit, stretch, center, tile

#### 4.2 Window Decorations
- [ ] Titlebar with app name, icon
- [ ] Window buttons: close, minimize, maximize
- [ ] Draggable titlebar for move
- [ ] Resize handles (edges/corners)
- [ ] Themeable decorations (colors, border radius)

#### 4.3 Desktop Panel/Taskbar
- [ ] Persistent panel at bottom/top
- [ ] App launcher button (opens menu)
- [ ] Running app indicators (pinned + dynamic)
- [ ] System tray: clock, network, volume, battery
- [ ] Workspace switcher

#### 4.4 Application Launcher
- [ ] Grid/menu of installed apps (from .desktop files in `/usr/share/applications`)
- [ ] Search/filter
- [ ] Categories

#### 4.5 Core Desktop Apps
- [ ] Terminal emulator (PTY + shell)
- [ ] File manager (GUI for VFS)
- [ ] Text editor
- [ ] Settings app (wallpaper, theme, network, users)

---

### Phase 5: System Infrastructure (Priority: MEDIUM)

#### 5.1 Configuration System
- [ ] `/etc` filesystem (persistent, FAT32 or ext2 on eMMC/SD)
- [ ] Config file parser (INI/TOML-like)
- [ ] `sysctl` - runtime kernel parameters

#### 5.2 Logging
- [ ] Kernel ring buffer (dmesg)
- [ ] syslog daemon (`/sbin/syslogd`)
- [ ] `/var/log/messages`, `/var/log/kernel.log`
- [ ] Log rotation

#### 5.3 Device Hotplug (udev-like)
- [ ] Device event queue (USB insert/remove, block devices)
- [ ] Rules engine (`/etc/udev/rules.d/`)
- [ ] Auto-mount removable storage
- [ ] Auto-load drivers

#### 5.4 Namespaces/Cgroups (Container Support)
- [ ] PID namespaces
- [ ] Mount namespaces
- [ ] Network namespaces
- [ ] UTS/IPC/User namespaces
- [ ] Cgroup v1/v2 (cpu, memory, io limits)

---

### Phase 6: Testing & Polish (Priority: HIGH)

#### 6.1 Build Verification
- [ ] `cargo build --target aarch64-unknown-none` - zero errors
- [ ] `cargo clippy -- -D warnings` - zero warnings
- [ ] `cargo fmt` - formatting clean

#### 6.2 QEMU Testing
- [ ] Boot to shell
- [ ] WiFi scan/connect (simulated)
- [ ] Network stack (ping, HTTP)
- [ ] Filesystem operations
- [ ] Process spawn/kill
- [ ] WM compositor rendering

#### 6.3 Hardware Testing (Pi 5)
- [ ] Boot on real hardware
- [ ] WiFi connect to real AP
- [ ] Display output (HDMI)
- [ ] USB keyboard/mouse
- [ ] Audio output

---

## Technical Approach

### Development Principles
1. **TDD**: Write failing test first, then implement
2. **Small increments**: Each commit < 200 lines where possible
3. **No `unwrap()` in production**: Use `?`, `ok_or()`, `expect()` with context
4. **Safety comments**: Every `unsafe` block documented
5. **Domain modules**: Code organized by feature (net/, fs/, wm/, not models/)

### Testing Strategy
- Unit tests in `#[cfg(test)]` modules alongside code
- Integration tests via QEMU (serial output parsing)
- Property-based testing for allocators, MMU, network parsers

### Build System
- `cargo xtask` for custom build steps (kernel8.img generation)
- Cross-compilation: `aarch64-unknown-none`
- Profile: `release` with LTO, panic=abort

---

## File Structure for New Code

```
src/
├── net/
│   ├── wifi_supplicant.rs    # NEW: WPA state machine
│   └── dhcp_client.rs        # NEW: DHCP client logic
├── pkg/                      # NEW: package manager
│   ├── mod.rs
│   ├── repo.rs
│   ├── package.rs
│   └── install.rs
├── users/                    # NEW: multi-user
│   ├── mod.rs
│   ├── passwd.rs
│   ├── shadow.rs
│   ├── auth.rs
│   └── session.rs
├── wm/
│   ├── wallpaper.rs          # NEW: background layer
│   ├── decorations.rs        # NEW: window chrome
│   ├── panel.rs              # NEW: taskbar
│   └── launcher.rs           # NEW: app menu
├── config/                   # NEW: /etc parser
│   └── mod.rs
├── log/                      # NEW: syslog
│   └── mod.rs
├── udev/                     # NEW: hotplug
│   └── mod.rs
└── ns/                       # NEW: namespaces
    └── mod.rs
```

---

## Dependencies to Add (Cargo.toml)

```toml
[dependencies]
# Crypto for package signing, password hashing
ed25519-dalek = "2.0"
argon2 = { version = "0.5", features = ["std"] }  # or no_std variant
sha2 = "0.10"
aes-gcm = "0.10"  # for WPA CCMP

# Config parsing
toml = "0.8"

# Date/time for logs
time = { version = "0.3", features = ["std"] }
```

> Note: Most crates need `std`. For `#![no_std]`, we'll use `alloc` + `hashbrown` + custom implementations or `no_std` compatible crates.

---

## Estimated Effort

| Phase | Estimated LOC | Time |
|-------|--------------|------|
| 1. WiFi | ~2,000 | 1-2 weeks |
| 2. Package Manager | ~3,000 | 2-3 weeks |
| 3. Multi-User | ~2,500 | 1-2 weeks |
| 4. Desktop | ~4,000 | 3-4 weeks |
| 5. Infrastructure | ~3,000 | 2-3 weeks |
| 6. Testing | ~1,000 | 1 week |
| **Total** | **~15,500** | **10-15 weeks** |

---

## Next Steps

1. Start with **Phase 1.1**: Add WiFi shell commands (scan, connect, status, disconnect)
2. Implement WPA supplicant state machine in `drivers/wifi.rs`
3. Test with QEMU + real hardware
4. Proceed to Phase 2

---

*This plan will be updated as implementation progresses.*