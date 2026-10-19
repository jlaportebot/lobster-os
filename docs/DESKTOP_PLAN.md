# LobsterOS Desktop — Master Plan (v1)
**Goal:** A polished, everyday-driver desktop OS (Windows/macOS-class UX) on the LobsterOS kernel +
QEMU Pi 5 emulator. Every item ships with a live end-to-end test before it's marked done.

Legend: [x] done + verified live · [~] in progress · [ ] planned

## A. Core Desktop Shell
### A1. Desktop surface
- [x] Full-screen framebuffer with compositor (2237-line compositor)
- [x] Desktop icons (launcher + files)
- [~] **A1.1 Wallpaper system**: gradient exists in compositor; bitmap + /sd persistence TODO
- [ ] **A1.2 Desktop right-click menu**: New File/Folder, Open Terminal, Display Settings, Refresh
- [ ] **A1.3 Icon selection + rubber-band multi-select + drag-to-arrange (grid snap)**
### A2. Panel (taskbar)
- [x] Panel with clock + task list + launcher button
- [x] **A2.1 Start-menu style app launcher** — DONE 39afe4b: scrollable app tiles w/ colored icons, hover+keyboard nav, focus-or-create, Esc/outside-click close. (Search + power row still TODO)
- [ ] **A2.2 System tray**: network state icon, CPU/RAM mini-graph, volume, battery/clock popup
- [ ] **A2.3 Notification center**: toasts + actionable notifications + history
- [ ] **A2.4 Window list previews** (hover peek), pinning apps, grouping by workspace
### A3. Window management
- [x] create/destroy/move/resize/focus/minimize/maximize/snap/workspaces
- [ ] **A3.1 Window animations** (open/close/minimize genie fade, snap transitions)
- [ ] **A3.2 Double-click title maximize, drag-to-edge snap preview overlay, resize from all 8 edges**
- [ ] **A3.3 Alt-Tab switcher overlay** with window thumbnails
- [ ] **A3.4 Per-app "dirty" state dot + close confirmation for unsaved docs**
### A4. Theme engine
- [x] Theme struct + palette + fonts
- [ ] **A4.1 Light/Dark mode toggle** (Settings + tray), animated transition
- [ ] **A4.2 Accent color picker** (persisted), rounded-corner radius control
- [ ] **A4.3 Custom cursor** bitmaps (default, resize, text I-beam, busy)
- [ ] **A4.4 Theme file format on /sd** (/sd/etc/theme.conf) + live reload

## B. Applications
### B1. Terminal (v1 exists)
- [x] VT100 parser + screen buffer + window render
- [ ] **B1.1 Tabbed terminal + font size shortcuts + copy/paste via mouse**
- [ ] **B1.2 Shell-in-a-window**: interactive shell drives a per-window VT session
### B2. Files (file manager v1 exists)
- [x] Tree browse, /sd integration
- [ ] **B2.1 Sidebar (Home/Storage/etc), toolbar (up/refresh/new/view toggle), status bar (item count + free space)**
- [ ] **B2.2 Icon/list views, selection, rename, delete, drag-drop move, properties dialog**
- [ ] **B2.3 Create/copy/move/delete with undo toast; persistent across reboot**
### B3. Settings (v1 exists)
- [ ] **B3.1 Settings app pages**: Appearance (A4), Display (res/theme), Network, Users, Storage, About**
- [ ] **B3.2 Live-apply every setting + persist all to /sd/etc/**
### B4. Text editor (v1 exists)
- [ ] **B4.1 Multi-buffer tabs, save-as, unsaved indicator, find**
### B5. Browser (new)
- [ ] **B5.1 Text-mode browser: URL bar + back/forward + simple HTML render via lobster-net fetch**
- [ ] **B5.2 History + bookmarks persisted to /sd**
### B6. Media & extras
- [ ] **B6.1 Image viewer from disk (BMP/PNG)**, wallpaper setter integration
- [ ] **B6.2 Calculator (v1 exists) → keyboard input + history**
- [ ] **B6.3 Process manager + sysmon merged into one Task Manager w/ graphs**
- [ ] **B6.4 App Store (v1 exists) → real package install from repo into /sd, launcher integration**
- [ ] **B6.5 Clock/timer, Notes app (persisted), Minesweeper (fun proof of polish)**

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
