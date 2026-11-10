#!/usr/bin/env python3
"""Native C popup descriptors, exact canvases, and a cursor-free tooltip region."""
import argparse
import json
import re
import tempfile
import tomllib
from test_c_ui import ROOT, prepare_reference, run_native, verify

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--accel',choices=('tcg','kvm','auto'),default='tcg')
    args=parser.parse_args();prepare_reference()
    geometry=json.loads((ROOT/'build/c-ui-reference/c-ui-popup-geometry.json').read_text())
    rule=tomllib.loads((ROOT/'assets/tcc/popups.toml').read_text())['rules']['popups']
    resolution=re.search(r'^\s*resolution:\s*(\d+)x(\d+)',(ROOT/'limine_test_terminal.conf').read_text(),re.M)
    if resolution is None:raise ValueError('test display resolution is unspecified')
    screen=tuple(map(int,resolution.groups()))
    contract=json.loads((ROOT/'tools/c_ui_popups_markers.json').read_text())
    actions=[]
    for action in contract['run']['command_actions']['exec /app/gui_server']:
        if action in ({'hover_tooltip':True},{'leave_tooltip':True}):
            # Clamp to a known PS/2 origin before moving in measured SDK pixels.
            actions.append({'relative_motion':{'x':-127,'y':-127,'repeat':32,'interval':0.02}})
        if action=={'hover_tooltip':True}:
            point=[rule['x']+geometry['help'][0],rule['y']+geometry['help'][1]]
            if any(not 0<=point[n]<screen[n]for n in range(2)):raise ValueError('tooltip target outside display')
            dx,dy=map(round,point)
            while dx or dy:
                x,y=min(dx,127),min(dy,127);actions.append({'relative_motion':{'x':x,'y':y,'interval':0.03}});dx-=x;dy-=y
        elif action!={'leave_tooltip':True}:actions.append(action)
    contract['run']['command_actions']['exec /app/gui_server']=actions
    with tempfile.TemporaryDirectory(prefix='c-popup-',dir=ROOT/'build')as folder:
        markers=ROOT/folder/'markers.json';markers.write_text(json.dumps(contract))
        run_native(args.accel,markers)
    verify(560,420,'c-ui-popup',('initial','menu','select','context'))
    expected=(ROOT/'build/c-ui-reference/c-ui-popup-tooltip.rgb').read_bytes()
    capture=(ROOT/'build/c-ui-popup-tooltip.ppm').read_bytes()
    header=re.match(rb'P6\s+(\d+)\s+(\d+)\s+255\s',capture)
    if header is None:raise ValueError('invalid tooltip framebuffer')
    width,height=map(int,header.groups());data=capture[header.end():]
    if len(data)!=width*height*3 or len(expected)!=560*420*3:raise ValueError('invalid tooltip image length')
    x,y,w,h=geometry['tooltip'];left,top=rule['x']+x,rule['y']+y
    if w<=0 or h<=0 or min(left,top,x,y)<0 or left+w>width or top+h>height or x+w>560 or y+h>420:raise ValueError('invalid tooltip region')
    wanted=b''.join(expected[((y+r)*560+x)*3:((y+r)*560+x+w)*3]for r in range(h))
    actual=b''.join(data[((top+r)*width+left)*3:((top+r)*width+left+w)*3]for r in range(h))
    if actual!=wanted:raise ValueError('native tooltip pixels differ from Rust SDK')
    print(f'[C-POPUP] tooltip: exact Rust SDK region {x},{y} {w}x{h}')

if __name__=='__main__':main()
