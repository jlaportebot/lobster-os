# LobsterOS GUI Desktop Overhaul Plan

## Problem Summary

The GUI desktop has three major issues:
1. **Can't resize windows** — no resize handles exist anywhere
2. **Clicking barely works** — mouse click handler drags the focused window on ANY left click, regardless of where the cursor is (title bar, client area, border). No click-to-focus, no close button hit testing, no resize handle detection.
3. **Not a real desktop** — no desktop icons, background is a flat color, windows have minimal chrome (just a title bar + 1 border), no resize handles, no maximize/minimize buttons

## Root Causes

### Mouse Click Handling (window.rs:546-572)
`handle_mouse_button` starts dragging the *currently focused* window on any left press. It never:
- Checks if cursor is over ANY window (needs hit testing)
- Checks if cursor is on the title bar (drag), close button (close), or border (resize)
- Sets `DRAG_START_X/Y` from cursor position (uses 0, leaving drag broken)

### Mouse Move Dragging (window.rs:528-543)
Drag math is broken: uses `dx/dy` (per-frame deltas) but subtracts `drag_start_x/Y` (absolute coordinates), mixing relative and absolute values. The delta needs to be accumulated incrementally.

### No Resize Logic
`resize(id, w, h)` exists at line 345 but is never called from mouse input — no border hit detection, no resize state tracking.

### No Desktop Icons
Launcher parses `.desktop` files but renders nothing on the desktop background. No icon grid, no double-click-to-launch.

### Rendering Gaps
- `draw_wallpaper` is a no-op on QEMU (skipped entirely)
- Window frames only draw title bar + borders, no minimize/maximize buttons
- No cursor rendering on QEMU (hardware cursor may not exist)

## Implementation Plan

### Phase 1: Fix Mouse Click → Window Interaction (Critical)

**File: `src/wm/window.rs`**

1. Add proper hit testing: `fn window_at(cx, cy) -> Option<u32>` — returns topmost window containing cursor point
2. Add title bar hit testing: check if cursor is in top 24px of window
3. Add close button hit testing: check if cursor is within 16x16 close button area (top-right +4px)
4. Add resize handle detection: check if cursor is within 6px of any window border
5. Rewrite `handle_mouse_button` to:
   a. On left press: hit test → determine action (focus, drag, close, resize)
   b. Set drag/resize state with absolute cursor coords from `input::cursor_pos()`
   c. On left release: clear all drag/resize state
6. Rewrite `handle_mouse_move` to:
   a. If dragging: move window by delta (cursor_now - cursor_start)
   b. If resizing: compute new w/h from cursor delta, call `resize()`
7. Add minimize/maximize buttons to title bar rendering

### Phase 2: Fix Drag Math

**File: `src/wm/window.rs`**

- Store drag start as: `DRAG_START_CURSOR_X/Y` (absolute cursor) + `DRAG_WIN_START_X/Y` (window start pos)
- On mouse move: `new_x = DRAG_WIN_START_X + (cursor_now_x - DRAG_START_CURSOR_X)`
- Same for resize: `new_w = WIN_START_W + (cursor_now_x - DRAG_START_CURSOR_X)`

### Phase 3: Desktop Background + Icons

**File: `src/wm/compositor.rs`**

1. Enable `draw_wallpaper` on QEMU — fill with theme desktop_bg color
2. Add gradient or pattern for visual appeal
3. Draw desktop icons on the left side of the screen (Terminal, Files, Settings, Calculator)

**File: `src/wm/mod.rs`**
- Add `desktop_icons::init()` and `desktop_icons::draw()` calls

**New file: `src/wm/desktop_icons.rs`**
- Static icon list with name + position + icon glyph
- Hit test on click → launch app or focus window
- Draw icons as colored rectangles with labels

### Phase 4: Software Cursor

**File: `src/wm/compositor.rs`**

1. After all rendering, draw a software cursor at `input::cursor_pos()`
2. Simple arrow cursor: 11x11 pixels, white with black border
3. This is critical — without a visible cursor, the user can't tell where clicks will land

### Phase 5: Window Chrome Improvements

**File: `src/wm/compositor.rs` (render_layer_opaque)**

1. Add minimize button (yellow circle left of close)
2. Add maximize button (green circle left of minimize)
3. Draw resize cursor indicator on borders when hovered
4. Make title bar taller (28px) and add proper window title text

### Phase 6: Build, Test, Verify

1. Build with `make FEATURES="qemu_raspi5b"`
2. Boot QEMU with GTK display
3. Take screenshot via QMP
4. Verify: desktop background visible, icons placed, cursor rendered, windows have proper chrome
5. Test mouse interaction: move mouse, click to focus, drag title bar, click close button, resize via border

## Success Criteria

- [ ] Desktop renders with background color (not pure splash)
- [ ] Desktop icons visible on left side
- [ ] Mouse cursor visible on screen
- [ ] Click on window title bar → focuses + starts drag
- [ ] Drag title bar → window moves with cursor
- [ ] Click close button → window disappears
- [ ] Click+drag window border → window resizes
- [ ] Click minimize → window hides (removed from compositor)
- [ ] Panel/taskbar visible at bottom with app buttons
- [ ] QEMU window can be resized (GTK handles this automatically)
