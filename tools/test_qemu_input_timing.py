"""Host monotonic injection-to-marker timing uses the real action sequence."""
import contextlib
import io
import json
import unittest
from unittest.mock import patch
import qemu_test
class Serial:
    text='ready\ndone\n'
    def pump(self):pass
class Qmp:
    def __init__(self):self.calls=[]
    def execute(self,name,arguments):self.calls.append((name,arguments));return {'return':{}}
class Timing(unittest.TestCase):
    @patch('qemu_test.time.perf_counter_ns',side_effect=[100,20000100])
    def test_injection_and_completion_marker_are_bracketed_in_order(self,clock):
        q=Qmp();output=io.StringIO()
        with contextlib.redirect_stdout(output):
            qemu_test.run_command_actions(q,Serial(),[{'measure_start':'keys'},{'execute':'input-send-event','arguments':{'events':[]}},{'wait_marker':'done'},{'measure_end':'keys'}],0,float('inf'))
        self.assertEqual(q.calls,[('input-send-event',{'events':[]})])
        record=json.loads(output.getvalue().split(' ',1)[1]);self.assertEqual(record,{'phase':'keys','wall_ns':20000000})
    def test_unknown_end_duplicate_start_and_unfinished_are_errors(self):
        for actions in [[{'measure_end':'keys'}],[{'measure_start':'x'},{'measure_start':'x'}],[{'measure_start':'x'}],[{'measure_start':''}]]:
            with self.subTest(actions=actions),self.assertRaises(ValueError):qemu_test.run_command_actions(Qmp(),Serial(),actions,0,float('inf'))
    def test_timing_cannot_extend_command_deadline(self):
        with self.assertRaises(RuntimeError):qemu_test.run_command_actions(Qmp(),Serial(),[{'measure_start':'x'}],0,0)
if __name__=='__main__':unittest.main()
