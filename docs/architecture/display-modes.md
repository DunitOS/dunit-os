# Display modes through 3K

Settings → Display offers 2560×1440, 2560×1600, 2880×1800 and
3072×1920 in addition to the existing modes and keep-boot-mode option.
The selected width and height use the existing `[display]` configuration and
compositor session rebuild path. Pointer button edges and the captured client PID
survive session recreation, so holding the resolution button does not generate
extra clicks and the actual release still reaches its client. No syscall or GUI
protocol changes are needed.

The Bochs/DISPI backend previously limited scanout to the boot framebuffer's
pixel count. It now measures the matching PCI VGA BAR0, intersects that resource
with the device's reported video memory, and maps the resulting framebuffer
before userspace starts. Mode validation checks both device dimension limits
and mapped memory capacity. If no suitable PCI mapping is available, the boot
mapping budget remains in force. Fixed Limine/GOP backends remain unchanged.

Framebuffer aliases inherit the boot mapping's PAT/cache selector, including
translation of huge-page PAT bits into the 4 KiB leaf format. Register MMIO
keeps its existing uncached mapping. Mapping framebuffer VRAM as register MMIO
would slow every full-frame copy substantially.

Validation: `tools/display_3k_markers.json` uses real Settings clicks to move
from a 1920×1080 boot through 3072×1920, 2880×1800, 2560×1600,
2560×1440 and back to 1920×1080. Run through `tools/qemu_test.py` with
`--build --ready-marker 'gui_server: desktop ready' --cmd-timeout 240` and
`--workdir /tmp/dunit-3k`. The captured PPM sizes must match each selected mode;
check them with `identify /tmp/dunit-3k/*.ppm`. Existing GUI smoke exercises
cross-process clients, shared buffers, maximize, restore and recycling.
