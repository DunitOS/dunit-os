"""Explicit production source × receiver matrix and native QMP action builder."""
import json
import os
from collections import Counter
from pathlib import Path
import tomllib
ROOT=Path(__file__).resolve().parents[1]
MATRIX=ROOT/'tools/dnd_matrix.json'
REGISTRY=ROOT/'assets/dwm/default.toml'

def load():
    data=json.loads(MATRIX.read_text())
    registered=set(tomllib.loads(REGISTRY.read_text())['application'])
    apps=data['applications']
    if set(apps)!=registered:
        raise ValueError(f'DnD matrix application coverage differs: missing={registered-set(apps)}, stale={set(apps)-registered}')
    pairs=[(cell['source'],cell['target']) for cell in data['cells']]
    if len(pairs)!=len(set(pairs)) or set(pairs)!={(a,b) for a in apps for b in apps}:
        raise ValueError('DnD matrix must declare each ordered production application pair exactly once')
    for cell in data['cells']:
        source=apps[cell['source']]['source'];target=apps[cell['target']]['target']
        expected=('unsupported_source' if source=='none' else 'unsupported_target' if target=='none'
                  else 'file_copy' if source=='files' and target=='files_or_filename' else 'text')
        if cell['expected']!=expected:raise ValueError(f'Contradictory matrix cell: {cell}')
    return data

def prepare(overlay):
    data=load();apps=list(data['applications']);cases=data['cells']
    selected=os.environ.get('DUNIT_MATRIX_CASES')
    if selected:
        wanted=set(selected.split(','));cases=[c for c in cases if f"{c['source']}->{c['target']}" in wanted]
        if len(cases)!=len(wanted):raise ValueError('Unknown selected matrix cell')
    quoted_files=os.environ.get('DUNIT_MATRIX_QUOTED')=='1'
    if quoted_files and (len(cases)!=1 or cases[0]['source']!='gui_files' or cases[0]['target']!='gui_terminal'):raise ValueError('Quoted path proof requires Files to Terminal')
    source_leaf="файл 'quoted'.txt" if quoted_files else 'item'
    move_files=os.environ.get('DUNIT_MATRIX_MOVE')=='1'
    if move_files and (len(cases)!=1 or cases[0]['expected']!='file_copy'):raise ValueError('Move proof requires exactly one file receiver cell')
    ids=['sdk_matrix_fixture']+[f'm77_{role}_{app}' for role in ('s','t') for app in apps]
    config=['[meta]','schema_version=2','[startup]','entries=["sdk_matrix_fixture"]','self_test=false',
        '[session]','restore=false','[layout]','max_windows=8','traffic_lights=true',
        '[transfers]','trace=true','[effects]','anim=false','[widgets]','enabled=false','[notifications]','enabled=false',
        '[panels.dock]','edge="top"','size=44','children=["apps"]','[components.apps]',
        'kind="apps"','size=40','gap=4','entries='+json.dumps(ids),
        '[rules.observer]','app="sdk_matrix_fixture"','x=1200','y=100','width=240','height=120','floating=true']
    for role,x in [('s',100),('t',600)]:
        for app in apps:
            identity=f'm77_{role}_{app}'
            metadata=overlay/'system/share/dunit/apps'/identity/'dunit.toml';metadata.parent.mkdir(parents=True)
            metadata.write_text(f'schema_version=1\n[application]\nid="{identity}"\nname="{identity}"\nkind="gui"\nbinary="{app}"\nicon="/assets/icons/monitor.rgba"\ncapabilities=[]\n')
            config += [f'[rules.{identity}]',f'app="{identity}"',f'x={x}','y=100','width=500','height=600','floating=true']
            if app=='gui_files':config += [f'[application.{identity}]','arguments='+json.dumps([f'/tmp/matrix-{"source" if role=="s" else "target"}'])]
    (overlay/'sdk-matrix.toml').write_text('\n'.join(config)+'\n')
    for path,contents in [('tmp/matrix-source/'+source_leaf,'matrix-file'),('tmp/matrix-target/.anchor','anchor')]:
        file=overlay/path;file.parent.mkdir(parents=True,exist_ok=True);file.write_text(contents)
    if move_files:
        (overlay/'tmp/matrix-source/item2').write_text('second-file')
    rows=[]
    for slot,cell in enumerate(data['cells'],1):
        src,dst=cell['source'],cell['target'];positive=cell['expected'] in ('text','file_copy')
        value='/tmp/matrix-source/'+source_leaf if src=='gui_files' else '0'
        mode='file' if cell['expected']=='file_copy' else 'terminal' if dst=='gui_terminal' else 'clipboard'
        expected=('matrix-file' if mode=='file' else f'm{slot}: '+value if mode=='terminal' and positive else f'm{slot}:' if mode=='terminal' else value if positive else 'm77-empty')
        rows.append('\t'.join([src,dst,mode,expected]))
    actions=[];counts=Counter();exits=Counter();position=[0,0];completed=0;published=0;offered=0
    def motion(x,y,n=1):actions.append({'relative_motion':{'x':x,'y':y,'repeat':n,'interval':0.012}})
    motion(-127,-127,24)
    def move(x,y):
        while position!=[x,y]:
            dx=max(-127,min(127,x-position[0]));dy=max(-127,min(127,y-position[1]));motion(dx,dy);position[0]+=dx;position[1]+=dy
    def wait(marker,count=1):actions.append({'wait_marker':marker,'occurrences':count,'timeout':45})
    def delay(ms=180):actions.append({'delay_ms':ms})
    def button(down,which='left'):actions.append({'execute':'input-send-event','arguments':{'events':[{'type':'btn','data':{'down':down,'button':which}}]}})
    def click(x,y,which='left'):move(x,y);button(True,which);button(False,which);delay(100)
    def key(*keys):
        # send-key's release timer uses guest virtual time. Under TCG it can
        # overlap the next wall-clock action; inject ordered make/break instead.
        def edges(down,names):
            return {'execute':'input-send-event','arguments':{'events':[{'type':'key','data':{'down':down,'key':{'type':'qcode','data':name}}} for name in names]}}
        actions.append(edges(True,keys));delay(60)
        actions.append(edges(False,reversed(keys)));delay(60)
    def type_text(text,submit=False):actions.append({'type_text':text,'submit':submit,'key_interval_ms':50});delay()
    def focus(identity):click(26+44*ids.index(identity),22)
    def launch(identity):
        focus(identity);counts[identity]+=1;wait('[GUI-PRESENT] app='+identity,counts[identity]);wait('[GUI-SIZE] app='+identity+' width=500 height=600',counts[identity])
    def close(identity,x):
        click(x+18,84);exits[identity]+=1;wait('[GUI-EXIT] app='+identity+' kind=0 code=0',exits[identity])
    def control(down):actions.append({'execute':'input-send-event','arguments':{'events':[{'type':'key','data':{'down':down,'key':{'type':'qcode','data':'ctrl'}}}]}})
    wait('[MATRIX] observer ready')
    for case_index,cell in enumerate(cases,1):
        src,dst=cell['source'],cell['target'];source_id=f'm77_s_{src}';target_id=f'm77_t_{dst}'
        positive=cell['expected'] in ('text','file_copy');value='/tmp/matrix-source/'+source_leaf if src=='gui_files' else '0'
        mode='file' if cell['expected']=='file_copy' else 'terminal' if dst=='gui_terminal' else 'clipboard'
        expected='matrix-file' if mode=='file' else value if positive else '' if mode=='terminal' else 'm77-empty'
        case_index=data['cells'].index(cell)+1
        launch(source_id);launch(target_id)
        # Prepare an empty receiving field or an actual shell echo command.
        if dst=='gui_settings':click(660,610);click(790,270);key('ctrl','a');key('backspace')
        elif dst=='gui_files' and cell['expected']!='file_copy':
            click(1050,300,'right');click(970,312);click(730,400);key('ctrl','a');key('backspace')
        elif dst=='gui_terminal':
            click(900,400);key('ctrl','u');type_text(f'echo m{case_index}: ');wait(f'[STDOUT] echo m{case_index}: ')
        focus(source_id)
        if src=='gui_settings':
            click(160,610);click(290,270);key('ctrl','a');type_text('0');key('ctrl','a')
            # Verify the source selection was processed before testing drag.
            key('ctrl','c');published+=1;wait('[XFR] clipboard transfer=',published)
        elif src=='gui_terminal':
            click(150,150);key('ctrl','l');type_text('echo 0',True);delay(250)
            move(109,132);button(True);move(117,132);button(False);delay()
        elif src=='gui_files':
            click(260,166)
            if move_files:
                key('ctrl','a');key('ctrl','c');published+=1
                wait('[XFR] clipboard transfer=',published)
        focus('sdk_matrix_fixture');key('g');type_text(str(case_index-1),True);key('n');wait(f'[MATRIX] case {case_index} ready');published+=1
        if move_files:key('m');wait('[MATRIX] multi-file move armed')
        focus(source_id)
        # Keep each narrow-glyph pointer gesture independent of earlier cases.
        motion(-127,-127,24);position[:]=[0,0]
        sx,sy=(280,270) if src=='gui_settings' else (109,132) if src=='gui_terminal' else (260,166) if src=='gui_files' else (150,150)
        tx,ty=(780,270) if dst=='gui_settings' else (900,300) if dst=='gui_files' and cell['expected']=='file_copy' else (710,400) if dst=='gui_files' else (900,400)
        move(sx,sy);actions.append({'execute':'screendump','arguments':{'filename':'/workspace/scratch/m77-matrix-source.ppm'}});button(True);move(sx+20,sy)
        if data['applications'][src]['source']!='none':
            offered+=1;wait(' kind=DragOffer transfer=0',offered)
        else:delay(180)
        if not move_files:control(True)
        move(tx,ty);delay(250);actions.append({'execute':'screendump','arguments':{'filename':'/workspace/scratch/m77-matrix-drop.ppm'}});button(False)
        if not move_files:control(False)
        if positive:
            completed+=1;wait(' kind=Complete transfer=',completed)
        delay(180)
        focus(target_id)
        if dst=='gui_terminal':type_text(' > /tmp/matrix-result',True);wait(f'[MATRIX] filesystem case {case_index} ready')
        elif mode=='file':wait(f'[MATRIX] filesystem case {case_index} ready')
        elif dst in ('gui_files','gui_settings') and mode=='clipboard':
            key('ctrl','a');key('ctrl','c')
            if positive:
                published+=1;wait('[XFR] clipboard transfer=',published)
        actions.append({'execute':'screendump','arguments':{'filename':'/workspace/scratch/m77-matrix-before.ppm'}})
        focus('sdk_matrix_fixture');key('v');wait(f'[DND] {src}->{dst} ok')
        if mode=='clipboard':completed+=1
        if move_files:wait('[DND] multi-file native move ok')
        close(source_id,100);close(target_id,600)
    focus('sdk_matrix_fixture');key('q');wait('[MATRIX] all selected native cells checked')
    (overlay/'sdk-matrix.tsv').write_text('\n'.join(rows)+'\n')
    markers={'run':{'config':'limine_test_terminal.conf','cmd':['exec /app/setup import-desktop /sdk-matrix.toml','exec /app/gui_server'],
        'command_actions':{'exec /app/gui_server':actions}},
        'required':[{'marker':m,'claim':m,'source':'tools/fixtures/sdk_matrix.rs'} for m in ['[TERM-007] Starting main loop','[MATRIX] all selected native cells checked']+[f"[DND] {c['source']}->{c['target']} ok" for c in cases]],
        'forbidden':[{'marker':m,'why':'Native matrix failure','source':'tools/fixtures/sdk_matrix.rs'} for m in ['[PANIC]','[SDK] PANIC','application failed:','gui_server: FAIL','gui_files: PANIC','gui_settings: PANIC','gui_calc: PANIC','gui_terminal: PANIC']]}
    if move_files:markers['required'].append({'marker':'[DND] multi-file native move ok','claim':'Two real source inodes moved through Files and broker; neither source remains','source':'tools/fixtures/sdk_matrix.rs'})
    path=overlay.parent/'matrix-markers.json';path.write_text(json.dumps(markers,indent=2)+'\n');return path

if __name__=='__main__':
    data=load();print(f"DnD application coverage: {len(data['applications'])} applications, {len(data['cells'])} explicit cells")
