# ADR 0002: Per-process address spaces and validated user pointers

Status: proposed (2026-11-25, Phase A design for roadmap O5.1; implementation waits for review)

All file:line references are against `feature/o51-address-spaces` at 8c37bf5.

## Context

**EL0 runs on the kernel's own TTBR0.** Every ELF load builds a private user
tree (`alloc_user_page_table`, `sched/elf.rs:204`, `map_user_page` :221) and
then dual-maps each page into the kernel's live TTBR0 as well
(`sched/elf.rs:354-359`, :411-415, :488-491). The comment at
`sched/elf.rs:684` states it: "EL0 runs on the kernel's own TTBR0". Two
programs alive at once share VAs (code `0x1000_0000`, heap `0x2000_0000`,
stack top `0x7F00_0000`, `memory/process_vmem.rs:46-49`), so the kernel table
is rewritten at every slice switch: `activate_user_mappings`
(`sched/elf.rs:801`) clears the user range (`clear_user_pages` :737), copies
the incoming task's leaf PTEs (`copy_user_pages` :752) and issues
`tlbi vmalle1is` (:816); it is called from `sched/mod.rs:1441`. A process
that guesses a frame address cannot be told apart from a legal pointer, and
`INSTALLED_USER_PID` (:685) is a cache-coherence hazard waiting to happen.
`set_task_argv` (`sched/mod.rs:807-838`) relies on the same mirror ("dual-mapped
into the kernel address space", :823) to write argv at the user VA.

**The per-process tree is already half there but never loaded.** `switch_address_space`
(`sched/mod.rs:1139`, `sched/process.rs:283`) writes TTBR0 and is only reached
from the pick-next path (`sched/mod.rs:1041`); the EL0 entry path at :1440-1446
never switches. The per-process tree is also EMPTY apart from user pages (no
kernel mappings), so loading it at EL1 would immediately fault on the next
instruction fetch: the kernel executes from the low identity map (`boot.S:30`
`ldr x9,=0x80000`; `linker.ld` `. = 0x80000`).

**The kernel lives in both halves.** `memory/mmu.rs:305-` builds two roots:
- TTBR0 root: 128 MiB identity of RAM as 2 MiB blocks (`mmu.rs:316-322`,
  flags `PTE_KERNEL_CODE_BLOCK`, AP=00 = EL1-only, no UXN/PXN), QEMU peripherals
  `0x7C00_0000..0x7E00_0000` (:329-340) and GIC `0x10_7FFF_0000` (:349-357).
- TTBR1 root: kernel image at `KERNEL_IMAGE_START` (`mmu.rs:365-372`), direct
  physical map at `PHYS_MAP_OFFSET = 0xFFFF_8000_0000_0000` (:374-382,
  `memory/mod.rs:75`), kernel heap `0xFFFF_9000_0000_0000` (`memory/mod.rs:93`),
  devices `0xFFFF_C000_0000_0000` (:384-447). All page tables, frames, kernel
  stacks and the ELF loader's data copies are reached through the direct map,
  never through TTBR0.

So what really needs TTBR0 at EL1 is: the executing kernel text/data/BSS and the
boot stack (identity, `0x80000..~0x41f2000`), plus the identity-mapped QEMU
peripheral and GIC blocks that early drivers still use.

**No validation of mapping.** `sched/uaccess.rs:19-60` only range-checks
`0x1000_0000..0x8000_0000` (`user_slice`, `user_slice_mut`, `user_str`); it
does not check the page is mapped or EL0-accessible. `getrandom` with `0x5000_0000`
(valid range, unmapped) writes through an unmapped VA from EL1 and takes an EL1
data abort (`arch/exception.rs:45`). Only 4 of the ~60 raw-pointer sites use the
helpers (`syscall.rs:1109, 1154, 1174, 1202`); the rest are listed below.

**Other facts the design depends on.**
- TCR_EL1: 4 KiB granule, T0SZ=T1SZ=16, A1=0 (ASID in TTBR0), AS=0 (8-bit ASIDs)
  (`mmu.rs:~240-255`). User PTEs already carry nG (`elf.rs:179-193`).
- User code PTE is `AP=EL0_RW` with no UXN/AP[2] (`elf.rs:179-193`): every user
  page is currently writable AND executable, and EL1-executable (no PXN).
- Page faults from EL0: `handle_sync_exception_el0` (`arch/exception.rs:102`,
  data abort :158) calls `process::handle_page_fault` (`sched/process.rs:629`,
  COW only) then `kill_current_and_exit(-1)`.
- Teardown today: `release_task_resources` -> `free_user_address_space`
  (`sched/mod.rs:1861-1899`) after `user_windows::close_all_for_pid`.
- Caches are on (SCTLR C|I, `mmu.rs:~280`); the kernel also boots under KVM.
- Secondary cores only run kernel code (`smp/mod.rs:264`, they re-use the boot
  TTBR1 and identity table); they must keep the boot root (see Risks).

## Decision

### 1. Per-process root that embeds the kernel low-half as EL1-only (not TTBR1-only kernel)

Each user process gets its own L0 root. The kernel stays where it is. The
process root contains:

- **the kernel's identity region, EL1-only**, by *copying* the relevant table
  entries from the boot root (descriptors AP=00, so EL0 faults on them even at
  the same VA);
- **the user pages**, nG, at `0x1000_0000..0x8000_0000` only.

Rejected alternative: move the kernel to TTBR1-only (link at `0xFFFF_8000_0008_0000`,
no low identity). TTBR1 already maps the image and RAM, but the boot code, the
linker script (`. = 0x80000`), position-dependent early assembly, the MMIO
identity blocks, the secondary-core entry and the PS4/KVM boot path all assume
low addresses. That is a re-link plus boot rewrite whose only extra benefit is
that kernel threads need no table at all; it is a 1-2 week, high-risk change for
the same isolation. The embedded-identity design reaches the same isolation
property (a process can never name a kernel or foreign frame) with page-table
code only, and can be replaced by the TTBR1-only kernel later without changing
any caller (everything goes through `AddressSpace` below).

Layout of the per-process root (indices for 4 KiB granule, 48-bit VA):

| Level | Index | Content in the per-process tree | Shared with kernel table? |
|---|---|---|---|
| L0 | 0 (VA 0..512 GiB) | table -> process L1 | no (own page) |
| L0 | 1..511 | copied from the boot root (empty today; identity GIC at `0x10_7FFF_0000` sits in L0[0]) | copied descriptors |
| L1 | 0 (VA 0..1 GiB) | table -> process L2a | no |
| L1 | 1 (1..2 GiB) | table -> process L2b | no |
| L1 | 2.. (incl. GIC slot 0x41) | copied from the boot L1 | copied descriptors |
| L2a | 0..63 (0..128 MiB) | kernel identity 2 MiB blocks, AP=00 (EL1 RW) | copied entries |
| L2a | 128..511 (0x1000_0000..0x4000_0000) | user table descriptors -> private L3 | no |
| L2b | 0x1E0..0x1EF (0x7C00_0000..0x7E00_0000) | QEMU peripheral device blocks, AP=00 | copied entries |
| L2b | 0..0x1DF, 0x1F0..0x1FF | user L3 tables (stack `0x7EF0_0000..0x7F00_0000` is slot 0x1F7) | no |
| L3 | any | user 4 KiB pages | no |

That is 4 table pages per process (L0, L1, L2a, L2b) plus L3s on demand, each
4 KiB, built by `process_vmem::new_address_space()` which copies ~70 descriptors.
Sharing below L2 is not needed because the copied entries are 2 MiB *blocks*.

Permission bits, before and after (`PTE_USER_*` are rewritten; AP[2:1]: 00 EL1 RW,
01 EL0+EL1 RW, 10 EL1 RO, 11 EL0+EL1 RO; UXN bit 54, PXN bit 53, nG bit 11):

| Region | Who maps it today | Table today | AP | UXN | PXN | nG | After (per-process tree) |
|---|---|---|---|---|---|---|---|
| Kernel text/data/BSS/boot stack (identity 0..128 MiB) | boot, `mmu.rs:316` | kernel TTBR0 | 00 | 0 | 0 | 0 | copied block entries, AP=00, **UXN=1** added; PXN=0 (text) |
| Peripheral/GIC identity blocks | boot, `mmu.rs:329-357` | kernel TTBR0 | 00 | 0 | 0 | 0 | copied, AP=00, **UXN=1, PXN=1** |
| Kernel image, phys map, heap, devices | boot | TTBR1 | 00 | 0 | 0 | 0 | unchanged (TTBR1 never changes); phys map gets PXN=1 |
| User code (PF_X) | `elf.rs:389`, plus mirror | user tree AND kernel TTBR0 | 01 (RW!) | 0 | 0 | 1 (user) / 0 (mirror) | user tree only: AP=11 (RO), UXN=0, **PXN=1**, nG=1 |
| User rodata | `elf.rs:393` | both | 01 | 0 | 0 | 1/0 | user tree only: AP=11, **UXN=1, PXN=1**, nG=1 |
| User data, bss, heap, stack | `elf.rs:391`, :488 | both | 01 | 0 | 0 | 1/0 | user tree only: AP=01, **UXN=1, PXN=1**, nG=1 |
| COW page (`PTE_COW`, bit 52) | `process_vmem.rs:37` | user tree | 11 until fault | as segment | 1 | 1 | unchanged semantics, PXN=1 |
| Window pixel buffer (`PTE_WIN_USER`, `user_windows.rs:385`) | `user_windows.rs` | user tree AND kernel TTBR0 | 01 | 0 | 0 | | user tree only, AP=01, UXN=1, PXN=1, nG=1; kernel uses phys map |
| GOT (`PTE_GOT`, `dlinker.rs:418`) | dlinker | user tree | | | | | user tree only, AP=01, UXN=1, PXN=1 |

W^X for code (PF_X pages become read-only) is a behaviour change: any test
program that writes into its own text must be found by the Phase B run of the
existing programs; if one exists it gets PF_W moved to data. PXN everywhere user
is the SMEP equivalent; PAN is NOT enabled in this ADR (see uaccess), but because
the kernel only touches user data through the direct map it can be turned on later.

### 2. TTBR0 switching, ASIDs, TLB

- `AddressSpace { root_phys, asid, generation }` in `memory/process_vmem.rs` owns
  the root; `Task.ttbr0_phys` stays the root address and gains `asid: u16`
  (`sched/mod.rs:358`).
- TTBR0_EL1 value = `root_phys | (asid << 48)`. The kernel/boot table is ASID 0;
  kernel threads run on the boot root (`KERNEL_ASPACE`).
- ASIDs: 8-bit (TCR.AS=0), allocated 1..255 with a global generation counter. A
  process whose `generation` is stale gets a fresh ASID; on rollover do ONE
  `tlbi vmalle1is; dsb ish; isb` and bump the generation. User PTEs are nG;
  kernel PTEs are global, so no per-switch flush is required in the common case.
- Switch sequence (one function `memory::process_vmem::activate(&AddressSpace)`):
  `dsb ish` (page-table stores visible) -> `msr TTBR0_EL1, x` -> `isb`. With a
  fresh ASID/generation no TLBI is needed; when an ASID is reused on the same
  core after teardown, `tlbi aside1is, asid<<48; dsb ish; isb` runs at free time.
- Phase B commit 2 starts with the simpler "full `tlbi vmalle1is` per switch"
  and a flag to turn ASIDs on after the gate is green, so the risky part is
  separable.
- The `tlbi vaae1is` after every `map_user_page` (`elf.rs:306, :322`;
  `process_vmem.rs:345, :422`) stays but becomes ASID-scoped (`tlbi vae1is` with
  the ASID) once ASIDs are on; cross-core is irrelevant (user code runs on one core).

### 3. Context switch and exception entry

- **Exception entry from EL0** (`arch/exception.rs:102, :283`; vectors in
  `boot.S:~482`): no change. The process root already contains the kernel's
  identity region, so the kernel keeps running on the process table. This is the
  main reason for choosing the embedded-identity design: no table switch on the
  trap path, no trampoline, no extra instructions on every syscall.
- **Switch to EL0** (first run, `sched/mod.rs:1434-1460`, and every slice
  `sched/mod.rs:1041`): call `activate()` immediately before `restore_context`.
  `elf::activate_user_mappings` and the `INSTALLED_USER_PID` machinery are deleted.
- **Switch to a kernel thread** or return to the shell after exit
  (`finish_cooperative_return`, `arch/exception.rs:234`): `activate(KERNEL_ASPACE)`
  so a dead process's tree can be freed while its table is no longer live.
- Preemption path (`sched/preempt.rs`) performs the same `activate()` for the
  incoming task. The old root is never freed while still loaded on this core
  (teardown order below).

### 4. User memory access: software walk + direct-map copy (no fixup table)

Pick: `copy_from_user` / `copy_to_user` / `strncpy_from_user` in
`sched/uaccess.rs` that, per page, walk the CURRENT process's table (software
walk through the direct map), require a valid L3 leaf with AP[1]=1 (EL0
accessible) and, for writes, AP[2]=0 (writable; COW pages are resolved by calling
`handle_cow_fault` first, exactly as a hardware write fault would), then copy via
`PHYS_MAP_OFFSET + pa`. Any failure returns `-EFAULT` (`u64::MAX - 13`, matching
`syscall.rs:1110`) with zero bytes copied past the bad page.

Why not an exception fixup table: the kernel would dereference user VAs at EL1,
which needs the process table live (it is) but also needs a fixup section in the
linker script, per-instruction fixup entries for `memcpy`-style loops (Rust's
`copy_nonoverlapping` lowers to `memcpy` calls we do not control), and a data
abort handler at EL1 that is today a panic (`exception.rs:45`). The software
walk is ~120 lines, host-testable against a model table in host memory (a stated
test requirement), takes no EL1 faults at all, keeps PAN enableable, and makes
the identity-mapped peripheral blocks inside `0x7C00_0000..0x8000_0000`
unreachable by construction: they are AP=00 in the walk, so EL0 pointers into them
get -EFAULT even though the numeric range check passes.

Rules for Phase B code:
- API: `copy_from_user(dst: &mut [u8], src: UserPtr) -> Result<(), Fault>`,
  `copy_to_user(dst: UserPtr, src: &[u8])`, `read_user<T: Pod>`, `write_user<T: Pod>`,
  `user_cstr(ptr, max, out: &mut [u8]) -> Result<usize, Fault>`. `UserPtr(u64)` is
  a newtype so a raw `u64` cannot be dereferenced. The old `user_slice*` helpers
  are deleted (they return references into user memory, which cannot be made
  safe: the process can unmap/replace between check and use, and the page-crossing
  check does not exist).
- Large copies are chunked through a 4 KiB kernel buffer so no kernel reference
  to user memory outlives a page.
- The walk reads the current task's root, not a global, so a syscall running while
  a different table is loaded (kernel thread) returns -EFAULT.
- Mid-copy preemption: syscalls run with the task's own table and the
  scheduler does not free another task's pages while this task is in the kernel;
  the only way to lose a page mid-copy is the process's own `munmap`/`exit`, which
  cannot run concurrently with its own syscall (single-threaded processes, no
  `CLONE_VM`). Phase B asserts that no process threads exist; the walk is done page
  by page immediately before copying that page, so even a future cross-core unmap
  is limited to a one-page window (documented limitation).

### 5. GUI window buffers ("global window mirror")

The window pixel buffers are already frame-backed, one frame per page
(`wm/user_windows.rs:16-31`). Today the compositor reads them through the direct
map (`pixel_ptr` :201 uses `PHYS_MAP_OFFSET + pa`) and the kernel ALSO mirrors the
frames into the kernel TTBR0 (`user_windows.rs:379-388`, unmap :425-431) purely
so the app's stores at the user VA are visible through the kernel table.
After this change the mirror is deleted: the app maps the frames into ITS tree only
(`map_user_page(ttbr0, ...)`, AP=01 UXN PXN nG), the compositor keeps reading
through the direct map. Frames are owned by the window slot (`slot.frames`), so
process exit order is: close windows (unmap from the owner's tree, free frames) ->
free tree. `current_user_ttbr0()`/`kernel_ttbr0()` (:208-225) lose the second
function. Title strings (:261) and any other user pointer in the win syscalls move
to `user_cstr`. There is no cross-process window mapping today, so no shared
mapping design is needed; if one is added, `PTE_SHARED` (`process_vmem.rs:40`)
frames are refcounted by `frame_alloc` per page and each process's tree holds its
own PTE.

### 6. Shared libraries, GOT, dlinker

`map_got` maps into the process tree via `map_user_page(ttbr0_phys, ...)`
(`dlinker.rs:399-418`) and relocations are written by
`write_user_u64` (:838-860), which today translates through `vmem::translate_va`
(a TTBR1 walk that cannot see user VAs, hence the "HACK" comment at :844-850).
Replace with `uaccess::write_user` against the target process's table:
`write_user_in(&AddressSpace, va, u64)`, the cross-process variant (used while
loading, when the target is not the current task). `shared_lib.rs:208-230`
(`dlopen`/`dlsym` path and symbol strings) use `user_cstr`. The shared library
image itself is a set of kernel-owned frames mapped RO+X into each process that
`dlopen`s it, one PTE per process (frame refcount via `PTE_SHARED`), so no frame
is ever mapped into the kernel table at a user VA.

### 7. Teardown order

1. `user_windows::close_all_for_pid` (unmap window pages from the owner's tree, free frames).
2. Switch the CPU to `KERNEL_ASPACE` (`activate`) so the tree being freed is not live.
3. `tlbi aside1is` for the process ASID; release the ASID.
4. `free_user_pages` for every user L3 leaf whose frame is not `PTE_SHARED` (or drop
   the refcount), then free L3s, the kernel-copied L2a/L2b/L1/L0 pages (never recurse
   into the copied kernel block entries; the free walk skips AP=00 descriptors and
   anything outside `USER_CODE_START..USER_ADDR_LIMIT`).
5. Free the kernel stack (`sched/mod.rs:1868-1873`), as today.

`release_task_resources` (`sched/mod.rs:1861`) already has steps 1 and 5; Phase B
adds 2-3 and replaces :1877-1899 (which today unmaps the kernel mirror first).
`exec` (`sched/process.rs:226-262`) frees the old tree only after
`activate(new)`; today it destroys the old tree before the switch.

## Test plan

Host (`--features std`, `serial_guard`): build a model address space in host
memory (the walk accepts a `PhysToVirt` closure) and test: map/unmap/protect;
walk of unmapped, kernel (AP=00) and read-only pages; page-crossing into an
unmapped page; COW write; ASID rollover; teardown frees exactly the frames
allocated (counting allocator). Exhaustive page-boundary cases for `copy_to_user`.

EL0 programs (`tools/build_test_programs.sh`): `two_apps` (A writes a secret at
a VA and sleeps; B at the same VA reads its own data; B reading A's frame via
any PA/phys-map/kernel VA faults and is killed, kernel survives), `leak`
(spawn+exit x100, print `frame_alloc::stats()` before/after), `efault`
(unmapped `0x5000_0000`, kernel `0x80000`, NULL, page-crossing into unmapped,
read-only page for a read syscall, peripheral block `0x7C00_0000`; each returns
-EFAULT and the kernel stays alive), existing `/bin/spin` (preemption, Ctrl+C) and
`getrandom_test` (the unmapped-pointer case now returns -EFAULT instead of
aborting). Live under TCG AND KVM; GUI harness 12/12 under TCG; `hello_lobster`
still renders a window; gate green. A new shell diagnostic `aspace` prints the
current root/ASID and `frames` the allocator free count so tests are provable from
the serial console.

## Risks and rollback

1. **Caches.** Page-table writes go through the cacheable direct map and walks are
   configured WB-cacheable inner-shareable (TCR `IRGN/ORGN/SH`), so no clean to
   PoU is needed; each table update is `dsb ishst` before the `msr TTBR0_EL1`.
   Frames that become user code need I-cache coherence: `ic ialluis; dsb ish; isb`
   after the ELF loader fills a PF_X page (today implicit because the mirror and
   the kernel share the data path; must be explicit).
2. **KVM.** KVM/QEMU enforce AP/UXN/PXN and ASIDs exactly as hardware; the main
   difference is stale TLB reuse, so the ASID-rollover path gets a `tlbi vmalle1is`
   test under KVM, and KVM runs `two_apps` and `leak` explicitly.
3. **Preemption/IRQ mid-switch.** `activate()` runs with IRQs masked; the root is
   in place before `eret` and the tree being freed is never loaded.
4. **Kernel dependence on the low identity.** If anything the kernel does after
   boot edits the boot root's identity entries (it does not today:
   `map_page_virt`/`remap_non_cacheable`/`unmap_page_virt` edit TTBR1 via
   `PAGE_TABLE_STORAGE.0[1]`, `mmu.rs:552-642`), the copies in every process tree
   would go stale. Phase B adds a debug assertion at `new_address_space` that the
   boot identity descriptors equal the copy source, and a `[mmu]` boot log of the
   identity range.
5. **`map_user_page` block splitting** (`elf.rs:243-283`) would silently split a
   copied kernel 2 MiB block if a user VA hit it; the new mapper returns an error
   instead, and `brk`/`mmap` are capped below `0x7C00_0000`.
6. **W^X/PXN behaviour change** may break a test program that writes to its
   text; found by the program sweep in Phase B commit 2 (flags can be relaxed
   per-segment without touching the design).
7. **Secondary cores** keep the boot root; they never run EL0.

Rollback: every Phase B commit is independently gate-green and revertible. The
dual-map path is kept (behind `const PER_PROCESS_TTBR0: bool = false`) until the
two-app and KVM runs pass, so the flag is the escape hatch for commits 2-4.
If the whole feature must go, `git revert` of the merge restores today's
behaviour; no on-disk format changes.

## Phase B plan (ordered, each gate-green; estimate)

| # | Commit | Approx. lines |
|---|---|---|
| 1 | `test(uaccess)`: host model page table + walk tests, `UserPtr`, `copy_*_user` with the walk; `getrandom`/`write`/`read` converted; old range-only helpers kept | +350 |
| 2 | `feat(mmu)`: `new_address_space()` (kernel entry copy, AP/UXN/PXN flags), `AddressSpace`, `activate()`, ASID allocator (full-flush mode), `aspace`/`frames` shell diagnostics, flag off | +300 |
| 3 | `refactor(syscall)`: convert every raw pointer site below to `uaccess` (path syscalls, stat/fstat, `getcwd`, `uname`, Linux-compat group, ioctl/pty/shm_ctl, msgq/unix/fifo, klogctl, signals, argv block, `shared_lib`, `dlinker`) | +420 / -220 |
| 4 | `feat(sched)`: switch the flag on: activate on EL0 entry/slice/exit, delete dual-map, `activate_user_mappings`, `INSTALLED_USER_PID`, `set_task_argv` via `write_user_in`; user PTE flags (W^X, PXN/UXN), I-cache maintenance | +180 / -250 |
| 5 | `feat(wm)`: remove window mirror, teardown order, `exec` ordering | +60 / -90 |
| 6 | `feat(mmu)`: ASID mode with generation rollover + TLBI scoping | +120 |
| 7 | `test`: `two_apps`, `leak`, `efault` EL0 programs + `build_test_programs.sh`, TCG+KVM runs, harness | +280 |

Total about +1700 / -560 lines across `memory/process_vmem.rs`, `memory/mmu.rs`,
`sched/{uaccess,syscall,elf,mod,process,signal,shared_lib,dlinker}.rs`,
`wm/user_windows.rs`, `ipc/mod.rs`, `arch/exception.rs`, `tools/`.

## Appendix: raw user-pointer sites to convert in Phase B

Line numbers are from this commit. Unmarked entries are user pointers in
`src/sched/syscall.rs`.

- `SYS_EXEC`: :234 (`while *path_ptr.add(len) != 0` + `from_raw_parts`)
- `SYS_GETGROUPS` :324, `SYS_SETGROUPS` :335 (`from_raw_parts[_mut]`)
- `SYS_CHMOD` :363, `SYS_CHOWN` :381, `SYS_STAT` :449, `SYS_LSTAT` :465,
  `SYS_MKDIR` :485, `SYS_RMDIR` :504, `SYS_UNLINK` :523, `SYS_CREAT` :543,
  `SYS_ACCESS` :567, `SYS_CHDIR` :602, `SYS_TRUNCATE` :669 (path strings)
- `SYS_GETCWD` :584 (`from_raw_parts_mut`), `SYS_UNAME` :635 (6x65 buffer)
- Linux-compat group (dispatch ~:850-1100): `NANOSLEEP` :863 (`*(args[0] as *const u64)`),
  `UNAME` :875, `CLOCK_GETTIME`/`GETTIMEOFDAY` :905-906, :913-914 (direct stores),
  `OPENAT` :934, `OPEN` :952, `FSTAT`/`NEWFSTATAT` :961, :966, :968, :974,
  `EXECVE` :1049
- `sys_read` :1220 (raw `from_raw_parts_mut`); `sys_open` :1255; `sys_pipe` :1366-1367
- `sys_msgq_send` :1422, `sys_msgq_recv` :1429
- `sys_sigprocmask` :1519; `sys_klogctl` :1555, :1578
- `sys_mkfifo` :1592, `sys_open_fifo` :1599
- `sys_unix_send` :1670, `sys_unix_recv` :1690
- `sys_openpty` :1834-1835; `sys_ioctl` :1885 (`copy_nonoverlapping`)
- `sys_stat` :1986, :1991-1998; `sys_fstat` :2012, :2017-2021; `sys_getdents` :2027 (check body)
- Already via helpers but to be re-pointed at the new API: `sys_getrandom` :1109,
  `sys_write` :1154, :1174, :1202
- `sched/mod.rs:771-801` `write_argv_block` (writes argv at the user SP via raw stores; used by `set_task_argv`)
- `sched/signal.rs:379-385` (signal frame copied to user stack), `sigreturn` :435 (reads it back)
- `sched/shared_lib.rs:208-212` (dlopen path), :226-230 (dlsym symbol)
- `sched/dlinker.rs:704, :787` (`from_raw_parts` on strings; audit whether the source is
  user memory or the loaded image), :838-860 `write_user_u64` (GOT/relocation stores)
- `ipc/mod.rs:1425` (`SHM_CTL_SET` reads `result_ptr`), :2570, :2582, :2595 (pty ioctls via
  `arg as *mut u16/u32`); the `SHM_CTL_STAT` store just above (`dst`, not re-checked)
- `wm/user_windows.rs:261` (title pointer read byte-by-byte, `*((title_ptr + i) as *const u8)`)

`wm/user_windows.rs` pixel accesses (:155, :171, :201, :332, :366, :686) go
through the direct map of window frames and are not user pointers.

## Amendments (orchestrator review, 2026-11-25)

**A. Bit-identical kernel descriptors.** Global (nG=0) TLB entries are not
ASID-tagged: an identity entry cached while the boot root is live would be used
by EL0 while a process root is live. The commit that starts copying descriptors
also applies UXN=1 (identity RAM blocks) and UXN=1+PXN=1 (peripheral and GIC
blocks) to the BOOT ROOT's own entries, so every table holds identical kernel
descriptors. Host test: the copied descriptors equal the boot descriptors.

**B. TCR_EL1.** `setup_tcr` (`memory/mmu.rs`, the `msr TCR_EL1` write) composes
T0SZ=T1SZ=16, TG0=4 KiB, TG1=4 KiB, SH/ORGN/IRGN=3/1/1, IPS=2 and nothing else:
A1 (bit 22) is 0, so the ASID comes from TTBR0, and AS (bit 36) is 0, so ASIDs
are 8 bits. Verified by reading the constant; no change needed.

**C. Kernel low half is frozen after boot.** Grep of every runtime mapper:
`map_page_virt`, `unmap_page_virt`, `protect_page_virt` and `remap_non_cacheable`
(`memory/mmu.rs`; callers `memory/vmem.rs:29-39` and `display/fb.rs:439`) all edit
the TTBR1 root (`PAGE_TABLE_STORAGE.0[1]`) at `PHYS_MAP_OFFSET` addresses. The
framebuffer remap, MMIO and DMA buffers therefore never touch the low half.
`map_range_blocks`/`map_range` on the TTBR0 root run only inside `mmu::init`.
Phase B adds a debug assertion that `mmu::init` is the only writer after the
first process root exists.

**D. I-cache.** Code pages are filled through the direct-map alias, so before
the first EL0 entry of a process (and after any write into an executable page)
the loader runs `dc cvau` over the alias range, `dsb ish`, `ic ialluis`,
`dsb ish`, `isb` (A76 I-cache is PIPT, so the alias is coherent after that).
With W^X on there must be no GOT/PLT patching of executable pages; the
relocation writer asserts the target PTE has UXN=1.

**E. Flag lifetime.** `PER_PROCESS_TTBR0` exists during commits 2-4 only. The
last commit deletes the dual-map path, `activate_user_mappings`,
`INSTALLED_USER_PID` and the window mirror, leaving no dead code.
