# LobsterOS Desktop — Master Plan (v1)
**Goal:** A polished, everyday-driver desktop OS (Windows/macOS-class UX) on the LobsterOS kernel +
QEMU Pi 5 emulator. Every item ships with a live end-to-end test before it's marked done.

Legend: [x] done + verified live · [~] in progress · [ ] planned

## A. Core Desktop Shell
### A1. Desktop surface
- [x] Full-screen framebuffer with compositor (2237-line compositor)
- [x] Desktop icons (launcher + files)
- [x] **A1.1 Wallpaper system** — DONE 09c2285: 7 gradient presets, live repaint, /sd persistence + restore (incl. system-wide FB color-order fix)
- [x] **A1.2 Desktop right-click menu** — DONE 09c2285: New File/Folder (with collision-safe names), Open Terminal, File Manager, Display Settings, Change Wallpaper, Toggle Dark/Light, Refresh
- [~] **A1.3**: single-selection + hover DONE (visual-verified); rubber-band multi-select + drag-arrange TODO
### A2. Panel (taskbar)
- [x] Panel with clock + task list + launcher button
- [x] **A2.1 Start-menu style app launcher** — DONE 39afe4b: scrollable app tiles w/ colored icons, hover+keyboard nav, focus-or-create, Esc/outside-click close. (Search + power row still TODO)
- [x] **A2.2 System tray**: popups DONE (clock/notifications center, network, volume, battery)
- [x] **A2.3 Notification center**: toasts + history + clock-popup center DONE (verified live)
- [ ] **A2.4 Window list previews** (hover peek), pinning apps, grouping by workspace
### A3. Window management
- [x] create/destroy/move/resize/focus/minimize/maximize/snap/workspaces
- [~] **A3.1 Window animations**: open/close/minimize scale+fade engine done (window_frame); per-app chrome conversion + snap transitions TODO
- [~] **A3.2**: double-click maximize + drag-edge snap DONE; snap preview overlay TODO
- [x] **A3.3 Alt-Tab switcher overlay** — DONE: tiles + selection + Alt-release activation (visual-verified)
- [ ] **A3.4 Per-app "dirty" state dot + close confirmation for unsaved docs**
### A4. Theme engine
- [x] Theme struct + palette + fonts
- [x] **A4.1 Light/Dark toggle** — DONE 09c2285: theme::toggle + /sd persistence + restore (toast on switch)
- [ ] **A4.2 Accent color picker** (persisted), rounded-corner radius control
- [ ] **A4.3 Custom cursor** bitmaps (default, resize, text I-beam, busy)
- [ ] **A4.4 Theme file format on /sd** (/sd/etc/theme.conf) + live reload

## B. Applications
### B1. Terminal (v1 exists)
- [x] VT100 parser + screen buffer + window render
- [ ] **B1.1 Tabbed terminal + font size shortcuts + copy/paste via mouse**
- [x] **B1.2 Shell-in-a-window** — DONE: GUI-typed commands execute and render inside the Terminal window (visual-verified via gkey automation)
### B2. Files (file manager v1 exists)
- [x] Tree browse, /sd integration
- [x] **B2.1 File manager shell** — DONE: Places sidebar, Up/Re toolbar + path pill, zebra list w/ type markers + sizes, status bar (visual-verified)
- [~] **B2.2**: selection + rename + delete DONE (GUI keys + fm shell, persisted across reboots); drag-drop + properties dialog TODO
- [~] **B2.3**: delete + rename persist across reboots DONE (verified); copy/move + undo toast TODO
### B3. Settings (v1 exists)
- [x] **B3.1 Settings app pages** — DONE: Appearance/Display/Network/Storage/About with live data (visual-verified); Users page TODO
- [ ] **B3.2 Live-apply every setting + persist all to /sd/etc/**
### B4. Text editor (v1 exists)
- [ ] **B4.1 Multi-buffer tabs, save-as, unsaved indicator, find**
### B5. Browser (new)
- [x] **B5.1 Text-mode browser** — DONE: URL bar + back/forward + HTML-strip rendering via lobster-net, visual-verified (B5.2 history/bookmarks persist TODO)
- [x] **B5.2 History + bookmarks persisted to /sd** — DONE (verified round trip)
### B6. Media & extras
- [ ] **B6.1 Image viewer from disk (BMP/PNG)**, wallpaper setter integration
- [ ] **B6.2 Calculator (v1 exists) → keyboard input + history**
- [ ] **B6.3 Process manager + sysmon merged into one Task Manager w/ graphs**
- [ ] **B6.4 App Store (v1 exists) → real package install from repo into /sd, launcher integration**
- [~] **B6.5**: Notes app DONE (persisted to /sd, verified); clock/timer + Minesweeper TODO

## C. System Services (kernel-level)
- [ ] **C1. Per-app processes**: GUI apps as user processes (EL0) not kernel threads — spawn from panel/launcher via sched, IPC events to compositor (in-window content via shared pages)**
- [ ] **C2. Compositor event loop**: input routing table, click-through, focus follows window z-order, drag threshold**
- [ ] **C3. Storage UX: /sd → user home overlay (/home/user on disk), /sd/etc as persistent /etc**
- [ ] **C4. Audio device** (QEMU: lobster-audio MMIO window + host audio) + volume tray + per-app streams (stretch)
- [ ] **C5. Multi-user login screen** (greeter + sessions + /home per user on /sd)**

## D. Emulator support (lobster-qemu)
- [x] 4 cores, framebuffer display stack, lobster-input kbd/mouse, lobster-net, bulk serial, lobster-disk
- [ ] **D1. lobster-fb v2: expose damage/dirty rects so compositor only re-blits changed regions (perf)**
- [ ] **D2. Snapshot/resume: QEMU savevm of full machine incl. disk (VM menu in launcher)**
- [ ] **D3. Shared-folder device: host dir ↔ guest file manager (9p-style over lobster-disk MMIO)**
- [ ] **D4. Resize: display size property + guest reinit framebuffer**
- [ ] **D5. Audio passthrough device** (ties C4)

## E. Testing & Release
- [ ] **E1. Automated GUI test: script drives HID events via QEMU monitor → screenshots → assertions (a11y of the desktop)**
- [ ] **E2. Boot-to-desktop time budget ≤ 5s in emulator**
- [ ] **E3. Nightly build + release notes per merged feature (already doing manual r-notes)**

## Ship order (dependencies):
A1.1→A4.1→A4.2 (theme base) → A2.1 launcher → B2/B3 apps → A2.2/A2.3 tray+notifications →
A3.x polish → B5 browser → C1/C3 process+home → D2 snapshots → E1 CI.
