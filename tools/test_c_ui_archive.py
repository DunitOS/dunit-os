#!/usr/bin/env python3
"""Exercise the actual C archive packaging/linking commands on a host fixture."""
import contextlib
import io
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch
import ensure_c_ui

class Archive(unittest.TestCase):
    def test_export_root_and_real_undefined_libc_symbol_survive_packaging(self):
        with tempfile.TemporaryDirectory() as folder:
            root=Path(folder);source=root/'runtime.c';obj=root/'runtime.o';raw=root/'raw.a'
            # This is a packaging fixture, not a substitute UI implementation.
            source.write_text('#include <stdio.h>\nint unused(void){return 99;}\nint dunit_ui_run_v1(void){puts("archive callable");return 17;}\nint dunit_ui_run_styled_v1(void){puts("styled archive callable");return 23;}\n')
            subprocess.run(['cc','-ffunction-sections','-fdata-sections','-c',str(source),'-o',str(obj)],check=True)
            subprocess.run(['ar','rcs',str(raw),str(obj)],check=True)
            compiler=root/'cargo';compiler.write_text('#!/usr/bin/env python3\nprint('+repr(json.dumps({'reason':'compiler-artifact','target':{'name':'dunit_c'},'filenames':[str(raw)]}))+')\n');compiler.chmod(0o755)
            sysroot=root/'sysroot'
            with patch.object(ensure_c_ui,'cargo',return_value=str(compiler)),contextlib.redirect_stdout(io.StringIO()):
                ensure_c_ui.install(sysroot)
            archive=sysroot/'lib/libdunit_ui.a'
            symbols=subprocess.check_output(['nm','-g','--defined-only',str(archive)],text=True)
            self.assertIn(' dunit_ui_run_v1',symbols);self.assertNotIn(' unused',symbols)
            self.assertIn(' dunit_ui_run_styled_v1',symbols)
            unresolved=subprocess.check_output(['nm','-u',str(archive)],text=True)
            self.assertIn(' puts',unresolved)
            client=root/'client.c';exe=root/'client'
            client.write_text('extern int dunit_ui_run_v1(void); extern int dunit_ui_run_styled_v1(void); int main(void){return dunit_ui_run_v1()!=17 || dunit_ui_run_styled_v1()!=23;}\n')
            subprocess.run(['cc',str(client),str(archive),'-o',str(exe)],check=True)
            result=subprocess.run([str(exe)],capture_output=True,text=True)
            self.assertEqual(result.returncode,0);self.assertEqual(result.stdout,'archive callable\nstyled archive callable\n')
if __name__=='__main__':unittest.main()
