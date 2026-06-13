# LobsterOS Daily Driver Roadmap

## Executive Summary

Transforming LobsterOS from a working hobby OS to a daily driver requires addressing ~16 major gaps. This is **~3-5 person-years of work**. This roadmap prioritizes by **impact vs effort** and **dependency order**.

---

## Phase 0: Foundation (Weeks 1-4) — DO FIRST

| Item | Effort | Dependencies | Impact | Status |
|------|--------|--------------|--------|--------|
| **Embed WiFi firmware blobs** | 1 week | None | 🔥 Unlocks WiFi | ✅ **DONE** |
| **RP1 peripheral drivers (GPIO, I2C, SPI, UART, PWM, I2S)** | 2 weeks | None | 🔥 Unlocks audio, GPIO userspace, HATs | ✅ **DONE** |
| **GPIO userspace API (/dev/gpiochip + libgpiod)** | 1 week | RP1 GPIO | 🔥 Enables user apps | ✅ **DONE** |
| **NVMe hardware validation & fixes** | 1 week | NVMe driver | 🔥 Storage reliability | 🔄 PENDING HW |
| **Bootloader: minimal UEFI stub / config.txt generator** | 1 week | None | 🔧 Easier deployment | 🔄 PENDING |

---

## Phase 1: Core Desktop (Weeks 5-12)

| Item | Effort | Dependencies | Impact | Status |
|------|--------|--------------|--------|--------|
| **Audio Advanced: I2S/PCM/HDMI via RP1 + mixer + ALSA-like API** | 3 weeks | RP1 I2S | 🔥 Real audio | ✅ **I2S DONE** |
| **Bluetooth: HCI over SDIO + minimal BT stack (L2CAP, GATT)** | 4 weeks | WiFi firmware, SDIO | 🔥 Peripherals | 🔄 PENDING |
| **Package repository + 20+ .lobpkg packages** | 2 weeks | pkg manager | 🔥 App ecosystem | 🔄 PENDING |
| **Graphical installer (SD card writer + config generator)** | 2 weeks | Bootloader | 🔧 UX | 🔄 PENDING |
| **Logging daemon (syslog + log rotation)** | 1 week | VFS, shell | 🔧 Debuggability | 🔄 PENDING |
| **Systemd-like init (service supervision, dependencies, sockets)** | 2 weeks | shell, IPC | 🔧 Service mgmt | 🔄 PENDING |

---

## Phase 2: Graphics & Media (Weeks 13-26)

| Item | Effort | Dependencies | Impact |
|------|--------|--------------|--------|
| **V3D GPU compute: minimal Vulkan-like API (shader dispatch, buffers)** | 6 weeks | GPU driver, mailbox | 🔥 GPU acceleration |
| **OpenGL ES 3.1 subset via V3D** | 4 weeks | V3D compute | 🔥 3D apps |
| **Hardware video decode: HEVC via RP1 V4L2-lite** | 4 weeks | RP1 media, V4L2 | 🎬 Media playback |
| **Camera stack: CSI/MIPI via RP1 + V4L2-lite** | 3 weeks | RP1 media | 📷 Camera apps |

---

## Phase 3: Power & Polish (Weeks 27-36)

| Item | Effort | Dependencies | Impact |
|------|--------|--------------|--------|
| **CPU frequency scaling (mailbox + governors)** | 2 weeks | Mailbox, timer | 🔋 Battery life |
| **Suspend/resume (limited: RAM retention + wake GPIO)** | 2 weeks | Power domains | 🔋 Laptop use |
| **OpenGL compositor (replace CPU blitting)** | 2 weeks | V3D, OpenGL ES | 🖥️ Smooth UI |
| **UEFI bootloader (PI4/PI5 UEFI port)** | 3 weeks | Bootloader | 🔧 Standard boot |

---

## Phase 4: Apps Ecosystem (Ongoing)

| App | Effort | Priority |
|-----|--------|----------|
| Terminal (done) | — | ✅ |
| File Manager (done) | — | ✅ |
| Text Editor (done) | — | ✅ |
| Settings (done) | — | ✅ |
| **Web Browser (minimal: WebKitGTK port or NetSurf)** | 8 weeks | 🔥 |
| **Media Player (GStreamer-lite + HW decode)** | 4 weeks | 🎬 |
| **PDF Viewer** | 2 weeks | 📄 |
| **Image Viewer** | 1 week | 🖼️ |
| **Office: LibreOffice port (headless) / OnlyOffice** | 12 weeks | 📝 |

---

## Technical Reality Checks

### ❌ NOT FEASIBLE Bare-Metal (Require Linux Kernel)
- **Full Vulkan/OpenGL/EGL** — Requires Mesa + DRM/KMS + Linux memory management
- **Full V4L2** — Requires Linux media framework, DMA-BUF, dmabuf heaps
- **Full ALSA/PulseAudio/PipeWire** — Requires Linux kernel audio subsystem
- **Full BlueZ** — Requires Linux kernel BT subsystem, Netlink, udev
- **systemd** — Requires Linux cgroups, namespaces, kdbus

### ✅ FEASIBLE Bare-Metal (Custom Implementation)
- **Minimal Vulkan-like compute API** — Direct V3D register programming
- **V4L2-lite** — Subset for HEVC decode / camera capture
- **ALSA-lite** — PCM playback/capture via I2S
- **BT HCI + L2CAP + GATT** — Userspace stack over SDIO
- **Service supervision** — Custom init with dependency graph
- **syslog daemon** — Standard RFC 5424 over UDP/Unix socket

---

## Immediate Next Steps (This Session)

1. ~~**Embed WiFi firmware** — Extract from Raspberry Pi OS / LibreELEC~~ ✅ **DONE** (brcmfmac43455-sdio.bin + .txt embedded via include_bytes!)
2. ~~**RP1 peripheral drivers** — Implement GPIO, I2C, SPI, UART, PWM, I2S at 0x1F_0000_0000~~ ✅ **DONE** (src/drivers/rp1.rs)
3. ~~**GPIO userspace API** — /dev/gpiochip character device~~ ✅ **DONE** (src/drivers/gpiochip.rs, registered via devfs)
4. **NVMe validation fixes** — Test on real hardware, fix bugs
5. **Boot config generator** — Tool to generate config.txt for Pi 5

---

## Resource Links

| Component | Source |
|-----------|--------|
| RP1 Datasheet | https://pip.raspberrypi.com/documents/RP-008370-DS-1-rp1-peripherals.pdf |
| WiFi Firmware (LibreELEC) | https://github.com/LibreELEC/brcmfmac_sdio-firmware |
| WiFi Firmware (OpenWrt) | https://github.com/openwrt/openwrt/tree/master/package/firmware/cypress-firmware |
| Pi 5 UEFI | https://github.com/pftf/RPi4 |
| V3D Register Docs | Broadcom Videocore VII (partial, via Linux kernel v3d driver) |
| BCM2712 Peripherals | Linux kernel arch/arm64/boot/dts/broadcom/bcm2712*.dtsi |

---

## Success Metrics for "Daily Driver"

- [ ] Boots to graphical desktop in <10s
- [ ] WiFi connects to WPA2/3 networks reliably
- [ ] Bluetooth keyboard/mouse/audio works
- [ ] Audio plays via HDMI/3.5mm jack
- [ ] Can install apps from local repo (`pkg install firefox`)
- [ ] NVMe SSD works as root filesystem
- [ ] Suspend/resume works (lid close/open)
- [ ] Browser renders modern websites
- [ ] Video plays 1080p HEVC smoothly
- [ ] GPIO controllable from userspace apps
- [ ] Logs persist across reboots with rotation
- [ ] Services auto-start on boot

---

*Generated: 2025-06-12 | LobsterOS v0.1.0 | Updated: Phase 0 complete, I2S driver complete*