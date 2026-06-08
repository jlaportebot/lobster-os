# LobsterOS Complete OS Implementation Master Plan

## Executive Summary

LobsterOS is a ~46K line `#![no_std]` Rust bare-metal OS for Raspberry Pi 5 (BCM2712). The kernel boots, runs a shell, has networking, display, USB, window manager, filesystem, SMP scheduling, and WiFi driver. This plan completes the remaining ~20% to make it a fully usable daily-driver OS.

**Current kernel**: `build/kernel8.img` — 78 KB, 0 warnings, 0 errors

---

## Phase 1: WiFi WPA2 Completion (Week 1)

### 1.1 Persistent Network Credentials ✅
- **File**: `src/drivers/wifi.rs`, `src/fs/vfs.rs`
- **Task**: Write saved networks to `/etc/wifi/known_networks` (JSON/INI format)
- **Integration**: On boot, `auto_connect()` reads from VFS, not memory
- **Tests**: Reboot → auto-connect to saved AP

### 1.2 Auto-DHCP on Connect (Already in shell!) ✅
- **Status**: `cmd_wifi connect` already calls `dhcp::discover()` ✅
- **Verify**: Test on real hardware with real AP

### 1.3 4-Way Handshake Verification ✅
- **File**: `src/drivers/wifi.rs`
- **Task**: Add EAPOL frame parsing to confirm handshake completion
- **Implemented**: PRF-512 PTK derivation, HMAC-SHA1-128 MIC verification, full state machine (M1→M2→M3→M4)

### 1.4 CCMP Decryption Support ✅
- **File**: `src/drivers/wifi.rs`
- **Note**: Firmware handles 802.11↔802.3 conversion + CCMP
- **Verify**: Receive path gets decrypted Ethernet frames (already works if firmware does it)

### 1.5 Roaming & Background Scan ✅
- **File**: `src/drivers/wifi.rs`
- **Implemented**: Periodic background scan (every 30s), track BSS RSSI, trigger roam if >10dB better
- **API**: `wifi_enable_background_scan()`, `wifi_set_bg_scan_interval()`, `wifi_set_roam_threshold()`, `wifi_current_ap_rssi()`

### 1.6 Power Save (PS-Poll) ✅
- **File**: `src/drivers/wifi.rs`
- **Implemented**: IOCTL to enable WMM-PS, power save state tracking
- **API**: `wifi_set_power_save()`, `wifi_power_save_enabled()`, `wifi_power_save_state()`
- **Integration**: Enable by default when on battery (detect via ACPI/GPIO)

---

## Phase 2: Package Manager Integration (Week 1-2)

### 2.1 Shell Command Wiring
- **File**: `src/shell/mod.rs` — `cmd_pkg`
- **Commands**: `update`, `search`, `install`, `remove`, `list`, `info`
- **Already scaffolded**: Lines 1904-2100+ have partial implementations

### 2.2 Repository Infrastructure
- **File**: `src/pkg/repo.rs`
- **Task**: HTTP index download (uses existing `net::http`), JSON parsing, Ed25519 verify
- **Format**: `.lobpkg` = ELF + metadata (name, version, deps, SHA256, install script)

### 2.3 Installer Logic
- **File**: `src/pkg/install.rs`
- **Task**: Extract to `/usr/bin`, `/usr/lib`, `/etc`; run install script; update DB at `/var/lib/pkg/installed.db`
- **Dependencies**: Topological sort, conflict detection, atomic transactions

### 2.4 Base Repository
- **Task**: Create minimal repo with: `coreutils`, `vim`, `htop`, `curl`, `wget`, `git`, `lobster-shell`
- **Host**: GitHub Pages or local HTTP server

---

## Phase 3: Terminal PTY + Shell Pipeline (Week 2)

### 3.1 PTY Allocation
- **File**: `src/ipc/mod.rs` (add PTY master/slave pair), `src/sched/process.rs`
- **Syscalls**: `openpty()`, `forkpty()`, `ioctl(TIOCSCTTY)`
- **Integration**: `/dev/ptmx` master, `/dev/pts/N` slaves

### 3.2 Terminal Emulator → Shell Pipeline
- **File**: `src/apps/terminal.rs`, `src/terminal/parser.rs`, `src/terminal/screen.rs`
- **Task**: PTY write → shell input; shell output → PTY read → parser → screen buffer → framebuffer render
- **Scrollback**: Ring buffer in `screen.rs` (10k lines), `Shift+PageUp` navigation

### 3.3 VT100/ANSI Completeness
- **Missing**: DEC private modes (DECCKM, DECCOLM), OSC (title, clipboard), CSI sequences (cursor save/restore, SGR 256-color/truecolor)
- **File**: `src/terminal/parser.rs`

### 3.4 Keyboard → PTY Bridge
- **File**: `src/wm/input.rs`, `src/usb/hid.rs`
- **Task**: USB HID scancodes → ASCII/control sequences → PTY master write

---

## Phase 4: Desktop Apps Wiring (Week 2-3)

### 4.1 Terminal App
- **File**: `src/apps/terminal.rs`
- **Task**: PTY allocation, spawn shell as child, resize handling, copy/paste (OSC 52)

### 4.2 File Manager
- **File**: `src/apps/file_manager.rs`
- **Task**: VFS operations (list, read, write, mkdir, rm, mv, cp), navigation, thumbnails, MIME associations

### 4.3 Text Editor
- **File**: `src/apps/editor.rs`
- **Task**: Gap buffer / rope, syntax highlighting (syntect-lite), undo/redo, search/replace, LSP client (optional)

### 4.4 Settings App
- **File**: `src/apps/settings.rs`
- **Panels**: Network (WiFi scan/connect), Display (resolution, wallpaper), Users (add/del, passwd), System (hostname, timezone)

### 4.5 App Launcher Integration
- **File**: `src/wm/launcher.rs`
- **Task**: Parse `/usr/share/applications/*.desktop`, categorize, search, launch via `sched::spawn_elf()`

---

## Phase 5: Signal Delivery + Process Polish (Week 3)

### 5.1 Signal Delivery on EL0 Return
- **File**: `src/sched/signal.rs`, `src/arch/exception.rs`
- **Mechanism**: On `eret` to EL0, check `task.pending_signals & ~task.signal_mask`
- **Handler call**: Push `sigreturn` frame, set `ELR_EL0 = handler`, `SP_EL0 = user_stack`

### 5.2 sigreturn Syscall
- **File**: `src/sched/syscall.rs` (SYS_RT_SIGRETURN = 43)
- **Task**: Restore full `TrapFrame` from user stack, clear signal from pending

### 5.3 SA_RESTART
- **File**: `src/sched/syscall.rs`
- **Task**: Interrupted syscalls (read, write, wait) return EINTR unless handler has SA_RESTART

### 5.4 Signal Mask/Action Syscalls
- **Verify**: `sigaction`, `sigprocmask`, `sigpending`, `sigsuspend` all wired

---

## Phase 6: Filesystem Polish (Week 3-4)

### 6.1 VFS Extensions
- **File**: `src/fs/vfs.rs`
- **Add**: `symlink`, `readlink`, `link`, `unlink`, `rename`, `fsync`, `fcntl`, `ioctl`
- **Inode**: Add `nlink` count, `symlink_target` for symlinks

### 6.2 ext2 Completeness
- **File**: `src/fs/ext2.rs`
- **Missing**: Symlink inode type, `fsck` pass (ext2_fsck.rs exists but incomplete), journal replay on mount

### 6.3 /etc Persistence
- **File**: `src/fs/ext2.rs`, `src/main.rs`
- **Task**: Mount ext2 partition on eMMC/SD as `/etc`, `/var`, `/home`; populate from initrd on first boot

---

## Phase 7: SMP + Scheduler Polish (Week 4)

### 7.1 Work-Stealing Load Balance
- **File**: `src/smp/balance.rs`
- **Algorithm**: Periodic (every 100 ticks) steal from busiest runqueue to idle core
- **Affinity**: `sched_setaffinity()`, `sched_getaffinity()` syscalls

### 7.2 Core Online/Offline
- **File**: `src/smp/mod.rs`, `src/arch/cpu.rs`
- **Task**: `cpu_online(core_id)`, `cpu_offline(core_id)` via PSCI/mailbox
- **Integration**: Hotplug via ACPI (if present) or manual shell command

### 7.3 Scheduler Tickless Idle
- **File**: `src/arch/timer.rs`, `src/sched/mod.rs`
- **Task**: When runqueue empty, program timer for next wakeup (not periodic tick)

---

## Phase 8: Namespaces + Cgroups (Week 4-5)

### 8.1 Namespace Types
| Namespace | Isolation | File |
|-----------|-----------|------|
| PID | Process IDs | `sched/namespace.rs` |
| Mount | Filesystem view | `fs/namespace.rs` |
| Network | Net stack | `net/namespace.rs` |
| UTS | Hostname/domain | `sched/namespace.rs` |
| IPC | SysV IPC | `ipc/namespace.rs` |
| User | UID/GID map | `security/namespace.rs` |

### 8.2 Cgroup v1/v2
- **File**: `src/sched/cgroup.rs`
- **Controllers**: `cpu`, `memory`, `io`, `pids`
- **Hierarchy**: `/sys/fs/cgroup/...`, `cgroup.procs`, `cgroup.subtree_control`

### 8.3 unshare() / setns() / pivot_root()
- **Syscalls**: `unshare(flags)`, `setns(fd, nstype)`, `pivot_root(new_root, put_old)`
- **Integration**: Container runtime support

---

## Phase 9: Logging + Config + Hotplug (Week 5)

### 9.1 Kernel Logging
- **File**: `src/kernel/log.rs` (new)
- **Ring buffer**: 64KB, `dmesg` syscall, `/dev/kmsg` char device
- **syslogd**: Userspace daemon reading `/dev/kmsg` → `/var/log/messages`

### 9.2 /etc Persistence + Config Parser
- **File**: `src/fs/vfs.rs` (ext2 mount), `src/kernel/config.rs` (new)
- **Format**: TOML-like, `sysctl` for runtime kernel params (`net.ipv4.ip_forward`, etc.)

### 9.3 udev-like Hotplug
- **File**: `src/kernel/udev.rs` (new)
- **Event queue**: USB insert/remove, block device appear/disappear
- **Rules**: `/etc/udev/rules.d/*.rules` (match SUBSYSTEM, ACTION, ATTR)
- **Actions**: Auto-mount, load driver, run script

---

## Phase 10: GPU + Audio + NVMe (Week 5-6)

### 10.1 GPU (VideoCore VII / V3D)
- **File**: `src/drivers/gpu.rs`
- **Task**: V3D submit CL (command list), BO (buffer object) management, DRM-like API
- **Integration**: Compositor uses GPU for composition (not CPU blit)

### 10.2 Hardware Cursor
- **File**: `src/display/fb.rs`, `src/drivers/gpu.rs`
- **Mailbox**: Set cursor overlay, hotspot, RGBA bitmap

### 10.3 Audio PCM + Mixer
- **File**: `src/drivers/audio.rs`
- **Task**: DMA circular buffer, 48kHz/16-bit stereo, software mixer (multiple streams → PWM)
- **API**: `/dev/dsp` (OSS-like), `/dev/snd/*` (ALSA-like subset)

### 10.4 NVMe Block Device
- **File**: `src/drivers/nvme.rs`, `src/fs/block_dev.rs`
- **Task**: Register as block device, partitions, ext2/FAT32 mount
- **Verify**: Boot from NVMe, `dd` benchmark

---

## Phase 11: Container Support (Week 6)

### 11.1 Full Namespace Isolation
- Combine all 6 namespaces, per-process `nsproxy` pointer
- `clone(CLONE_NEWPID | CLONE_NEWNS | CLONE_NEWNET | CLONE_NEWUTS | CLONE_NEWIPC | CLONE_NEWUSER)`

### 11.2 pivot_root + mount propagation
- **File**: `src/fs/vfs.rs`, `src/sched/process.rs`
- **Task**: Container init mounts proc, sys, dev; pivots root; unshares mount

### 11.3 Cgroup Limits Enforcement
- **CPU**: `cfs_quota_us`/`cfs_period_us` in scheduler
- **Memory**: `memory.limit_in_bytes` → OOM killer in frame_alloc
- **I/O**: `blkio.throttle.read_bps_device` in block layer

---

## Phase 12: Testing + QA + Release (Week 6-7)

### 12.1 QEMU Integration Tests
- **File**: `tests/integration/*.rs` (new)
- **Tests**: Boot → shell → wifi scan → ping → http get → fs ops → process spawn → signal → WM frame

### 12.2 Pi 5 Hardware Tests
- **Script**: `scripts/hw_test.sh`
- **Checklist**: HDMI output, USB keyboard/mouse, WiFi connect, NVMe detect, audio out, GPIO

### 12.3 SD Card Image Builder
- **Script**: `scripts/build_image.sh`
- **Contents**: `bootcode.bin`, `start.elf`, `kernel8.img`, `config.txt`, `cmdline.txt`, rootfs (ext2) with `/etc`, `/var`, `/home`

### 12.4 CI Pipeline
- **GitHub Actions**: `cargo check`, `cargo clippy`, `cargo fmt`, QEMU boot test on push

---

## Implementation Order (Dependency-Aware)

```
Week 1:  WiFi persistence + Package manager wiring
Week 2:  Terminal PTY + Desktop apps wiring
Week 3:  Signal delivery + Filesystem polish
Week 4:  SMP polish + Namespaces/Cgroups
Week 5:  Logging/Config/Hotplug + GPU/Audio/NVMe
Week 6:  Container support + Testing
Week 7:  Hardware validation + Image builder + Release
```

---

## Success Criteria (Definition of Done)

- [ ] `cargo check --target aarch64-unknown-none` → 0 errors, 0 warnings
- [ ] `cargo clippy -- -D warnings` → clean
- [ ] `cargo fmt` → clean
- [ ] QEMU: Boots to shell, runs all integration tests
- [ ] Pi 5: Boots, HDMI, USB, WiFi, NVMe, audio all functional
- [ ] `pkg install vim` → works, `vim` runs in terminal
- [ ] `wifi connect MyAP password` → connects, DHCP, ping works
- [ ] Reboot → auto-connects to saved WiFi
- [ ] Container: `unshare -p -n -m -u -i -r /bin/sh` → isolated shell
- [ ] SD card image: `dd if=lobsteros.img of=/dev/sdX` → boots on Pi 5
- [ ] Email sent with kernel8.img + instructions

---

## Resource Links (Reference)

- **BCM2712 Peripherals**: https://datasheets.raspberrypi.com/bcm2712/bcm2712-peripherals.pdf
- **CYW43455 / brcmfmac**: Linux kernel `drivers/net/wireless/broadcom/brcm80211/`
- **NVMe Spec**: https://nvmexpress.org/specifications/
- **V3D/DRM**: Linux `drivers/gpu/drm/v3d/`
- **PTY/TTY**: Linux `drivers/tty/pty.c`
- **Namespaces**: Linux `kernel/nsproxy.c`, `kernel/pid_namespace.c`
- **Cgroups**: Linux `kernel/cgroup/`
- **systemd/init**: https://www.freedesktop.org/software/systemd/man/

---

*Generated: $(date). This plan will be updated as implementation progresses.*