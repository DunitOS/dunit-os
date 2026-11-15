# Hardware & Drivers

This article describes the driver architecture and device support in **Dunit OS (Green Tea)**.

---

## 1. Driver Subsystem Architecture

Dunit OS combines low-level hardware routines in the Hardware Abstraction Layer (HAL) with modular drivers implemented in Rust:

```text
+-------------------------------------------------------------------------+
|                        Userspace / System Servers                       |
|               DWM v2 Compositor · gui_server · VFS Server               |
+------------------------------------+------------------------------------+
                                     |
                                     v (Kernel driver interfaces)
+-------------------------------------------------------------------------+
|                             Kernel Drivers                              |
|   - PS/2 Keyboard & Mouse Translator                                    |
|   - Framebuffer & Bochs VBE Display Driver                              |
|   - AHCI (SATA) & VirtIO Block Storage Drivers                          |
|   - Intel e1000 PCI Network Controller                                  |
|   - Programmable Interval Timer (PIT) & Local APIC Timer                |
+-------------------------------------------------------------------------+
                                     |
                                     v
+-------------------------------------------------------------------------+
|                  HAL Primitives (I/O Ports, MMIO, IDT)                  |
+-------------------------------------------------------------------------+
```

---

## 2. Timers & System Clock

### Programmable Interval Timer (8254 PIT)
* **Configuration**: Initialized in Mode 2 (Rate Generator) on I/O ports `0x40`–`0x43`.
* **Frequency**: Programmed for 100 Hz (10 ms tick interval).
* **Interrupt**: Bound to IRQ 0 (IDT Vector 32).
* **Purpose**: Drives preemptive context switching and the `sleep()` system call.

### Local APIC (LAPIC) Timer
* Provides high-frequency, per-core interrupt generation required for Symmetric Multiprocessing (SMP) scheduling.

---

## 3. Input Devices (8042 PS/2 Controller)

The PS/2 driver (`kernel/src/drivers/ps2.rs`) communicates via I/O port `0x60` (data) and `0x64` (status/command):

### Keyboard Driver (Port 1)
* Operates on IRQ 1 (IDT Vector 33).
* Decodes Scan Code Set 1 and Set 2.
* Tracks modifier key states (`Shift`, `Ctrl`, `Alt`, `Caps Lock`).
* Translates scancodes into printable UTF-8 characters and standard system keys (`Enter`, `Backspace`, `Escape`, `Arrows`).

### Mouse Driver (Port 2)
* Operates on IRQ 12 (IDT Vector 44).
* Decodes standard 3-byte PS/2 data packets:
  * **Byte 1**: Button flags (Left, Right, Middle), sign bits (X/Y sign), and overflow flags.
  * **Byte 2**: Relative X displacement ($\Delta X$).
  * **Byte 3**: Relative Y displacement ($\Delta Y$).
* Accumulates coordinates within linear screen boundaries ($0 \le X < \text{width}$, $0 \le Y < \text{height}$) and forwards events to `gui_server`.

---

## 4. Video & Display Adapters

### Linear Framebuffer
* Initialized during early boot via the Limine protocol.
* Configured in 32-bit True Color (ARGB/XRGB).
* Provides direct hardware memory mapping for smooth rendering.

### Bochs VBE (DISPI) Driver
* Detected via PCI device scan (`vendor = 0x1234, device = 0x1111`).
* Uses I/O ports `0x01CE` (index register) and `0x01CF` (data register).
* Allows dynamic video mode switching (e.g. 1024x768, 1280x800, 1920x1080) and hardware display panning.

---

## 5. Storage Controllers

### AHCI / SATA Driver
The AHCI driver (`kernel/src/drivers/ahci.rs`) provides high-speed block storage access:
* **PCI Discovery**: Enumerates Class `0x01` (Mass Storage), Subclass `0x06` (SATA).
* **ABAR MMIO**: Configures the AHCI Base Address Register (BAR 5).
* **DMA Transfers**: Implements 64-bit Physical Region Descriptor Tables (PRDT) to perform scatter-gather direct memory access without CPU polling.
* **Command Lists**: Dispatches non-blocking ATA command FIS packets to SATA disks.

### VirtIO-Block
* Provides paravirtualized disk I/O when running inside QEMU and KVM environments.

---

## 6. Network Interface Controllers (Intel e1000)

Dunit OS includes hardware detection and MMIO mapping for Intel gigabit Ethernet controllers:
* **Supported Device**: Intel 82540EM / 82545EM / 82574L (`vendor = 0x8086, device = 0x100E` or `0x10D3`).
* **Hardware Initialization**:
  * Scans PCI bus and maps the device's MMIO register block into kernel memory.
  * Reads the hardware MAC address from EEPROM (e.g. `52:54:00:12:34:56`).
  * Configures receive and transmit circular descriptor rings for packet processing.

