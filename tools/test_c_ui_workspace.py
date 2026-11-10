#!/usr/bin/env python3
"""C collections/workspace, clipboard/capabilities, and a real PS/2 drag/drop."""
import argparse
import json
import tempfile
import shutil
import tomllib
from test_c_ui import ROOT, prepare_reference, run_native, verify

def actions(contract,geometry,rule):
    expanded=[]
    def origin():expanded.append({'relative_motion':{'x':-127,'y':-127,'repeat':32,'interval':0.02}})
    def move(name):
        origin();dx,dy=[round(rule[axis]+geometry[name][i])for i,axis in enumerate(('x','y'))]
        while dx or dy:
            x,y=min(dx,127),min(dy,127)
            expanded.append({'relative_motion':{'x':x,'y':y,'interval':0.03}});dx-=x;dy-=y
    def button(down):
        expanded.extend([{'execute':'input-send-event','arguments':{'events':[{'type':'btn','data':{'down':down,'button':'left'}}]}},{'delay_ms':180}])
    for action in contract['run']['command_actions']['exec /app/gui_server']:
        if action=={'focus_client':True}:
            move('title');button(True);button(False);origin();expanded.append({'delay_ms':300})
        elif action=={'drag_self':True}:
            move('title');button(True);expanded.append({'relative_motion':{'x':80,'y':0,'interval':0.03}})
            expanded.append({'wait_marker':'[C-TRANSFER] drag accepted','timeout':30});expanded.append({'delay_ms':500});button(False);origin()
        elif action=={'click_quit':True}:move('quit');button(True);button(False)
        else:expanded.append(action)
    contract['run']['command_actions']['exec /app/gui_server']=expanded
    return contract

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--accel',choices=('tcg','kvm','auto'),default='tcg')
    args=parser.parse_args();prepare_reference()
    geometry=json.loads((ROOT/'build/c-ui-reference/c-ui-workspace-geometry.json').read_text())
    rule=tomllib.loads((ROOT/'assets/tcc/workspace.toml').read_text())['rules']['workspace']
    contract=actions(json.loads((ROOT/'tools/c_ui_workspace_markers.json').read_text()),geometry,rule)
    with tempfile.TemporaryDirectory(prefix='c-workspace-',dir=ROOT/'build')as folder:
        markers=ROOT/folder/'markers.json';markers.write_text(json.dumps(contract))
        run_native(args.accel,markers)
    verify(640,620,'c-ui-workspace',('initial','collections','dialog','split','renamed'))
    shutil.copyfile(ROOT/'build/qemu-serial.full.log',ROOT/'build/c-ui-workspace.serial.log')
    run_native(args.accel,ROOT/'tools/c_ui_picker_markers.json',build=False)
    verify(640,500,'c-ui-picker',('initial',))
    shutil.copyfile(ROOT/'build/qemu-serial.full.log',ROOT/'build/c-ui-picker.serial.log')
if __name__=='__main__':main()
