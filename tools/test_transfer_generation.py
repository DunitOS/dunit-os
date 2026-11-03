import json,tempfile,unittest
from pathlib import Path
import sys
sys.path.insert(0,str(Path(__file__).resolve().parent))
from gen_transfer_values import outputs
from gen_abi import ROOT,screaming
class GeneratedTransferTests(unittest.TestCase):
    def test_checked_outputs_are_exact(self):
        for path,content in outputs(ROOT,screaming):self.assertEqual(path.read_text(),content)
    def invalid(self,old,new):
        with tempfile.TemporaryDirectory() as folder:
            root=Path(folder);(root/'abi').mkdir();source=(ROOT/'abi/transfers.abi').read_text();self.assertIn(old,source);(root/'abi/transfers.abi').write_text(source.replace(old,new,1))
            with self.assertRaises(ValueError):outputs(root,screaming)
    def test_duplicate_kind_rejected(self):self.invalid('DragOffer = 2','DragOffer = 1')
    def test_negative_operation_rejected(self):self.invalid('ClipboardOffer = 1','ClipboardOffer = -1')
    def test_out_of_range_kind_rejected(self):self.invalid('ClipboardOffer = 1','ClipboardOffer = 65536')
    def test_boolean_rejected(self):self.invalid('ClipboardOffer = 1','ClipboardOffer = true')
    def test_unknown_section_rejected(self):self.invalid('[Kind]','[Other]')
if __name__=='__main__':unittest.main()
