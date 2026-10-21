# LobsterOS Desktop — Master Plan v2 (everyday-driver OS)
**North star:** a Rust, Pi 5-native desktop OS you can *daily drive*: boot to desktop,
browse, download/install apps (incl. interpreters like Python), write and run code,
manage files, multi-task real user processes — Windows/macOS-class UX, Pi5-optimized,
fast. Every item ships with a live end-to-end verification before it's marked done.

Legend: [x] done + verified live · [~] in progress · [ ] planned

Kernel today: 78K lines, EL0 processes + GUI syscalls work; ONE cooperative EL0 app at
a time; input during app run is blocked by the nested-tick guard (→ S1). Emulator
supports dirty-rect, snapshots, shared folder, audio, live resolution.

## S0. C1 v2 — real multitasking (THE keystone for "download & run apps")
Everything else (App Store installs, multiple apps, shell while an app runs) unlocks here.
- [x] **S1. Input during EL0 app run** — DONE 0dcbe42: root cause was the QEMU
      lobster-input ring — the guest's head-advance write hit the CLEAR handler and
      wiped all unconsumed events (move+press+release bursts lost). New ADVANCE_HEAD
      register (0x1C) QEMU-side; guest driver writes 0x1C. Plus deferred-event replay
      ring, per-slot epochs, chrome-vs-content hit routing, pending-click lifecycle
      fixes (Claude-reviewed). Harness el0_app click now passes 3/3 + full suite 5/5.
- [x] **S2. Non-blocking app launch** — DONE 8bc0c86: `run <path> &` spawns a sliced
      background EL0 task (wake-deadline slicing on SYS_SLEEP; frame saved on yield).
      Shell stays fully responsive; verified with 2 concurrent bg apps + new bg_app
      harness test (6/6 suite).
- [ ] **S3. QEMU timer IRQ**: fix the PSTATE.IL-class UNDEF at IRQ-vector entry so
      hardware ticks fire → true preemption (removes the cooperative crutch).
- [x] **S4. Multiple concurrent EL0 apps** — DONE (with S2): two EL0 GUI apps run
      concurrently in their own windows with per-slot event rings and round-robin
      slices (verified live). Focus routing + hit_test already topmost-first.
- [ ] **S5. Zero-copy shared framebuffer page**: SYS_WIN mmap'd shared page (removes
      syscall-per-fill overhead) + flush-driven damage tracking.
- [ ] **S6. App lifecycle**: kill on close button, zombie reap, crash containment
      (exception → kill that process only, desktop survives).

## A. Core Desktop Shell
### A1. Desktop surface
- [x] Full-screen framebuffer + compositor (dirty-gated)
- [x] Desktop icons (launcher + files); A1.1 wallpapers (7 presets, persisted); A1.2 right-click menu
- [~] **A1.3** single-selection + hover done; rubber-band multi-select + drag-arrange TODO
- [ ] **A1.4** Desktop widget layer (clock/weather sticky on wallpaper)
### A2. Panel (taskbar)
- [x] Panel, clock, task list, launcher button; A2.1 start menu (tiles, hover+kb nav);
      A2.2 tray popups; A2.3 notification center + toasts
- [ ] **A2.4** Window-list hover previews, pinning, workspace grouping
- [ ] **A2.5** Quick-settings tiles (WiFi/BT/volume/brightness) in tray popup
- [ ] **A2.6** Start-menu search + power row (logout/reboot/shutdown)
### A3. Window management
- [x] create/destroy/move/resize/focus/min/max/snap/workspaces
- [x] A3.3 Alt-Tab switcher overlay
- [~] **A3.1** open/close/minimize animations done; per-app chrome + snap transitions TODO
- [~] **A3.2** dbl-click maximize + edge-snap done; snap preview overlay TODO
- [ ] **A3.4** dirty-state dot + unsaved-close confirmation (needs B4.1)
- [ ] **A3.5** Per-app mute in volume tray (ties C4)
### A4. Theme engine
- [x] theme struct, A4.1 light/dark, A4.2 accent (persisted), A4.4 /sd/etc/theme.conf
- [~] **A4.2b** radius control TODO
- [ ] **A4.3** cursor bitmaps (default/resize/I-beam/busy/hand)
- [ ] **A4.5** Font engine v2: scalable PSF/TTF glyphs (beyond 8x16 bitmap)

## B. Applications
### B1. Terminal
- [x] VT100 parser + screen buffer; B1.2 shell-in-a-window (gkey-verified)
- [ ] **B1.1** Tabbed terminal, font-size keys, mouse copy/paste
### B2. Files
- [x] tree browse + /sd; B2.1 shell UI; B2.3 create/copy/move/delete (persist verified)
- [~] **B2.2** selection/rename/delete done; drag-drop + properties dialog TODO
### B3. Settings
- [x] B3.1 pages (Appearance/Display/Network/Storage/About)
- [ ] **B3.2** live-apply + persist every setting to /sd/etc/; Users page
### B4. Editor
- [ ] **B4.1** multi-buffer tabs, save-as, find, unsaved indicator
- [ ] **B4.2** syntax highlight (keyword/comment/string tinting)
### B5. Browser
- [x] B5.1 text-mode browser; B5.2 persisted history/bookmarks
- [ ] **B5.3** HTML-lite images + clickable links
### B6. Media & extras
- [x] B6.1 BMP image viewer; B6.3 Task Manager (SysMon+ProcMgr merged)
- [ ] **B6.2** calculator keyboard + history
- [x] **B6.4** App Store real install — DONE ceb4497: `pkg install hello` downloads from
      the GitHub package repo (release `pkg-repo`), sha256-verifies twice, extracts via
      vfs::write_file, records the DB; `run /apps/hello &` launches the DOWNLOADED app.
      App-Store-GUI wiring to this flow is the remaining polish.
- [~] **B6.5** clock/timer + Minesweeper (notes done)
- [ ] **B7. Code app**: editor panel wired to `cc` + `run` (write→compile→execute flow)
- [x] **B8. Package runtime** — DONE 550ea52: embedded MicroPython runs as an EL0
      user process. First-class `python`/`python3` shell command (REPL verified live:
      comprehensions, arithmetic, print output). Background `python &` works via S2.
      Script-args pending argv wiring (C8).

## C. System Services (kernel-level)
- [~] **C1** → S0 above
- [ ] **C2. Compositor event loop**: formal input routing table, click-through,
      drag threshold, focus-follows-z
- [ ] **C3. Storage UX**: /sd → /home/user overlay, /sd/etc as persistent /etc
- [x] **C4/D5. Audio device** — lobster-audio MMIO + host audio + guest mixer (beep/play)
- [ ] **C5. Multi-user login**: greeter, sessions, per-user /home
- [ ] **C6. Dynamic linker v2**: shared-lib resolution for EL0 apps
- [ ] **C7. Signals for EL0**: SIGINT/SIGTERM delivery to user processes
- [ ] **C8. exec/fork for GUI apps**: launcher uses fork+exec (ties S2)
- [ ] **C9. USB stack on real Pi5** (xHCI exists; needs on-metal bring-up)

## D. Emulator (lobster-qemu)
- [x] 4 cores, fb, lobster-input, lobster-net, bulk serial, lobster-disk
- [x] D1 dirty-rect, D2 snapshots, D3 shared folder, D4 live resolution, D5 audio
- [ ] **D6. dt warnings cleanup** (bcm2712 disabled-node spam)
- [ ] **D7. icount deterministic mode** for CI-stable GUI tests
- [ ] **D8. Multi-EL0 debug view**: HMP task list + per-task backtrace

## E. Testing & Release
- [x] E1 gui harness (5 tests) with HMP lmouse/lclick/lkey injection
- [ ] **E2.** boot-to-desktop ≤ 5s (measure first, then optimize)
- [ ] **E3.** nightly build + auto r-notes per merge
- [ ] **E4.** perf budget suite: fps counter, syscall latency, boot time (logged)

## Ship order (dependencies):
S1 → S2 → S4 (multitask core) → B6.4/B8 (install & run apps — headline ask)
→ S3 (true preemption) → C3/C5 (home + users) → B1/B4 app polish → E2/E4 perf → E3 nightly.

## Perf principles (always active):
- keep hot paths barrier-light (dirty gating, 30fps frame cap, opt-level=z + LTO)
- zero-copy where possible (S5), batched MMIO, no per-pixel syscalls in steady state
- boot: defer non-critical init until after first desktop frame
