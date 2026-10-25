<!-- SUPERSEDED 2026-10-25: see docs/DESKTOP_PLAN.md (living) and docs/AUDIT_2026-10-25.md (queue). Do not add work here. -->

# LobsterOS + Pi5 Emulator — Deep Audit & Fix Plan

## Audit Results (Aug 6, 2026)

### Architecture
- **LobsterOS**: Bare-metal Rust no_std OS, 68K LOC, 135 .rs files, aarch64-unknown-none target
- **QEMU Emulator**: Custom QEMU 10.0.8 build with raspi5b (BCM2712) machine
- **Desktop Launcher**: Python/Tk control panel + QEMU GTK framebuffer window

### What Works
✅ Kernel fully boots on raspi5b with `-dtb` flag
✅ MMU, page tables, kernel heap, frame allocator
✅ Framebuffer: 1920x1080@32bpp via BCM2835 mailbox emulation
✅ Window manager: compositor, 5 desktop windows, panel, theme
✅ Shell: interactive serial console (help, ps, version, mem all respond)
✅ Scheduler, syslog, panic recovery, filesystem (VFS)

### What's Broken
❌ **CRITICAL: GUI input completely broken** — no mouse/keyboard reaches WM
❌ Kernel panics without `-dtb` flag (DTB ptr=0x100)
❌ Manifest: "4123168604160 MB" — DTB memory parse bug
❌ Launcher uses cortex-a72 (should be cortex-a76)
❌ Stale kernel8.img in pi5-emulator-repo (Jun 21, 174KB vs current 500KB)
❌ Secondary cores 1-3 fail to come online (non-critical)
❌ QEMU raspi5b has no USB bus — `-device usb-mouse` silently fails

### Root Cause of "Can't Click Around"
1. Kernel boots with `qemu_raspi5b` feature → USB/XHCI init SKIPPED
2. WM input bridge polls `usb::hid::poll()` → always returns None
3. QEMU raspi5b machine has no emulated USB controller
4. Desktop renders but receives zero input events

## Fix Plan (8 Phases)

### Phase 1: Fix Immediate Bugs
- [x] Audit codebase
- [ ] Fix launcher: cortex-a72 → cortex-a76
- [ ] Fix stale kernel8.img copy
- [ ] Fix DTB memory parse bug (wrong /memory/reg interpretation)
- [ ] Sync DTB file to pi5-emulator-repo

### Phase 2: Build QEMU Input Device (C)
Add a memory-mapped input device to the QEMU raspi5b machine:
- New file: `hw/misc/qemu_input_mmio.c`
- MMIO region at unused address (0x7d008000, near UART)
- Ring buffer of input events (32 events × 16 bytes)
- Hijacks QEMU display mouse/keyboard events → writes to ring buffer
- Register in raspi5b machine init
- Rebuild QEMU

### Phase 3: Build Kernel Input Driver (Rust)
Add `src/drivers/qemu_input.rs`:
- Polls MMIO ring buffer at 0x7d008000 (qemu_raspi5b only)
- Parses event types: mouse-move, mouse-button, key-press, key-release
- Converts to `HidEvent` type (matching existing USB HID events)
- Feeds events into a queue that `hid::poll()` reads from
- Replace `hid::poll()` with dual-source: USB HID + QEMU MMIO input

### Phase 4: Wire Input Through WM
- Modify `wm/input.rs pump_events()` to also check QEMU MMIO input
- Ensure cursor movement, clicks, key events reach windows
- Test: clicks should focus/move windows, keyboard types in terminal

### Phase 5: Activate Desktop Windows
- Make terminal window actually interactive (keypress → terminal input)
- Make calculator clickable (buttons respond)
- Panel/launcher click handling
- Context menu (right-click) on desktop

### Phase 6: Rebuild
- Rebuild QEMU with new input device (incremental, ~2 min)
- Rebuild kernel8.img with qemu_raspi5b feature
- Copy new kernel8.img to pi5-emulator-repo
- Update DTB if needed

### Phase 7: Fix Desktop Launcher
- Update lobsteros_gui.py: cortex-a76, correct paths
- Update run_lobsteros_gui.sh and run_pi5_gui.sh
- Update .desktop entry
- Test full launcher flow

### Phase 8: End-to-End Verification
- Boot with GTK display
- Screenshot showing desktop
- Click windows to demonstrate input works
- Test keyboard in terminal
- Document working state
