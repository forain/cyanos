#!/usr/bin/env bash
# Run the LOS kernel in QEMU (x86-64 Q35 machine via multiboot2).
#
# Prerequisites:
#   cargo build --target x86_64-unknown-none --release
#   qemu-system-x86_64

set -euo pipefail

KERNEL="${1:-target/x86_64-unknown-none/release/kernel}"

if [[ ! -f "$KERNEL" ]]; then
    echo "error: kernel binary not found at '$KERNEL'" >&2
    echo "Build with:" >&2
    echo "  cargo build --target x86_64-unknown-none --release" >&2
    exit 1
fi

exec qemu-system-x86_64 \
    -machine q35 \
    -m 256M \
    -nographic \
    -serial mon:stdio \
    -kernel "$KERNEL"
