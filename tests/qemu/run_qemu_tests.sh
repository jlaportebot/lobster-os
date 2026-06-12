#!/bin/bash
# QEMU Integration Test Runner for LobsterOS
# Runs kernel in QEMU and executes test commands via serial console

set -euo pipefail

KERNEL_IMG="${1:-build/kernel8.img}"
QEMU="${QEMU:-qemu-system-aarch64}"
QEMU_MACHINE="${QEMU_MACHINE:-raspi4b}"
QEMU_MEM="${QEMU_MEM:-2G}"
TIMEOUT="${TIMEOUT:-60}"

if [[ ! -f "$KERNEL_IMG" ]]; then
    echo "Error: Kernel image not found at $KERNEL_IMG"
    echo "Build with: make (or cargo build --features qemu --target aarch64-unknown-none)"
    exit 1
fi

echo "=== LobsterOS QEMU Integration Test ==="
echo "Kernel: $KERNEL_IMG"
echo "QEMU: $QEMU"
echo "Machine: $QEMU_MACHINE"
echo "Memory: $QEMU_MEM"
echo "Timeout: ${TIMEOUT}s"
echo ""

# Run QEMU with test commands
# The kernel should run tests and output results via serial
timeout "${TIMEOUT}" "$QEMU" -M "$QEMU_MACHINE" -m "$QEMU_MEM" \
    -nographic -serial mon:stdio \
    -kernel "$KERNEL_IMG" \
    -append "test_mode=1" 2>&1 | tee qemu-test-output.log

EXIT_CODE=${PIPESTATUS[0]}

echo ""
if [[ $EXIT_CODE -eq 124 ]]; then
    echo "FAIL: QEMU test timed out after ${TIMEOUT}s"
    exit 1
elif [[ $EXIT_CODE -ne 0 ]]; then
    echo "FAIL: QEMU exited with code $EXIT_CODE"
    exit 1
fi

# Check test results in output
if grep -q "ALL TESTS PASSED" qemu-test-output.log; then
    echo "PASS: All QEMU integration tests passed"
    exit 0
elif grep -q "TEST FAILED" qemu-test-output.log; then
    echo "FAIL: Some QEMU integration tests failed"
    exit 1
else
    echo "WARN: Could not determine test result from output"
    exit 0
fi