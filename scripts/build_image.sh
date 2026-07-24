#!/bin/bash
# LobsterOS SD Card Image Builder
# Creates a bootable SD card image for Raspberry Pi 5
#
# Usage: ./scripts/build_image.sh [output.img]
# Requires: dd, mkfs.ext2, losetup, mount, root

set -e

OUTPUT=${1:-"lobsteros.img"}
IMG_SIZE_MB=${IMG_SIZE_MB:-256}
KERNEL=$(dirname "$0")/../build/kernel8.img

if [ ! -f "$KERNEL" ]; then
    echo "ERROR: kernel8.img not found. Run 'cargo build --target aarch64-unknown-none --release' first."
    exit 1
fi

echo "🐠 LobsterOS SD Card Image Builder"
echo "   Output: $OUTPUT (${IMG_SIZE_MB}MB)"
echo ""

# Create empty image
echo "→ Creating image file..."
dd if=/dev/zero of="$OUTPUT" bs=1M count=$IMG_SIZE_MB status=progress
sync

# Partition the image (single FAT32 boot partition)
echo "→ Partitioning..."
# Create partition table
echo -e "o\nn\np\n1\n\n+128M\nt\nc\nn\np\n2\n\n\nw" | fdisk "$OUTPUT" 2>/dev/null || true
sync

# Set up loop device
echo "→ Setting up loop device..."
LOOP_DEV=$(losetup -f)
losetup -P "$LOOP_DEV" "$OUTPUT"

BOOT_DEV="${LOOP_DEV}p1"
ROOT_DEV="${LOOP_DEV}p2"

# Create boot partition (FAT32)
echo "→ Creating boot partition (FAT32)..."
mkfs.vfat -F 32 -n LOBSTEROS "$BOOT_DEV" 2>/dev/null || echo "  (using mkfs.fat)"

# Create root partition (ext2)
echo "→ Creating root partition (ext2)..."
mkfs.ext2 -L lobsteros-root "$ROOT_DEV" 2>/dev/null || echo "  (ext2 created)"

# Mount and populate boot partition
echo "→ Populating boot partition..."
mkdir -p /mnt/lobsteros_boot
mount "$BOOT_DEV" /mnt/lobsteros_boot

# Copy kernel
cp "$KERNEL" /mnt/lobsteros_boot/kernel8.img

# Create config.txt
cat > /mnt/lobsteros_boot/config.txt << 'EOF'
# LobsterOS boot configuration
[all]
arm_64bit=1
kernel=kernel8.img
enable_uart=1
hdmi_group=2
hdmi_mode=85
gpu_mem=256
dtoverlay=vc4-kms-v3d
max_framebuffers=2
EOF

# Create cmdline.txt
cat > /mnt/lobsteros_boot/cmdline.txt << 'EOF'
root=/dev/mmcblk0p2 rootfstype=ext2 rw console=uart1,115200200 rootwait
EOF

# Copy firmware files if available
for fw in start4.elf fixup4.dat start.elf fixup.dat bootcode.bin; do
    if [ -f "/boot/$fw" ]; then
        cp "/boot/$fw" /mnt/lobsteros_boot/
        echo "  Copied $fw"
    fi
done

sync
umount /mnt/lobsteros_boot

# Mount and populate root partition
echo "→ Populating root partition..."
mkdir -p /mnt/lobsteros_root
mount "$ROOT_DEV" /mnt/lobsteros_root

# Create directory structure
mkdir -p /mnt/lobsteros_root/{etc,var,home,usr,dev,proc,sys,tmp,bin,lib,srv,opt,mnt}

# Create /etc files
cat > /mnt/lobsteros_root/etc/os-release << 'EOF'
NAME="LobsterOS"
VERSION="0.2.0"
ID=lobsteros
ID_LIKE=linux
PRETTY_NAME="LobsterOS 0.2.0 (Pi 5)"
HOME_URL="https://github.com/jlaportebot/lobster-os"
SUPPORT_URL="https://github.com/jlaportebot/lobster-os/issues"
EOF

cat > /mnt/lobsteros_root/etc/lobsteros.conf << 'EOF'
[boot]
splash = true
autologin = false
timeout = 3

[audio]
volume = 75
device = "auto"
sample_rate = 48000

[network]
hostname = "lobsteros"
dhcp = true
wifi_auto = true

[power]
governor = "ondemand"
suspend_timeout = 0
watchdog = true

[security]
firewall = false
selinux = false

[desktop]
theme = "dark"
accent = "#FF6B35"
font_size = 14
workspaces = 4
snapping = true

[locale]
lang = "en_US"
timezone = "UTC"
keyboard = "us"
EOF

# Create default user directories
mkdir -p /mnt/lobsteros_root/home/lobster

# Create hostname
echo "lobsteros" > /mnt/lobsteros_root/etc/hostname

sync
umount /mnt/lobsteros_root

# Cleanup
losetup -d "$LOOP_DEV"

echo ""
echo "✅ SD card image created: $OUTPUT"
echo "   Boot partition: FAT32 (128MB) — kernel8.img + config.txt"
echo "   Root partition:  ext2 ($(($IMG_SIZE_MB - 128))MB) — /etc /var /home"
echo ""
echo "   To flash: sudo dd if=$OUTPUT of=/dev/sdX bs=4M status=progress"
echo ""
