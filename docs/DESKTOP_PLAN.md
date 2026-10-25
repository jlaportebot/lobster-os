# LobsterOS Desktop — Master Plan v2 (everyday-driver OS)
**North star:** a Rust, Pi 5-native desktop OS you can *daily drive*: boot to desktop,
browse, download/install apps (incl. interpreters like Python), write and run code,
manage files, multi-task real user processes — Windows/macOS-class UX, Pi5-optimized,
fast. Every item ships with a live end-to-end verification before it's marked done.

Legend: [x] done + verified live · [~] in progress · [ ] planned

Kernel today: ~84K lines (83.7K counted 2026-10-25), EL0 processes + GUI syscalls work; sliced
multi-app tasks run via SYS_SLEEP wake-deadlines; hardware 10ms ticks are armed ON by default,
but on QEMU nothing ends a slice on a tick (exception.rs IRQ path is cfg'd out) → Q5 in
docs/AUDIT_2026-10-25.md. All apps share the kernel page table + one fixed stack address on
QEMU (no per-process address spaces yet → Q4). Emulator
supports dirty-rect, snapshots, shared folder, audio, live resolution.
Deep audit 2026-10-25: docs/AUDIT_2026-10-25.md is the authoritative queue (Q0–Q27).

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
- [~] **S3. True preemption** — ticks ON by default (993fefd; PPI 29, 10ms, safe frames),
      BUT on QEMU no IRQ path ever ends a slice — only SYS_SLEEP yields (→ Q5;
      docs/AUDIT_2026-10-25.md finding 1). A non-sleeping EL0 app still freezes the desktop.
- [~] **S4. Multiple concurrent EL0 apps** — works for apps that sleep: two EL0 GUI apps
      run concurrently with per-slot event rings and round-robin slices. CAVEAT (audit):
      on QEMU all apps share the kernel page table + the same fixed stack address
      (USER_STACK_TOP) — different binaries running together can corrupt each other (→ Q4).
      Focus routing is NOT topmost-first (hit_test is index-order; → Q1g).
- [x] **S5. Zero-copy shared framebuffer** — DONE cee0b41: SYS_WIN_MAP(115) maps a
      window's backing frames into the app's address space (0x4000_0000 + slot*16MB);
      apps write pixels directly, SYS_WIN_FLUSH composites; kernel reads via phys map.
      Live-verified: direct-write animation, clean close, suite 8/8. (No harness test
      exercises SYS_WIN_MAP — el0_gui_demo doesn't use it; → Q2 regression tests.)
- [~] **S6. App lifecycle** — close kills the EL0 owner + tears down (b1a84df), but
      freed window/stack frames are NOT unmapped (still EL0-writable via the global
      window mirror; page tables + ELF image leak) → Q4/H5.

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
- [~] **A3.1** only the Open animation is wired; close/minimize animations + per-app
      chrome + snap transitions TODO (5 apps still draw legacy title bars → Q17)
- [~] **A3.2** dbl-click maximize + edge-snap done; snap preview overlay TODO
- [ ] **A3.4** dirty-state dot + unsaved-close confirmation (needs B4.1)
- [ ] **A3.5** Per-app mute in volume tray (ties C4)
### A4. Theme engine
- [x] theme struct, A4.1 light/dark, A4.2 accent (persisted), A4.4 /sd/etc/theme.conf
- [~] **A4.2b** radius control TODO
- [~] **A4.3** cursor bitmaps — glyphs + shape enum + draw done (8bbdbd6, QEMU only);
      gaps: no resize glyph, hotspots ignored (I-beam drawn 7px off), busy/hand never
      selected, shape switching is arrow↔I-beam only → Q9b
- [ ] **A4.5** Font engine v2: scalable PSF/TTF glyphs (beyond 8x16 bitmap)

## B. Applications
### B1. Terminal
- [x] VT100 parser + screen buffer; B1.2 shell-in-a-window (gkey-verified)
- [x] **B1.1** Tabbed terminal + font-size keys — DONE (merge 05b1170): 4-tab bar
      with parked VT screens swapped on switch; Ctrl+/- scales text 1x–2x.
      (Mouse copy/paste deferred.)
### B2. Files
- [x] tree browse + /sd; B2.1 shell UI; B2.3 create/copy/move/delete (persist verified)
- [~] **B2.2** selection/rename/delete done; drag-drop + properties dialog TODO
### B3. Settings
- [x] B3.1 pages (Appearance/Display/Network/Storage/About)
- [ ] **B3.2** live-apply + persist every setting to /sd/etc/; Users page
### B4. Editor
- [x] **B4.1** multi-buffer tabs, save-as, find, unsaved indicator — DONE (f560772 /
      merge e46ea98; checkbox was stale). Gaps: no close-tab, no undo in QEMU editor,
      no click-to-place cursor → Q16.
- [ ] **B4.2** syntax highlight (highlight_line exists in non-QEMU editor, never called → Q8)
### B5. Browser
- [x] B5.1 text-mode browser; B5.2 persisted history/bookmarks
- [ ] **B5.3** HTML-lite images + clickable links
### B6. Media & extras
- [x] B6.1 BMP image viewer; B6.3 Task Manager (SysMon+ProcMgr merged)
- [x] **B6.2** Calculator keyboard + history — DONE 1fbf3c0: digits/ops/Enter/BS/Esc
      when focused. CAVEATS (audit): `5 % 0` panics the kernel (unguarded rem → Q1a);
      history tape is painted over by the first button row; no `=` key; mouse clicks
      dead (handle_click has no caller); legacy title bar (→ Q1a, Q17).
- [x] **B6.4** App Store real install — DONE ceb4497: `pkg install hello` downloads from
      the GitHub package repo (release `pkg-repo`), sha256-verifies twice, extracts via
      vfs::write_file, records the DB; `run /apps/hello &` launches the DOWNLOADED app.
      App-Store-GUI wiring to this flow is the remaining polish (GUI today = empty mock
      window; app_store.rs render never called → Q11).
- [~] **B6.5** clock/timer + Minesweeper (NOT STARTED per audit — no code found → Q23)
- [ ] **B7. Code app**: editor panel wired to `cc` + `run` (write→compile→execute flow → Q8)
- [x] **B8. Package runtime** — DONE 550ea52: embedded MicroPython runs as an EL0
      user process. First-class `python`/`python3` shell command (REPL verified live:
      comprehensions, arithmetic, print output). Background `python &` works via S2.
      Script-args pending argv wiring (C8). PARTIAL per audit: script loading is a stub
      (ENOENT), imports always "missing", nlr_jump_fail spins forever; shipped binary
      is out of sync with source → Q7.

## C. System Services (kernel-level)
- [x] **C1** → S0 complete (S1/S2/S4/S5/S6 all done — see above)
- [x] **C2. Compositor event loop** — DONE (merge 1d5fa29): formal 10-stage
      event-router table (event_router.rs), drag threshold, 'wmconfig ffz'
      focus-follows-z option (default off).
- [x] **C3. Storage UX** — DONE (merge 5f8d641): /home/user overlays /sd/home/user
      (alias in VFS resolve/create/open/mkdir/rename — no copying); boot prompt starts
      in home; 'home' command + 'cd ~'; welcome.txt seeded. Live-verified 5/5.
- [x] **C4/D5. Audio device** — lobster-audio MMIO + host audio + guest mixer (beep/play)
- [x] **C5. Multi-user login** — DONE (merge a2c090e): greeter card when >1 user,
      click-to-login, per-user home aliasing, useradd/userlist with /sd/etc/passwd
      persistence. Single-user boot unchanged. Live-verified end-to-end.
      CAVEAT (audit): no authentication — greeter never calls verify_password, root's
      default password is empty → Q14.
- [ ] **C6. Dynamic linker v2**: PARKED (audit: all binaries static, no .so files,
      PLT stub all zeros, PIE rejected — low value until a libc.so exists → Q27)
- [~] **C7. Signals for EL0** — frame groundwork DONE (993fefd): TrapFrame.elr/spsr
      are now real frame slots (frame-backed SAVE/RESTORE); signal delivery writes
      land correctly. Ctrl+C/SIGINT wiring to the terminal + fg apps remains.
      BUGS (audit): sigreturn trusts user SPSR (EL0→EL1 escalation) and Rust TrapFrame
      is 280B vs 272B asm frame (8-byte overflow) → Q1c/Q1d. GUI terminal maps Ctrl+'z'
      scancode as Ctrl+C → Q6.
- [x] **C8. argv for EL0 apps** — DONE bd45d85: `run <path> [args...] [&]` and
      `python <script.py>` pass argc/argv at EL0 entry (argv[0]=program path, block
      below SP so program pushes can't corrupt it). Verified with args-demo ELF in
      fg+bg. fork/exec-style launcher spawn remains a later refinement.
      (Correction: argv block sits BELOW the stack pointer, not above.)
- [ ] **C9. USB stack on real Pi5** (PARKED — blocked on hardware; rp1.rs only on
      unmerged feat/wifi-shell-enhancements branch, tagged archive/rp1-2681b2d → Q27)
- [ ] **D6. dt warnings cleanup** (bcm2712 disabled-node spam, 51-node removal list
      with 5 duplicates, warning per node → Q20)

## D. Emulator (lobster-qemu)
- [x] 4 cores, fb, lobster-input, lobster-net, bulk serial, lobster-disk
- [x] D1 dirty-rect, D2 snapshots, D3 shared folder, D4 live resolution, D5 audio
- [ ] **D7. icount deterministic mode** for CI-stable GUI tests (→ Q21; needs E2 markers)
- [ ] **D8. Multi-EL0 debug view**: HMP task list + per-task backtrace (→ Q22; pull
      forward if Q4/Q5 debugging stalls)

## E. Testing & Release
- [x] E1 gui harness with HMP lmouse/lclick/lkey injection (8 tests on one shared
      boot, not 5; several "done" features untested: S5/S6/C3/C8/B1.1/B4.1/B6.2 → Q2)
- [ ] **E2.** boot-to-desktop ≤ 5s (measure first, then optimize)
- [ ] **E3.** nightly build + auto r-notes per merge
- [ ] **E4.** perf budget suite: fps counter, syscall latency, boot time (logged)

## Ship order (dependencies):
S1 → S2 → S4 (multitask core) → B6.4/B8 (install & run apps — headline ask)
→ S3 (true preemption) → C3/C5 (home + users) → B1/B4 app polish → E2/E4 perf → E3 nightly.

## Next (audit queue, 2026-10-25 — see docs/AUDIT_2026-10-25.md):
Q0 truth pass → Q1 hotfixes (calc %0 panic, gkey scancodes, TrapFrame/sigreturn,
kill, z-order, ticks) → Q2 boot markers + idle WFI + regression tests →
Q3 FP/SIMD context (kernel has 204 q/v-reg instructions, never saved!) →
Q4 per-process address spaces (keystone) → Q5 real preemption → Q6 Ctrl+C →
Q7 python scripts → Q8 Code app (flagship) — then Q9–Q25 per audit.

## Perf principles (always active):
- keep hot paths barrier-light (dirty gating, 30fps frame cap, opt-level=z + LTO)
- zero-copy where possible (S5), batched MMIO, no per-pixel syscalls in steady state
- boot: defer non-critical init until after first desktop frame
