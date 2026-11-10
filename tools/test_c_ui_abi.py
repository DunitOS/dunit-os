#!/usr/bin/env python3
"""Freeze published C library ABI while allowing additive declarations."""
import json
from pathlib import Path
import re
import subprocess
import tempfile
import unittest
from gen_abi import ROOT
from gen_c_ui import TYPES

BASELINE=ROOT/'tools/fixtures/c_ui_abi_v1.json'

class StableAbi(unittest.TestCase):
    def setUp(self):
        self.baseline=json.loads(BASELINE.read_text())
        self.schema=json.loads((ROOT/'runtime/dunit-c/api.json').read_text())
    def test_existing_records_callbacks_constants_and_exports_are_immutable(self):
        for section,entries in self.baseline['schema'].items():
            for name,value in entries.items():
                with self.subTest(section=section,name=name):
                    self.assertEqual(self.schema[section].get(name),value)
        constants=dict((name,int(value))for name,value in re.findall(r'^#define (DUNIT_\w+) (\d+)u$',(ROOT/'abi/include/dunit/ui.h').read_text(),re.M))
        for name,value in self.baseline['generated_constants'].items():
            self.assertEqual(constants.get(name),value,name)
        for name,value in self.baseline['types'].items():
            self.assertEqual(list(TYPES[name]),value,name)
    def test_public_c_layouts_and_function_pointer_types_match_the_frozen_abi(self):
        schema=self.baseline['schema'];types=self.baseline['types']
        def ctype(kind):return kind if kind in schema['callbacks']else types[kind][0]
        def signature(value):return ', '.join(ctype(kind)for _,kind in value['args'])or'void'
        source=['#include <dunit/ui.h>']
        for name,record in schema['records'].items():
            source.extend([f'_Static_assert(sizeof({name}) == {record["size"]}, "{name} size");',f'_Static_assert(_Alignof({name}) == {record["align"]}, "{name} align");'])
        for name,value in schema['callbacks'].items():
            source.extend([f'typedef {ctype(value["result"])} (*Frozen{name})({signature(value)});',f'{name} check_{name}(Frozen{name} f) {{ return f; }}'])
        for name,value in schema['functions'].items():
            source.append(f'{ctype(value["result"])} (*check_{name})({signature(value)}) = {name};')
        with tempfile.TemporaryDirectory()as folder:
            c=Path(folder)/'abi.c';c.write_text('\n'.join(source)+'\n')
            subprocess.run(['cc','-std=c11','-Wall','-Wextra','-Werror','-I',str(ROOT/'abi/include'),'-c',str(c),'-o',str(Path(folder)/'abi.o')],check=True,capture_output=True)

if __name__=='__main__':unittest.main()
