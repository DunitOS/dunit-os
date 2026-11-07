#!/usr/bin/env python3
"""Native C→Rust UI proof; OS execution remains exclusively qemu_test.py."""
import argparse
import os
from pathlib import Path
import re
import subprocess
import sys
from dunit_cli import ROOT,cargo


def verify(canvas_width=400,canvas_height=180,prefix='c-ui',states=('initial','updated')):
    for state in states:
        expected=(ROOT/f'build/c-ui-reference/{prefix}-{state}.rgb').read_bytes()
        if len(expected)!=canvas_width*canvas_height*3:raise ValueError('invalid reference canvas')
        capture=(ROOT/f'build/{prefix}-{state}.ppm').read_bytes()
        header=re.match(rb'P6\s+(\d+)\s+(\d+)\s+255\s',capture)
        if not header:raise ValueError('invalid framebuffer header')
        width,height=map(int,header.groups());data=capture[header.end():]
        if len(data)!=width*height*3:raise ValueError('truncated framebuffer')
        start=0;matched=False
        while (position:=data.find(expected[:canvas_width*3],start))>=0:
            start=position+3
            if position%3:continue
            x,y=(position//3)%width,position//(width*3)
            if x+canvas_width>width or y+canvas_height>height:continue
            pixels=b''.join(data[((y+r)*width+x)*3:((y+r)*width+x+canvas_width)*3]for r in range(canvas_height))
            if pixels==expected:matched=True;break
        if not matched:raise ValueError(f'{state}: C client pixels differ from direct Rust widgets')
        print(f'[CGUI] {state}: exact Rust SDK pixels at {x},{y}')


def prepare_reference():
    (ROOT/'build/c-ui-reference').mkdir(parents=True,exist_ok=True)
    env=dict(os.environ,DUNIT_C_GOLDEN_DIR=str(ROOT/'build/c-ui-reference'))
    env.setdefault('CARGO_TARGET_DIR',str(ROOT/'runtime/dunit/target'))
    subprocess.run([cargo(),'test','--manifest-path',str(ROOT/'runtime/dunit-c/Cargo.toml'),'--lib'],cwd=ROOT,env=env,check=True)


def run_native(accel, markers=None, overlay=None):
    subprocess.run([cargo(),'build','--release','--features','boot-smoke-tests','-Z','build-std=core,alloc,compiler_builtins','-Z','build-std-features=compiler-builtins-mem','-Z','json-target-spec'],cwd=ROOT/'kernel',check=True)
    command=[sys.executable,str(ROOT/'tools/qemu_test.py'),'--build','--markers-file',str(markers or ROOT/'tools/c_ui_markers.json'),'--accel',accel,'--post-boot-delay','3','--settle','3','--cmd-timeout','180','--timeout','480','--boot-timeout','180']
    if overlay is not None:command+=['--initrd-overlay',str(overlay)]
    subprocess.run(command,cwd=ROOT,check=True)


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--accel',choices=('tcg','kvm','auto'),default='tcg')
    parser.add_argument('--verify-only',action='store_true');args=parser.parse_args()
    prepare_reference()
    if not args.verify_only:run_native(args.accel)
    verify()
if __name__=='__main__':main()
