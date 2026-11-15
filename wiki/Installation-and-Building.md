# Installation & Building

This guide describes how to set up the host development environment, compile the kernel and userspace components from source, build bootable ISO images, and test **Dunit OS (Green Tea)** using QEMU.

---

## 1. Host Prerequisites

Dunit OS requires a modern 64-bit Linux distribution (such as Arch Linux, Ubuntu, or Fedora).

### Package Dependencies

Install the necessary build utilities and emulator tools:

* **Arch Linux**:
  ```bash
  sudo pacman -S base-devel nasm xorriso mtools qemu-system-x86 lld python
  ```
* **Ubuntu / Debian**:
  ```bash
  sudo apt-get update
  sudo apt-get install build-essential nasm xorriso mtools qemu-system-x86 lld python3
  ```
* **Fedora**:
  ```bash
  sudo dnf install @development-tools nasm xorriso mtools qemu-system-x86 lld python3
  ```

### Rust Nightly Toolchain
The kernel and native runtime rely on unstable Rust features (`no_std`, naked functions, custom target JSON specs):
```bash
# Install rustup if not already present
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Configure nightly toolchain and source components
rustup toolchain install nightly
rustup component add rust-src --toolchain nightly
rustup component add llvm-tools-preview --toolchain nightly
```

---

## 2. Obtaining the Source Code

Clone the official repository with submodules:
```bash
git clone https://github.com/DunitOS/dunit-os.git
cd dunit-os
git submodule update --init --recursive
```

Directory structure overview:
* `hal/`: C and NASM low-level CPU initialization, GDT/IDT, syscall trampolines.
* `kernel/`: Core Rust `no_std` kernel (memory, scheduling, VFS, IPC, ELF loader).
* `userspace/`: System libraries (`libdunit`) and core user applications.
* `assets/`: Wallpapers, fonts, DWM styles (`.dss`), and icons.
* `tools/`: Build scripts, test harnesses, disk installers, and SDK tools.

---

## 3. Building the Bootable ISO

Dunit OS provides flexible build targets:

### Standard Build (`make iso`)
Compiles the HAL, Rust microkernel, userspace applications, packs the initial ramdisk (`initrd.img`), and generates a bootable hybrid ISO (`build/dunit.iso`):
```bash
make iso
```

### Full Desktop Application Bundle
To build an image containing the complete DWM v2 desktop and ported third-party software (Doom, Quake, Minecraft C Edition, GrafX2, Lite XL, Video Player, Tracker):
```bash
./build_bundle_iso.sh
```
This produces `dwm_bundle.iso`, embedding the full userspace overlay and application suite.

---

## 4. Running in QEMU

### Recommended QEMU Parameters
To launch Dunit OS with optimal performance and complete serial output:

```bash
qemu-system-x86_64 \
  -machine q35,accel=kvm \
  -cpu host \
  -m 1024M \
  -cdrom build/dunit.iso \
  -boot d \
  -device VGA,vgamem_mb=32 \
  -serial stdio \
  -display sdl,show-cursor=off
```

### Key Parameter Explanations
* `-machine q35,accel=kvm`: Enables modern PCI Express chipset emulation and KVM hardware virtualization.
* `-cpu host`: Exposes host CPU features directly to the guest (including SSE4, AVX, and FXSAVE instructions).
* `-device VGA,vgamem_mb=32`: Configures standard high-resolution linear framebuffer emulation.
* `-serial stdio`: Redirects kernel debug logs to the terminal stdout in real-time.
* `-display sdl,show-cursor=off`: Uses SDL rendering and hides the host cursor to let Dunit OS render its own hardware cursor.

### Fallback for Systems Without KVM (TCG Emulation)
If running inside a nested virtual machine or environment without KVM access:
```bash
qemu-system-x86_64 \
  -machine q35 \
  -cpu max \
  -m 1024M \
  -cdrom build/dunit.iso \
  -boot d \
  -device VGA,vgamem_mb=32 \
  -serial stdio
```

---

## 5. Automated Verification Harness

Dunit OS enforces deterministic CI validation using automated test scripts:

### Running the QEMU Test Runner
```bash
python3 tools/qemu_test.py --markers-file tools/boot_markers.json
```
The test harness boots the operating system headlessly, listens on the serial port, verifies that all expected architectural markers are logged (such as `[HAL] OK`, `[PMM] OK`, `[SCHED] UP round-robin init`), verifies that no forbidden panic strings appear, and automatically terminates QEMU.

### Checking Compiler Warning Budgets
To ensure code cleanliness, the codebase enforces zero tolerance for unexpected compiler warnings:
```bash
python3 tools/check_warnings.py
```

---

## 6. Installation to Hard Disk

Dunit OS can be installed directly onto a virtual disk image or physical block device using `tools/install_disk.py`:

```bash
python3 tools/install_disk.py --disk /dev/sdX --iso build/dunit.iso
```

The installer:
1. Creates a standard GPT partition table with an EFI System Partition (ESP) and a Dunit system partition.
2. Formats the system partition and deploys system files, applications, and assets.
3. Installs Limine BIOS and UEFI boot stages to the disk header for native hardware booting.

