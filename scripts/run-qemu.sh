#!/bin/bash
# CyanOS Cross-Platform QEMU Runner Script
# Boots CyanOS on both AArch64 and x86_64 architectures using Limine UEFI

set -e  # Exit on any error

# Default to AArch64 if no architecture specified
ARCH="${1:-aarch64}"

# Validate architecture
case "$ARCH" in
    aarch64|arm64)
        ARCH="aarch64"
        DISK_IMAGE="cyanos-limine-aarch64.img"
        UEFI_FIRMWARE="/opt/homebrew/share/qemu/edk2-aarch64-code.fd"
        QEMU_SYSTEM="qemu-system-aarch64"
        MACHINE_ARGS="-machine virt -cpu cortex-a57"
        ;;
    x86_64|amd64)
        ARCH="x86_64"
        DISK_IMAGE="cyanos-limine-x86_64.img"
        UEFI_FIRMWARE="/opt/homebrew/share/qemu/edk2-x86_64-code.fd"
        QEMU_SYSTEM="qemu-system-x86_64"
        MACHINE_ARGS="-machine q35"
        ;;
    *)
        echo "❌ Unsupported architecture: $ARCH"
        echo "💡 Usage: $0 [aarch64|x86_64|amd64]"
        echo "   Examples:"
        echo "     $0 aarch64    # Boot AArch64 CyanOS"
        echo "     $0 x86_64     # Boot x86_64 CyanOS"
        echo "     $0            # Boot AArch64 CyanOS (default)"
        exit 1
        ;;
esac

echo "🚀 Starting CyanOS ($ARCH) with Limine UEFI bootloader"
echo "====================================================="

# Check if disk image exists
if [ ! -f "$DISK_IMAGE" ]; then
    echo "❌ Disk image not found: $DISK_IMAGE"
    echo "💡 Run './scripts/build-all.sh --arch $ARCH' to build the disk image"
    exit 1
fi

# Check if UEFI firmware exists
if [ ! -f "$UEFI_FIRMWARE" ]; then
    echo "❌ UEFI firmware not found: $UEFI_FIRMWARE"
    echo "💡 Install QEMU with Homebrew: brew install qemu"
    exit 1
fi

# Check if QEMU system is available
if ! command -v "$QEMU_SYSTEM" &> /dev/null; then
    echo "❌ QEMU system not found: $QEMU_SYSTEM"
    echo "💡 Install QEMU with Homebrew: brew install qemu"
    exit 1
fi

echo "🏗️  Architecture: $ARCH"
echo "📁 Using disk image: $DISK_IMAGE"
echo "🔧 Using UEFI firmware: $UEFI_FIRMWARE"
echo "⚡ Using QEMU: $QEMU_SYSTEM"
echo ""
echo "🎮 Boot sequence:"
echo "   1. EDK2 UEFI firmware initializes"
echo "   2. Limine bootloader loads from EFI partition"
echo "   3. CyanOS kernel boots with runtime initrd loading"
echo "   4. Init process starts → '@' debug marker appears"
echo ""
echo "⏹️  Press Ctrl+C to exit QEMU"
echo "🔄 Booting..."
echo ""

# Launch QEMU with architecture-specific parameters
if [ "$ARCH" = "aarch64" ]; then
    exec $QEMU_SYSTEM \
        $MACHINE_ARGS \
        -m 256M \
        -nographic \
        -drive file="$DISK_IMAGE",format=raw,if=none,id=drive0 \
        -device virtio-blk-device,drive=drive0 \
        -bios "$UEFI_FIRMWARE"
else
    # x86_64
    exec $QEMU_SYSTEM \
        $MACHINE_ARGS \
        -m 256M \
        -nographic \
        -serial mon:stdio \
        -drive "if=pflash,format=raw,readonly=on,file=$UEFI_FIRMWARE" \
        -drive "format=raw,file=$DISK_IMAGE" \
        -no-reboot
fi
