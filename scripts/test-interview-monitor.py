"""Synthetic parser/coverage/comparison checks; no audio, files or network."""
import copy
import importlib.util
from pathlib import Path
import subprocess
import sys
import unittest
import uuid

from interview_monitor_metrics import (
    METRICS, ResourceStats, compare_baseline, distribution, machine_info,
    parse_swap, parse_vm_stat, system_metrics, valid_baseline,
)

spec = importlib.util.spec_from_file_location("monitor", Path(__file__).with_name("monitor-interview.py"))
monitor = importlib.util.module_from_spec(spec)
spec.loader.exec_module(monitor)


class MonitorTests(unittest.TestCase):
    def test_swap_units_and_invalid_formats(self):
        value = parse_swap("total = 4.00G used = 512.50M free = 524288K (encrypted)")
        self.assertEqual(value, {"swap_total_mib": 4096, "swap_used_mib": 512.5, "swap_free_mib": 512})
        self.assertEqual(parse_swap("total=0B used=0B free=0B")["swap_used_mib"], 0)
        for raw in ("Operation not permitted", "total=1M used=3X free=0M", "total=1M used=-2M free=3M"):
            with self.assertRaises(ValueError):
                parse_swap(raw)

    def test_vm_page_size_not_assumed_and_missing_not_zero(self):
        for page_size in (4096, 16384):
            result = parse_vm_stat(f"page size of {page_size} bytes\nPages occupied by compressor: 256.\nSwapins: 12.\nSwapouts: 15.\n")
            self.assertEqual(result["compressor_mib"], 256 * page_size / 1024 ** 2)
            self.assertEqual(result["swapouts_pages"], 15)
        self.assertNotIn("swapouts_pages", parse_vm_stat("page size of 16384 bytes"))
        with self.assertRaises(ValueError):
            parse_vm_stat("invalid")

    def test_partial_failure_and_unknown_pressure_keep_other_metrics(self):
        def reader(command):
            if command[-1] == "vm.swapusage":
                raise subprocess.CalledProcessError(1, command, stderr="synthetic private error")
            if command[-1] == "kern.memorystatus_vm_pressure_level":
                return "8\n"
            return "page size of 16384 bytes\nPages occupied by compressor: 64.\nSwapins: 1.\nSwapouts: 2.\n"
        result = system_metrics(reader)
        self.assertIsNone(result["swap_used_mib"])
        self.assertEqual(result["memory_pressure"], "unknown")
        self.assertEqual(result["memory_pressure_raw"], 8)
        self.assertEqual(result["compressor_mib"], 1)
        self.assertNotIn("private", str(result))
        for raw, expected in ((1, "normal"), (2, "warning"), (4, "critical")):
            def with_pressure(command):
                return str(raw) if command[-1] == "kern.memorystatus_vm_pressure_level" else reader(command)
            self.assertEqual(system_metrics(with_pressure)["memory_pressure"], expected)

    def test_no_samples_never_claim_zero_peaks(self):
        summary = ResourceStats().summary()
        self.assertEqual(summary["samples"], 0)
        for key in METRICS:
            self.assertIsNone(summary["metrics"][key]["max"])

    def test_distribution_signed_growth_peaks_and_coverage(self):
        result = distribution([200, 300, 100], 4)
        self.assertEqual(result["change"], -100)
        self.assertEqual(result["peak_increase_from_first"], 100)
        self.assertEqual(result["missing_samples"], 1)
        self.assertEqual(result["p95"], 300)
        stats = ResourceStats()
        for elapsed, pressure in ((0, "normal"), (2, "warning"), (18, "unknown")):
            stats.add({"elapsed_s": elapsed, "memory_pressure": pressure})
        self.assertEqual(stats.summary()["max_sample_gap_s"], 16)
        self.assertEqual(stats.summary()["valid_process_samples"], 0)
        self.assertEqual(stats.summary()["pressure_sample_counts"], {"normal": 1, "warning": 1, "unknown": 1})

    def test_process_tree_identity_and_baseline_contamination(self):
        output = """10 1 10.5 2048 Tue Nov 3 04:30:11 2026 /test/Meetily Interview.app/Contents/MacOS/meetily
11 10 2.0 1024 Tue Nov 3 04:30:12 2026 /test/child
12 11 4.0 512 Tue Nov 3 04:30:13 2026 /test/grandchild
20 1 25.0 4096 Tue Nov 3 04:20:00 2026 /Applications/Feishu.app/Contents/MacOS/Feishu
21 20 5.0 2048 Tue Nov 3 04:20:01 2026 /Applications/Feishu.app/Contents/Frameworks/helper
30 1 90.0 9999 Tue Nov 3 04:20:00 2026 /unrelated
"""
        result = monitor.processes(10, lambda _: output)
        self.assertEqual(result["app_descendants_rss_mib"], 3.5)
        self.assertEqual(result["app_descendants_cpu_pct"], 16.5)
        self.assertEqual(result["feishu_cpu_pct"], 30)
        self.assertEqual(result["assistant_main_process_count"], 1)
        self.assertTrue(result["app_main"]["is_assistant"])
        self.assertFalse(monitor.processes(30, lambda _: output)["app_main"]["is_assistant"])
        reused = monitor.processes(10, lambda _: output.replace("04:30:11", "06:30:11"))
        self.assertNotEqual(result["app_main"]["identity"], reused["app_main"]["identity"])
        baseline = monitor.processes(None, lambda _: output)
        self.assertIsNone(baseline["app_alive"])
        self.assertIsNone(baseline["app_descendants_rss_mib"])
        self.assertEqual(baseline["assistant_main_process_count"], 1)
        self.assertFalse(monitor.processes(99, lambda _: output)["app_alive"])

    def report(self, phase):
        stats = ResourceStats()
        for elapsed in (0, 2, 4):
            stats.add({"elapsed_s": elapsed, "assistant_main_process_count": int(phase == "assistant"),
                       "feishu_process_count": 2, "memory_pressure": "normal", "swap_used_mib": 100 + elapsed,
                       "feishu_cpu_pct": 20, "feishu_rss_mib": 500, "compressor_mib": 150})
        return {"schema_version": 2, "phase": phase, "resources": stats.summary(),
                "machine": {"architecture": "arm64", "macos": "14.6.1", "memory_bytes": 17179869184,
                            "boot_epoch_s": 100, "chip": "synthetic"}}

    def test_comparison_is_descriptive_and_rejects_bad_baselines(self):
        baseline, current = self.report("baseline"), self.report("assistant")
        self.assertTrue(valid_baseline(baseline))
        self.assertFalse(valid_baseline(current))
        result = compare_baseline(baseline, current)
        self.assertEqual(result["status"], "descriptive_only")
        self.assertEqual(result["acceptance"], "not_evaluated")
        self.assertEqual(result["deltas"]["swap_used_mib"]["assistant_minus_baseline"]["change"], 0)
        for section, key, value, problem in (
            ("machine", "boot_epoch_s", 101, "machine_or_boot_mismatch:boot_epoch_s"),
            ("machine", "memory_bytes", None, "machine_or_boot_mismatch:memory_bytes"),
            ("resources", "assistant_present_samples", 1, "baseline_contains_assistant"),
            ("resources", "valid_process_samples", 2, "baseline_process_samples_incomplete"),
            ("resources", "feishu_present_samples", 0, "baseline_feishu_not_always_observed"),
        ):
            changed = copy.deepcopy(baseline)
            changed[section][key] = value
            result = compare_baseline(changed, current)
            self.assertIn(problem, result["problems"])
            self.assertEqual(result["deltas"], {})

    def test_malformed_baseline_shape_and_numbers(self):
        baseline = self.report("baseline")
        for raw in (None, [], {}, {**baseline, "resources": "wrong"}, {**baseline, "machine": None}):
            self.assertFalse(valid_baseline(raw))
        for value in (float("nan"), float("inf"), True, "100"):
            changed = copy.deepcopy(baseline)
            changed["resources"]["metrics"]["swap_used_mib"]["median"] = value
            self.assertFalse(valid_baseline(changed))

    def test_invalid_cli_does_not_create_output(self):
        output = Path("/private/tmp") / f"interview-monitor-invalid-{uuid.uuid4()}"
        for options in (("--phase", "baseline", "--pid", "0"),
                        ("--phase", "assistant"), ("--phase", "baseline", "--seconds", "0"),
                        ("--pid", "1", "--baseline", str(output / "missing"))):
            result = subprocess.run([sys.executable, str(Path(monitor.__file__)), "--out", str(output), *options],
                                    capture_output=True, text=True, timeout=5)
            self.assertEqual(result.returncode, 2)
            self.assertFalse(output.exists())

    def test_machine_info_permission_failure_never_guesses(self):
        def denied(_):
            raise PermissionError("synthetic")
        result = machine_info(denied)
        self.assertIsNone(result["memory_bytes"])
        self.assertIsNone(result["boot_epoch_s"])
        self.assertEqual(len(result["errors"]), 3)


if __name__ == "__main__":
    unittest.main()
