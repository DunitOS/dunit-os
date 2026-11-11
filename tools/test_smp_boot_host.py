import tempfile
from pathlib import Path
import unittest
from test_smp_boot import verify


class APProbe(unittest.TestCase):
    def fixture(self, rows):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "serial"
            path.write_text(rows + "\n[PERF-CORE] all checks passed\n")
            return verify(path, 2)

    def test_actual_progress_is_required_beyond_an_online_marker(self):
        rows = "[SMP] cpu=0 apic=0 ready=1 ipi=1 timer=2 work=0\n[SMP] cpu=1 apic=7 ready=1 ipi=1 timer=2 work=4294901760"
        self.assertEqual(self.fixture(rows)["dispatch_cpus"], 1)
        for old, new in [
            ("cpu=1", "cpu=0"),
            ("apic=7", "apic=0"),
            ("ready=1", "ready=0"),
            ("ipi=1", "ipi=0"),
            ("timer=2", "timer=0"),
            ("4294901760", "0"),
        ]:
            with self.subTest(old=old), self.assertRaises(ValueError):
                self.fixture(rows.replace(old, new, 1))


if __name__ == "__main__":
    unittest.main()
