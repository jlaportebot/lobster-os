<!-- SUPERSEDED 2026-10-25: see DESKTOP_PLAN.md (living) and AUDIT_2026-10-25.md (queue). Do not add work here. -->

# GUI Interactivity Research for LobsterOS

## Executive Summary

This document researches how to add GUI interactivity to LobsterOS, a bare-metal Rust OS for Pi 5. Current status: USB HID driver exists, WM exists with input bridge, but keyboard input doesn't reach the shell (serial-only).

---

## 1. USB HID Keyboard → Shell/WM Input Pipeline

### Current State (LobsterOS)
- **USB HID Driver**: `src/usb/hid.rs` implements boot protocol parsing for keyboards/mice
- **HID Events**: `HidEvent` enum (KeyPress, KeyRelease, MouseMove, MouseButton)
- **Polling Model**: `hid::poll()` returns events from USB interrupt endpoints
- **WM Input Bridge**: `src/wm/input.rs` pumps events and routes to focused window

### Problem
Keyboard events go to WM windows but **NOT to the shell process**. The shell only receives serial input via UART.

### Solution Architecture
```
USB xHCI → HID poll → HidEvent → WM input pump → Terminal window → Shell process
```

Steps to implement:
1. **Create a terminal emulator window** that captures keyboard input
2. **Route keyboard events** to the terminal's input buffer instead of generic window handling
3. **Terminal feeds chars to shell** process via IPC or direct function call
4. **Shell output** goes to terminal's screen buffer, which renders to framebuffer

### Key Code Changes Needed
```rust
// In wm/input.rs handle_key_press():
if focused_window_is_terminal() {
    terminal::on_keypress(scancode, modifiers);
} else {
    window::handle_event(event);
}
```

---

## 2. VT100/ANSI Terminal Emulator Implementation in Rust

### Required Components

**(1) Parser State Machine** - DEC ANSI parser with states:
- `GET_ESCAPE`: Awaiting ESC (0x1B)
- `GET_CSI`: Awaiting '[' after ESC
- `PARAM`: Collecting CSI parameters
- `DISPATCH`: Execute final byte

**(2) Screen Buffer** - 2D cell grid:
- Each cell: (char, foreground, background, attrs)
- Cursor position (row, col), mode flags
- Scroll region, charset, color palette

**(3) Render Function** - Draw screen to framebuffer:
- Iterate cells, draw glyphs at character positions
- Apply colors from cell attributes
- Draw cursor (block/underline/blink)

### VT100 Escape Sequences to Support
| Sequence | Meaning |
|----------|---------|
| ESC [ H | Move cursor to home (0,0) |
| ESC [ 2 J | Clear screen |
| ESC [ n A | Cursor up n lines |
| ESC [ n B | Cursor down n lines |
| ESC [ n C | Cursor right n chars |
| ESC [ n D | Cursor left n chars |
| ESC [ n m | Set SGR (color/attribute) |
| ESC [ ? 25 h | Show cursor |
| ESC [ ? 25 l | Hide cursor |

### Rust Implementation Options
- **fontdue**: Pure Rust, no_std compatible - for glyph rendering
- **vt100 crate**: Full parser but requires std - NOT suitable
- **Custom implementation**: ~200 lines of state machine, viable for no_std

### Minimal Parser Sketch (no_std)
```rust
enum TerminalState {
    Ground,
    Escape,
    CsiEntry,
    CsiParam,
}

struct Terminal {
    state: TerminalState,
    row: u8, col: u8,
    params: [u8; 16],
    param_count: u8,
    screen: [[Cell; COLS]; ROWS],
}

impl Terminal {
    fn feed_byte(&mut self, byte: u8) {
        match self.state {
            TerminalState::Ground => {
                if byte == 0x1b { self.state = TerminalState::Escape; }
                else { self.print_char(byte); }
            }
            TerminalState::Escape => {
                if byte == b'[' { self.state = TerminalState::CsiEntry; }
                else { self.state = TerminalState::Ground; }
            }
            // ... handle CSI params and dispatch
        }
    }
}
```

### References
- VT100.net parser: https://vt100.net/emu/dec_ansi_parser
- codebrainz/vt100: Comprehensive VT100 parser library

---

## 3. Desktop Environment Components

### Taskbar/Panel
- Fixed-position window (bottom 40px or top)
- Shows: app launcher button, window titles, clock, system tray
- Z-order: always on top of other windows
- Click behavior: click window title to focus, click launcher to show app menu

### File Manager
- Window with directory listing from VFS
- Two-column: left sidebar (drives/folders), right main view
- Icons: simple boxes for files/folders, colored by type
- Double-click: navigate into folder or open file
- Renders file names using font rendering

### Text Editor
- Full window widget with text buffer
- Line-wrapped display, cursor indicator
- Basic editing: insert, delete, backspace, cursor movement
- File I/O: read/write to FAT32 via VFS
- Optional: syntax highlighting (simple regex-based)

### App Launcher
- Button that opens a menu of available apps
- Apps: Shell, File Manager, Text Editor, Settings
- Click spawns a new process/window
- Could show icons + names in a grid

### Architecture
```
DE Manager (process)
├── Taskbar window
├── Window stack (WM integration)
└── Spawns:
    ├── Terminal window + shell process
    ├── File manager window
    ├── Text editor window
```

---

## 4. TrueType/OpenType Font Rendering for Framebuffer

### Options Comparison

| Library | Pros | Cons | Recommendation |
|---------|------|------|----------------|
| **fontdue** | Pure Rust, no_std, small API, active | None major | ✅ BEST CHOICE |
| stb_truetype | Familiar C code, works | Unmaintained, huge | Avoid |
| Custom parser | Total control | Hundreds of lines, complex | ❌ |

### fontdue Integration Plan

1. **Add dependency**: `fontdue = { version = "0.9", default-features = false }`
2. **Load font at boot**: Read .ttf file from FAT32 into memory
3. **Create font instance**: `fontdue::Font::from_bytes(...)`
4. **Rasterize glyphs**: `font.rasterize(codepoint, size)` → (width, height, pixels)
5. **Draw to framebuffer**: Alpha-blend glyph pixels into screen

### Alpha Blending Formula
```rust
fn blend_glyph(fb: &mut Framebuffer, x: u32, y: u32, bitmap: &fontdue::Bitmap) {
    for (i, &alpha) in bitmap.bytes.iter().enumerate() {
        if alpha == 0 { continue; }  // Skip transparent
        let px_x = x + (i as u32) % bitmap.width as u32;
        let px_y = y + (i as u32) / bitmap.width as u32;
        let fb pixel = fb.get(px_x, px_y);
        let glyph_color = mix(glyph_fg, fb_pixel, alpha / 255);
        fb.set(px_x, px_y, glyph_color);
    }
}
```

### Font Cache Strategy
- Pre-render common characters (ASCII 32-126) into a bitmap cache
- LRU cache for rarely-used glyphs (accents, symbols, Unicode)
- For desktop UI: cache button labels, menu items at startup

---

## 5. Pi 5 VideoCore VII GPU Acceleration

### Current Knowledge
- **VideoCore VII**: GPU in BCM2712 SoC (Pi 5)
- **Supported APIs**: OpenGL ES 3.1, Vulkan 1.2
- **Linux drivers**: V3D (OpenGL) and V3DV (Vulkan) in Mesa 23.3+, Linux 6.8+
- **Clock**: 800 MHz

### Bare-Metal Reality
- **NO public bare-metal documentation** for VideoCore VII
- No公开 GPU register maps or command protocols
- Linux drivers are DRM/KMS based, not bare-metal friendly

### Can VideoCore VII Accelerate 2D Compositing?
Technically **yes** - it supports:
- OpenGL ES 3.1 for 2D rendering
- Hardware texture blending
- Multi-render target compositing

**But practically**: Without bare-metal drivers, you'd need to:
1. Reverse-engineer the GPU command protocol (advanced)
2. Port existing VideoCore VI bare-metal code (macoy/rpi-bare-metal)
3. Work with Linux userspace drivers (defeats bare-metal purpose)

### Recommendation
For LobsterOS:
1. **Start with CPU rendering** (current approach) - simple and fast for 2D
2. Pi 5 A76 CPU at 2.4 GHz is powerful enough for basic GUI
3. Add GPU acceleration **later** if needed (profile first)

### GPU Path (Future)
If GPU acceleration becomes necessary:
- Study: macoy/rpi-bare-metal (VideoCore VI for Pi 4)
- Linux V3D source: `drivers/gpu/drm/v3d/` in Linux kernel
- Mesa V3D driver: `src/galler/dri/v3d/` in Mesa

---

## 6. How Existing Lightweight DEs Work

### dwarfwm (dynamic window manager)
- **Architecture**: Single C file, ~2000 lines
- **Event loop**: Xlib `XNextEvent()` → dispatch → redraw
- **Tiling mode**: Windows automatically arranged (master+stack)
- **No decor**: Just window borders, no title bars
- **Config**: Edit source code, recompile

### evilwm (minimalist floating WM)
- **Architecture**: ~1500 lines C
- **Events**: X11 protocol (SelectInput on windows)
- **Focus**: Click-to-focus or follow mouse
- **Features**: Just window borders, alt+tab, simple keybindings
- **No compositing**: Direct window rendering

### LXDE (Lightweight Desktop Environment)
- **Components**:
  - Openbox (window manager)
  - lxpanel (taskbar/panel)
  - pcmanfm (file manager)
  - leafpad (text editor)
- **Architecture**: Separate processes with IPC
- **Event loop**: Each app runs its own GTK/Qt event loop
- **Synaptic**: `X11` provides input/window coordination

### Lessons for LobsterOS
1. **Keep it simple**: Start with evilwm-style minimal borders
2. **Single WM process**: Handle all window logic in one place
3. **No IPC overhead**: Since bare-metal, windows can share memory directly
4. **Event-driven**: Poll HID + timer events, dispatch, composite

---

## 7. Event Loop Architecture for Bare-Metal GUI

### Main Loop (No X11/Wayland)

```rust
// Main scheduler loop in init.rs
loop {
    // 1. Poll hardware
    hid::poll().map(|e| wm::input::handle_event(e));
    timer::poll();  // Check for timeout events
    
    // 2. Update application state
    for window in windows.iter() {
        window.update();  // Self-update (animations, blinking cursor)
    }
    
    // 3. Compose frame
    compositor::composite();  // Draw all windows to back buffer
    
    // 4. Swap buffers
    fb::swap_buffers();  // Display back buffer
    
    // 5. Yield or sleep briefly
    sched::yield_();
}
```

### Event Dispatch Pattern

```
Input Event Types:
├── Keyboard: scancode, modifiers, repeat flag
├── Mouse: position, button state, scroll
├── Timer: timeout ID, interval elapsed
└── Window: expose, resize, move request

Dispatch Flow:
1. Event enters system (ISR or poll)
2. Stored in event queue (ring buffer)
3. WM loops through queue
4. For each event:
   a. Determine target (focused window, cursor window, broadcast)
   b. Window-specific handler (on_key, on_click, on_timer)
   c. Window may update internal state
   d. Mark as "dirty" if visual change needed
5. Compositor redraws dirty regions
```

### Shell Integration

```rust
// Terminal emulator captures keyboard, feeds shell
struct Terminal {
    screen: ScreenBuffer,
    input: Vec<u8>,  // Raw bytes from keyboard
    shell_pid: Option<u32>,  // Shell process ID
}

impl Terminal {
    fn on_keypress(&mut self, scancode: u8) {
        if let Some(char) = scancode_to_ascii(scancode) {
            self.input.push(char as u8);
            self.send_to_shell(char);  // Write to shell stdin
        }
    }
    
    fn shell_stdout_handler(&mut self, byte: u8) {
        self.feed_byte(byte);  // VT100 parser processes output
        self.screen.mark_dirty();  // Mark for redraw
    }
}
```

---

## Implementation Roadmap

### Phase 1: Terminal Emulator (Priority 1)
1. Create `src/terminal/` module with VT100 parser
2. Implement screen buffer and parser state machine
3. Anchor terminal to WM window (client area renders to framebuffer)
4. Hook keyboard: `handle_key_press()` → `terminal::on_keypress()`
5. Connect shell: `terminal::send_to_shell()` → shell process stdin

### Phase 2: Font Rendering (Priority 1)
1. Add `fontdue` dependency
2. Load a .ttf file from FAT32 at boot
3. Implement glyph rasterization in `src/display/font.rs`
4. Replace 8x16 bitmap font with TTF rendering
5. Test with terminal output and WM title bars

### Phase 3: Desktop Components (Priority 2)
1. **Taskbar**: Fixed window at bottom, simple buttons
2. **App launcher**: Button spawns terminal or file manager
3. **File manager**: Directory listing with icons
4. **Text editor**: Basic buffer + render loop

### Phase 4: Polish (Priority 3)
1. Smooth animations (window open/close)
2. Color themes for WM
3. Better z-ordering and focus management
4. Screen redraw optimization (dirty regions)

---

## File List to Create/Modify

### New Files
- `src/terminal/mod.rs` - Terminal emulator module
- `src/terminal/parser.rs` - VT100/ANSI parser
- `src/terminal/screen.rs` - Screen buffer (cell grid)
- `src/display/font.rs` - fontdue integration
- `src/de/taskbar.rs` - Taskbar/panel implementation
- `src/de/filemanager.rs` - File manager window
- `src/de/texteditor.rs` - Text editor implementation
- `src/de/launcher.rs` - Application launcher

### Modified Files
- `src/wm/input.rs` - Route keyboard to terminal
- `src/wm/mod.rs` - Add DE initialization
- `src/wm/window.rs` - Add terminal window type
- `Cargo.toml` - Add `fontdue` dependency

---

## Key Decisions

1. **Font**: Use fontdue (pure Rust, no_std compatible)
2. **Terminal**: Custom VT100 parser (~300 lines)
3. **GPU**: Skip for now, CPU rendering is sufficient on Pi 5
4. **Desktop architecture**: Integrates with existing WM
5. **Shell**: Use existing shell, connect to terminal emulator

---

## References

### Rust Terminal Emulators
- https://github.com/codebrainz/vt100 - VT100 parser library
- https://docs.rs/vt100 - vt100 crate docs
- https://vt100.net/emu/dec_ansi_parser - VT100 parser specification

### Font Rendering
- https://github.com/intel/fdont - fontdue documentation
- https://github.com/jeaye/jc_font - Rust TTF rasterizer

### Window Managers
- https://dwm.suckless.org/ - dwm source (2k lines C)
- https://github.com/nikolas/evilwm - evilwm source (~1.5k lines)
- https:// LXDE.org - LXDE architecture

### Pi 5 / VideoCore VII
- https://www.phoronix.com/review/raspberry-pi-5-graphics - Phoronix review
- https://docs.mesa3d.org/drivers/v3d.html - V3D driver docs (Linux)
- https://github.com/Towdo/py-videocore7 - Python VideoCore VII library

---

*Research completed: 2026-06-04*
*Status: Ready for implementation*
