# LobsterOS Master Development Plan

## Goal
Transform LobsterOS from a bare-metal kernel into a fully usable operating system
for the Raspberry Pi 5, with WiFi, interactive GUI shell, and a web browser.

---

## Current State (as of 2026-06-04)

| Module | Files | Lines | Status |
|--------|-------|-------|--------|
| arch (cpu/gic/timer/exception) | 4 | 647 | Working |
| memory (mmu/frame_alloc/heap/vmem/process_vmem) | 5 | 1,664 | Working |
| drivers (uart/gpio/mailbox/nvme/pcie/power/gpu/audio/serial_input) | 9 | 4,347 | Working |
| display (fb/graphics/console/logo) | 4 | 1,211 | Working |
| net (genet/arp/ipv4/icmp/udp/tcp/dhcp/dns/socket) | 10 | 3,386 | Ethernet only |
| usb (xhci/hid) | 2 | 1,430 | Working |
| wm (compositor/window/theme/widget/input) | 5 | 2,246 | Structure only |
| fs (vfs/fat32/initrd/block_dev/devfs/procfs/sysfs) | 7 | 2,768 | Working |
| sched (process/signal/syscall/elf/dlinker/init) | 6 | 5,193 | Working |
| smp (runqueue/balance/ipi) | 3 | 1,048 | Working |
| shell | 1 | 1,930 | Serial-only |
| terminal (parser/screen) | 3 | 487 | Stub |
| ipc | 1 | 946 | Structure |
| security | 1 | 774 | Structure |
| dtb | 1 | 114 | Basic parsing |
| sync | 1 | 114 | Working |
| main + boot.S | 2 | 824 | Working |
| **Total** | | **~28,300** | |

**Build:** `cargo build --release --target aarch64-unknown-none` → 0 errors, kernel8.img = 109KB

**Missing:** WiFi driver, SDIO controller, GUI terminal, WPA2 supplicant, HTTP client, HTML renderer, desktop environment, keyboard→shell input pipeline

---

## Phase 1: SDIO Controller + WiFi Driver (largest phase, ~8,000-10,000 new lines)

The Pi 5 WiFi chip (Infineon CYW43455 / BCM43455) connects via SDIO2 at address
0x11_000_0000 (BCM2712 AXI offset), GIC SPI interrupt 274.

### 1.1 SDIO Host Controller Driver (`src/drivers/sdio.rs`)

The BCM2712 uses the same Arasan SDHCI-compatible controller as earlier Pi models
but mapped at a different base address. Circle's `emmc.c` uses `VIRTIO+0x1100000`
for Pi 5 (where `VIRTIO = MEM_IOMEM_AXI_START`).

**Subtasks:**
1.1.1 Define SDHCI register map (offsets from base 0x11_000_0000):
  - Arg2 (0x00), Blksizecnt (0x04), Arg1 (0x08), Cmdtm (0x0C)
  - Resp0-3 (0x10-0x1C), Data (0x20), Status (0x24)
  - Control0 (0x28), Control1 (0x2C), Interrupt (0x30)
  - Irptmask (0x34), Irpten (0x38), Control2 (0x3C)
  - Forceirpt (0x50), Slotisrver (0xFC)

1.1.2 Implement controller reset and clock configuration:
  - Write SRSTHC | SRSTCMD | SRSTDATA to Control1
  - Wait for clock stable, set Initfreq (400 KHz for SDIO init)
  - Enable internal clock (Clkintlen), wait for Clkstable

1.1.3 Implement `send_cmd(cmd: u32, arg: u32) -> Result<[u32; 4], SdioError>`:
  - Build CmdTM register from command index + response type
  - Use the `cmdinfo[]` table from Circle (maps CMD→flags)
  - Wait for Cmddone interrupt, read response registers
  - Handle timeouts and CRC errors

1.1.4 Implement data transfer (CMD52/CMD53 for SDIO I/O):
  - CMD52: `sdio_read_byte(fn, addr)` / `sdio_write_byte(fn, addr, data)`
  - CMD53: `sdio_read_block(fn, addr, buf, len)` / `sdio_write_block(fn, addr, buf, len)`
  - Set up Blksizecnt, handle Readrdy/Writerdy interrupts
  - Pi 5 specific: track `blksize` field for block operations

1.1.5 Implement DMA-based data transfer for bulk operations:
  - Allocate DMA-coherent buffer (4KB aligned)
  - Program data register address, wait for Datadone interrupt
  - Cache coherency: clean/invalidate around DMA ops

1.1.6 Wire up GIC SPI 274 (`ARM_IRQ_SDIO2`) interrupt handler:
  - Register in GIC, mask/unmask on command boundaries
  - Interrupt handler signals completion via static flag

1.1.7 GPIO pin configuration for Pi 5 SDIO2:
  - Pi 5 uses RP1 pin controller (ARM_PINCTRL1_BASE), not legacy GPIO
  - Pin function selection: Func7 for SDIO2 data/cmd pins
  - Pull-up on CMD + DATA0-DATA3 lines
  - Handle SoC stepping (D0 vs pre-D0 register layout)

1.1.8 SDIO bus initialization sequence:
  1. CMD0 (GO_IDLE_STATE) — reset all cards
  2. CMD5 (IO_SEND_OP_COND) — query SDIO card capabilities
  3. CMD3 (SEND_RELATIVE_ADDR) — get card RCA
  4. CMD7 (SELECT_CARD) — select the card
  5. CMD52 to CCCR.Ioenable — enable functions 1+2
  6. CMD52 to CCCR.Blksize — set block size to 64
  7. CMD52 to CCCR.Highspeed — enable high-speed mode (50 MHz)
  8. Switch clock from Initfreq (400KHz) → SDfreq (25MHz) → SDfreqhs (50MHz)
  9. CMD52 to set bus width to 4-bit (Setbuswidth in CCCR)

### 1.2 BCM43455 WiFi Driver (`src/drivers/wifi.rs`)

Port the logic from Circle's `ether4330.c` (2646 lines, Plan 9 derived) to Rust.
This driver handles firmware loading, the Sonics Silicon Backplane (SSB), and
802.11 frame I/O.

**Subtasks:**
1.2.1 SDIO function access layer:
  - `sdiord(fn, addr)` → CMD52 read
  - `sdiowr(fn, addr, data)` → CMD52 write
  - `sdiorwext(fn, write, buf, len, addr, incr)` → CMD53 read/write
  - `sdioabort(fn)` → abort pending I/O on function

1.2.2 Sonics Silicon Backplane (SSB) access:
  - The WiFi chip's internal cores are accessed via SDIO window registers
  - `sbwindow(addr)` — set Sbaddr registers to target 32KB window
  - `sbrw(fn, write, buf, len, off)` — read/write within current window
  - `sbmem(write, buf, len, off)` — access RAM/ROM through backplane
  - Key registers: Sbaddr (0x1000A), Framectl (0x1000D), Clkcsr (0x1000E)
  - Window register: 0x1000A = [addr>>8][addr>>16][addr>>24] (3 bytes)

1.2.3 Core scan and initialization:
  - `corescan()` — enumerate chip cores (ARM CM3/CR4, 802.11, SOCRAM)
  - `ramscan()` — find and size internal SOCRAM
  - `sbinit()` — reset backplane, verify chip ID (0x43455 for CYW43455)
  - `sbenable()` — enable HT clock, enable SDIO functions 1+2

1.2.4 Firmware loading:
  - Read `brcmfmac43455-sdio.bin` from FAT32 filesystem (must be on SD card)
  - Upload to chip RAM via `sbmem()` writes in 2KB blocks
  - Verify by reading back (Firmwarecmp=1)
  - Also load `brcmfmac43455-sdio.txt` (NVRAM/config) and `.clm_blob`
  - Signal firmware to start, wait for "firmware ready" event
  - **Firmware files source:** Linux firmware repo or Circle's `addon/wlan/firmware/Makefile`
  - Pi 5 specific: also load `brcmfmac43455-sdio.raspberrypi,5-model-b.bin`

1.2.5 802.11 frame I/O:
  - TX: write frame to SDIO function 2 via `packetrw()`
  - RX: read frames from SDIO function 2 (interrupt-driven)
  - Frame format: 2-byte length header, then 802.11 frame
  - Handle flow control (Rfhalt/Wfhalt in Framectl)

1.2.6 Event handling and ioctl interface:
  - `etherbcmintr()` — process WiFi chip events
  - `bcmevent()` — handle scan results, link status, auth events
  - Ioctl interface: send commands to firmware (set SSID, set security, scan, etc.)
  - Circular ioctl buffer with request/response rings

### 1.3 Network Interface Abstraction

Currently the net stack is hardcoded to GENET. Need to abstract it.

**Subtasks:**
1.3.1 Create `src/net/iface.rs` — network interface trait:
  ```rust
  pub trait NetInterface {
      fn mac_addr(&self) -> [u8; 6];
      fn send_packet(&mut self, buf: &[u8]) -> Result<(), NetError>;
      fn receive_packet(&mut self, buf: &mut [u8]) -> usize;
      fn is_link_up(&self) -> bool;
      fn name(&self) -> &str;
  }
  ```

1.3.2 Implement `GenetInterface` wrapping existing genet driver

1.3.3 Implement `WifiInterface` wrapping BCM43455 driver:
  - WiFi frames need 802.11 header, not Ethernet
  - The firmware handles 802.11→Ethernet conversion in "managed" mode
  - So from the IP stack's perspective, WiFi still looks like Ethernet

1.3.4 Update `net/mod.rs` to support multiple interfaces:
  - Static array of interface pointers
  - `poll()` iterates all interfaces
  - ARP/DHCP per-interface state
  - `ifconfig` shell command shows all interfaces

### 1.4 WPA2 Supplicant (`src/net/wpa.rs`)

WPA2-PSK authentication for WiFi. This is ~2,000-3,000 lines.

**Subtasks:**
1.4.1 Cryptographic primitives (needed for WPA2):
  - HMAC-SHA1 (for PRF-512 key derivation)
  - AES-128-ECB decrypt (for key unwrapping)
  - HMAC-MD5 (for Michael MIC in TKIP — if supporting legacy)
  - CRC32 (for TKIP)
  - AES-128-CCMP (for WPA2-AES data encryption)
  - These can be implemented from scratch in ~800 lines total (no std dependency)

1.4.2 WPA2 4-way handshake implementation:
  1. Receive EAPOL-Key frame #1 (ANonce from AP)
  2. Generate SNonce (random 32 bytes)
  3. Derive PTK using PRF-512(PMK, "Pairwise key expansion", min(AA,SPA)||max(AA,SPA)||min(ANonce,SNonce)||max(ANonce,SNonce))
  4. Send EAPOL-Key frame #2 (SNonce + MIC)
  5. Receive EAPOL-Key frame #3 (GTK encrypted + MIC)
  6. Send EAPOL-Key frame #4 (ACK)
  7. Install PTK and GTK into driver

1.4.3 EAPOL frame parser/builder:
  - Version, type, length fields
  - Key descriptor type (RC4 or HMAC-SHA1/AES)
  - Key info bitmask (Pairwise/Group, Install, Ack, MIC, Secure, Encrypted)
  - Key length, replay counter, nonce, key IV, key RSC, key ID, key data

1.4.4 Passphrase → PMK derivation:
  - PBKDF2-HMAC-SHA1(passphrase, SSID, 4096 iterations, 256-bit output)
  - This is the slow part (~2 seconds on Pi 5), do it once at connect time

1.4.5 WiFi connection flow (exposed to shell):
  - `wifi scan` — trigger scan, display BSSID/SSID/RSSI/channel/security
  - `wifi connect <SSID> <password>` — authenticate and associate
  - `wifi disconnect` — deauthenticate
  - `wifi status` — show current connection info

### 1.5 DHCP over WiFi

Once WiFi is connected, run DHCP on the WiFi interface.

**Subtasks:**
1.5.1 Extend `dhcp::discover()` to accept an interface parameter
1.5.2 Send DHCPDISCOVER on WiFi interface
1.5.3 Configure WiFi interface IP from DHCPOFFER

---

## Phase 2: Interactive GUI Terminal (~2,500-3,000 new lines)

The terminal emulator code exists as a stub (487 lines). It needs to be completed
and connected to the shell via the keyboard/mouse input pipeline.

### 2.1 Complete VT100/ANSI Parser (`src/terminal/parser.rs`)

**Subtasks:**
2.1.1 Add missing CSI sequences:
  - Cursor: H/f (absolute position), s (save), u (restore)
  - Erase: J (screen), K (line), P (delete chars), @ (insert chars)
  - Scroll: S (up), T (down)
  - Tab: I (tab set), Z (tab backward)
  - Mode: h (set mode), l (reset mode) — for bracketed paste, alternate screen
  - SGR 256-color and RGB color sequences (38;5;N, 48;2;R;G;B)

2.1.2 Add OSC (Operating System Command) sequences:
  - OSC 0 — set window title
  - OSC 52 — clipboard access
  - Parse `\x1b]...BEL` and `\x1b]...\x1b\\` terminators

2.1.3 Add DEC private mode sequences (CSI ? ... h/l):
  - ?1 — application cursor keys
  - ?25 — show/hide cursor
  - ?1000 — mouse tracking (X10)
  - ?1002 — mouse tracking (button event)
  - ?1006 — mouse tracking (SGR extended)
  - ?1049 — alternate screen buffer

2.1.4 Add scrollback buffer (Shift+PageUp/Down):
  - Ring buffer of 1000 lines beyond visible 25 rows
  - Scroll offset tracking

### 2.2 Enhanced Screen Buffer (`src/terminal/screen.rs`)

**Subtasks:**
2.2.1 Dynamic terminal sizing (detect framebuffer resolution):
  - Calculate COLS/ROWS from fb.width/fb.height and font metrics
  - Support resizing (CSIColumnsAndRows response)

2.2.2 Unicode support (basic):
  - UTF-8 byte sequence handling in parser
  - Wide character handling (CJK = 2 columns)
  - Replacement character for invalid bytes

2.2.3 Selection and clipboard:
  - Mouse drag to select text
  - Copy to internal clipboard buffer
  - Paste via middle-click or Shift+Insert

### 2.3 Terminal Renderer (`src/terminal/render.rs`)

New file — renders the screen buffer to the framebuffer via the WM.

**Subtasks:**
2.3.1 Character rendering:
  - Built-in bitmap font (8x16 PC BIOS font, ~4KB of glyph data)
  - Draw each cell as: background rect + foreground glyph
  - Cursor rendering (block/underline/bar styles, blink timer)

2.3.2 Dirty region rendering:
  - Only redraw cells where `dirty` flag is set
  - Mark dirty on: character write, cursor move, scroll, resize
  - Full redraw on compositor request

2.3.3 Framebuffer integration:
  - Render into a Window's content buffer
  - Return dirty rects to compositor
  - Support double-buffering via compositor

### 2.4 Keyboard → Shell Input Pipeline

**Subtasks:**
2.4.1 Connect `wm::input::pump_events()` to terminal:
  - Keyboard KeyPress → `Terminal::on_keypress()`
  - Special keys generate escape sequences (arrow keys = `\x1b[A/B/C/D`)
  - Function keys, Home/End/PgUp/PgDn

2.4.2 Terminal → Shell bridge:
  - Terminal collects input bytes in `input_buf`
  - Shell reads from terminal instead of UART
  - Shell output goes through `Terminal::feed_byte()` (via `Write` trait)

2.4.3 Mouse event routing:
  - Mouse clicks/scroll in terminal window → terminal selection/scrollback
  - Mouse tracking mode: encode events as escape sequences for apps

2.4.4 Multiple terminal windows:
  - Each window gets its own Terminal instance
  - Shell runs in the focused terminal window
  - Alt+Tab / window click to switch focus

---

## Phase 3: Desktop Environment (~2,000 new lines)

### 3.1 Desktop Shell (`src/wm/desktop.rs`)

New file — manages the desktop, taskbar, and app launching.

**Subtasks:**
3.1.1 Desktop background:
  - Solid color or gradient fill
  - Desktop icons (positioned rectangles with label + glyph)

3.1.2 Taskbar / Panel:
  - Horizontal bar at bottom of screen
  - Shows: active windows, system clock, CPU/mem usage indicators
  - Window list: click to focus/minimize
  - Right-click context menu

3.1.3 Window management enhancements:
  - Drag to move (title bar)
  - Drag edges to resize
  - Minimize/maximize/close buttons
  - Alt+Tab window cycling
  - Z-order management (click to raise)

3.1.4 Application launcher:
  - Simple menu with keyboard navigation
  - Launch terminal, browser, system monitor
  - Future: launch ELF binaries from filesystem

### 3.2 System Monitor App (`src/apps/sysmon.rs`)

**Subtasks:**
3.2.1 CPU usage graph (per-core utilization from scheduler)
3.2.2 Memory usage bar (heap + frame allocator stats)
3.2.3 Network stats (packets/sec, bytes transferred)
3.2.4 Process list (scrollable table: PID, name, state, CPU%)

### 3.3 Text Editor App (`src/apps/editor.rs`)

**Subtasks:**
3.3.1 Basic line editor with cursor movement
3.3.2 File open/save (FAT32 filesystem)
3.3.3 Search/replace
3.3.4 Syntax highlighting (simple keyword matching)

---

## Phase 4: Web Browser (~6,000-8,000 new lines)

This is the most complex new feature. We'll build a custom lightweight browser
optimized for our framebuffer, not port an existing engine.

### 4.1 HTTP/HTTPS Client (`src/net/http.rs`)

**Subtasks:**
4.1.1 HTTP/1.1 client:
  - DNS resolution (already have `src/net/dns.rs`)
  - TCP connect (already have `src/net/tcp.rs`)
  - Send request: `GET /path HTTP/1.1\r\nHost: example.com\r\n\r\n`
  - Parse response: status line + headers + body
  - Chunked transfer encoding support
  - Connection: keep-alive reuse

4.1.2 TLS 1.3 client (`src/net/tls.rs`):
  - This is ~2,000-3,000 lines alone — needed for HTTPS
  - **Alternative approach:** Start with HTTP-only, add TLS later
  - Minimal TLS 1.3: ECDHE key exchange (X25519), AES-128-GCM, SHA-256
  - Certificate verification: pin known CAs or skip verification initially
  - **Phased:** HTTP first, then add TLS for Google, Wikipedia, etc.

4.1.3 URL parser (`src/net/url.rs`):
  - Parse `scheme://host:port/path?query#fragment`
  - Percent-encoding/decoding
  - Default ports (80 for HTTP, 443 for HTTPS)

### 4.2 HTML Parser (`src/browser/html.rs`)

**Subtasks:**
4.2.1 Tokenizer (state machine per HTML5 spec — simplified):
  - Tags: open, close, self-closing
  - Attributes: name="value"
  - Text nodes
  - Comments: `<!-- ... -->`
  - DOCTYPE declaration

4.2.2 DOM tree builder:
  - Node types: Element, Text, Document
  - Tree structure: parent, children, siblings
  - Supported elements: html, head, body, div, p, span, a, h1-h6,
    table/tr/td, ul/ol/li, img, form, input, button, pre, code, br, hr
  - Ignore: script, style, svg, canvas (too complex for v1)

4.2.3 Text extraction mode (for simple pages):
  - Walk DOM tree, extract text content
  - Apply basic formatting (headings → bold, paragraphs → newlines)
  - This gives us a "text browser" like lynx/w3m quickly

### 4.3 CSS Parser (`src/browser/css.rs`)

**Subtasks:**
4.3.1 Tokenizer: selectors, properties, values
4.3.2 Property support (minimal subset):
  - color, background-color, font-size, font-weight, text-align
  - margin, padding (as spacing values)
  - width, height (as constraints)
  - display: block, inline, inline-block, none
4.3.3 Selector matching:
  - Tag selectors: `p`, `div`
  - Class selectors: `.classname`
  - ID selectors: `#id`
  - Descendant combinators: `div p`
4.3.4 Style resolution:
  - Cascade: author > user > browser defaults
  - Specificity calculation
  - Inherited properties

### 4.4 Layout Engine (`src/browser/layout.rs`)

**Subtasks:**
4.4.1 Box model:
  - Each DOM element → LayoutBox with content/padding/border/margin
  - Block formatting context (vertical stack)
  - Inline formatting context (horizontal text flow with line wrapping)

4.4.2 Block layout:
  - Vertical stack of block-level boxes
  - Available width = parent content width - margins/padding
  - Auto margins for centering

4.4.3 Inline layout:
  - Text segmentation (break at word boundaries)
  - Line box construction (accumulate inline boxes until full)
  - Line wrapping at available width

4.4.4 Replaced elements:
  - Images: load from HTTP, decode to pixel buffer, scale to fit
  - Input fields: render as native text widgets

### 4.5 Renderer (`src/browser/render.rs`)

**Subtasks:**
4.5.1 Paint each LayoutBox to a pixel buffer:
  - Background color fill
  - Text rendering (using terminal's bitmap font)
  - Border rendering (1px solid)
  - Image blitting

4.5.2 Scrollable viewport:
  - Document height may exceed window height
  - Scroll offset tracking
  - Scrollbar widget on right edge
  - Mouse wheel / arrow key scrolling

4.5.3 Link handling:
  - Hover detection (mouse position → hit-test → cursor change)
  - Click on `<a>` → navigate to href URL
  - URL resolution (relative → absolute)
  - Navigation history (back/forward stack)

### 4.6 Browser Shell (`src/browser/mod.rs`)

**Subtasks:**
4.6.1 Browser window in the WM:
  - Title bar shows current URL
  - Content area is the rendered page
  - Address bar widget at top (TextInput + "Go" button)
  - Back/Forward/Reload buttons

4.6.2 User interaction:
  - Click links → navigate
  - Scroll → page scroll
  - Type in address bar → URL entry
  - Forms: text input fields → native TextInput widgets
  - Tab key → cycle through focusable elements

4.6.3 Page lifecycle:
  - Start: empty page with address bar focused
  - Enter URL → DNS → TCP → HTTP → parse → layout → render
  - Loading indicator (progress bar or spinner)
  - Error pages (DNS failure, connection refused, timeout)

### 4.7 Image Decoder (`src/browser/image.rs`)

**Subtasks:**
4.7.1 BMP decoder (simplest format, no compression):
  - Parse BMP header, read pixel data
  - Convert to RGBA pixel buffer

4.7.2 PNG decoder (most common on the web):
  - Parse IHDR, IDAT chunks
  - Decompress zlib/deflate data
  - Handle filter types (None, Sub, Up, Average, Paeth)
  - Support 8-bit RGB/RGBA only (skip 16-bit, indexed)

4.7.3 JPEG decoder (common but complex):
  - Parse JFIF markers, Huffman tables, quantization tables
  - IDCT (integer discrete cosine transform)
  - YCbCr → RGB conversion
  - Support baseline sequential only (skip progressive)
  - ~500-700 lines for a minimal decoder

---

## Phase 5: Polish and Integration (~1,500 new lines)

### 5.1 Boot Sequence Integration

**Subtasks:**
5.1.1 Update `kernel_init()` to initialize new subsystems in order:
  1. UART + Console (existing)
  2. GIC + Timer + IRQs (existing)
  3. Memory: MMU, frame alloc, heap (existing)
  4. DTB: parse WiFi node, MAC address, SDIO2 info
  5. **SDIO controller init**
  6. **WiFi chip init + firmware load**
  7. **WPA2 connect (if config exists on SD card)**
  8. GENET Ethernet init (existing)
  9. DHCP on active interface(s)
  10. USB + HID init (existing)
  11. Framebuffer + display init (existing)
  12. **Window manager + desktop init**
  13. **Terminal window creation**
  14. **Shell launch in terminal**
  15. Scheduler main loop (existing)

5.1.2 WiFi auto-connect:
  - Read `/boot/wifi.conf` from FAT32 (SSID + password)
  - If present, auto-connect during boot
  - If absent, start in offline mode (Ethernet only)

### 5.2 Shell Enhancements

**Subtasks:**
5.2.1 New shell commands:
  - `wifi scan` / `wifi connect` / `wifi disconnect` / `wifi status`
  - `ifconfig` — show all network interfaces
  - `browser <url>` — launch browser window
  - `edit <file>` — launch text editor
  - `sysmon` — launch system monitor
  - `screenshot` — dump framebuffer to FAT32 file

5.2.2 Shell dual-mode:
  - Serial mode (UART) — existing, for headless debug
  - GUI mode (terminal window) — new, for desktop use
  - Both run simultaneously, share state

### 5.3 Performance Optimization

**Subtasks:**
5.3.1 Zero-copy networking where possible
5.3.2 Compositor dirty-rect optimization (already structured)
5.3.3 Font glyph caching (render once, blit many)
5.3.4 Browser rendering: only re-layout on resize/viewport change, not on scroll
5.3.5 WiFi DMA buffer pooling (avoid per-packet allocation)

### 5.4 Error Handling and Robustness

**Subtasks:**
5.4.1 WiFi reconnect on disconnect
5.4.2 TCP retransmit timeout handling
5.4.3 Browser graceful degradation on parse errors
5.4.4 OOM handling in compositor and browser

---

## Implementation Order (priority)

The phases should be implemented in this order because each depends on the previous:

1. **Phase 1.1** — SDIO controller (foundation for WiFi)
2. **Phase 1.2** — BCM43455 driver (uses SDIO)
3. **Phase 1.3** — Network interface abstraction (decouples net stack)
4. **Phase 1.4** — WPA2 supplicant (enables secure WiFi)
5. **Phase 1.5** — DHCP over WiFi (auto IP config)
6. **Phase 2.1-2.3** — Terminal emulator completion (GUI shell foundation)
7. **Phase 2.4** — Keyboard→Shell pipeline (interactive GUI shell)
8. **Phase 3.1** — Desktop environment (usable desktop)
9. **Phase 4.1** — HTTP client (browser foundation)
10. **Phase 4.2-4.5** — HTML/CSS/layout/renderer (browser core)
11. **Phase 4.6** — Browser shell (usable browser)
12. **Phase 3.2-3.3** — System monitor + text editor (bonus apps)
13. **Phase 4.7** — Image decoder (rich browser content)
14. **Phase 4.1.2** — TLS client (HTTPS support)
15. **Phase 5** — Polish and integration

---

## File Structure (new files to create)

```
src/
├── drivers/
│   ├── sdio.rs          (Phase 1.1 — SDIO host controller, ~800 lines)
│   └── wifi.rs          (Phase 1.2 — BCM43455 driver, ~2500 lines)
├── net/
│   ├── iface.rs         (Phase 1.3 — Network interface trait, ~200 lines)
│   ├── wpa.rs           (Phase 1.4 — WPA2 supplicant, ~2500 lines)
│   ├── http.rs          (Phase 4.1 — HTTP client, ~600 lines)
│   ├── url.rs           (Phase 4.1 — URL parser, ~200 lines)
│   └── tls.rs           (Phase 4.1 — TLS 1.3 client, ~2500 lines, LATER)
├── terminal/
│   ├── mod.rs           (Phase 2 — update existing, ~150→300 lines)
│   ├── parser.rs        (Phase 2.1 — update existing, ~200→600 lines)
│   ├── screen.rs        (Phase 2.2 — update existing, ~150→500 lines)
│   └── render.rs        (Phase 2.3 — NEW, ~400 lines)
├── wm/
│   ├── desktop.rs       (Phase 3.1 — NEW, ~500 lines)
│   └── (existing files updated)
├── browser/
│   ├── mod.rs           (Phase 4.6 — browser shell, ~300 lines)
│   ├── html.rs          (Phase 4.2 — HTML parser, ~800 lines)
│   ├── css.rs           (Phase 4.3 — CSS parser, ~500 lines)
│   ├── layout.rs        (Phase 4.4 — layout engine, ~700 lines)
│   ├── render.rs        (Phase 4.5 — renderer, ~500 lines)
│   └── image.rs         (Phase 4.7 — image decoder, ~700 lines)
├── apps/
│   ├── mod.rs           (Phase 3 — app registry, ~100 lines)
│   ├── sysmon.rs        (Phase 3.2 — system monitor, ~400 lines)
│   └── editor.rs        (Phase 3.3 — text editor, ~500 lines)
└── crypto/
    ├── mod.rs           (Phase 1.4 — crypto primitives, ~50 lines)
    ├── hmac_sha1.rs     (Phase 1.4 — HMAC-SHA1, ~150 lines)
    ├── aes.rs           (Phase 1.4 — AES-128, ~300 lines)
    ├── sha256.rs        (Phase 1.4/4.1 — SHA-256, ~200 lines)
    ├── pbkdf2.rs        (Phase 1.4 — PBKDF2, ~100 lines)
    └── chacha20.rs      (Phase 4.1 — for TLS, LATER)
```

---

## Estimated Total New Code

| Component | Lines |
|-----------|-------|
| SDIO controller | 800 |
| BCM43455 WiFi driver | 2,500 |
| Network interface abstraction | 200 |
| WPA2 supplicant + crypto | 3,000 |
| Terminal emulator (completion) | 1,000 |
| Desktop environment | 500 |
| HTTP client + URL parser | 800 |
| HTML parser | 800 |
| CSS parser | 500 |
| Layout engine | 700 |
| Browser renderer | 500 |
| Browser shell | 300 |
| Image decoder | 700 |
| TLS 1.3 client (deferred) | 2,500 |
| Apps (sysmon, editor) | 1,000 |
| Integration + polish | 1,500 |
| **Total new code** | **~16,300** |
| **Projected total** | **~44,600** |

---

## Key Technical Decisions

1. **WiFi firmware is loaded from the SD card FAT32 partition** — the same partition
   that holds kernel8.img. The firmware files (`brcmfmac43455-sdio.bin`, `.txt`, `.clm_blob`)
   must be placed in `/boot/firmware/` on the SD card.

2. **SDIO2 controller base address on Pi 5:** `0x10_0110_0000` (physical), mapped via
   our device map at `DEVICE_MAP_START + 0x1100000`. This is confirmed by Circle's
   `VIRTIO+0x1100000` and the Linux DTB node `/axi/mmc@1100000`.

3. **GIC SPI 274** is the SDIO2 interrupt. Register with our GIC driver.

4. **GPIO/Pinmux on Pi 5** uses the RP1 pin controller (PINCTRL1_BASE), not the
   legacy BCM2835 GPIO. Must handle SoC stepping (pre-D0 vs D0 register layout).
   WiFi pins: function 7 (Func7) for SDIO2, pull-ups on CMD+DATA.

5. **WPA2 crypto is implemented from scratch** — no external crate. We need
   HMAC-SHA1, AES-128-ECB, AES-128-CCMP, and PBKDF2. These are ~1,500 lines total
   and well-understood algorithms.

6. **Browser is built from scratch** — no existing engine can run in #![no_std].
   We target a "lynx+" experience: full HTML parsing, basic CSS, block+inline layout,
   bitmap font rendering. No JavaScript initially (would be 10K+ lines for a minimal
   engine). This is sufficient for Wikipedia, text-heavy sites, simple dashboards.

7. **TLS is deferred to last** — HTTP-only browser is useful immediately for
   local network services, Wikipedia via HTTP, etc. TLS 1.3 is ~2,500 lines of
   complex crypto code and can be added later without architecture changes.

8. **No JavaScript engine** — far too complex for a bare-metal OS at this stage.
   A future `js.rs` could add a simple interpreter, but it's out of scope.

9. **WiFi MAC address** comes from the DTB node `/axi/mmc@1100000/wifi@1`
   property `local-mac-address` (6 bytes). Our DTB parser needs a `find_node()`
   and `find_property()` function.

10. **Dual shell mode** — serial (UART) and GUI (terminal window) run simultaneously.
    The shell reads from whichever input source has data. Output goes to both.
