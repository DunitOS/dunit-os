#!/usr/bin/env python3
"""Exercise installed manifest discovery through the one system/QEMU harness."""
import argparse
import os
from pathlib import Path
import subprocess
import sys
import tempfile
from dunit_cli import ROOT, cargo
from dunit_manifest import load_manifest, installed_manifest

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--accel',choices=('auto','kvm','tcg'),default='tcg')
    parser.add_argument('--reuse-image',action='store_true')
    args=parser.parse_args()
    environment=os.environ.copy()
    environment['RUSTUP_TOOLCHAIN']='nightly'
    environment['PATH']=str(Path(cargo()).parent)+os.pathsep+environment['PATH']
    command=[sys.executable,str(ROOT/'tools/qemu_test.py'),
        '--markers-file',str(ROOT/'tools/sdk_catalog_markers.json'),
        '--accel',args.accel,'--post-boot-delay','3','--settle','1.5',
        '--timeout','360','--boot-timeout','180','--cmd-timeout','120']
    (ROOT/'build').mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='catalog-',dir=ROOT/'build') as folder:
        if not args.reuse_image:
            subprocess.run([cargo(),'build','--release','--features','boot-smoke-tests',
                '-Z','build-std=core,alloc,compiler_builtins','-Z','build-std-features=compiler-builtins-mem',
                '-Z','json-target-spec'],cwd=ROOT/'kernel',env=environment,check=True)
            app=load_manifest(ROOT/'tools/fixtures/sdk_catalog/dunit.toml',['gui_stat'],ROOT/'abi/rights.abi')
            overlay=Path(folder)/'overlay'
            target=overlay/'system/share/dunit/apps'/app.id/'dunit.toml'
            target.parent.mkdir(parents=True)
            target.write_text(installed_manifest(app),encoding='utf-8')
            command+=['--build','--initrd-overlay',str(overlay)]
        return subprocess.run(command,cwd=ROOT,env=environment).returncode
if __name__=='__main__':
    raise SystemExit(main())
