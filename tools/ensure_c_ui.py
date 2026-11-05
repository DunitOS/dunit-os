#!/usr/bin/env python3
"""Build/install the real Rust component adapter for native C static linking."""
import argparse
import filecmp
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
from dunit_cli import ROOT, FLAGS, cargo

def install(sysroot):
    command=[cargo(),'build','--manifest-path',str(ROOT/'runtime/dunit-c/Cargo.toml'),
             '--lib',*FLAGS,'--message-format=json']
    process=subprocess.run(command,cwd=ROOT,capture_output=True,text=True)
    print(process.stderr,end='')
    if process.returncode:raise subprocess.CalledProcessError(process.returncode,command)
    artifacts=[]
    for line in process.stdout.splitlines():
        item=json.loads(line)
        if item.get('reason')=='compiler-artifact'and item['target']['name']=='dunit_c':
            artifacts.extend(Path(name)for name in item['filenames']if name.endswith('.a'))
    if len(artifacts)!=1:raise ValueError('expected one native UI static archive')
    destination=Path(sysroot)/'lib/libdunit_ui.a';destination.parent.mkdir(parents=True,exist_ok=True)
    # Rust archives also contain public Rust code unrelated to the C entry
    # points and LTO bitcode. Resolve the declared C roots into one relocatable
    # machine-code object, retaining its real undefined libc dependencies.
    schema=json.loads((ROOT/'runtime/dunit-c/api.json').read_text())
    with tempfile.TemporaryDirectory(prefix='c-ui-',dir=ROOT/'build') as folder:
        folder=Path(folder);obj=folder/'ui.o';archive=folder/'libdunit_ui.a'
        subprocess.run(['ld.lld','-r','--gc-sections','-T',str(ROOT/'runtime/dunit-c/staticlib.ld'),
            *('--undefined='+name for name in schema['functions']),str(artifacts[0]),'-o',str(obj)],check=True)
        subprocess.run(['objcopy','--remove-section=.llvmbc','--remove-section=.llvmcmd','--strip-debug',
            *('--keep-global-symbol='+name for name in schema['functions']),'--strip-unneeded',str(obj)],check=True)
        subprocess.run(['ar','rcs',str(archive),str(obj)],check=True)
        if not destination.exists()or not filecmp.cmp(archive,destination,shallow=False):
            shutil.copy2(archive,destination)
    print(f'[CUI] installed {destination.name} ({destination.stat().st_size} bytes)')

if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--sysroot',type=Path,required=True)
    args=parser.parse_args();os.environ['RUSTUP_TOOLCHAIN']='nightly';install(args.sysroot)
