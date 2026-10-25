<!-- SUPERSEDED 2026-10-25: see docs/DESKTOP_PLAN.md (living) and docs/AUDIT_2026-10-25.md (queue). Do not add work here. -->

# LobsterOS Completion Plan — v0.5 Daily Driver

## Current State
- 61,272 LOC, 115 .rs files, compiles clean (aarch64), clippy clean
- 174KB kernel8.img, v0.1.0
- 20+ subsystems implemented, Pi 5 BCM2712 target

## Phase A: Core System Fixes (Foundation)
### A1. 16GB RAM Detection
- [ ] Replace hardcoded 8GB with DTB memory node parsing
- [ ] Add 16GB variant support in frame allocator
- [ ] Test with 16GB memory map

### A2. Config System
- [ ] /etc persistence via ext2
- [ ] TOML config parser (no_std)
- [ ] sysctl-like runtime config
- [ ] /etc/os-release, /etc/lobsteros.conf

### A3. Syslog Daemon
- [ ] Userspace syslogd reading /dev/kmsg
- [ ] /var/log/messages with rotation
- [ ] Log levels and filtering

## Phase B: Kernel Completeness
### B1. Namespaces
- [ ] PID namespace
- [ ] Mount namespace
- [ ] Network namespace
- [ ] UTS namespace
- [ ] User namespace

### B2. cgroups
- [ ] CPU controller
- [ ] Memory controller
- [ ] IO controller
- [ ] PIDs controller

### B3. Container Syscalls
- [ ] unshare()
- [ ] setns()
- [ ] pivot_root()

## Phase C: Desktop Polish
### C1. Window Manager
- [ ] Window snapping (half-screen, maximize)
- [ ] Virtual desktops (4 workspaces)
- [ ] Keyboard shortcuts (Alt+Tab, Super+D, etc.)
- [ ] Window resize from any edge
- [ ] Minimize/maximize animations
- [ ] Right-click context menus

### C2. New Apps
- [ ] Calculator
- [ ] System Monitor (CPU, mem, net, temp)
- [ ] Image Viewer (basic bitmap)
- [ ] Process Manager
- [ ] Network Manager (WiFi scan, connect)
- [ ] App Store (package manager GUI)

### C3. Terminal Improvements
- [ ] Tab support
- [ ] Copy/paste (keyboard + mouse)
- [ ] Scrollback search
- [ ] Unicode support

### C4. File Manager
- [ ] Directory tree navigation
- [ ] File operations (copy, move, delete, rename)
- [ ] File preview
- [ ] Bookmarkbar

### C5. Themes
- [ ] Light theme
- [ ] Dark theme (default)
- [ ] System accent colors
- [ ] Font size adjustment

## Phase D: Pi 5 Hardware Completeness
### D1. Audio
- [ ] I2S audio driver (RP1)
- [ ] HDMI audio
- [ ] Audio mixer API (ALSA-like)
- [ ] Volume control applet

### D2. Bluetooth
- [ ] HCI over SDIO (CYW43455)
- [ ] L2CAP + GATT
- [ ] BT audio (A2DP)
- [ ] Bluetooth settings panel

### D3. Power Management
- [ ] CPU frequency scaling (DVFS)
- [ ] Suspend/resume
- [ ] Thermal management (temp sensor)
- [ ] Watchdog timer

### D4. Advanced Display
- [ ] V3D GPU 3D acceleration
- [ ] HDMI dual display
- [ ] Display rotation
- [ ] Brightness control

### D5. GPIO Userspace
- [ ] /dev/gpiochip0 device
- [ ] libgpiod-like API

### D6. Camera (CSI)
- [ ] MIPI CSI-2 receiver
- [ ] Basic camera capture

## Phase E: System Tooling
### E1. SD Card Image Builder
- [ ] Create bootable SD image
- [ ] bootcode.bin + start.elf + kernel8.img
- [ ] config.txt generator
- [ ] ext2 rootfs builder

### E2. First-Run Setup
- [ ] Welcome wizard
- [ ] WiFi configuration
- [ ] User account creation
- [ ] Timezone selection

### E3. CI/CD
- [ ] cargo check (aarch64)
- [ ] clippy (aarch64, no features)
- [ ] fmt check
- [ ] release build
- [ ] QEMU boot smoke test

## Phase F: Polish & Testing
### F1. Boot Performance
- [ ] Boot time optimization
- [ ] Boot messages (clean, informative)
- [ ] Splash screen → desktop

### F2. Stability
- [ ] Panic recovery
- [ | OOM handler
- [ ] Watchdog reset

### F3. README + Docs
- [ ] Updated README with full feature list
- [ ] Installation guide
- [ ] User guide
