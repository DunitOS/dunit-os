#!/usr/bin/env python3
"""Guard shared shell appearance sources; pixel/codec arithmetic is not theme data."""
from pathlib import Path
import re

ROOT=Path(__file__).resolve().parents[1]
SERVER=ROOT/'userspace/system_apps/gui_server/src'
# Arguments naming actual geometry/style, excluding buffers, format dimensions,
# rounding flags, and clock/resource limits. Zero and a single raster pixel are
# primitives, not an independent theme metric.
GEOMETRY={'fill_rect':range(3,7),'fill_rect_alpha':range(3,7),
          'fill_rrect':range(3,8),'fill_rrect_grad':range(3,8),
          'draw_soft_shadow':range(4,11),'blit_icon':range(6,10),'shade':range(1,2)}
PAINTERS={'main.rs','window_model.rs','window_painter.rs','shell_runtime.rs','shell_text.rs','shell_layout.rs','desktop_motion.rs','protocol_runtime.rs'}

def production(source):
    source=source.split('#[cfg(test)]',1)[0]
    return re.sub(r'//[^\n]*|/\*[\s\S]*?\*/|"(?:\\.|[^"\\])*"',lambda m:'\n'*m[0].count('\n') if m[0].startswith(('/',)) else '""',source)

def arguments(source,start):
    depth=0;begin=start
    for i in range(start,len(source)):
        ch=source[i]
        if ch in '([{':depth+=1
        elif ch in ')]}':
            if ch==')' and depth==0:
                yield source[begin:i];return
            depth-=1
        elif ch==',' and depth==0:
            yield source[begin:i];begin=i+1

def inspect_source(source,name):
    code=production(source);errors=[]
    def fail(position,why):errors.append(f'{name}:{code[:position].count(chr(10))+1}: {why}')
    for m in re.finditer(r'\b0x[0-9a-fA-F_]{6,}\b',code):
        value=int(m[0].replace('_',''),16)
        before=code[:m.start()].rstrip();after=code[m.end():].lstrip()
        if value not in (0xffffff,0xff000000) or not (before.endswith(('&','|')) or after.startswith('|')):
            fail(m.start(),'private color; use resolved theme data')
    if name in PAINTERS:
        for m in re.finditer(r'\b('+ '|'.join(GEOMETRY)+r')\s*\(',code):
            if code[max(0,m.start()-4):m.start()].strip()=='fn':continue
            args=list(arguments(code,m.end()))
            for index in GEOMETRY[m[1]]:
                if index<len(args) and re.fullmatch(r'\s*[+-]?\d+(?:\.\d+)?\s*',args[index]) and float(args[index]) not in (0,1):
                    fail(m.start(),f'private {m[1]} geometry/style argument {index}; use DSS')
        for m in re.finditer(r'\blet(?:\s+mut)?\s+(?:width|height|padding|gap|radius|size|inset|font_px|title_h|border|shadow|sx|sy|cx|cy)\s*(?::[^=;]+)?=\s*([+-]?\d+)\s*(?:;|[+-])',code):
            if int(m[1]) not in (0,1):fail(m.start(),'private appearance assignment; use resolved DSS')
        for m in re.finditer(r'\.(?:max|unwrap_or)\(\s*([+-]?\d+)\s*\)',code):
            if int(m[1]) not in (0,1):fail(m.start(),'private appearance fallback; use resolved DSS')
    return errors

def check():
    errors=[];count=0
    for path in sorted(SERVER.glob('*.rs')):
        if path.stem.endswith('_tests') or path.name=='bringup.rs':continue
        count+=1;errors+=inspect_source(path.read_text(),path.name)
    if errors:raise ValueError('\n'.join(errors))
    print(f'Shell style: {count} production modules checked; no private colors/direct paint sizes/fallback metrics')

if __name__=='__main__':check()
