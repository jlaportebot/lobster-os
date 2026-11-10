# LobsterOS + Pi 5 Emulator — Master Roadmap

**This is the single source of truth.** It supersedes `docs/DESKTOP_PLAN.md` and absorbs the
open items of `docs/AUDIT_2026-10-25.md` (the audit stays as the evidence file; its `Qn`
numbers are cross-referenced here).

**Goal.** A Rust desktop operating system built specifically for the Raspberry Pi 5, usable
like any mainstream desktop OS (files, browser, settings, apps, multitasking), plus a
VirtualBox-style emulator of the Pi 5 that it runs in.

**Repos**

| Repo | Path on the dev Pi | Remote |
|---|---|---|
| OS kernel + desktop | `/mnt/data/sd-overflow/LobsterOS/lobster-os` | `jlaportebot/lobster-os` (master) |
| Emulator front end (`lobstervm`, harness, images) | `/mnt/data/sd-overflow/pi5-emulator-repo` | `jlaportebot/pi5-emulator` (master) |
| QEMU fork (raspi5b machine + lobster devices) | `/mnt/data/sd-overflow/qemu-src` | `jlaportebot/lobster-qemu` (lobster/main) |

**Legend:** `[x]` done and verified live · `[~]` partly done · `[ ]` not started.
Every item names how it is verified. Nothing is ticked without that check passing.

**Dev loop (short form).** Build in the kernel repo with
`cargo build --release --target aarch64-unknown-none --features qemu_raspi5b`, run
`tools/install-kernel.sh`, then drive a VM with `lobstervm` (emulator repo):
`./lobstervm.sh start dev --headless --wait`, `shell`, `click`, `screenshot`.
Gate every push on `tools/gui_test.py` in the emulator repo.

---

## 0. Where things stand (verified 2026-11-09)

Baseline measured this session: kernel builds clean, boots to desktop in ~1.2–2.0 s guest
time under TCG, GUI harness 10/10.

What exists: EL0 processes with GUI syscalls, a window manager with panel/start
menu/notifications/alt-tab, 12 built-in apps, ext2 persistence, package install over the
network, MicroPython, C compile via the host, multi-user records, 133 host unit tests, CI.

What is wrong, in order of how much it hurts a desktop user (all seen live this session):

1. **The desktop boots into a cluttered demo.** Five app windows open at boot, kernel log
   lines scroll into the Terminal window as a staircase (no CR handling), desktop icons sit
   underneath the Terminal, the clock reads `00:00`.
2. **Window stacking is fake.** Built-in apps paint straight into the framebuffer in a fixed
   order (`wm::tick`), not in z-order, and there are no per-window surfaces
   (`compositor::composite` is a no-op on QEMU). Overlapping windows bleed through each
   other, and one app's redraw forces everything after it to redraw.
3. **Most windows cannot be used with the mouse.** Settings has no input at all, Files and
   the Calculator ignore clicks, tray popups ignore clicks, the App Store is an empty window.
4. **The browser panics the kernel** on `browser http://example.com`
   (`apps/browser.rs:249`, line buffer never reset; entity decoding is also mangled).
5. **Text and icons look like a 1990 BIOS.** One 8×16 bitmap font, icons are coloured blobs,
   rounded corners leave black artefacts.
6. **No power control or wall clock.** `shutdown` spins forever; no RTC.
7. **A non-sleeping app freezes the whole desktop** (no real preemption, audit Q5), and apps
   still share kernel-visible mappings in places (audit Q4 steps 3+).
8. **Login accepts anyone** (audit Q14).
9. **Emulator front end is a pile of scripts** with hard-coded paths, a stale Tk launcher
   that passes flags the machine rejects, and a harness that writes to the tracked base disk.

---

## Part E — The emulator ("VirtualBox for the Pi 5")

### E1. Machine model (QEMU fork)
- [x] **E1.1 `lobster-sys` device** at `0x7d50_0000`: guest power-off/reboot, host UTC clock +
      timezone offset, guest state register, preferred-resolution hint (`LBST_RES`).
      *Verified:* `xp /4wx 0x7d500010` returns the host epoch.
- [x] **E1.2 RAM variants**: `raspi5b-2g`, `raspi5b` (4 GB), `raspi5b-8g`, `raspi5b-16g`; board
      revision memory fields corrected. *Verified:* `-M help` lists all four with the right RAM.
- [x] **E1.3 Quiet device tree fix-ups** (audit Q20): removal list de-duplicated, one optional
      summary line instead of ~35 warnings.
- [ ] **E1.4 GIC "duplicate property" errors** at realize (double init in `bcm2712.c`).
- [ ] **E1.5 Device config as properties**, not environment variables (`LBST_DISKIMG`,
      `LBST_SHARE`, `LBST_RES` → `-global`/machine options); keep env as fallback.
- [ ] **E1.6 Host clipboard bridge** in `lobster-sys` (text both ways via QEMU's clipboard API).
- [ ] **E1.7 Host-driven resize**: resizing the VM window sends the guest a new mode.
- [ ] **E1.8 Real Pi 5 peripherals** so a stock Pi 5 kernel can use them instead of the
      paravirtual lobster devices: RP1 GPIO/UART/I2C/SPI behaviour, SDHCI on RP1, xHCI USB
      (keyboard/mouse/storage), GENET/Cadence Ethernet with a user-mode network backend,
      HDMI/pixelvalve timing, RNG, thermal sensor, PWM fan, RTC, watchdog/PM reset.
- [ ] **E1.9 Raspberry Pi OS boots** to a serial login (today: no console output). Needs
      E1.4 + timer/IRQ path review + E1.8 UART/SD at minimum.
- [ ] **E1.10 SMP**: bring cores 1–3 up via the spin-table/PSCI path the guest expects.
- [ ] **E1.11 Debug view** (audit Q22): `info ltasks`, `lbt <pid>` monitor commands.
- [ ] **E1.12 Deterministic mode** (audit Q21): `-icount` profile for the harness.
- [ ] **E1.13 Performance**: profile TCG hot paths for the desktop workload; evaluate KVM on
      the Pi 5 host (same CPU: Cortex-A76) behind a `--accel kvm` option.

### E2. `lobstervm` — VM manager (emulator repo)
- [x] **E2.1 Core library**: VM definitions on disk (`~/.local/share/lobstervm/<name>/vm.json`),
      per-VM qcow2 disk layered on the pristine base image, detached QEMU with a runtime
      directory (monitor socket, serial FIFO, logs, pid). No hard-coded paths.
- [x] **E2.2 CLI** (`lobstervm.sh`): `list create modify show delete start stop reset pause
      resume factory-reset snapshot screenshot click move drag key type shell hmp serial doctor`.
      *Verified:* create → `start --headless --wait` → `shell uptime` → `screenshot`.
- [ ] **E2.3 Graphical manager** (`lobstervm gui`): VM list, details pane, New/Settings
      dialogs, Start/Stop/Pause/Reset, snapshot tree, live serial console, screenshot button.
- [ ] **E2.4 Clean guest shutdown** from the manager (`stop` → guest `shutdown` → QEMU exits)
      and guest-initiated reboot. Needs O1.1.
- [ ] **E2.5 Harness on `lobstervm`**: port `tools/gui_test.py` to `Machine`, throwaway disk
      per run (never touch the tracked base image), unique runtime paths, JUnit-style summary.
- [ ] **E2.6 Desktop integration**: one `.desktop` launcher, icon, `install.sh` that puts
      `lobstervm` on `PATH`; delete the stale `run_*.sh`, `lobsteros_gui.py`, `vm.sh`, tracked
      `__pycache__`, and the stale `hw/` + `include/` copies of QEMU sources.
- [ ] **E2.7 Disk tools**: `lobstervm disk import/export/resize`, build the base image from a
      manifest (`tools/mkbase.sh`) instead of tracking a 64 MB binary.
- [ ] **E2.8 Unit tests** for `lobstervm` (config validation, command line, HMP parsing) in CI.
- [ ] **E2.9 Docs**: README rewritten around `lobstervm`; STATUS.md regenerated from facts.

---

## Part O — The operating system

### O1. Platform services
- [ ] **O1.1 Power**: `shutdown`/`reboot` through `lobster-sys` on QEMU, PSCI/PM watchdog on
      hardware; filesystems synced first. *Verify:* `shutdown` makes QEMU exit 0; `reboot`
      prints a second boot banner.
- [ ] **O1.2 Wall clock**: read host RTC at boot, keep time with the arch timer, timezone,
      `date` command, panel clock shows real local time and date.
- [ ] **O1.3 Resolution from the VM manager**: honour the `lobster-sys` hint at boot.
- [ ] **O1.4 Memory sizing**: frame allocator and heap sized from the device tree for
      2/4/8/16 GB. *Verify:* `mem` total matches on each machine variant.
- [ ] **O1.5 Entropy + crypto hygiene**: seeded CSPRNG for ASLR, salts, TLS later.

### O2. Graphics stack (the keystone for "looks and behaves like a desktop")
- [ ] **O2.1 Surface compositor**: every window owns an off-screen surface; apps draw in
      window-local coordinates; the compositor blends wallpaper → windows in z-order → shell
      overlays into a back buffer and copies only damaged rectangles to the framebuffer.
      Removes the fixed draw order, the redraw cascades and the bleed-through.
      *Verify:* overlap two windows, raise each in turn, pixel-check the overlap region.
- [ ] **O2.2 Hit regions**: apps register clickable rectangles while drawing; one router
      stage dispatches presses to the topmost window's region. Replaces per-app router stages.
- [ ] **O2.3 Font engine** (audit Q24): anti-aliased proportional UI font and a monospace
      font, several sizes, generated at build time into alpha atlases; `text::draw/width`.
- [ ] **O2.4 Icon set**: real 48/24 px icons for apps, files, folders, tray, generated at
      build time; used by desktop, start menu, panel, Files.
- [ ] **O2.5 Effects**: soft shadows, clean rounded corners (no black artefacts), open/close/
      minimise animations, snap preview (audit Q17).
- [ ] **O2.6 Image decoders**: PNG (inflate) and JPEG baseline, for wallpapers, the image
      viewer and the browser.
- [ ] **O2.7 Wallpaper images**: picture wallpapers from `/sd`, fit/fill/centre.
- [ ] **O2.8 Cursor set**: arrow, I-beam, hand, resize (4), busy, with correct hotspots
      (audit Q9b).
- [ ] **O2.9 HiDPI scale factor** (100/125/150/200 %).
- [ ] **O2.10 V3D GPU acceleration** on hardware (long-term).

### O3. Desktop shell
- [ ] **O3.1 Clean boot**: no windows at boot; kernel log stays on serial/`dmesg`, not in the
      Terminal; icon grid on the desktop; first-run welcome only once.
- [ ] **O3.2 Taskbar**: start button, pinned + running apps with titles, click to
      focus/minimise, tray (network, volume, clock with date), show-desktop corner.
- [ ] **O3.3 Start menu**: real icons, search box, All apps incl. installed packages, user
      row, power row (lock, log out, restart, shut down).
- [ ] **O3.4 Desktop icons**: grid layout, drag to arrange (persisted), rubber-band select,
      double-click to open, files in `~/Desktop` appear as icons, right-click menu
      (new folder/file, change wallpaper, display settings).
- [ ] **O3.5 Window management polish**: resize from all edges with cursor feedback, snap
      preview, keyboard shortcuts (Super, Alt+Tab, Alt+F4, Super+arrows, Super+E/T/L),
      window list previews.
- [ ] **O3.6 Dialogs** (audit Q9a): alert/confirm/prompt, file open/save picker.
- [ ] **O3.7 Notifications + quick settings** (volume, brightness, network, theme toggle).
- [ ] **O3.8 Lock screen + login with passwords** (audit Q14): salted hash, lock on demand,
      logout tears the session down.
- [ ] **O3.9 Clipboard**: system-wide text clipboard, Ctrl+C/V/X in every text field, host
      bridge via E1.6.
- [ ] **O3.10 Drag and drop** between Files, desktop and apps.
- [ ] **O3.11 Widgets toolkit**: buttons, checkboxes, sliders, text inputs, lists, tabs,
      scrollbars, menus — one shared implementation used by every built-in app.
- [ ] **O3.12 Accessibility basics**: keyboard navigation everywhere, high-contrast theme,
      large-text mode.

### O4. Applications
- [ ] **O4.1 Settings**: fully clickable; pages Appearance (theme, accent, wallpaper),
      Display (resolution, scale), Sound, Network, Users (add/remove/password), Date & time,
      Storage, Apps, About; every change applies live and persists.
- [ ] **O4.2 Files**: mouse selection, double-click open-with by type, back/forward/up,
      breadcrumb, new/rename/delete/copy/cut/paste, drag-move, properties, trash, list/icon
      views, sidebar places, shared folder.
- [ ] **O4.3 Browser v2**: fix the panic; HTML tokenizer + block/inline layout (headings,
      paragraphs, lists, links, emphasis, pre, tables-lite), proportional text, address bar,
      back/forward/reload, clickable links, scrolling, history/bookmarks, images (O2.6),
      basic CSS colours; HTTPS through the host proxy on QEMU; native TLS on hardware later.
- [ ] **O4.4 Terminal**: own shell per tab, correct CR/LF, scrollback, copy/paste, resize
      reflow (audit Q15).
- [ ] **O4.5 Text editor / Code**: undo/redo, selection, click-to-place, syntax highlight,
      run button (compile + run with output pane) (audit Q8, Q16).
- [ ] **O4.6 App Store**: real catalogue from the package index, install/remove/update with
      progress, installed apps appear in the start menu (audit Q11).
- [ ] **O4.7 Calculator**: mouse input, decimals, history, shared window frame (audit Q17).
- [ ] **O4.8 Task Manager**: per-process CPU/memory, end task, performance graphs.
- [ ] **O4.9 Image viewer** (PNG/JPEG/BMP, zoom, next/previous), **Notes**, **Clock**
      (alarm/timer/stopwatch), **Screenshot tool**, **Paint**, **Minesweeper/Solitaire**,
      **Media player** (WAV first), **PDF/Markdown viewer**, **Archive tool**.
- [ ] **O4.10 Help / welcome app** documenting the OS from inside it.

### O5. Kernel
- [~] **O5.1 Per-process address spaces** (audit Q4): pointer checks + page-table teardown
      done; remaining: stop dual-mapping user pages into the kernel table, drop the global
      window mirror, validate every syscall pointer, `two_apps`/`leak`/`efault` tests.
- [ ] **O5.2 Real preemption** (audit Q5): timer IRQ ends an EL0 slice; foreground `run`
      becomes a sliced task. *Verify:* `while(1);` app does not freeze the desktop.
- [ ] **O5.3 Signals + Ctrl+C + kill** end to end (audit Q6).
- [ ] **O5.4 SMP**: bring up cores 1–3, per-core run queues, locked task table.
- [ ] **O5.5 Syscall surface for real programs**: `mmap`/`brk`, `fork`/`exec`/`wait`, pipes,
      `poll`, time, sockets — enough for a libc (newlib or relibc) and then ports.
- [ ] **O5.6 Dynamic linking / PIE** (audit Q27, parked until a libc.so exists).
- [ ] **O5.7 Robustness**: OOM handling, kernel stack guard pages, panic screen with
      reboot, watchdog; remove crate-wide lint allows one at a time (audit Q26).
- [ ] **O5.8 Function-size and TODO debt** from the audit (kernel_main 769 lines, shell
      dispatch 731, …) — split when touched.

### O6. Storage and filesystems
- [ ] **O6.1 ext2 hardening**: fsck-clean after every harness run (`e2fsck -fn` on the host
      as a test), indirect blocks, large files, free-space accounting, timestamps from O1.2.
- [ ] **O6.2 FAT32** read/write (boot partition, USB sticks).
- [ ] **O6.3 Page cache + write-back**, `sync` on shutdown.
- [ ] **O6.4 Larger disks**: resize the system image, partition table support.
- [ ] **O6.5 Trash, file metadata, permissions enforced by uid/gid.**

### O7. Networking
- [ ] **O7.1 Guest TCP/IP on QEMU** via an emulated NIC (E1.8) instead of the HTTP proxy
      device; DHCP, DNS, sockets API for user programs.
- [ ] **O7.2 TLS** in the guest (rustls-style no_std stack) for HTTPS without the host proxy.
- [ ] **O7.3 Wi-Fi/Bluetooth on hardware** (CYW43455) — existing code is untested on metal.
- [ ] **O7.4 Network settings UI**, connection indicator, proxy settings.

### O8. Software ecosystem
- [ ] **O8.1 Userland SDK**: `liblobster` (C + Rust) for windows, widgets, files, network;
      documented app template; apps built outside the kernel tree.
- [ ] **O8.2 Move built-in apps out of the kernel** to EL0 processes on the SDK (after O5.1/5.2).
- [ ] **O8.3 Package manager**: dependency resolution, remove/upgrade, signed index
      (ed25519 already linked), GUI in O4.6.
- [ ] **O8.4 Python scripts + modules** (audit Q7), on-device C compiler toolchain as packages.
- [ ] **O8.5 Ports**: a libc, then busybox-class tools, Lua, SQLite, a terminal multiplexer.

### O9. Real hardware (needs a second Pi 5 + serial cable)
- [ ] **O9.1 Boot on metal**: SD image builder, config.txt, UART console, framebuffer.
- [ ] **O9.2 RP1 bring-up** (PCIe → RP1 → USB xHCI, Ethernet, GPIO); recover `rp1.rs` from
      the archived branch (audit Q27).
- [ ] **O9.3 USB HID + mass storage, SD/eMMC, NVMe, HDMI audio, fan/thermal, RTC, power button.**
- [ ] **O9.4 Pi 5-specific optimisation**: Cortex-A76 tuning (`-C target-cpu=cortex-a76`),
      NEON blitters for the compositor, DVFS, 16 K pages evaluation, V3D (O2.10).

### O10. Quality, performance, release
- [ ] **O10.1 Harness coverage**: one end-to-end test per shipped feature (audit Q2 list).
- [ ] **O10.2 Perf budgets** (audit Q18): boot ≤ 2 s guest, idle ≥ 30 fps, input latency,
      `perf` command, CSV tracked per release.
- [ ] **O10.3 CI**: build the QEMU-feature kernel, boot it headless, run a smoke subset.
- [ ] **O10.4 Nightly build + release notes** (audit Q19), versioned releases with images.
- [ ] **O10.5 Docs**: user guide, developer guide, architecture overview kept in sync.

---

## Execution order

Work top to bottom; each step is committed and pushed when its check passes.

1. **Foundations** — E1.1–E1.3, E2.1–E2.2 (done), O1.1, O1.2, O1.3, E2.4, O4.3 panic fix.
2. **Desktop core** — O3.1, O2.1, O2.2, O2.3, O2.4, O3.2, O3.3.
3. **Apps people expect** — O4.1, O4.2, O4.3, O4.7, O4.6, O3.4, O3.6, O3.8.
4. **Emulator product** — E2.3, E2.5, E2.6, E2.8, E2.9.
5. **Kernel keystones** — O5.2, O5.3, O5.1 remainder.
6. **Breadth** — the rest of O2–O8, E1.4–E1.13.
7. **Hardware** — O9 once a second board is available.

## Progress log

| Date | Change | Check |
|---|---|---|
| 2026-11-09 | Roadmap written; baseline harness 10/10 | `tools/gui_test.py` |
| 2026-11-09 | E1.1, E1.2, E1.3 (QEMU), E2.1, E2.2 (`lobstervm`) | boot + `shell uptime` + `xp` RTC |
