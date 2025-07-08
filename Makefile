APP ?= sdmmc
COUNT ?= 1024
ARCH ?= aarch64
TARGET ?= aarch64-unknown-none
DIR ?= ./firmware
KERNEL ?= $(DIR)/kernel.bin
DISK ?= $(DIR)/uboot.disk
DTB ?= $(DIR)/qemu.dtb

dtb: 
	@echo "Building device tree binary"
	@rm -f $(DTB)
	@qemu-system-$(ARCH) -M virt,dumpdtb=$(DIR)/qemu.dtb \
		-smp 1 -cpu cortex-a72 -nographic \
		-drive file=$(DISK),format=raw,if=none,id=sdmmc \
		-device sdhci-pci,id=sdhci \
		-device sd-card,drive=sdmmc
	@dtc -I dts -O dtb -o $(DTB) $(DIR)/qemu.dts

disk_img: 
	@echo "Creating disk image"
	@rm -f $(DISK)
	@if [ ! -d $(DIR) ]; then \
		mkdir $(DIR); \
	fi;
	@dd if=/dev/zero of=$(DISK) bs=1M count=$(COUNT)
	@mkfs.ext4 $(DISK)

build: 
	@echo "Building $(APP)"
	@cargo build --release
	@rust-objcopy --binary-architecture=$(ARCH) ./target/$(TARGET)/release/$(APP) --strip-all -O binary $(KERNEL)

# run: build disk_img
# 	@echo "Running QEMU ....."
# 	@qemu-system-$(ARCH) -M virt -smp 1 -cpu cortex-a72 \
# 		-kernel ./$(KERNEL) -nographic \
#     	-drive file=$(DISK),format=raw,if=none,id=disk \
#     	-device sdhci-pci,id=sdhci0 \
#     	-device sd-card,drive=disk

test: 
	@echo "Running tests" 
	@cargo test --test test -- --show-output

define run_uboot_test
	@echo "Running $(1) U-Boot tests"
	@if [ -f 'configs/$(1).bare-test.toml' ] && [ -f 'configs/$(1).test.rs' ]; then \
		cp configs/$(1).bare-test.toml .bare-test.toml; \
		cp configs/$(1).test.rs tests/test.rs; \
		cargo test --release --test test -- --show-output --uboot; \
	else \
		echo "Error: $(1) config file not found!"; \
		exit 1; \
	fi
endef

phytium_uboot:
	$(call run_uboot_test,phytium)

rockchip_uboot:
	$(call run_uboot_test,rockchip)
clean:
	@echo "Cleaning up"
	@cargo clean

PHONY: build run disk_img clean dtb test phytium_uboot rockchip_uboot
