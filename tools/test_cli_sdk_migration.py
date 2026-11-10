#!/usr/bin/env python3
"""M7.11 CLI parity: frozen pre-migration bytes plus real native stdout capture."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
from dunit_cli import ROOT, cargo
from test_c_ui import run_native

FIXTURE=ROOT/'tools/fixtures/dufetch_stdout.json'

def rust_string(text):
    return re.sub(r'\\u([0-9a-fA-F]{4})',lambda m:'\\u{'+m[1]+'}',json.dumps(text,ensure_ascii=False))

def native_report(log):
    chunks=re.findall(r'\[CLI-REPORT\] ([0-9a-f]+)(?=\s|$)',log)
    if not chunks or any(len(c)%2 or len(c)>64 for c in chunks):raise ValueError('invalid native report capture')
    return bytes.fromhex(''.join(chunks)).decode('utf-8')

def stable_report(text):
    # Only real time/counters/PID vary across processes and boots. Validate each
    # format first and retain all units, punctuation, ANSI, layout, logo and facts.
    patterns={
        'Uptime':r'(?:\d+h \d+m \d+s|\d+m \d+s|\d+s|unknown)',
        'Memory':r'\d+ KiB / \d+ KiB \(\d+%\)',
        'Heap':r'\d+ KiB / \d+ KiB',
        'Processes':r'\d+ total, \d+ running',
        'FS':r'\d+ files, \d+ dirs, \d+ KiB',
        'Open FDs':r'\d+', 'IPC':r'\d+ queues, \d+ msgs, \d+ shm regions',
        'NICs':r'\d+ detected, \d+ supported', 'PID':r'[1-9]\d*',
    }
    seen=set()
    def replace(match):
        label,value=match[1],match[2];seen.add(label)
        if label in patterns:
            if not re.fullmatch(patterns[label],value):raise ValueError('invalid live '+label+': '+value)
            value='<uptime>'if label=='Uptime'else re.sub(r'\d+','#',value)
        return '\x1b[1;36m'+label+'\x1b[0m: \x1b[97m'+value+'\x1b[0m'
    result=re.sub(r'\x1b\[1;36m([^\x1b\n]+)\x1b\[0m: \x1b\[97m([^\x1b\n]*)\x1b\[0m',replace,text)
    if not {'Memory','Heap','Processes','FS','Open FDs','PID','CWD'}.issubset(seen):raise ValueError('native report lost required facts')
    return result

def host_parity():
    fixture=json.loads(FIXTURE.read_text())
    with tempfile.TemporaryDirectory(prefix='m711-host-',dir=ROOT/'build')as temp:
        temp=Path(temp);(temp/'src').mkdir()
        (temp/'Cargo.toml').write_text('[package]\nname="dufetch_migration_parity"\nversion="0.0.0"\nedition="2021"\n[dependencies]\ndunit={path='+rust_string(str(ROOT/'runtime/dunit'))+'}\n')
        code='''extern crate alloc;
use dunit::io::Write;
#[path=REPORT_PATH]mod report;
struct Sink{data:Vec<u8>,limit:usize}
impl Write for Sink{fn write(&mut self,b:&[u8])->dunit::Result<usize>{
 if self.data.len()>=self.limit{return Err(dunit::Error(dunit::errno::EIO))}
 let n=b.len().min(3).min(self.limit-self.data.len());self.data.extend_from_slice(&b[..n]);Ok(n)
}}
fn main(){
'''.replace('REPORT_PATH',rust_string(str(ROOT/'userspace/system_apps/dufetch/src/report.rs')))
        for case in fixture['cases']:
            fields=','.join(f'{key}:{value}'for key,value in case['stats'].items())
            stats='dunit::system::SystemStats{'+fields+(','if fields else'')+'..Default::default()}'
            d=case['display'];display='None'if d is None else'Some(dunit::system::DisplayInfo{'+','.join(f'{k}:{v}'for k,v in d.items())+'})'
            args=f'&stats,{display},{case["pid"]},{rust_string(case["cwd"])}'
            code+='{' + chr(10)+f'let stats={stats};let expected={rust_string(case["stdout"])}.as_bytes();'
            code+=f'let mut sink=Sink{{data:Vec::new(),limit:usize::MAX}};report::render({args},&mut sink).unwrap();assert_eq!(sink.data,expected,{rust_string(case["name"])});'
            code+=f'let mut failed=Sink{{data:Vec::new(),limit:17}};assert_eq!(report::render({args},&mut failed),Err(dunit::Error(dunit::errno::EIO)));assert_eq!(failed.data,&expected[..17]);\n}}\n'
        code+='}\n';(temp/'src/main.rs').write_text(code)
        env=dict(os.environ);env.setdefault('CARGO_TARGET_DIR',str(ROOT/'runtime/dunit/target'))
        subprocess.run([cargo(),'run','--manifest-path',str(temp/'Cargo.toml')],cwd=ROOT,env=env,check=True)
    print(f'[CLI-MIGRATION] {len(fixture["cases"])} legacy exact stdout snapshots; partial writes and EIO propagation OK',flush=True)
    # Same portable C application against the real umbrella header on the host.
    with tempfile.TemporaryDirectory(prefix='m711-c-',dir=ROOT/'build')as folder:
        folder=Path(folder);exe=folder/'wc_demo'
        subprocess.run(['cc','-std=c11','-Wall','-Wextra','-Werror','-I',str(ROOT/'abi/include'),str(ROOT/'userspace/ctests/wc_demo.c'),'-o',str(exe)],check=True)
        result=subprocess.run([str(exe)],cwd=folder,capture_output=True,check=True)
        if result.stdout!=b'wc demo.txt -> 3 lines, 13 words, 61 bytes\n' or result.stderr:raise ValueError('host wc stdout differs')
        if(folder/'demo.txt').read_bytes()!=b'the quick brown fox\njumps over the lazy dog\nthe fox is quick\n':raise ValueError('host wc file differs')
    print('[CLI-MIGRATION] C SDK umbrella preserves portable wc stdout and file bytes',flush=True)

def save_result(capture):
    (ROOT/'build/m711-dufetch.stdout').write_bytes(capture.encode())
    fixture=json.loads(FIXTURE.read_text())
    receipt={'source_commit':fixture['source_commit'],'legacy_source_sha256':fixture['source_sha256'],'native_stdout_sha256':hashlib.sha256(capture.encode()).hexdigest(),'exact_snapshot_cases':len(fixture['cases']),'native_wc_exact_stdout_stderr_status':True,'native_dufetch_format_parity':True}
    (ROOT/'build/m711-cli-result.json').write_text(json.dumps(receipt,indent=2)+'\n')

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--host-only',action='store_true')
    parser.add_argument('--reuse-image',action='store_true')
    parser.add_argument('--accel',choices=('tcg','kvm','auto'),default='tcg')
    args=parser.parse_args();host_parity()
    if args.host_only:return
    run_native(args.accel,ROOT/'tools/sdk_cli_migration_markers.json',build=not args.reuse_image)
    capture=native_report((ROOT/'build/qemu-serial.full.log').read_text())
    if stable_report(capture)!=json.loads(FIXTURE.read_text())['native_format']:raise ValueError('live report changed beyond volatile values')
    save_result(capture)
    print('[CLI-MIGRATION] native exact wc stdout/stderr/status; dufetch ANSI/layout/facts parity OK',flush=True)

if __name__=='__main__':main()
