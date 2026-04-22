#!/bin/bash
# CyanOS Cross-Platform Build Script
# Builds userland, kernels for both AArch64 and x86_64, creates initrds, and generates disk images

set -e  # Exit on any error

# Default configuration
DEFAULT_ARCH="both"
DEFAULT_LIMINE_VERSION="11.4.1"
LIMINE_CACHE_DIR=".limine-cache"

# Parse command line arguments
ARCH="$DEFAULT_ARCH"
LIMINE_VERSION="$DEFAULT_LIMINE_VERSION"

show_usage() {
    echo "Usage: $0 [OPTIONS]"
    echo "Options:"
    echo "  --arch ARCH          Build for specific architecture: aarch64, x86_64, or both (default: both)"
    echo "  --limine-version VER Limine version to use (default: 11.4.1)"
    echo "  --help               Show this help message"
    echo ""
    echo "Examples:"
    echo "  $0                                    # Build for both architectures"
    echo "  $0 --arch aarch64                    # Build only for AArch64"
    echo "  $0 --arch x86_64 --limine-version 11.3.0  # Build only for x86_64 with Limine 11.3.0"
}

while [[ $# -gt 0 ]]; do
    case $1 in
        --arch)
            ARCH="$2"
            if [[ "$ARCH" != "aarch64" && "$ARCH" != "x86_64" && "$ARCH" != "both" ]]; then
                echo "❌ Invalid architecture: $ARCH. Must be aarch64, x86_64, or both"
                exit 1
            fi
            shift 2
            ;;
        --limine-version)
            LIMINE_VERSION="$2"
            shift 2
            ;;
        --help)
            show_usage
            exit 0
            ;;
        *)
            echo "❌ Unknown option: $1"
            show_usage
            exit 1
            ;;
    esac
done

echo "🚀 CyanOS Cross-Platform Build Process Started"
echo "=============================================="
echo "🏗️  Architecture(s): $ARCH"
echo "📦 Limine version: $LIMINE_VERSION"
echo ""

# Function to download and cache Limine
download_limine() {
    local version=$1
    local cache_dir="$LIMINE_CACHE_DIR/limine-$version-binary"

    if [ -d "$cache_dir" ]; then
        echo "✅ Using cached Limine $version from $cache_dir"
        return 0
    fi

    echo "📥 Downloading Limine $version..."
    mkdir -p "$LIMINE_CACHE_DIR"

    # Extract major version for branch name (e.g., 11.4.1 -> 11)
    local major_version=$(echo "$version" | cut -d'.' -f1)
    local branch_name="v${major_version}.x-binary"
    local url="https://github.com/limine-bootloader/limine/archive/refs/heads/$branch_name.tar.gz"

    cd "$LIMINE_CACHE_DIR"
    echo "  Downloading from branch: $branch_name"
    curl -L -o "limine-$version-binary.tar.gz" "$url"

    if [ $? -ne 0 ]; then
        echo "❌ Failed to download Limine $version from $url"
        cd ..
        exit 1
    fi

    tar -xzf "limine-$version-binary.tar.gz"

    if [ $? -ne 0 ]; then
        echo "❌ Failed to extract Limine $version"
        cd ..
        exit 1
    fi

    # Rename the extracted directory to expected name (GitHub capitalizes repo name and strips v prefix)
    local extracted_dir_name="Limine-${major_version}.x-binary"
    mv "$extracted_dir_name" "limine-$version-binary"
    rm "limine-$version-binary.tar.gz"
    cd ..

    echo "✅ Limine $version downloaded and cached"
}

# Function to build userland for a specific architecture
build_userland() {
    local arch=$1
    echo "📦 Building $arch userland programs (init, shell, hello)..."

    if [[ "$arch" == "aarch64" ]]; then
        ./scripts/build-userland.sh --release
    else
        ./scripts/build-userland.sh --target amd64 --release
    fi

    if [ $? -ne 0 ]; then
        echo "❌ $arch userland build failed"
        exit 1
    fi
    echo "✅ $arch userland build complete"
}

# Function to create initrd using gzipped cpio (Newc format)
create_initrd() {
    local arch=$1
    local initrd_name="initrd-$arch.cpio.gz"

    echo "📦 Creating $arch initrd using gzipped cpio (Newc format)..."

    # Map architecture names for directory structure
    local target_arch
    if [[ "$arch" == "aarch64" ]]; then
        target_arch="aarch64-unknown-none"
    else
        target_arch="x86_64-unknown-none"
    fi

    local userland_dir="userland/target/$target_arch/release"

    if [ ! -d "$userland_dir" ]; then
        echo "❌ Userland directory not found: $userland_dir"
        exit 1
    fi

    # Create temporary directory for initrd contents
    local temp_dir=$(mktemp -d)
    cp "$userland_dir/init" "$temp_dir/"
    cp "$userland_dir/shell" "$temp_dir/"
    cp "$userland_dir/hello" "$temp_dir/"

    # Create cpio archive with Newc format and gzip it
    local original_dir=$(pwd)
    cd "$temp_dir"
    find . -print0 | cpio --null -ov --format=newc | gzip > "$original_dir/$initrd_name"
    cd "$original_dir"

    rm -rf "$temp_dir"

    if [ ! -f "$initrd_name" ]; then
        echo "❌ Failed to create initrd: $initrd_name"
        exit 1
    fi

    echo "✅ $arch initrd created: $initrd_name"
}

# Function to build kernel for a specific architecture
build_kernel() {
    local arch=$1
    echo "🔧 Building $arch kernel in release mode..."

    local target
    if [[ "$arch" == "aarch64" ]]; then
        target="targets/aarch64-unknown-kernel.json"
    else
        target="targets/x86_64-unknown-kernel.json"
    fi

    cargo +nightly rustc --package kernel --target "$target" --release -Z build-std=core,alloc -Zbuild-std-features=compiler-builtins-mem -Zjson-target-spec -- -C link-arg=-z -C link-arg=max-page-size=0x1000
    if [ $? -ne 0 ]; then
        echo "❌ $arch kernel build failed"
        exit 1
    fi
    echo "✅ $arch kernel build complete"
}

# Function to create disk image using dd, mcopy, mmd
create_disk_image() {
    local arch=$1
    local limine_dir="$2"
    local image_name="cyanos-limine-$arch.img"

    echo "💽 Creating $arch Limine UEFI disk image..."

    # Remove existing disk image
    rm -f "$image_name"

    # Create 64MB disk image with FAT32 filesystem
    echo "  Creating 64MB $arch disk image with FAT32..."
    dd if=/dev/zero of="$image_name" bs=1M count=64 2>/dev/null
    mkfs.fat -F 32 -n CYANOS "$image_name" >/dev/null 2>&1

    # Create directory structure
    echo "  Creating directory structure..."
    mmd -i "$image_name" ::/EFI
    mmd -i "$image_name" ::/EFI/BOOT

    # Copy appropriate UEFI bootloader
    echo "  Copying UEFI bootloader..."
    if [[ "$arch" == "aarch64" ]]; then
        mcopy -oi "$image_name" "$limine_dir/BOOTAA64.EFI" ::/EFI/BOOT/BOOTAA64.EFI
        # Copy kernel and initrd
        mcopy -oi "$image_name" "target/aarch64-unknown-kernel/release/kernel" ::/cyanos-kernel
        mcopy -oi "$image_name" "initrd-aarch64.cpio.gz" ::/initrd-raw.bin
    else
        mcopy -oi "$image_name" "$limine_dir/BOOTX64.EFI" ::/EFI/BOOT/BOOTX64.EFI
        # Copy kernel and initrd
        mcopy -oi "$image_name" "target/x86_64-unknown-kernel/release/kernel" ::/cyanos-kernel
        mcopy -oi "$image_name" "initrd-x86_64.cpio.gz" ::/initrd-raw.bin
    fi

    # Copy Limine configuration
    echo "  Copying Limine configuration..."
    mcopy -oi "$image_name" limine/limine.conf ::/limine.conf

    echo "✅ $arch Limine UEFI disk image created: $image_name"
}

# Main build process

# Step 1: Download and cache Limine
download_limine "$LIMINE_VERSION"
LIMINE_DIR="$LIMINE_CACHE_DIR/limine-$LIMINE_VERSION-binary"

# Step 2: Build userland, kernel, and create images for requested architectures
if [[ "$ARCH" == "both" || "$ARCH" == "aarch64" ]]; then
    echo ""
    echo "🔷 Building AArch64 components..."
    build_userland "aarch64"
    create_initrd "aarch64"
    build_kernel "aarch64"
    create_disk_image "aarch64" "$LIMINE_DIR"
fi

if [[ "$ARCH" == "both" || "$ARCH" == "x86_64" ]]; then
    echo ""
    echo "🔶 Building x86_64 components..."
    build_userland "x86_64"
    create_initrd "x86_64"
    build_kernel "x86_64"
    create_disk_image "x86_64" "$LIMINE_DIR"
fi

# Summary
echo ""
echo "🎉 Cross-Platform Build Complete!"
echo "================================="
echo "📁 Generated files:"

if [[ "$ARCH" == "both" || "$ARCH" == "aarch64" ]]; then
    echo "   🔷 AArch64 Architecture:"
    echo "      - target/aarch64-unknown-kernel/release/kernel (ELF kernel)"
    echo "      - initrd-aarch64.cpio.gz (gzipped cpio initrd)"
    echo "      - cyanos-limine-aarch64.img (64MB UEFI disk image)"
fi

if [[ "$ARCH" == "both" || "$ARCH" == "x86_64" ]]; then
    echo "   🔶 x86_64 Architecture:"
    echo "      - target/x86_64-unknown-kernel/release/kernel (ELF kernel)"
    echo "      - initrd-x86_64.cpio.gz (gzipped cpio initrd)"
    echo "      - cyanos-limine-x86_64.img (64MB UEFI disk image)"
fi

echo "   📦 Shared:"
echo "      - limine/limine.conf (bootloader configuration)"
echo "      - $LIMINE_CACHE_DIR/limine-$LIMINE_VERSION-binary/ (cached Limine binaries)"
echo ""
echo "🚀 Ready to boot!"

if [[ "$ARCH" == "both" ]]; then
    echo "   AArch64: qemu-system-aarch64 -machine virt -cpu cortex-a57 -m 256M -nographic -drive file=cyanos-limine-aarch64.img,format=raw"
    echo "   x86_64:  qemu-system-x86_64 -machine q35 -cpu qemu64 -m 256M -nographic -drive file=cyanos-limine-x86_64.img,format=raw"
elif [[ "$ARCH" == "aarch64" ]]; then
    echo "   AArch64: qemu-system-aarch64 -machine virt -cpu cortex-a57 -m 256M -nographic -drive file=cyanos-limine-aarch64.img,format=raw"
else
    echo "   x86_64:  qemu-system-x86_64 -machine q35 -cpu qemu64 -m 256M -nographic -drive file=cyanos-limine-x86_64.img,format=raw"
fi