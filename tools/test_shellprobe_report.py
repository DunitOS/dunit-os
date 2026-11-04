import unittest
from sdk_shellprobe_report import verify
class ProbeReport(unittest.TestCase):
    def sample(self):
        stages=' '.join(f'{name}: MetricTime {{ samples: 24, total_ns: 10000000, max_ns: 10000000, clock_failures: 0 }}' for name in ('render','redraw_total','import_attach','commit_result','frame_done','retire'))
        return ('[USER-FAULT] pid=6 reason=page-fault\n[SHELLPROBE] ready pid=55\n'
                '[SHELLPROBE] metrics commits: 24, completed_frames: 24, input_queue_high_water: 2 '+stages+'\n'
                '[USER-FAULT] pid=55 reason=invalid-opcode\n[GUI-EXIT] app=sdk_shellprobe_fixture kind=3 code=-4\n'
                '[SHELLPROBE] survivor armed after reap\n[SHELLPROBE] survivor 24 commits after peer loss OK\n[GUI-EXIT] app=sdk_shell_survivor kind=0 code=0\n')
    def test_complete_observations(self):self.assertEqual(verify(self.sample())['commits'],24)
    def test_survivor_cannot_start_before_reap(self):
        with self.assertRaises(ValueError):verify(self.sample().replace("[SHELLPROBE] survivor armed after reap\n", "").replace("[USER-FAULT] pid=55", "[SHELLPROBE] survivor armed after reap\n[USER-FAULT] pid=55"))
    def test_wrong_fault_identity(self):
        with self.assertRaises(ValueError):verify(self.sample().replace('pid=55 reason','pid=56 reason'))
    def test_additional_client_fault(self):
        with self.assertRaises(ValueError):verify(self.sample()+'[USER-FAULT] pid=54 reason=page-fault\n')
    def test_missing_frame_completion(self):
        with self.assertRaises(ValueError):verify(self.sample().replace('completed_frames: 24','completed_frames: 23'))
    def test_failed_clock(self):
        with self.assertRaises(ValueError):verify(self.sample().replace('clock_failures: 0','clock_failures: 1',1))
    def test_survivor_must_finish(self):
        with self.assertRaises(ValueError):verify(self.sample().replace('survivor 24 commits after peer loss OK','survivor waiting'))
if __name__=='__main__':unittest.main()
