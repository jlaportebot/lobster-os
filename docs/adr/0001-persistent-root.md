# ADR 0001: Persistent system state through a prefix map

Status: accepted (2026-11-18)

## Context

The root filesystem is a RAM VFS. The ext2 disk (`lobster-vd0`) is mounted at
`/sd`, and only `/home/<user>` was redirected onto it (a hard-coded alias in
`vfs.rs`). Everything else the system writes at runtime lived in RAM and
vanished on reboot: the package database (`/var/lib/pkg`), installed files
(`/usr`, `/apps`), repository config (`/etc/pkg`), shadow and group files,
DHCP and Wi-Fi state. Individual modules worked around this with ad-hoc
`/sd/...` paths (`PERSIST_PATH` in `passwd.rs`, the settings and theme files,
the welcome marker), each with a "volatile" twin for the no-disk case.

Separately, `/proc`, `/dev` and `/sys` were never reachable: the procfs,
devfs and sysfs generators existed, but nothing initialised them or created
their VFS nodes (`mount_virtual_fs` had no caller and registered mount
records only).

## Decision

1. **One table-driven prefix map** (`src/fs/prefix_map.rs`). While the disk is
   mounted, the paths `/bin /etc /var /usr /apps /opt /root /home` (an exact
   component match, so `/etcetera` is not touched) are rewritten to
   `/sd/<same path>` before lookup. The old home alias becomes one row of the
   table. Each prefix is enabled only after its backing directory exists on
   disk (created when missing), so a damaged disk degrades one prefix to RAM
   instead of breaking boot. Without a disk the map is empty and everything
   stays RAM as before. `/tmp`, `/dev`, `/proc`, `/sys` are never mapped.
2. **`/bin` is mapped too.** The kernel's embedded programs are written to
   `/bin` at boot only when the file is missing or differs
   (`vfs::write_file_if_changed`), so the disk's own `/sd/bin` programs
   (`spin`, ...) appear next to them and a kernel upgrade refreshes the
   embedded ones. The cost is one read-and-compare of about 330 KiB per boot.
3. **Boot order:** mount the disk and enable the map right after `vfs::init()`
   and before anything reads or writes `/etc` or `/var` (embedded programs,
   users, setup wizard, dhcp, wifi, pkg, settings). Seeding of default files
   is create-if-missing only (users and pkg repo config already check first).
4. **Paths are normalised** (`.`, `..`, `//`) when the map is active, so
   `/etc/../tmp/x` is `/tmp/x`, not `/sd/tmp/x` by accident.
5. **Per-module persistence code goes away:** `passwd::persist`/`load_persisted`
   and the welcome marker's volatile twin become plain `/etc/...` paths.
6. **Virtual filesystems** are projected into the VFS as RAM nodes by
   `src/fs/virtual_fs.rs`: `/proc`, `/dev`, `/sys` are created at boot, the
   three generators are initialised, and their nodes are regenerated whenever
   a path under them is resolved (so `cat /proc/uptime` is fresh and stale
   `/proc/<pid>` entries disappear).

## Alternatives considered

- **Mount the ext2 disk at `/` (root-on-ext2).** One mechanism and no
  rewriting, but the VFS root node is a RAM directory created in
  `vfs::init`; making it an ext2 node means the embedded programs, `/tmp`,
  `/proc`, `/dev` and the no-disk fallback all need special cases, and a
  corrupt disk would leave no usable root. The prefix map is about 100 lines
  on top of a mechanism that already exists and keeps RAM as the safe default.
- **Keep per-module `/sd/...` paths.** Rejected: every writer must know about
  the disk, and `/etc` or `/var` writers that forgot (pkg, shadow, dhcp, wifi)
  are exactly the bug.
- **Keep `/bin` in RAM and merge `/sd/bin` into listings.** Needs a union
  directory in the VFS for one directory; rewriting plus write-if-changed is
  simpler.

## Consequences

- Every path-taking VFS entry point already calls `alias_path`; it now
  consults the map, so no caller changes.
- Files under the mapped prefixes are visible at both `/etc/x` and
  `/sd/etc/x` (same node).
- `/tmp`, `/proc`, `/dev`, `/sys` and the `/share` host folder stay
  RAM/virtual on purpose.
- `/proc`, `/dev`, `/sys` nodes are writable RAM files from the VFS's point of
  view; writes are discarded at the next regeneration.
- The real-hardware eMMC path (`probe_and_mount_etc`, mounts a partition at
  `/etc`) is untouched and unverified here.
