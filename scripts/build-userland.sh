#!/usr/bin/env bash
# Build Cyanos user-space programs.
#
# Usage:
#   ./scripts/build-userland.sh           # build all userland crates
#   ./scripts/build-userland.sh --check   # type-check only (fast)
#   ./scripts/build-userland.sh --release # optimised build
#
# Output:
#   userland/target/aarch64-linux-android/[debug|release]/
#     libcyanos_libc.a    — static C runtime archive
#     hello               — example ELF binary (static, no dynamic deps)
#
# The resulting ELF can be loaded by the Cyanos ELF loader and run as a
# user-space process.  It makes raw Linux-ABI syscalls (SVC #0) which
# Cyanos's kernel dispatches through its in-kernel VFS/net/TTY servers.

set -euo pipefail
cd "$(dirname "$0")/.."

TARGET="aarch64-linux-android"
MODE="debug"
CHECK=false

for arg in "$@"; do
    case "$arg" in
        --check)   CHECK=true ;;
        --release) MODE="release" ;;
    esac
done

CARGO_ARGS=(--target "$TARGET" --manifest-path userland/Cargo.toml)

if [[ "$MODE" == "release" ]]; then
    CARGO_ARGS+=(--release)
fi

if $CHECK; then
    echo "[userland] cargo check …"
    cargo check "${CARGO_ARGS[@]}"
    echo "[userland] OK — type-check passed"
    exit 0
fi

echo "[userland] cargo build …"
cargo build "${CARGO_ARGS[@]}" \
    --config "target.${TARGET}.rustflags=[\"-C\",\"link-arg=-nostartfiles\",\"-C\",\"link-arg=-static\"]"

OUT="userland/target/${TARGET}/${MODE}"
echo ""
echo "[userland] Build complete."
echo "  Library : ${OUT}/libcyanos_libc.a"
echo "  Binary  : ${OUT}/hello"
echo ""
echo "  To inspect the ELF:"
echo "    llvm-objdump -d ${OUT}/hello | head -60"
echo "    llvm-readelf -h ${OUT}/hello"
echo ""
echo "  Stage 2 note:"
echo "  When VFS/net servers move to user space, replace cyanos-libc's"
echo "  syscall wrappers with direct IPC calls to server ports stored in"
echo "  TLS (initialised from auxv AT_CYANOS_VFS_PORT / AT_CYANOS_NET_PORT)."
