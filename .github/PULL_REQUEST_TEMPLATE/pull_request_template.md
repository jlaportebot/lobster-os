## Description

What does this PR do? Why?

Fixes #<!-- issue number -->

## Type of Change

- [ ] Bug fix
- [ ] New feature
- [ ] Documentation update
- [ ] Refactor (no behavior change)
- [ ] Test improvement
- [ ] CI / build / tooling
- [ ] Other: _______

## Testing Done

- [ ] `cargo fmt --all -- --check` passes
- [ ] `cargo clippy --features qemu -- -D warnings` passes (zero warnings)
- [ ] `cargo build --features qemu --target aarch64-unknown-none` succeeds
- [ ] `cargo test --features std` passes
- [ ] `cargo test --target aarch64-unknown-none --features qemu --test integration` passes (or N/A)
- [ ] Manual testing: _______

## Checklist

- [ ] Linked to an issue (`Fixes #N` or `Closes #N`)
- [ ] Commits are atomic and logical (one concern per commit)
- [ ] No debug code left (`dbg!`, `println!`, `unimplemented!`, commented-out code)
- [ ] No `unwrap()` / `expect()` in production kernel paths
- [ ] All `unsafe` blocks have `SAFETY` comments
- [ ] Public API changes have doc comments (`///`)
- [ ] New code has tests (unit or integration)
- [ ] Documentation updated if user-facing behavior changed
- [ ] No new dependencies added without discussion

## Screenshots / Logs (if applicable)

Serial output, QEMU window, etc.

## Additional Notes

Anything reviewers should know — tradeoffs, follow-ups, concerns.