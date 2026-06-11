# Contributing to LobsterOS

Thank you for your interest in contributing to LobsterOS! This document outlines how to get started, our development practices, and what we expect from contributions.

## Table of Contents

- [Code of Conduct](#code-of-conduct)
- [Getting Started](#getting-started)
- [Development Environment](#development-environment)
- [How to Contribute](#how-to-contribute)
- [Pull Request Process](#pull-request-process)
- [Coding Standards](#coding-standards)
- [Testing](#testing)
- [Architecture Overview](#architecture-overview)
- [Good First Issues](#good-first-issues)
- [Communication](#communication)
- [Recognition](#recognition)

## Code of Conduct

This project adheres to the [Contributor Covenant Code of Conduct](CODE_OF_CONDUCT.md). By participating, you are expected to uphold this code. Please report unacceptable behavior to the maintainers.

## Getting Started

### Prerequisites

- **Rust nightly** (required for `#![no_std]` and unstable features)
  ```bash
  rustup install nightly
  rustup default nightly
  rustup target add aarch64-unknown-none
  rustup component add llvm-tools-preview rustfmt clippy
  ```
- **QEMU** (for emulation testing): `qemu-system-aarch64` ≥ 8.0
- **Git** with LFS support (for any binary assets)

### Quick Start

```bash
# Clone the repository
git clone https://github.com/yourusername/lobster-os.git
cd lobster-os

# Build for QEMU (raspi4b emulation)
cargo build --target aarch64-unknown-none --features qemu

# Run in QEMU
qemu-system-aarch64 -M raspi4b -kernel target/aarch64-unknown-none/debug/lobster-os -serial stdio -display none

# Run tests
cargo test --features std          # Host tests
cargo test --target aarch64-unknown-none --features qemu --test integration  # QEMU tests

# Lint and format
cargo fmt --all -- --check
cargo clippy --features qemu -- -D warnings
```

## Development Environment

### Recommended: Dev Container

For a zero-config setup, use the provided dev container (VS Code / GitHub Codespaces):

1. Open repository in VS Code
2. Install "Dev Containers" extension
3. Command Palette → "Dev Containers: Reopen in Container"

The container includes:
- Rust nightly with `aarch64-unknown-none` target
- QEMU with `raspi4b` machine support
- `cargo-binutils`, `llvm-tools`
- Pre-configured `rust-analyzer`

### Local Setup (Alternative)

If not using dev container, ensure your local environment matches the CI configuration in `.github/workflows/ci.yml`.

## How to Contribute

### 1. Find or Create an Issue

- Browse [open issues](https://github.com/yourusername/lobster-os/issues)
- Look for labels: `good first issue`, `help wanted`, `documentation`
- For new work: **open an issue first** to discuss the approach before implementing

### 2. Claim the Issue

Comment on the issue: "I'd like to work on this." A maintainer will assign it to you.

### 3. Create a Branch

```bash
git checkout -b feature/short-description
# or
git checkout -b fix/short-description
# or
git checkout -b docs/short-description
```

Branch naming convention:
- `feature/*` — New functionality
- `fix/*` — Bug fixes
- `docs/*` — Documentation only
- `refactor/*` — Code restructuring without behavior change
- `test/*` — Test additions/improvements

### 4. Make Changes

- Write code following our [Coding Standards](#coding-standards)
- Add tests for new functionality
- Update documentation as needed
- Keep commits atomic and focused

### 5. Test Your Changes

```bash
# Format check
cargo fmt --all -- --check

# Lint (must pass with zero warnings)
cargo clippy --features qemu -- -D warnings

# Build
cargo build --features qemu --target aarch64-unknown-none

# Run tests
cargo test --features std
cargo test --target aarch64-unknown-none --features qemu --test integration
```

### 6. Submit a Pull Request

- Push your branch and open a PR against `master`
- Fill out the PR template completely
- Reference the issue: `Fixes #123` or `Closes #123`
- Wait for CI to pass and maintainer review

## Pull Request Process

### PR Requirements

- [ ] CI passes (format, clippy, build, tests)
- [ ] PR description explains **what** and **why**
- [ ] Linked to an issue (`Fixes #N`)
- [ ] Commits are logical and atomic
- [ ] No debug code (`dbg!`, `println!`, `unimplemented!` in production paths)
- [ ] Documentation updated if public API changed
- [ ] New code has tests (unit or integration)

### Review Process

1. **Automated checks** run on every push
2. **Maintainer review** within 48 hours (typically faster)
3. **Address feedback** — push new commits to the same branch
4. **Approval** — At least one maintainer approval required
5. **Merge** — Squash-merge by maintainer

### PR Size Guidelines

- **< 200 lines**: Quick review, same-day merge
- **200–500 lines**: Standard review, 1–2 days
- **500–1000 lines**: Thorough review, allow 2–3 days
- **> 1000 lines**: Consider splitting into smaller PRs

## Coding Standards

### Rust Style

- **`cargo fmt`** — Mandatory, enforced by CI
- **`cargo clippy -- -D warnings`** — Zero warnings allowed
- **Rust 2021 edition** — Use modern idioms
- **`#![no_std]`** — All kernel code; `std` only in `#[cfg(test)]` or host tools

### Code Organization

- **Domain-driven modules** — `memory/`, `net/`, `sched/`, not `models/`, `services/`
- **`pub(crate)` over `pub`** — Minimize public API surface
- **One public type per file** when types are large
- **Re-export public API** from `lib.rs` / module `mod.rs`

### Safety & Correctness

- **No `unwrap()` / `expect()` in production kernel code** — Use `?`, `ok_or()`, or handle `Result`/`Option`
- **Every `unsafe` block requires a `SAFETY` comment** explaining invariants
- **Make illegal states unrepresentable** — Use enums, not booleans
- **Newtypes for domain primitives** — `UserId(u64)`, `PhysAddr(u64)`, `VirtAddr(u64)`

### Concurrency (SMP)

- **`spinlock::Mutex` / `spinlock::RwLock`** — Not `std::sync`
- **IPI for cross-core communication**
- **Per-core runqueues** — No global scheduler lock
- **Disable interrupts** around critical sections, not spinlocks

### Error Handling

- **Custom error enums** — No `anyhow` in kernel space
- **`Result<T, E>`** — Explicit error paths, not panics
- **Add context** — `"failed to map page at {addr}"` not `"mmap failed"`

### Function Design

- **≤ 40 lines** per function
- **≤ 3 nesting levels** — Use guard clauses, early returns
- **No magic numbers** — Extract constants with descriptive names
- **Single responsibility** — Testable, composable

### Naming

| Element | Convention |
|---------|------------|
| Functions, variables | `snake_case` |
| Constants | `SCREAMING_SNAKE_CASE` |
| Types, traits | `PascalCase` |
| Modules, files | `snake_case` |
| Booleans | `is_`, `has_`, `should_`, `can_` prefix |

### Comments

- **Explain why, not what** — Code should be self-documenting
- **Document invariants** — Especially for `unsafe` code
- **Doc comments on all public items** — `///` for rustdoc
- **No TODO comments** — Create an issue or implement now
- **No section comments** — Extract a function instead

## Testing

### Test Organization

- **Unit tests**: `#[cfg(test)] mod tests` inside each module
- **Integration tests**: `tests/integration.rs` (runs in QEMU)
- **Test helpers**: Extract fixtures/factories into `tests/common/`

### Test Requirements

- **New code: ≥ 80% line coverage** (enforced in CI for changed files)
- **Test error paths** — Not just happy path
- **Deterministic data** — No `random()` in tests
- **Isolated tests** — Each test creates its own state

### Running Tests

```bash
# Host unit tests (fast)
cargo test --features std

# QEMU integration tests (slower, runs in CI)
cargo test --target aarch64-unknown-none --features qemu --test integration

# Specific test
cargo test --features std memory::frame_alloc::tests::alloc_and_free
```

### Property-Based Testing

For memory allocators, page tables, schedulers — use `proptest` to verify invariants.

## Architecture Overview

LobsterOS is organized by **domain**, not by layer. Key domains:

| Domain | Directory | Responsibility |
|--------|-----------|----------------|
| Architecture | `src/arch/` | AArch64, BCM2712 specifics (MMU, GIC, timer, exceptions) |
| Memory | `src/memory/` | Frame allocator, heap, virtual memory, process address spaces |
| Scheduler | `src/sched/` | Processes, threads, syscalls, signals, ELF loader, dynamic linker |
| SMP | `src/smp/` | Per-core runqueues, IPI, load balancing |
| Networking | `src/net/` | GENET, ARP, IPv4, ICMP, UDP, TCP, DHCP, DNS, HTTP |
| Display | `src/display/` | Framebuffer, graphics, console, window manager |
| Filesystems | `src/fs/` | VFS, FAT32, ext2, initrd, procfs, sysfs, devfs |
| USB | `src/usb/` | XHCI, HID (keyboard/mouse) |
| Security | `src/security/` | Capabilities, users/groups, auth, sessions |
| Shell | `src/shell/` | Interactive shell, built-in commands |
| IPC | `src/ipc/` | Message passing, shared memory, PTY |

### Key Design Principles

1. **Message passing over shared state** — Channels preferred over `Arc<Mutex<T>>`
2. **Capability-based security** — Fine-grained permissions, not ambient authority
3. **Explicit resource management** — RAII via `Drop`, no implicit globals
4. **Testable boundaries** — Pure logic separated from hardware side effects

## Good First Issues

We curate issues specifically for new contributors. Look for these labels:

| Label | Meaning |
|-------|---------|
| `good first issue` | Well-scoped, mentored, ideal for first PR |
| `help wanted` | Maintainers need help, but may be larger scope |
| `documentation` | Docs-only, no Rust knowledge required |
| `easy` | Low complexity, good learning opportunity |

### Current Good First Issues

*Check the [issue tracker](https://github.com/yourusername/lobster-os/issues?q=label%3A%22good+first+issue%22) for the latest list.*

Typical examples:
- Add missing doc comments to public APIs in `src/memory/`
- Write a `README.md` for a subsystem (e.g., `src/usb/`)
- Add unit tests for `src/sync/spinlock.rs`
- Improve error messages in `src/fs/vfs.rs`
- Add a new shell command (see `src/shell/commands/`)

Each good first issue includes:
- Clear acceptance criteria
- Pointers to relevant files
- Mentor availability (maintainer will guide you)

## Communication

### Channels

- **GitHub Issues** — Bug reports, feature requests, design discussions
- **GitHub Discussions** — Questions, ideas, general chat (preferred for questions)
- **Pull Requests** — Code review, implementation discussion

### Response Times

- **Issue triage**: Within 72 hours
- **PR first review**: Within 48 hours
- **Follow-up reviews**: Within 24 hours

### Etiquette

- Be respectful and constructive
- Assume good intent
- Ask questions early — don't struggle in silence
- Update your PR promptly after feedback

## Recognition

All contributors are recognized in:

- **`AUTHORS.md`** — List of all contributors (auto-updated via AllContributors bot)
- **Release notes** — Notable contributions highlighted
- **GitHub contributor graph** — Visible on repository page

We value **all** contributions: code, docs, tests, bug reports, ideas, reviews, and community support.

---

## Quick Reference: Common Commands

```bash
# Format check
cargo fmt --all -- --check

# Lint (zero warnings required)
cargo clippy --features qemu -- -D warnings

# Build for QEMU
cargo build --features qemu --target aarch64-unknown-none

# Run in QEMU
qemu-system-aarch64 -M raspi4b -kernel target/aarch64-unknown-none/debug/lobster-os -serial stdio -display none

# Host tests
cargo test --features std

# QEMU integration tests
cargo test --target aarch64-unknown-none --features qemu --test integration

# Check for unused dependencies
cargo machete

# Generate documentation
cargo doc --features qemu --target aarch64-unknown-none --no-deps --open
```

---

**Questions?** Open a [GitHub Discussion](https://github.com/yourusername/lobster-os/discussions) or comment on an issue. We're happy to help!