#!/usr/bin/env python3
"""Retain a frozen-v1 C object across a forced native Rust rebuild and VM run."""
import argparse
import copy
import hashlib
import json
import os
from pathlib import Path
import shlex
import struct
import subprocess
import sys
import tempfile
from check_image_profile import read_archive
from dunit_cli import ROOT, FLAGS, cargo
from ensure_c_ui import install
from test_c_ui import prepare_reference, run_native, verify

FIXTURE=ROOT/'tools/fixtures/c_ui_v1'
OBJECT='/c-ui-v1.o'


def digest(data):return hashlib.sha256(data).hexdigest()


def frozen_fixture(folder=FIXTURE):
    manifest=json.loads((folder/'manifest.json').read_text())
    if set(manifest['sha256'])!={'gui.c','include/dunit/ui.h'}:
        raise ValueError('unexpected frozen C UI fixture files')
    if {p.relative_to(folder).as_posix()for p in (folder/'include').rglob('*.h')}!={'include/dunit/ui.h'}:
        raise ValueError('unexpected frozen include override')
    for name,expected in manifest['sha256'].items():
        if digest((folder/name).read_bytes())!=expected:
            raise ValueError('frozen C UI fixture changed: '+name)
    return manifest


def compatibility_contract(base):
    result=copy.deepcopy(base)
    result['run']['cmd'][0]=f'exec /system/bin/tcc {OBJECT} -o /tmp/c-gui -ldunit_ui'
    if any(token.endswith('.c')for command in result['run']['cmd']for token in shlex.split(command)):
        raise ValueError('compatibility guest must link the retained object without C compilation')
    return result


def check_object(path,expected=None):
    data=path.read_bytes()
    if len(data)<64 or data[:6]!=b'\x7fELF\x02\x01' or struct.unpack_from('<HH',data,16)!=(1,62):
        raise ValueError('expected a native x86_64 ELF relocatable C object')
    actual=digest(data)
    if expected is not None and actual!=expected:
        raise ValueError('retained C object changed after compilation')
    return actual


def rebuild_runtime(sysroot):
    manifest=ROOT/'runtime/dunit-c/Cargo.toml'
    metadata=json.loads(subprocess.check_output([cargo(),'metadata','--no-deps','--format-version','1','--manifest-path',str(manifest)],cwd=ROOT,text=True))
    target=FLAGS[FLAGS.index('--target')+1]
    artifact=Path(metadata['target_directory'])/Path(target).stem/'release/libdunit_c.a'
    subprocess.run([cargo(),'clean','--manifest-path',str(manifest),'-p','dunit_c','--release','--target',target,'-Z','json-target-spec'],cwd=ROOT,check=True)
    if artifact.exists():raise ValueError('scoped clean left the old native runtime archive')
    install(sysroot)
    if not artifact.is_file():raise ValueError('native runtime was not rebuilt')
    return digest((sysroot/'lib/libdunit_ui.a').read_bytes())


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--accel',choices=('tcg','kvm','auto'),default='tcg')
    args=parser.parse_args();os.environ['RUSTUP_TOOLCHAIN']='nightly'
    manifest=frozen_fixture()
    subprocess.run([sys.executable,str(ROOT/'tools/gen_abi.py'),'--check'],cwd=ROOT,check=True)
    subprocess.run([sys.executable,str(ROOT/'tools/ensure_musl_sysroot.py')],cwd=ROOT,check=True)
    sysroot=ROOT/'build/dunit-sysroot'
    subprocess.run([sys.executable,str(ROOT/'tools/ensure_tcc.py'),'--sysroot',str(sysroot)],cwd=ROOT,check=True)
    (ROOT/'build').mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='c-ui-compat-',dir=ROOT/'build')as folder:
        folder=Path(folder);overlay=folder/'overlay';overlay.mkdir();obj=overlay/OBJECT.lstrip('/')
        subprocess.run([str(ROOT/'toolchains/dunit-tcc/tcc'),'-B'+str(ROOT/'toolchains/dunit-tcc'),'-L'+str(ROOT/'toolchains/dunit-tcc'),'-I'+str(FIXTURE/'include'),'-c',str(FIXTURE/'gui.c'),'-o',str(obj)],cwd=ROOT,check=True)
        retained=check_object(obj)
        print('[CGUI-COMPAT] compiled frozen v1 object sha256='+retained,flush=True)
        runtime=rebuild_runtime(sysroot)
        check_object(obj,retained)
        print('[CGUI-COMPAT] native runtime rebuilt; retained object unchanged',flush=True)
        prepare_reference()
        contract=compatibility_contract(json.loads((ROOT/'tools/c_ui_markers.json').read_text()))
        markers=folder/'markers.json';markers.write_text(json.dumps(contract,indent=2)+'\n')
        run_native(args.accel,markers,overlay)
        check_object(obj,retained)
        packed=read_archive((ROOT/'build/initrd.img').read_bytes())
        if digest(packed[OBJECT])!=retained:raise ValueError('initrd changed the retained object')
        if digest(packed['/system/lib/libdunit_ui.a'])!=runtime:
            raise ValueError('guest did not receive the rebuilt runtime')
        verify()
        receipt={'fixture':manifest,'retained_object_sha256':retained,'rebuilt_runtime_sha256':runtime,
                 'guest_object_sha256':digest(packed[OBJECT]),'forced_native_rebuild':True,
                 'guest_link_only':True,'exact_rust_pixels':['initial','updated']}
        (ROOT/'build/c-ui-compat-result.json').write_text(json.dumps(receipt,indent=2)+'\n')
        print('[CGUI-COMPAT] retained v1 object + rebuilt runtime + exact pixels OK',flush=True)
if __name__=='__main__':main()
