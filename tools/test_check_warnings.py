"""Warning coverage follows the build and the budget cannot move upward."""
import contextlib
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import check_warnings


class WarningBudgetTests(unittest.TestCase):
    def test_every_packaged_rust_app_is_measured(self):
        with patch.object(check_warnings, "count_rust_warnings", return_value=0) as count:
            self.assertEqual(check_warnings.measure()["total"], 0)
        crates = [call.args[0].name for call in count.call_args_list]
        for app in ("kernel", "gui_server", "gui_calc", "thread_test", "wait_test", "tls_test", "futex_test"):
            self.assertIn(app, crates)
        self.assertEqual(len(crates), len(set(crates)))

    def test_missing_app_fails_before_any_compile(self):
        with patch.object(check_warnings.subprocess, "check_output", return_value="missing-review-app\n"), \
                patch.object(check_warnings, "count_rust_warnings") as count:
            with self.assertRaisesRegex(ValueError, "missing userspace crate"):
                check_warnings.measure()
            count.assert_not_called()

    def test_compiler_selection_matches_makefile(self):
        with patch.object(check_warnings.subprocess, "check_output", side_effect=["gui_calc", "/tool/cargo"]), \
                patch.object(check_warnings, "count_rust_warnings", return_value=0) as count:
            check_warnings.measure()
        self.assertEqual(len(count.call_args_list), 2)
        for call in count.call_args_list:
            self.assertEqual(call.args[1][0], "/tool/cargo")

    def test_duplicate_apps_fail_before_any_compile(self):
        with patch.object(check_warnings.subprocess, "check_output", return_value="gui_calc gui_calc"), \
                patch.object(check_warnings, "count_rust_warnings") as count:
            with self.assertRaisesRegex(ValueError, "distinct userspace applications"):
                check_warnings.measure()
            count.assert_not_called()

    def update(self, budget, measured):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "budget.json"
            original = json.dumps({"kernel": budget, "userspace": 0, "total": budget}) + "\n"
            path.write_text(original)
            with patch.object(check_warnings, "BUDGET_FILE", path), \
                    patch.object(check_warnings, "measure", return_value={"kernel": measured, "userspace": 0, "total": measured}), \
                    patch("sys.argv", ["check_warnings.py", "--update"]), \
                    contextlib.redirect_stdout(io.StringIO()):
                result = check_warnings.main()
            return result, original, path.read_text()

    def test_update_cannot_accept_new_warnings_or_rewrite_budget(self):
        result, original, after = self.update(0, 1)
        self.assertEqual(result, 2)
        self.assertEqual(after, original)

    def test_update_lowers_the_budget(self):
        result, _, after = self.update(1, 0)
        self.assertEqual(result, 0)
        self.assertEqual(json.loads(after)["total"], 0)


if __name__ == "__main__":
    unittest.main()
