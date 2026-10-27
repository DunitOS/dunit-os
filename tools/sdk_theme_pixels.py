"""Compare the real QEMU framebuffer's client rectangle to native CPU pixels."""
from pathlib import Path
import hashlib
import json
import re


def rgb_hash(data):
    value = 0xcbf29ce484222325
    for byte in data:
        value = ((value ^ byte) * 0x100000001b3) & ((1 << 64) - 1)
    return value


def verify(root: Path):
    serial = (root / 'build/qemu-serial.full.log').read_text(errors='replace')
    expected = dict(re.findall(r'\[SDKTHEME\] (dark|light|restored) hash=([0-9a-f]{16})', serial))
    if set(expected) != {'dark', 'light', 'restored'}:
        raise RuntimeError('Theme fixture did not report all native pixel hashes')
    golden = json.loads((root / 'tools/fixtures/sdk_theme_golden.json').read_text())
    if golden['version'] != 1 or expected != golden['rgb_fnv1a64']:
        raise RuntimeError('Rendered defaults differ from the reviewed theme golden')
    if hashlib.sha256((root / 'assets/fonts/DejaVuSans.ttf').read_bytes()).hexdigest() != golden['font_sha256']:
        raise RuntimeError('Theme golden font changed; review the visual baseline')
    cw, ch = golden['width'], golden['height']
    for label, digest in expected.items():
        raw = (root / f'build/sdk-theme-{label}.ppm').read_bytes()
        header = re.match(rb'P6\s+(\d+)\s+(\d+)\s+255\s', raw)
        if not header:
            raise RuntimeError('Unexpected QEMU screendump format')
        width, height = map(int, header.groups())
        data = raw[header.end():]
        if len(data) != width * height * 3:
            raise RuntimeError('Truncated QEMU framebuffer capture')
        profile = 'light' if label == 'light' else 'dark'
        source = (root / f'assets/dwm/sdk/{profile}.dss').read_text()
        color = re.search(r'\$window-bg:\s*#([0-9a-f]{6});', source).group(1)
        row = bytes.fromhex(color) * cw
        start = 0
        matched = None
        while (position := data.find(row, start)) >= 0:
            start = position + 3
            if position % 3:
                continue
            x, y = (position // 3) % width, (position // 3) // width
            if x + cw > width or y + ch > height:
                continue
            canvas = b''.join(data[((y + r) * width + x) * 3:
                                  ((y + r) * width + x + cw) * 3] for r in range(ch))
            if rgb_hash(canvas) == int(digest, 16):
                matched = (x, y)
                break
        if matched is None:
            raise RuntimeError(f'{label}: actual framebuffer differs from the native rendered canvas')
        print(f'[SDKTHEME] framebuffer {label} {cw}x{ch} at {matched}: exact pixels OK')
