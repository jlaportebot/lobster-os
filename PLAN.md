# 🦞 LobsterOS — Full Build Plan

**A bare-metal operating system written in Rust, optimized for the Raspberry Pi 5 (BCM2712, ARM Cortex-A76)**

---

## 1. Vision

LobsterOS is a general-purpose operating system built from scratch in Rust for the Raspberry Pi 5. It targets full desktop-class capability: multitasking, virtual memory, filesystem, networking, display output, userspace programs, and a shell. The OS will leverage the BCM2712's quad-core Cortex-A76 at 2.4GHz, 2MB shared L3 cache, and the RP1 I/O controller via PCIe.

**Goal:** Boot on real Pi 5 hardware, run user programs, connect to the internet, render graphics to HDMI, read/write files on SD card.

---

## 2. Target Hardware: Raspberry Pi 5 (BCM2712)

### 2.1 CPU Subsystem
- **SoC:** Broadcom BCM2712
- **Cores:** 4× ARM Cortex-A76 (ARMv8.2-A), up to 2.4 GHz
- **L1 Cache:** 64 KB I-cache + 64 KB D-cache per core (4-way set associative, 64-byte lines)
- **L2 Cache:** 512 KB per core (8-way, 64-byte lines)
- **L3 Cache:** 2 MB shared (16-way, 64-byte lines)
- **ISA Extensions:** AES, SHA1, SHA256, CRC32, FP16, dot product
- **Enable Method:** PSCI (ARM Power State Coordination Interface) via SMC calls

### 2.2 Memory Map (from Linux device tree `bcm2712.dtsi`)

The BCM2712 uses a 40-bit physical address space. Key regions:

| Region | Physical Address | Size | Notes |
|---|---|---|---|
| **RAM** | `0x0000_0000_0000` | Up to 8 GB | Starts at address 0, firmware places kernel here |
| **SoC Peripheral Bus** | `0x10_0000_0000` – `0x10_7FFF_FFFF` | 2 GB | `soc@107c000000` with range offset `0x10_0000_0000` |
| **GIC-400 (GICD)** | `0x10_7FFF_9000` | 4 KB | Distributor |
| **GIC-400 (GICC)** | `0x10_7FFFA000` | 8 KB | CPU Interface |
| **GIC-400 (GICH)** | `0x10_7FFFC000` | 8 KB | Virtual Interface Control |
| **GIC-400 (GICV)** | `0x10_7FFFE000` | 8 KB | Virtual CPU Interface |
| **System Timer** | `0x10_7C00_3000` | 4 KB | BCM2835-style, 1 MHz, SPIs 64-67 |
| **Mailbox** | `0x10_7C01_3880` | 64 B | BCM2835 MBOX, SPI 33 |
| **PL011 UART10** | `0x10_7D00_1000` | 512 B | SPI 121, clocks: uart=9.216MHz, apb=750MHz |
| **Power Mgmt / WDT** | `0x10_7D20_0000` | 1.5 KB | BCM2712 PM + watchdog |
| **BCM2712 GPIO** | `0x10_7D50_8500` | 64 B | 32+22 pin banks, SPI 244 via main_irq |
| **AUX UART (uarta)** | `0x10_7D50_C000` | 32 B | BCM7271 UART, SPI 276, 96MHz clock |
| **AON Pin Control** | `0x10_7D51_0700` | 32 B | Always-on domain |
| **RNG200** | `0x10_7D20_8000` | 40 B | Hardware RNG |
| **AON L2 INTC** | `0x10_7D51_0600` | 48 B | SPI 239 |
| **SDIO1 (SD card)** | `0x10_0FFF_0000` | 640+512 B | BCM2712 SDHCI, SPI 273 |
| **SDIO2** | `0x10_0110_0000` | 640+512 B | BCM2712 SDHCI, SPI 274 |
| **PCIe0 (RP1 link)** | `0x10_0010_0000` | ~37 KB | Gen 2 x1, SPIs 213/214, MSI SPIs 209-212 |
| **PCIe1** | `0x10_0011_0000` | ~37 KB | Gen 2 x1, SPIs 223/224 |
| **PCIe2 (external)** | `0x10_0012_0000` | ~37 KB | Gen 2 x4, SPIs 233/234 |
| **MIP0 (MSI)** | `0x10_0013_0000` | 192 B | MSI controller, SPIs 128-191 |
| **MIP1 (MSI)** | `0x10_0013_1000` | 192 B | MSI controller, SPIs 247-254 |
| **V3D GPU** | `0x10_0200_0000` | Various | VideoCore VII 3D, SPIs 249-250 |
| **HVS** | `0x10_7C58_0000` | 106 KB | Hardware Video Scaler |
| **PixelValve0** | `0x10_7C41_0000` | 256 B | SPI 101 |
| **PixelValve1** | `0x10_7C41_1000` | 256 B | SPI 110 |
| **HDMI0** | `0x10_7C70_1000` area | Multi-region | MOP + PHY + CSC + CEC |
| **HDMI1** | `0x10_7C70_6000` area | Multi-region | Same structure |
| **RP1 (via PCIe2)** | `0x1F_0000_0000` | 4 MB BAR | All RP1 peripherals (USB, ETH, GPIO, SPI, I2C, UART) |
| **RP1 SRAM** | `0x1F_0400_0000` | 64 KB | Maps to 0x2000_0000 on RP1 |

**Note:** All `soc` child addresses are relative to `0x10_0000_0000`. So `reg = <0x7fff9000 0x1000>` → physical `0x10_7FFF_9000`.

### 2.3 RP1 Southbridge (via PCIe2 BAR)

RP1 is the I/O controller chip on Pi 5, connected via PCIe Gen 2 x4. It provides:
- **USB:** 2× XHCI controllers (USB 3.0 + USB 2.0 per controller)
- **Ethernet:** Cadence GEM Gigabit MAC (RGMII)
- **GPIO:** 28× 5V-tolerant GPIO pins
- **UART:** Multiple PL011 UARTs
- **SPI:** Multiple SPI interfaces
- **I2C:** Multiple I2C interfaces
- **PWM:** Pulse-width modulation
- **DPI:** Parallel display output

The RP1 BAR is mapped at CPU physical `0x1F_0000_0000`. Peripheral register offsets are documented in the RP1 Peripherals Datasheet (88 pages, publicly available from Raspberry Pi).

### 2.4 ARM Generic Timer
- PPI 13: EL1 Physical Timer (used for scheduling tick)
- PPI 14: EL1 Virtual Timer
- PPI 11: EL2 Physical Timer
- PPI 10: EL2 Virtual Timer
- PPI 12: EL0 Physical Timer (userspace)
- All level-triggered, per-CPU

### 2.5 Boot Process

The Pi 5 boot chain:
1. **SPI Flash EEPROM** — VPU firmware (replaces start.elf from Pi 4)
2. **VPU firmware** — initializes DDR, PCIe, HDMI, loads kernel from SD
3. **config.txt** — parsed by firmware (on FAT32 boot partition)
4. **TF-A (BL31)** — optionally loaded if `armstub=bl31.bin` in config.txt. Provides PSCI, drops to EL2
5. **kernel8.img** or **kernel_2712.img** — our kernel, loaded at `0x0_80000` (typical)
6. **Device Tree Blob** — passed in register x0 (or x1/x2 depending on boot flow)

**Recommended Boot Strategy:** Use the Pi 5 firmware + TF-A BL31 for initial bringup. This gives us:
- PSCI for secondary core management (SMC calls)
- EL2 → EL1 drop with proper setup
- Device tree pointer for hardware discovery
- We focus on kernel code, not bootloader plumbing

**SD Card Layout:**
- FAT32 partition with: `config.txt`, `kernel8.img`, `bcm2712-rpi-5-b.dtb`, optionally `bl31.bin`

### 2.6 GIC-400 (GICv2) Interrupt Controller
- **Distributor (GICD):** `0x10_7FFF_9000` — manages SPI routing, enable/priority
- **CPU Interface (GICC):** `0x10_7FFFA000` — per-core interrupt ack/eoi
- **Hypervisor (GICH):** `0x10_7FFFC000` — virtual interrupt control
- **Virtual CPU (GICV):** `0x10_7FFFE000` — virtual CPU interface
- SPIs: 0-31 = SGI (software generated), 32-63 = PPI (private per-CPU), 64+ = SPI (shared)
- Key SPIs: 33=mailbox, 64-67=system timer, 121=UART10, 273=SDIO1, 276=AUX UART, 213-214=PCIe0

---

## 3. Architecture Design

### 3.1 Kernel Architecture: Monolithic with Module Interface

We'll use a **monolithic kernel** design (like Linux) rather than microkernel (like Redox). Rationale:
- Simpler to implement — no IPC overhead for driver communication
- Better performance on Cortex-A76 — no context switches for driver calls
- More existing reference code (Linux, Circle, rust-raspberrypi-OS-tutorials)
- Modules can be loaded dynamically later

### 3.2 Kernel Subsystems

```
┌─────────────────────────────────────────────────┐
│                  Userspace (EL0)                │
│  Shell │ Apps │ Libraries (libc, libm, libtls)  │
├─────────────────────────────────────────────────┤
│              Syscall Interface (SVC)            │
├─────────────────────────────────────────────────┤
│               Kernel (EL1)                      │
│  ┌──────────┐ ┌──────────┐ ┌─────────────────┐ │
│  │ Scheduler│ │ VM Manager│ │ Process Manager │ │
│  └──────────┘ └──────────┘ └─────────────────┘ │
│  ┌──────────┐ ┌──────────┐ ┌─────────────────┐ │
│  │   VFS    │ │  Network │ │  Device Manager  │ │
│  └──────────┘ └──────────┘ └─────────────────┘ │
│  ┌──────────────────────────────────────────┐   │
│  │            Driver Layer                   │   │
│  │ UART │ SDHCI │ XHCI │ GEM │ FB │ GPIO   │   │
│  └──────────────────────────────────────────┘   │
│  ┌──────────────────────────────────────────┐   │
│  │          Hardware Abstraction             │   │
│  │ GIC │ MMU │ Timer │ DMA │ PCIe │ Mailbox │   │
│  └──────────────────────────────────────────┘   │
├─────────────────────────────────────────────────┤
│              Hardware (BCM2712 + RP1)           │
└─────────────────────────────────────────────────┘
```

### 3.3 Address Space Layout

```
Virtual Address Space (48-bit, split at TTBR0/TTBR1):
┌─────────────────────────────────────────┐ 0xFFFF_FFFF_FFFF_FFFF
│  Kernel Space (TTBR1)                   │
│  0xFFFF_0000_0000_0000+                 │
│  ├── Kernel image (identity-mapped)     │
│  ├── Kernel heap (bump → slab)          │
│  ├── Device MMIO mappings               │
│  ├── Page tables (recursive mapping)    │
│  └── Kernel stack per core (4 pages)    │
├─────────────────────────────────────────┤ 0x0000_FFFF_FFFF_FFFF
│  Userspace (TTBR0)                      │
│  0x0000_0000_0000+                      │
│  ├── Code segment (.text)               │
│  ├── Data segment (.data, .bss)         │
│  ├── Heap (brk/mmap)                    │
│  ├── Stack (grows down)                 │
│  └── Memory-mapped files                │
└─────────────────────────────────────────┘ 0x0000_0000_0000_0000
```

Page size: 4 KB (standard AArch64). 4-level page tables: PGD → PUD → PMD → PTE.

---

## 4. Build Phases

### Phase 1: Foundation (Boot + UART + GIC + Timer)
**Goal:** Boot on Pi 5, print to serial console, handle timer interrupts.

### Phase 2: Memory Management (MMU + Allocator + VM)
**Goal:** Virtual memory, kernel heap, page frame allocator.

### Phase 3: Multiprocessing (SMP + Scheduler)
**Goal:** 4 cores running, preemptive round-robin scheduler, kernel threads.

### Phase 4: Process Management + Userspace
**Goal:** EL0 process execution, syscall interface, context switching.

### Phase 5: Storage (SD Card + FAT32 Filesystem)
**Goal:** Read/write files on SD card, VFS layer.

### Phase 6: Display (Framebuffer + Console + Basic GUI)
**Goal:** Text console on HDMI, framebuffer graphics, keyboard input.

### Phase 7: Networking (Ethernet + TCP/IP)
**Goal:** Gigabit Ethernet via RP1, TCP/IP stack, socket API.

### Phase 8: USB + Input Devices
**Goal:** USB keyboard/mouse via XHCI, USB mass storage.

### Phase 9: Userspace Ecosystem
**Goal:** Shell, basic utilities, program loader, shared libraries.

### Phase 10: Advanced Features
**Goal:** Signals, pipes, advanced scheduling, performance optimization.

---

## 5. Detailed Phase Breakdown

### Phase 1: Foundation — Boot + UART + GIC + Timer

#### 1.1 Project Setup
- [ ] Create Cargo workspace with `kernel`, `boot`, `shared` crates
- [ ] Custom target JSON: `aarch64-lobsteros.json`
  ```json
  {
    "llvm-target": "aarch64-unknown-none",
    "data-layout": "e-m:e-i8:8:32-i16:16:32-i64:64-i128:128-n32:64-S128",
    "arch": "aarch64",
    "os": "none",
    "linker-flavor": "ld.lld",
    "linker": "rust-lld",
    "executables": true,
    "disable-redzone": true,
    "features": "+a76,+aes,+crc,+sha2,+sha3",
    "relocation-model": "static",
    "pre-link-ldflags": ["-Tlinker.ld"],
    "panic-strategy": "abort"
  }
  ```
- [ ] Linker script (`linker.ld`) defining memory layout:
  - Kernel loaded at `0x0_80000` (standard Pi 5 load address)
  - Sections: .text, .rodata, .data, .bss
  - 4KB alignment for all sections
  - Stack per core at end of kernel image
- [ ] `.cargo/config.toml` with build target and flags
- [ ] `#![no_std]`, `#![no_main]` entry point
- [ ] Custom panic handler
- [ ] Build script: `cargo build --target aarch64-lobsteros`, objcopy to raw binary, rename to `kernel8.img`

#### 1.2 Boot Assembly (`boot/src/aarch64.rs` or `.S`)
The Pi 5 firmware (with TF-A) drops us at EL2 (or EL1 if using armstub). We need:
1. **Check EL:** Read `CurrentEL`, decide if we need to drop from EL2 → EL1
2. **If EL2:** Configure `HCR_EL2` (disable stage-2 MMU, set EL1 to AArch64), set `SCR_EL3` if needed, `ERET` to EL1
3. **EL1 entry:**
   - Zero BSS section
   - Set up stack pointer (per-core, core 0 first)
   - Enable FP/SIMD: set `CPACR_EL1.FPEN = 0b11`
   - Invalidate caches: `ic IALLUIS`, `dc IVAC`
   - Call `kernel_init(dtb_ptr)` — pass x0 from firmware
4. **Secondary cores:** Use PSCI or spin-table. With TF-A, use `psci_cpu_on` SMC call from core 0 to wake cores 1-3. Each secondary core enters a spin loop until the kernel signals readiness, then jumps to `secondary_entry(core_id)`.

#### 1.3 UART Driver (PL011)
- Base: `0x10_7D00_1000` (UART10)
- Clock: UARTCLK = 9.216 MHz, APB = 750 MHz
- GIC SPI: 121
- Implementation:
  - Initialize: set baud rate (115200), configure line control (8N1), enable FIFOs
  - `uart_putc()`, `uart_getc()`, `uart_puts()` — polling mode first
  - Later: interrupt-driven RX with ring buffer
- Reference: ARM PL011 TRM, Circle's PL011 driver

#### 1.4 GIC-400 Driver
- **GICD (Distributor) at `0x10_7FFF_9000`:**
  - `GICD_CTLR` (0x000): enable distributor
  - `GICD_IGROUPRn` (0x080): group 0/1 (use group 1 for IRQs)
  - `GICD_ISENABLERn` (0x100): enable specific SPIs
  - `GICD_ICENABLERn` (0x180): disable SPIs
  - `GICD_IPRIORITYRn` (0x400): set priority (8-bit per IRQ, lower = higher priority)
  - `GICD_ITARGETSRn` (0x800): set target cores (8-bit per IRQ, bit per core)
  - `GICD_SGIR` (0xF00): generate SGI
- **GICC (CPU Interface) at `0x10_7FFFA000`:**
  - `GICC_CTLR` (0x000): enable CPU interface
  - `GICC_PMR` (0x004): priority mask
  - `GICC_BPR` (0x008): binary point register
  - `GICC_IAR` (0x00C): interrupt acknowledge (read to get IRQ ID)
  - `GICC_EOIR` (0x010): end of interrupt (write IRQ ID)
- Initialize both GICD and GICC on core 0, then GICC only on secondary cores
- Default: route all SPIs to core 0, timer PPIs are per-core automatically

#### 1.5 ARM Generic Timer
- Configure `CNTFRQ_EL0` (timer frequency, read from DTB or hardcode 54MHz oscillator / divisor)
- Set up EL1 Physical Timer (PPI 13):
  - `CNTP_TVAL_EL0`: set compare value (e.g., 1ms tick = freq/1000)
  - `CNTP_CTL_EL0`: enable timer, unmask interrupt
- Timer ISR: acknowledge GIC, update compare, call scheduler tick
- Use `CNTPCT_EL0` for high-resolution timestamps

#### 1.6 Exception Vector Table
AArch64 has 16 exception vectors (4 exception types × 4 source ELs):
```
VBAR_EL1 points to:
  +0x000: Synchronous EL1t
  +0x080: IRQ EL1t
  +0x100: FIQ EL1t
  +0x180: SError EL1t
  +0x200: Synchronous EL1h    ← main kernel exception handler
  +0x280: IRQ EL1h            ← timer/device interrupt handler
  +0x300: FIQ EL1h
  +0x380: SError EL1h
  +0x400-0x700: EL0 exceptions (for userspace later)
```
Each entry: save registers, branch to Rust handler, restore registers, `ERET`.

#### 1.7 Device Tree Parser (minimal)
- Parse DTB passed in x0 at boot to discover:
  - RAM size and location
  - Peripheral base addresses (cross-check with hardcoded)
  - Boot arguments
- Minimal FDT parser: walk structure block, read `/memory`, `/chosen`

**Deliverable:** Pi 5 boots, "LobsterOS v0.1" printed on serial, timer ticks printing, all 4 cores responsive.

---

### Phase 2: Memory Management

#### 2.1 Page Frame Allocator
- Physical memory: bitmap or buddy allocator
  - Start with simple bitmap (1 bit per 4KB page)
  - Evolve to buddy allocator for power-of-2 allocations
- Reserve regions: kernel image, DTB, GIC, MMIO, framebuffer
- `pfn_alloc(order)`, `pfn_free(pfn, order)`

#### 2.2 MMU Setup (4-level page tables)
- **MAIR_EL1:** Configure memory attributes
  - Attr0: Normal, Inner/Outer WB RA WA (0xFF) — RAM
  - Attr1: Device-nGnRnE (0x04) — MMIO
  - Attr2: Normal, Inner/Outer Non-Cacheable (0x44) — DMA buffers
- **TCR_EL1:** Configure translation control
  - `T0SZ=16` (TTBR0: 48-bit, 256TB user space)
  - `T1SZ=16` (TTBR1: 48-bit, 256TB kernel space)
  - `IRGN0=IRGN1=1` (inner write-back)
  - `ORGN0=ORGN1=1` (outer write-back)
  - `SH0=SH1=3` (inner shareable)
  - `TG0=0` (4KB pages at TTBR0)
  - `TG1=2` (4KB pages at TTBR1)
  - `IPS=2` (40-bit intermediate physical address for Pi 5)
- **TTBR0_EL1 / TTBR1_EL1:** Set page table roots
- **SCTLR_EL1:** Enable MMU + caches
  - M=1 (MMU enable), C=1 (data cache), I=1 (instruction cache)

#### 2.3 Kernel Identity Mapping
- Identity-map all of physical RAM (kernel image + free pages)
- Map all MMIO regions as Device memory (nGnRnE)
- Map kernel as Normal WB RA WA

#### 2.4 Kernel Heap Allocator
- Start with bump allocator for early boot
- Replace with linked-list or slab allocator:
  - `locked_heap()` with `#[global_allocator]`
  - Sizes: 8, 16, 32, 64, 128, 256, 512, 1024, 2048 bytes
- Use `alloc` crate: `Vec`, `Box`, `String`, `BTreeMap`, `Arc`, `Rc`

#### 2.5 Virtual Memory Manager
- `vm_create_process()` — allocate new TTBR0 page table tree
- `vm_map(page_table, vaddr, paddr, flags)` — map page with attributes
- `vm_unmap(page_table, vaddr)` — remove mapping, free page
- Page flags: R, W, X, UX (user execute), PX (privileged execute)
- `vm_copy_on_write()` — for fork()

**Deliverable:** MMU enabled, kernel heap working, `Vec`/`String`/`Box` functional.

---

### Phase 3: Multiprocessing — SMP + Scheduler

#### 3.1 SMP Bring-up
- Core 0 uses PSCI `cpu_on` SMC to start cores 1-3
- Each core gets its own:
  - Stack (8KB, guard page below)
  - GICC initialization
  - Timer initialization
  - Exception vector (VBAR_EL1)
- Per-core data: `struct PerCpu { core_id, current_thread, kernel_stack, ... }`
- Access via `TPIDR_EL1` register (thread pointer)

#### 3.2 Spinlocks + Atomics
- `struct SpinLock<T>` — basic, no IRQ protection
- `struct IrqLock<T>` — disables IRQs on lock, restores on unlock (for interrupt safety)
- Use `core::sync::atomic::{AtomicU32, Ordering}`
- Ticket lock or simple TAS lock for fairness

#### 3.3 Kernel Threads
- `struct Thread { id, state, context, kernel_stack, process }`
- Thread states: Ready, Running, Blocked, Zombie
- Context: saved registers (x0-x30, sp, spsr, elr, tpdir)
- `thread_create(func, arg)` — allocate thread + stack, push initial context
- `thread_yield()` — voluntary context switch

#### 3.4 Preemptive Round-Robin Scheduler
- Per-core run queue (or global with locking)
- Timer tick (1ms) → check timeslice (10ms default) → preempt if expired
- Context switch:
  1. Save current thread's registers to its kernel stack
  2. Switch to next thread's kernel stack
  3. Restore next thread's registers
  4. `ERET` back to where the thread was
- Priority levels later (Phase 10)

#### 3.5 Synchronization Primitives
- `Mutex<T>` — uses scheduler blocking (not spin)
- `Semaphore` — counting semaphore
- `Condvar` — condition variable
- `Channel` — MPSC channel for message passing

**Deliverable:** 4 cores active, kernel threads running, preemptive scheduling working.

---

### Phase 4: Process Management + Userspace

#### 4.1 Process Abstraction
- `struct Process { pid, page_table_ttbr0, threads, fd_table, cwd, brk, exit_status }`
- PID allocator (simple counter)
- Process tree (parent/children)
- Process states: Created, Ready, Running, Blocked, Zombie, Exited

#### 4.2 EL0 Context Setup
To run userspace code:
1. Set `SPSR_EL1` = EL0t (0x00) — targets EL0 with IRQs unmasked
2. Set `ELR_EL1` = entry point of user program
3. Set `SP_EL0` = top of user stack
4. Set `TTBR0_EL1` = process page table
5. Flush TLB: `TLBI VMALLE1IS`
6. `ERET` — drops to EL0, starts executing user code

#### 4.3 Syscall Interface (SVC)
- Userspace invokes `svc #N` where N is syscall number
- Kernel exception handler at `VBAR_EL1 + 0x400` (sync EL0 → EL1)
- Read `ESR_EL1` to decode SVC number
- Arguments in registers: x0-x5 (6 args max)
- Return value in x0
- **Syscalls:**
  - `sys_exit(code)` — terminate process
  - `sys_write(fd, buf, len)` — write to fd
  - `sys_read(fd, buf, len)` — read from fd
  - `sys_open(path, flags)` — open file
  - `sys_close(fd)` — close fd
  - `sys_fork()` — create child process (COW)
  - `sys_exec(path, argv)` — replace process image
  - `sys_waitpid(pid)` — wait for child
  - `sys_getpid()` — get PID
  - `sys_brk(addr)` — set program break
  - `sys_mmap(addr, len, prot, flags)` — map memory
  - `sys_munmap(addr, len)` — unmap memory
  - `sys_nanosleep(ts)` — sleep
  - `sys_socket(domain, type, proto)` — create socket
  - `sys_connect(fd, addr, len)` — connect socket
  - `sys_bind(fd, addr, len)` — bind socket
  - `sys_listen(fd, backlog)` — listen
  - `sys_accept(fd, addr, len)` — accept connection
  - `sys_send/recv` — send/recv data
  - `sys_ioctl(fd, cmd, arg)` — device control
  - `sys_pipe(fds)` — create pipe
  - `sys_dup/dup2` — duplicate fd
  - `sys_chdir` — change directory
  - `sys_getcwd` — get working directory
  - `sys_stat/fstat` — file metadata
  - `sys_mkdir` — create directory
  - `sys_unlink` — delete file
  - `sys_kill` — send signal
  - `sys_signal` — set signal handler

#### 4.4 Program Loader (ELF)
- Parse ELF64 headers (little-endian, AArch64)
- Load PT_LOAD segments into process address space
- Set up user stack (map pages, push argv/envp)
- Set up brk heap area
- Zero-fill .bss
- Jump to ELF entry point

#### 4.5 Fork + Copy-on-Write
- Clone process page table with COW marking
- Mark all writable pages as read-only in both parent and child
- On page fault (write to read-only COW page):
  - Allocate new physical page
  - Copy content
  - Map as writable in faulting process
  - Resume execution

#### 4.6 Page Fault Handler
- Read `FAR_EL1` (fault address), `ESR_EL1` (fault type)
- Cases:
  - COW write fault → copy page, remap writable
  - Stack growth → allocate new page, map
  - Demand paging → read from filesystem (later)
  - Invalid access → SIGSEGV, kill process

**Deliverable:** User programs execute in EL0, make syscalls, fork works.

---

### Phase 5: Storage — SD Card + Filesystem

#### 5.1 SDHCI Driver (BCM2712 SDIO1)
- Base: `0x10_0FFF_0000` (host), `0x10_0FFF_0400` (cfg)
- SPI: 273, Clock: 200 MHz
- Command sequence: CMD0 (reset), CMD8 (voltage check), ACMD41 (init), CMD2 (CID), CMD3 (RCA), CMD9 (CSD), CMD7 (select), CMD17/18 (read), CMD24/25 (write)
- Support SDHC (block size 512 bytes, up to 2TB)
- DMA transfers using BCM2712 DMA controller for performance
- Start with PIO (polled I/O), add DMA later

#### 5.2 Block Layer
- `trait BlockDevice { fn read_block(&self, lba, buf); fn write_block(&self, lba, buf); fn block_count(&self) -> u64; }`
- Partition table parsing (MBR first, GPT later)

#### 5.3 FAT32 Filesystem
- Use `embedded-sdmmc-rs` crate as reference, or implement from scratch
- FAT32 structures: BPB, FSInfo, FAT table, directory entries (LFN support)
- Operations: mount, open, read, write, create, delete, mkdir, seek
- LRU cache for FAT table entries and directory clusters
- Write support with journaling or simple commit

#### 5.4 Virtual Filesystem (VFS)
- `struct Inode { mode, size, nlinks, blocks, atime, mtime, ctime, fs_ops }`
- `trait FileSystem { fn mount(); fn lookup(); fn read(); fn write(); ... }`
- Mount table: `/` → SD card FAT32, `/dev` → devfs, `/proc` → procfs, `/tmp` → tmpfs
- File descriptor table per process (0=stdin, 1=stdout, 2=stderr)
- Path resolution with dentry cache

#### 5.5 devfs + procfs
- **devfs:** `/dev/uart0`, `/dev/fb0`, `/dev/sda`, `/dev/null`, `/dev/zero`, `/dev/random`
- **procfs:** `/proc/self`, `/proc/cpuinfo`, `/proc/meminfo`, `/proc/uptime`

**Deliverable:** Read/write files on SD card, directory listing, process file descriptors work.

---

### Phase 6: Display — Framebuffer + Console

#### 6.1 Framebuffer via Mailbox
- Use the BCM2835 mailbox property interface at `0x10_7C01_3880`
- Request framebuffer from VideoCore firmware:
  - Tag: `SET_PHYSICAL_SIZE` (e.g., 1920×1080)
  - Tag: `SET_VIRTUAL_SIZE` (same or double for scrolling)
  - Tag: `SET_DEPTH` (32-bit ARGB)
  - Tag: `ALLOCATE_BUFFER` → returns FB base address and size
- The firmware returns a bus address; translate to ARM physical address
- Map framebuffer as Normal Non-Cacheable (for DMA coherency) or WB with cache flush

#### 6.2 2D Graphics Primitives
- Pixel drawing: `fb_putpixel(x, y, color)`
- Line: Bresenham's algorithm
- Rectangle fill
- Bitmap font rendering (8×16 PC font or embedded TTF rasterizer)
- Double buffering (optional, for smooth rendering)

#### 6.3 Text Console
- VT100-compatible terminal emulator:
  - Cursor positioning (CSI H)
  - Color support (CSI m)
  - Scroll region
  - Line wrap
  - Backspace, delete, tab
- Connect to shell process stdin/stdout

#### 6.4 HDMI Driver (Advanced — Phase 6b)
- Direct HDMI programming via BCM2712 display pipeline:
  - MOP (Master Output Pipeline) at `0x10_7C50_0000`
  - HVS (Hardware Video Scaler) at `0x10_7C58_0000`
  - PixelValve at `0x10_7C41_0000` / `0x10_7C41_1000`
  - HDMI0 PHY/CSC at `0x10_7C70_1000` area
- This is optional if mailbox framebuffer works well enough

#### 6.5 Keyboard Input
- GPIO-based matrix scan (if directly connected keyboard)
- Or USB keyboard (requires Phase 8 XHCI driver)
- Start with serial console input (UART) for development

**Deliverable:** Text appears on HDMI monitor, console with VT100 support.

---

### Phase 7: Networking

#### 7.1 PCIe Driver
- Initialize BCM2712 PCIe0 controller at `0x10_0010_0000`
- Configure RC (root complex), enable link, wait for link training
- Scan bus 0 for RP1 device (vendor ID: Raspberry Pi Ltd)
- Read BAR0 (memory-mapped, 64-bit) → maps to `0x1F_0000_0000`
- Map RP1 BAR into kernel virtual address space
- Enable MSI via MIP0 controller at `0x10_0013_0000`

#### 7.2 RP1 Ethernet Driver (Cadence GEM)
- Base: offset within RP1 BAR (see RP1 datasheet)
- Cadence GEM (Gigabit Ethernet MAC) with DMA
- Initialize: configure clock, RGMII, DMA descriptors
- RX/TX descriptor rings (DMA-coherent memory)
- Interrupt handler for RX done / TX done
- Hardware checksum offload

#### 7.3 Network Stack (smoltcp)
- Use `smoltcp` crate (no_std, event-driven, bare-metal compatible)
- Features: Ethernet, ARP, IPv4, ICMP, TCP, UDP, DHCP
- Wire up: GEM driver provides `Device` trait impl for smoltcp
- DHCP client for automatic IP configuration
- Socket API for userspace (see syscall list)

#### 7.4 Loopback Interface
- 127.0.0.1 for local testing

#### 7.5 DNS Resolver
- Basic UDP DNS resolver (query → parse A record)
- Cache results

**Deliverable:** Pi 5 gets IP via DHCP, can ping, open TCP connections.

---

### Phase 8: USB + Input

#### 8.1 XHCI Driver (USB 3.0 via RP1)
- RP1 contains 2 XHCI controllers
- XHCI is complex but well-documented:
  - Command Ring, Event Ring, Transfer Rings
  - Doorbell registers
  - Context structures (Slot, Endpoint, Stream)
- Implementation steps:
  1. Reset controller, await ready
  2. Allocate command/event rings (DMA-coherent)
  3. Enable interrupts
  4. Enumerate root hub ports
  5. Reset and configure each device
  6. Set up HID driver for keyboard/mouse
  7. Set up MSC (Mass Storage Class) driver for USB drives

#### 8.2 HID Driver
- USB HID boot protocol for keyboard and mouse
- Keyboard: scancode → keycode mapping, modifier keys, LED control
- Mouse: movement deltas, button events

#### 8.3 USB Mass Storage
- SCSI commands over Bulk-Only Transport
- Expose as block device → mount filesystem

**Deliverable:** USB keyboard/mouse working, USB drives mountable.

---

### Phase 9: Userspace Ecosystem

#### 9.1 Shell (`lash` — Lobster Shell)
- Read-eval-print loop with line editing
- Command execution: fork + exec
- Pipes: `cmd1 | cmd2`
- Redirection: `>`, `<`, `>>`
- Builtins: `cd`, `exit`, `pwd`, `export`, `echo`
- PATH search
- Job control (bg, fg, &)

#### 9.2 Core Utilities
- `ls` — directory listing
- `cat` — file content display
- `cp` — copy files
- `mv` — move/rename
- `rm` — delete files
- `mkdir` — create directory
- `echo` — print text
- `head/tail` — first/last lines
- `wc` — word count
- `grep` — simple pattern search
- `sleep` — delay
- `ps` — process listing
- `kill` — send signal
- `mount/umount` — filesystem mount
- `ping` — ICMP ping
- `curl` / `wget` — HTTP client (simple GET)
- `clear` — clear screen

#### 9.3 C Library (liblobster)
- Minimal libc for userspace programs
- Syscall wrappers
- String functions: memcpy, strlen, strcpy, strcmp, ...
- stdio: printf, scanf, fopen, fclose, fread, fwrite
- malloc/free (sbrk-based)
- File operations, socket operations
- Signal handling

#### 9.4 Program Binary Format
- Static ELF64 for AArch64 (initially)
- Dynamic linking later (ELF with PT_INTERP)

**Deliverable:** Full interactive shell, can run programs, pipe data.

---

### Phase 10: Advanced Features

- **Signals:** SIGINT, SIGTERM, SIGKILL, SIGSEGV, SIGCHLD, signal handlers
- **Pipes:** `pipe()` syscall, kernel ring buffer
- **SMP Load Balancing:** migrate threads between cores
- **Priority Scheduling:** O(1) or CFS-like scheduler
- **swapon/swap:** use SD card or USB drive as swap space
- **Ext2/Ext4 filesystem** (read support for Linux partitions)
- **Framebuffer window system:** simple tiling window manager
- **Audio:** I2S driver via RP1, PCM playback
- **GPU acceleration:** V3D command submission (OpenGL ES)
- **Self-hosting:** build LobsterOS on LobsterOS

---

## 6. Key Rust Crates & Dependencies

| Crate | Purpose | Phase |
|---|---|---|
| `cortex-a` | ARM Cortex-A register access | 1 |
| `tock-registers` | Register abstraction macros | 1 |
| `aarch64-cpu` | AArch64 system register helpers | 1 |
| `spin` | Spin lock, mutex (no_std) | 3 |
| `linked_list_allocator` | Kernel heap | 2 |
| `smoltcp` | TCP/IP network stack | 7 |
| `embedded-sdmmc` | FAT32 filesystem (reference) | 5 |
| `bitflags` | Bit flag types | All |
| `log` | Logging facade (no_std) | 1 |
| `fdt` | Flattened Device Tree parser | 1 |
| `xhci` | XHCI types/structures (if available, or custom) | 8 |
| `uncased` | Case-insensitive string comparisons | 5 |
| `hashbrown` | HashMap (no_std) | 4 |

## 7. Development Tools

- **Cross-compiler:** `rustup target add aarch64-unknown-none`
- **QEMU:** `qemu-system-aarch64 -M raspi4b` (closest, not Pi 5 exact — use for early dev)
- **Flash tool:** Copy `kernel8.img` + `config.txt` + DTB to SD card
- **Serial console:** USB-TTL adapter on GPIO 14/15 (UART0 on 40-pin header)
- **Debugger:** OpenOCD + GDB via JTAG (optional, 3-pin debug port on Pi 5)
- **Objcopy:** `rust-objcopy -O binary target/.../kernel elf → kernel8.img`

## 8. File/Directory Structure

```
lobster-os/
├── Cargo.toml                  # Workspace root
├── Cargo.lock
├── .cargo/
│   └── config.toml             # Build config, target, linker
├── targets/
│   └── aarch64-lobsteros.json  # Custom target spec
├── linker.ld                   # Linker script
├── config.txt                  # Pi 5 boot config (for SD card)
├── boot/
│   └── src/
│       ├── aarch64_boot.S      # Assembly entry point
│       └── lib.rs
├── kernel/
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs             # kernel_init() entry
│       ├── arch/
│       │   └── aarch64/
│       │       ├── mod.rs
│       │       ├── cpu.rs      # EL transitions, system registers
│       │       ├── mmu.rs      # Page table ops, TLB
│       │       ├── exception.rs # Vector table, handlers
│       │       ├── context.rs  # Thread context save/restore
│       │       ├── gic.rs      # GIC-400 driver
│       │       └── smp.rs      # Secondary core bring-up
│       ├── drivers/
│       │   ├── mod.rs
│       │   ├── uart.rs         # PL011 UART10
│       │   ├── timer.rs        # ARM Generic Timer
│       │   ├── mailbox.rs      # BCM2835 mailbox
│       │   ├── sdhci.rs        # BCM2712 SD card
│       │   ├── gpio.rs         # BCM2712 + RP1 GPIO
│       │   ├── pcie.rs         # BCM2712 PCIe controller
│       │   ├── rp1.rs          # RP1 southbridge driver
│       │   ├── gem.rs          # Cadence GEM Ethernet
│       │   ├── xhci.rs         # USB 3.0 XHCI
│       │   ├── framebuffer.rs  # VideoCore framebuffer
│       │   ├── hdmi.rs         # HDMI output (advanced)
│       │   └── rng.rs          # Hardware RNG
│       ├── mm/
│       │   ├── mod.rs
│       │   ├── frame_alloc.rs  # Physical page allocator
│       │   ├── heap.rs         # Kernel heap allocator
│       │   ├── vmem.rs         # Virtual memory manager
│       │   └── page_table.rs   # AArch64 page table walks
│       ├── task/
│       │   ├── mod.rs
│       │   ├── scheduler.rs    # Round-robin scheduler
│       │   ├── thread.rs       # Thread struct + operations
│       │   ├── process.rs      # Process struct + operations
│       │   └── sync.rs         # Mutex, Semaphore, Condvar
│       ├── fs/
│       │   ├── mod.rs
│       │   ├── vfs.rs          # Virtual filesystem layer
│       │   ├── fat32.rs        # FAT32 implementation
│       │   ├── devfs.rs        # Device filesystem
│       │   ├── procfs.rs       # Process filesystem
│       │   └── elf.rs          # ELF64 program loader
│       ├── net/
│       │   ├── mod.rs
│       │   ├── ethernet.rs     # GEM glue layer
│       │   ├── socket.rs       # Socket abstraction
│       │   └── dns.rs          # DNS resolver
│       ├── sys/
│       │   ├── mod.rs
│       │   ├── syscall.rs      # Syscall dispatcher
│       │   └── signal.rs       # Signal delivery
│       └── lib.rs              # Re-exports
├── shared/
│   ├── Cargo.toml
│   └── src/
│       └── lib.rs              # Shared types, syscall numbers
├── userspace/
│   ├── lash/                   # Shell
│   ├── coreutils/              # ls, cat, cp, etc.
│   └── liblobster/             # C library
└── tools/
    ├── Makefile                # Build, flash, QEMU targets
    ├── flash.sh                # SD card flashing script
    └── qemu.sh                 # QEMU launch script
```

## 9. Key Technical Challenges & Mitigations

| Challenge | Risk | Mitigation |
|---|---|---|
| RP1 PCIe address translation | High — Pi 5 specific, poorly documented | Use Linux driver as reference, RP1 datasheet, Circle C++ code |
| XHCI driver complexity | High — 500+ page spec | Start with boot protocol only (keyboard/mouse), incrementally add features |
| HDMI direct programming | Very High — VideoCore VII is undocumented | Use mailbox framebuffer initially; direct HDMI is low priority |
| GIC-400 on Pi 5 | Medium — different base addresses from Pi 4 | Confirmed addresses from device tree, working Rust code in forum post |
| Userspace stability | Medium — page faults, COW, signal delivery | Extensive testing with simple programs, GDB debugging via JTAG |
| SD card DMA | Medium — BCM2712 DMA controller is different | Start with PIO, add DMA once basic SD works |
| Rust nightly breakage | Low-Medium | Pin nightly version, update deliberately |

## 10. Testing Strategy

1. **QEMU (Phase 1-3):** Use `qemu-system-aarch64 -M raspi4b` for early boot code testing. Note: QEMU doesn't emulate Pi 5 exactly, so transition to real hardware ASAP.
2. **Serial output (all phases):** UART10 is the primary debugging output.
3. **Real hardware (all phases):** SD card flash + serial console. Pi 5 has a 3-pin debug header.
4. **Unit tests:** Core logic (allocator, VFS, FAT32, ELF parser) can be tested on host with `#[cfg(test)]`.
5. **Integration tests:** Boot on Pi 5, verify output, run test programs.

## 11. Estimated Timeline

| Phase | Description | Estimated Duration |
|---|---|---|
| 1 | Boot + UART + GIC + Timer | 2-3 days |
| 2 | Memory Management | 3-4 days |
| 3 | SMP + Scheduler | 2-3 days |
| 4 | Process + Userspace | 4-5 days |
| 5 | SD Card + FAT32 + VFS | 4-5 days |
| 6 | Framebuffer + Console | 3-4 days |
| 7 | Networking | 4-5 days |
| 8 | USB + Input | 5-7 days |
| 9 | Userspace Ecosystem | 5-7 days |
| 10 | Advanced Features | Ongoing |
| **Total** | **Phases 1-9** | **32-47 days** |

This assumes focused, dedicated work with the Pi 5 hardware available. Debugging bare-metal issues can easily double these estimates.

---

## 12. What We Need

- **Raspberry Pi 5** (8GB model recommended) — ✅ Already have
- **MicroSD card** (16GB+) — for boot media and filesystem
- **USB-TTL Serial Adapter** (3.3V) — for UART debug console on GPIO 14/15
- **HDMI cable + monitor** — for display output testing
- **USB keyboard + mouse** — for input testing
- **Ethernet cable** — for networking testing
- **Rust nightly toolchain** — `rustup install nightly`
- **Cross-compilation setup** — `rustup target add aarch64-unknown-none`
- **ARM AArch64 documentation** — Architecture Reference Manual (ARM DDI 0487)
- **RP1 Peripherals Datasheet** — Available from Raspberry Pi (88 pages)
- **BCM2712 Device Tree** — From Linux kernel (already extracted above)

---

*This is a living document. Each phase will be tracked with git commits and tested incrementally. The OS grows from a blinking LED to a full system.*

🦞 **Let's build LobsterOS.**
