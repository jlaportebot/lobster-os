# ═══════════════════════════════════════════════════════════════════════
# LobsterOS — Makefile for BCM2712 (Raspberry Pi 5) bare-metal kernel
# ═══════════════════════════════════════════════════════════════════════
#
# Targets:
#   make            — build release kernel8.img
#   make dev        — build debug kernel8.img
#   make qemu       — run in QEMU (Raspi4b model)
#   make flash      — copy kernel8.img to SD card mount
#   make clean      — remove build artifacts
#   make size       — show kernel size breakdown
#   make objdump    — disassemble the kernel
#   make check      — cargo check only (no binary)

# ─── Toolchain ─────────────────────────────────────────────────────────
CROSS   ?= aarch64-linux-gnu-
OBJCOPY ?= $(CROSS)objcopy
OBJDUMP ?= $(CROSS)objdump
SIZE    ?= $(CROSS)size

# ─── Paths ─────────────────────────────────────────────────────────────
TARGET  := aarch64-unknown-none
BUILDDIR:= target/$(TARGET)
BINDIR  := build
IMG     := $(BINDIR)/kernel8.img
ELF     := $(BINDIR)/kernel8.elf
MAP     := $(BINDIR)/kernel8.map
DIS     := $(BINDIR)/kernel8.dis

# SD card mount point (adjust for your system)
SD_MOUNT ?= /mnt/sd/boot

# QEMU settings
QEMU    ?= qemu-system-aarch64
QEMU_M  := raspi4b
QEMU_MEM:= 1G
QEMU_SERIAL := -serial stdio -serial null -serial null -serial null

# ─── Phony targets ─────────────────────────────────────────────────────
.PHONY: all dev qemu qemu-debug flash clean size objdump check clippy

# ═══════════════════════════════════════════════════════════════════════
# Default: release build
# ═══════════════════════════════════════════════════════════════════════
all: $(IMG)

# ─── Release build ─────────────────────────────────────────────────────
$(BUILDDIR)/release/lobster-os:
	cargo build --release --target $(TARGET)

$(ELF): $(BUILDDIR)/release/lobster-os
	@mkdir -p $(BINDIR)
	cp $< $@

$(IMG): $(ELF)
	$(OBJCOPY) -O binary $< $@
	@echo ""
	@echo "╔══════════════════════════════════════════════════╗"
	@echo "║  LobsterOS kernel8.img built successfully!       ║"
	@echo "╚══════════════════════════════════════════════════╝"
	@ls -lh $@
	@echo ""

# ─── Debug build ───────────────────────────────────────────────────────
dev: $(BINDIR)/kernel8-debug.img

$(BUILDDIR)/debug/lobster-os:
	cargo build --target $(TARGET)

$(BINDIR)/kernel8-debug.elf: $(BUILDDIR)/debug/lobster-os
	@mkdir -p $(BINDIR)
	cp $< $@

$(BINDIR)/kernel8-debug.img: $(BINDIR)/kernel8-debug.elf
	$(OBJCOPY) -O binary $< $@
	@echo "[dev] Built kernel8-debug.img"
	@ls -lh $@

# ═══════════════════════════════════════════════════════════════════════
# QEMU emulation
# ═══════════════════════════════════════════════════════════════════════
qemu: $(IMG)
	$(QEMU) -M $(QEMU_M) -m $(QEMU_MEM) $(QEMU_SERIAL) -kernel $<

qemu-debug: $(BINDIR)/kernel8-debug.img
	$(QEMU) -M $(QEMU_M) -m $(QEMU_MEM) $(QEMU_SERIAL) \
		-d in_asm -D $(BINDIR)/qemu-debug.log \
		-kernel $<

# ═══════════════════════════════════════════════════════════════════════
# Flash to SD card
# ═══════════════════════════════════════════════════════════════════════
flash: $(IMG)
	@echo "Copying kernel8.img to $(SD_MOUNT)..."
	cp $< $(SD_MOUNT)/kernel8.img
	@echo "Done. Syncing..."
	sync
	@echo "SD card ready. Safe to eject."

# ═══════════════════════════════════════════════════════════════════════
# Analysis targets
# ═══════════════════════════════════════════════════════════════════════
size: $(ELF)
	$(SIZE) $<
	@echo ""
	@echo "Section breakdown:"
	$(CROSS)readelf -S $< | grep -E "\.text|\.rodata|\.data|\.bss|\.stack" || true

objdump: $(DIS)

$(DIS): $(ELF)
	$(OBJDUMP) -d $< > $@
	@echo "Disassembly written to $@"

# ═══════════════════════════════════════════════════════════════════════
# Lint / check (fast, no binary)
# ═══════════════════════════════════════════════════════════════════════
check:
	cargo check --target $(TARGET)

clippy:
	cargo clippy --target $(TARGET) -- -D warnings 2>&1 || true

# ═══════════════════════════════════════════════════════════════════════
# Clean
# ═══════════════════════════════════════════════════════════════════════
clean:
	cargo clean
	rm -rf $(BINDIR)
	@echo "Cleaned build artifacts."
