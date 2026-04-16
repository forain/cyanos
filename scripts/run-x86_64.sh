#!/usr/bin/env bash
# Run the Cyanos kernel in QEMU via Limine (UEFI) on x86-64.
#
# Boot path:
#   QEMU  →  OVMF (UEFI firmware)  →  Limine  →  kernel.elf
#
# First run: downloads BOOTX64.EFI from the Limine v11 binary branch
# into target/limine/ (cached).
# Every run: assembles a fresh FAT32 disk image and launches QEMU.
#
# Required packages (install once):
#   Debian/Ubuntu:  sudo apt install ovmf dosfstools mtools qemu-system-x86
#   Arch Linux:     sudo pacman -S edk2-ovmf dosfstools mtools qemu-system-x86
#   Fedora:         sudo dnf install edk2-ovmf dosfstools mtools qemu-system-x86
#
# The OVMF path can be overridden:  OVMF=/path/to/OVMF_CODE.fd ./run-x86_64.sh

set -euo pipefail

# ── Configuration ─────────────────────────────────────────────────────────────

KERNEL="${1:?Usage: run-x86_64.sh <kernel.elf>}"
LIMINE_VERSION="11.3.1"
LIMINE_EFI_URL="https://raw.githubusercontent.com/limine-bootloader/limine/v${LIMINE_VERSION}-binary/BOOTX64.EFI"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
TARGET_DIR="$REPO_ROOT/target"
LIMINE_CACHE="$TARGET_DIR/limine/$LIMINE_VERSION"
LIMINE_EFI="$LIMINE_CACHE/BOOTX64.EFI"
DISK="$TARGET_DIR/x86_64-disk.img"

# ── Helpers ───────────────────────────────────────────────────────────────────

die() { echo "ERROR: $*" >&2; exit 1; }

require_cmd() {
    command -v "$1" &>/dev/null || die "'$1' not found — install: $2"
}

# ── Locate OVMF ───────────────────────────────────────────────────────────────

find_ovmf() {
    local candidates=(
        "/usr/share/OVMF/OVMF_CODE.fd"             # Debian/Ubuntu (ovmf)
        "/usr/share/OVMF/OVMF.fd"
        "/usr/share/edk2/ovmf/OVMF_CODE.fd"        # Fedora (edk2-ovmf)
        "/usr/share/edk2-ovmf/OVMF_CODE.fd"
        "/usr/share/edk2-ovmf/x64/OVMF_CODE.fd"
        "/usr/share/ovmf/x64/OVMF_CODE.fd"         # Arch (edk2-ovmf)
        "/usr/share/qemu/OVMF.fd"
    )
    for f in "${candidates[@]}"; do
        [[ -f "$f" ]] && echo "$f" && return 0
    done
    return 1
}

OVMF="${OVMF:-}"
if [[ -z "$OVMF" ]]; then
    OVMF="$(find_ovmf)" || die "OVMF firmware not found.
  Install:  sudo apt install ovmf          (Debian/Ubuntu)
            sudo pacman -S edk2-ovmf       (Arch)
            sudo dnf install edk2-ovmf     (Fedora)
  Or set:   OVMF=/path/to/OVMF_CODE.fd"
fi

# ── Check tool dependencies ───────────────────────────────────────────────────

require_cmd mkfs.fat  "sudo apt install dosfstools"
require_cmd mmd       "sudo apt install mtools"
require_cmd mcopy     "sudo apt install mtools"
require_cmd qemu-system-x86_64 "sudo apt install qemu-system-x86"

# ── Fetch Limine BOOTX64.EFI (cached in target/limine/<version>/) ─────────────
#
# Limine v8+ uses a separate -binary branch for pre-built EFI binaries.
# We download only BOOTX64.EFI — no tarball needed for UEFI-only boots.

if [[ ! -f "$LIMINE_EFI" ]]; then
    echo "[limine] Downloading Limine $LIMINE_VERSION BOOTX64.EFI..."
    mkdir -p "$LIMINE_CACHE"

    if command -v curl &>/dev/null; then
        curl -sSL --fail "$LIMINE_EFI_URL" -o "$LIMINE_EFI"
    elif command -v wget &>/dev/null; then
        wget -qO "$LIMINE_EFI" "$LIMINE_EFI_URL"
    else
        die "Neither curl nor wget found. Install one and retry."
    fi

    [[ -f "$LIMINE_EFI" ]] || die "Failed to download BOOTX64.EFI from $LIMINE_EFI_URL"
    echo "[limine] Cached to $LIMINE_CACHE"
fi

# ── Generate limine.cfg ───────────────────────────────────────────────────────
#
# Limine v8+ config format: lowercase keys with colons, /Entry for entries.
# (v5-v7 used UPPERCASE keys and `:Entry` syntax — not compatible with v8+.)
# kernel path `boot()` means "the partition this cfg was loaded from".

LIMINE_CFG="$(mktemp)"
trap 'rm -f "$LIMINE_CFG"' EXIT

cat > "$LIMINE_CFG" <<'EOF'
timeout: 0

/Cyanos
    protocol: limine
    path: boot():/kernel.elf
    kaslr: no
EOF

# ── Build FAT32 disk image ────────────────────────────────────────────────────
#
# Layout inside the FAT32 image:
#   /EFI/BOOT/BOOTX64.EFI   — Limine UEFI application (fallback boot path)
#   /limine.cfg              — boot menu
#   /kernel.elf              — the kernel
#
# No partition table is needed; UEFI firmware can boot directly from a raw
# FAT32 image when passed as a removable drive.

DISK_SIZE_MB=64

echo "[disk] Building $DISK_SIZE_MB MiB FAT32 disk image..."
dd if=/dev/zero of="$DISK" bs=1M count="$DISK_SIZE_MB" status=none
mkfs.fat -F 32 -n CYANOS "$DISK" >/dev/null

mmd   -i "$DISK" ::/EFI
mmd   -i "$DISK" ::/EFI/BOOT
mcopy -oi "$DISK" "$LIMINE_EFI"    ::/EFI/BOOT/BOOTX64.EFI
mcopy -oi "$DISK" "$LIMINE_CFG"    ::/limine.cfg
mcopy -oi "$DISK" "$KERNEL"        ::/kernel.elf

# ── Launch QEMU ──────────────────────────────────────────────────────────────
#
# -drive if=pflash  — OVMF as a persistent flash device (standard UEFI setup)
# -drive format=raw — our disk image as a removable USB-like drive
# -no-reboot        — exit QEMU instead of rebooting (cleaner for cargo run)

echo "[qemu] Booting with OVMF: $OVMF"
exec qemu-system-x86_64 \
    -machine q35 \
    -m 256M \
    -nographic \
    -serial mon:stdio \
    -drive "if=pflash,format=raw,readonly=on,file=$OVMF" \
    -drive "format=raw,file=$DISK" \
    -no-reboot
