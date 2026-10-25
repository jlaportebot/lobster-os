<!-- SUPERSEDED 2026-10-25: see docs/DESKTOP_PLAN.md (living) and docs/AUDIT_2026-10-25.md (queue). Do not add work here. -->

# LobsterOS v0.2 → v0.5 Development Plan

## Testing Strategy
- QEMU `virt` machine for kernel logic tests (MMU, scheduler, syscalls, FS)
- Physical Pi 5 for hardware-specific tests (GENET, xHCI, framebuffer)
- Each phase ends with `cargo check` + QEMU smoke test where possible

## Phase 10: Process Isolation & Virtual Memory (per-process)
- Per-process page tables (TTBR0 per task)
- fork() syscall: duplicate address space COW
- exec() syscall: replace address space with ELF
- exit() syscall: tear down page tables, free frames
- Page fault handler: demand paging, COW, stack guard
- PID namespace + process credentials (uid/gid/euid)
- **~1500 LOC** across `memory/vmem.rs`, `sched/process.rs`, `arch/trap.rs`

## Phase 11: Signal Handling
- Signal struct: signo, handler, mask, pending
- sigaction() syscall: register handlers
- kill() syscall: send signal to process
- Signal delivery on return-from-exception
- SIGINT, SIGKILL, SIGSEGV, SIGTERM, SIGCHLD, SIGALRM
- **~400 LOC** in `sched/signal.rs`

## Phase 12: IPC — Pipes & Shared Memory
- Anonymous pipes: pipe() syscall, ring buffer
- Named pipes (FIFOs): mknod in VFS
- Shared memory: shm_open() + mmap()
- Message queues (optional)
- **~600 LOC** in `sched/ipc.rs`, `fs/pipefs.rs`

## Phase 13: SD/MMC Storage Driver
- BCM2712 SDHCI controller driver
- RP1 CM3 co-processor for SD card
- Block device abstraction (read/write sectors)
- DMA-based I/O for performance
- Integrate with VFS for persistent storage
- **~800 LOC** in `drivers/sdmmc.rs`, `drivers/block.rs`

## Phase 14: Security Model
- File permissions (mode_t: rwx for user/group/other)
- Process credentials (uid/gid/euid/suid)
- Capability system (CAP_NET_RAW, CAP_SYS_ADMIN, etc.)
- Permission checks in VFS operations
- **~500 LOC** in `sched/creds.rs`, `fs/perm.rs`

## Phase 15: Networking Extensions
- DNS client (recursive resolver, UDP-based)
- HTTP client (GET/POST over TCP)
- IPv6 addressing basics
- Socket options (SO_REUSEADDR, SO_REUSEPORT, TCP_NODELAY)
- DHCP lease renewal timer
- ARP cache eviction (LRU with timeout)
- **~1200 LOC** in `net/dns.rs`, `net/http.rs`, extensions

## Phase 16: Filesystem Completeness
- ext2 filesystem (simpler than ext4, good start)
- /dev device filesystem (devfs)
- /proc pseudo-filesystem (process info)
- /sys pseudo-filesystem (kernel params)
- VFS improvements: seek, fsync, fcntl, ioctl
- **~1500 LOC** in `fs/ext2.rs`, `fs/devfs.rs`, `fs/procfs.rs`

## Phase 17: SMP Improvements
- Per-core run queues
- Load balancing (work stealing)
- IPI (inter-processor interrupts) for scheduling
- Proper spinlocks with memory barriers
- Scheduler affinity
- **~600 LOC** in `sched/smp.rs`, `arch/ipi.rs`

## Phase 18: Power Management
- CPU idle states (WFI-based)
- DVFS (dynamic voltage/frequency scaling)
- Suspend/resume framework
- RTC wake-on-timer
- Watchdog timer driver
- **~400 LOC** in `arch/power.rs`, `drivers/wdog.rs`

## Phase 19: Device Driver Framework
- Unified driver model: probe, remove, suspend, resume
- Device tree parsing for hardware discovery
- Driver registry + automatic probing
- sysfs-like interface for driver introspection
- **~600 LOC** in `drivers/mod.rs`, `dtb/driver.rs`

## Phase 20: Standard Library Layer
- libc-like syscall wrappers
- errno handling
- POSIX file I/O (open, read, write, close, lseek)
- POSIX process (getpid, fork, exec, waitpid)
- Thread-local storage
- **~800 LOC** in `usrland/liblobster.rs`

## Estimated Total: ~8,400 LOC addition → ~23,000 LOC total
