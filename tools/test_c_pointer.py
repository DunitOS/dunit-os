#!/usr/bin/env python3
"""Native C pointer-mode contract plus exact software-cursor pixel checks."""
import argparse
import re
import subprocess
import sys
from dunit_cli import ROOT, cargo


def capture(name):
    b=(ROOT/f'build/c-pointer-{name}.ppm').read_bytes()
    m=re.match(rb'P6\s+(\d+)\s+(\d+)\s+255\s',b)
    if not m:raise ValueError('invalid capture')
    w,h=map(int,m.groups());data=b[m.end():]
    if len(data)!=w*h*3:raise ValueError('truncated capture')
    return w,h,data


def cursors(frame):
    w,h,data=frame
    shape=(ROOT/'assets/dwm/cursor.txt').read_text().splitlines()
    pixels=[(x,y,b'\0\0\0'if c=='X'else b'\xff\xff\xff')
            for y,row in enumerate(shape)for x,c in enumerate(row)if c!='_']
    # Every cursor has this exact opaque first pixel. Transparent pixels are
    # ignored so the proof works over both client content and the desktop.
    found=[];start=0
    while (p:=data.find(b'\0\0\0',start))>=0:
        start=p+1
        if p%3:continue
        x,y=(p//3)%w,p//(3*w)
        if x+len(shape[0])>w or y+len(shape)>h:continue
        if all(data[((y+dy)*w+x+dx)*3:((y+dy)*w+x+dx)*3+3]==rgb for dx,dy,rgb in pixels):found.append((x,y))
    return found


def verify():
    normal=capture('normal');locked=capture('locked');released=capture('released');after=capture('after-exit')
    if any(f[:2]!=normal[:2]for f in (locked,released,after)):raise ValueError('capture geometry changed')
    before=cursors(normal)
    if len(before)!=1:raise ValueError(f'expected one normal cursor, found {before}')
    if cursors(locked):raise ValueError('locked cursor is still visible')
    if cursors(released)!=before:raise ValueError('emergency release did not restore the cursor at its anchor')
    if cursors(after)!=before:raise ValueError('client exit did not restore the cursor at its anchor')
    x,y=before[0];w=normal[0]
    # The entire blank client area around the cursor must match the image
    # before lock after release, proving no cursor trail/changed anchor.
    region=lambda f:b''.join(f[2][((r*w+x)*3):((r*w+x+24)*3)]for r in range(y,y+24))
    if region(normal)!=region(released):raise ValueError('cursor restoration left different client pixels')
    print(f'[C-POINTER] pixels: visible -> hidden -> restored -> restored after exit at {x},{y}')


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--accel',choices=('auto','kvm','tcg'),default='tcg')
    parser.add_argument('--verify-only',action='store_true')
    parser.add_argument('--reuse-image',action='store_true')
    a=parser.parse_args()
    if not a.verify_only:
        if not a.reuse_image:
            subprocess.run([cargo(),'check','--release','--features','boot-smoke-tests','-Z','build-std=core,alloc,compiler_builtins','-Z','build-std-features=compiler-builtins-mem','-Z','json-target-spec'],cwd=ROOT/'kernel',check=True)
        command=[sys.executable,str(ROOT/'tools/qemu_test.py'),*(['--build']if not a.reuse_image else []),'--markers-file',str(ROOT/'tools/c_pointer_markers.json'),'--accel',a.accel,'--boot-timeout','240','--timeout','600','--cmd-timeout','240']
        subprocess.run(command,cwd=ROOT,check=True)
    verify()

if __name__=='__main__':main()
