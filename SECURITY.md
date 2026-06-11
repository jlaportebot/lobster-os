# Security Policy

## Reporting a Vulnerability

Found a security issue? **Don't open a public issue.**

Email **security@lobster-os.org** (or DM a maintainer on GitHub) with:
- What you found
- How to reproduce it
- Potential impact
- Any suggested fix

We'll acknowledge within 48 hours and keep you updated.

## Supported Versions

| Version | Supported |
|---------|-----------|
| `master` (latest) | ✅ Yes |
| Older releases | ❌ No |

LobsterOS moves fast. Security fixes land on `master` first.

## What We Consider Security Issues

- Memory safety bugs in kernel code (use-after-free, buffer overflows, etc.)
- Capability system bypasses
- Authentication/authorization flaws
- Network stack vulnerabilities (TCP/IP, DHCP, DNS)
- Driver bugs exploitable from userspace or hardware
- Supply chain / dependency issues

## What We Don't Consider Security Issues

- Bugs requiring physical access to the Pi
- Denial-of-service without privilege escalation
- Issues in unused/experimental features (marked `🚧` in README)
- Side-channel attacks (out of scope for this project)

## Disclosure Timeline

1. **Day 0**: You report privately
2. **Day 1-2**: We confirm and triage
3. **Day 7-30**: Fix developed and tested
4. **Day 30+**: Public disclosure with CVE (if applicable), fix released

We credit reporters by default (unless you ask to stay anonymous).

## Security-Focused Development

- All kernel code is `#![no_std]` — no standard library attack surface
- `panic = "abort"` in all profiles — no unwinding exploits
- Custom allocators, no `libc`
- Capability-based access control (not ambient authority)
- All `unsafe` blocks have `SAFETY` comments explaining invariants
- Fuzzing targets for parsers (network, filesystem, ELF)
- Property-based tests for memory allocator / page tables

## Dependencies

We vendor minimal dependencies. All crates:
- Are widely used in embedded Rust ecosystem
- Have active maintenance
- Are audited via `cargo audit` in CI

Run locally:
```bash
cargo audit
cargo deny check
```