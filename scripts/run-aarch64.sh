#!/usr/bin/env bash
# Run the LOS kernel in QEMU (AArch64 virt machine).
#
# Prerequisites:
#   cargo build --target aarch64-unknown-none --release   (requires nightly + build-std)
#   qemu-system-aarch64

set -euo pipefail

KERNEL="${1:-target/aarch64-unknown-none/release/kernel}"

if [[ ! -f "$KERNEL" ]]; then
    echo "error: kernel binary not found at '$KERNEL'" >&2
    echo "Build with:" >&2
    echo "  cargo +nightly build -Z build-std=core,compiler_builtins \\" >&2
    echo "      --target aarch64-unknown-none --release" >&2
    exit 1
fi

exec qemu-system-aarch64 \
    -machine virt \
    -cpu cortex-a57 \
    -m 256M \
    -nographic \
    -serial mon:stdio \
    -kernel "$KERNEL"
