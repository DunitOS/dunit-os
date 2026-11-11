import json
from pathlib import Path
import tempfile
import unittest
import performance_baseline as baseline
import performance_metadata as metadata


class Contracts(unittest.TestCase):
    def test_extension_retains_the_complete_existing_correctness_budget(self):
        original = json.loads(
            (baseline.ROOT / "tools/sdk_latency_markers.json").read_text()
        )
        expanded = baseline.contract()
        for key in ("required", "forbidden"):
            for item in original[key]:
                self.assertIn(item, expanded[key])
        actions = expanded["run"]["command_actions"]["exec /app/gui_server"]
        labels = [v["measure_start"] for v in actions if "measure_start" in v]
        self.assertEqual(len(labels), 25)
        self.assertEqual(len(set(labels)), 25)
        self.assertIn("wheel-burst", labels)
        for case in ("short", "long", "drag"):
            self.assertTrue(all(f"{case}-{i}" in labels for i in range(1, 7)))

    def test_small_sample_tails_are_explicitly_not_a_latency_sla(self):
        s = baseline.stats([5, 1, 2, 3, 4])
        self.assertEqual((s["p50"], s["p95"], s["p99"]), (3, 5, 5))
        self.assertIn("do not establish", s["tails"])

    def test_qemu_count_is_requested_not_guest_online(self):
        for args, n in [
            (["qemu"], 1),
            (["qemu", "-smp", "4"], 4),
            (["qemu", "-smp=cpus=8,sockets=2"], 8),
        ]:
            self.assertEqual(metadata.cpu_count(args), n)

    def test_hash_records_file_content(self):
        with tempfile.TemporaryDirectory() as folder:
            p = Path(folder) / "image"
            p.write_bytes(b"one")
            a = metadata.digest(p)
            p.write_bytes(b"two")
            self.assertNotEqual(a, metadata.digest(p))


if __name__ == "__main__":
    unittest.main()
