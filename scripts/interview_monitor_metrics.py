"""Read-only macOS resource observations; no audio, network or app mutations."""
from collections import Counter
import math
import os
import platform
import re
import statistics
import subprocess


METRICS = (
    "app_descendants_rss_mib", "app_descendants_cpu_pct", "feishu_rss_mib",
    "feishu_cpu_pct", "swap_used_mib", "compressor_mib", "swapins_pages", "swapouts_pages",
)
# This sysctl exports dispatch flags, not the internal VM pressure enum.
# Apple XNU xnu-10063.141.1/bsd/kern/kern_memorystatus_notify.c,
# sysctl_memorystatus_vm_pressure_level; dispatch/source.h constants 1/2/4.
PRESSURE = {1: "normal", 2: "warning", 4: "critical"}


def read_command(command):
    return subprocess.run(command, check=True, capture_output=True, text=True,
                          timeout=5, env={**os.environ, "LC_ALL": "C"}).stdout


def parse_swap(text):
    values = {}
    for name in ("total", "used", "free"):
        match = re.search(rf"\b{name}\s*=\s*(\d+(?:\.\d+)?)\s*([BKMGTP])\b", text)
        if not match:
            raise ValueError("unrecognized swap format")
        amount, unit = match.groups()
        values[f"swap_{name}_mib"] = float(amount) * 1024 ** ("BKMGTP".index(unit) - 2)
    return values


def parse_vm_stat(text):
    match = re.search(r"page size of (\d+) bytes", text)
    if not match or int(match.group(1)) <= 0:
        raise ValueError("missing VM page size")
    page_size = int(match.group(1))
    values = {"vm_page_size_bytes": page_size}
    for label, key in (("Pages occupied by compressor", "compressor_mib"),
                       ("Swapins", "swapins_pages"), ("Swapouts", "swapouts_pages")):
        count = re.search(rf"^{label}:\s+(\d+)\.\s*$", text, re.M)
        if count:
            amount = int(count.group(1))
            values[key] = amount * page_size / 1024 ** 2 if key == "compressor_mib" else amount
    return values


def system_metrics(reader=read_command):
    values = {key: None for key in ("swap_total_mib", "swap_used_mib", "swap_free_mib",
              "memory_pressure_raw", "compressor_mib", "swapins_pages", "swapouts_pages",
              "vm_page_size_bytes")}
    values["memory_pressure"] = "unknown"
    errors = []
    for name, command, parse in (
        ("swap", ["/usr/sbin/sysctl", "-n", "vm.swapusage"], parse_swap),
        ("pressure", ["/usr/sbin/sysctl", "-n", "kern.memorystatus_vm_pressure_level"],
         lambda raw: {"memory_pressure_raw": int(raw.strip())}),
        ("vm_stat", ["/usr/bin/vm_stat"], parse_vm_stat),
    ):
        try:
            values.update(parse(reader(command)))
        except (OSError, subprocess.SubprocessError, ValueError) as error:
            errors.append({"metric": name, "error": type(error).__name__})
    values["memory_pressure"] = PRESSURE.get(values["memory_pressure_raw"], "unknown")
    values["system_metric_errors"] = errors
    return values


def machine_info(reader=read_command):
    info = {"architecture": platform.machine(), "macos": platform.mac_ver()[0],
            "memory_bytes": None, "boot_epoch_s": None, "chip": None, "errors": []}
    for key, name, parse in (
        ("memory_bytes", "hw.memsize", lambda text: int(text.strip())),
        ("boot_epoch_s", "kern.boottime", lambda text: int(re.search(r"\bsec\s*=\s*(\d+)", text).group(1))),
        ("chip", "machdep.cpu.brand_string", str.strip),
    ):
        try:
            info[key] = parse(reader(["/usr/sbin/sysctl", "-n", name]))
        except (OSError, subprocess.SubprocessError, ValueError, AttributeError) as error:
            info["errors"].append({"metric": key, "error": type(error).__name__})
    return info


def distribution(values, total):
    result = {"valid_samples": len(values), "missing_samples": total - len(values),
              "first": None, "last": None, "change": None, "peak_increase_from_first": None,
              "min": None, "median": None, "p95": None, "max": None}
    if values:
        ordered = sorted(values)
        result.update(first=values[0], last=values[-1], change=round(values[-1] - values[0], 3),
                      peak_increase_from_first=round(max(values) - values[0], 3),
                      min=min(values), median=round(statistics.median(values), 3),
                      p95=ordered[math.ceil(len(values) * .95) - 1], max=max(values))
    return result


class ResourceStats:
    def __init__(self):
        self.count = 0
        self.values = {key: [] for key in METRICS}
        self.pressure = Counter()
        self.process_samples = 0
        self.assistant_present_samples = 0
        self.feishu_present_samples = 0
        self.system_error_samples = 0
        self.first_elapsed = self.last_elapsed = None
        self.max_gap = 0

    def add(self, sample):
        self.count += 1
        for key in METRICS:
            value = sample.get(key)
            if isinstance(value, (int, float)) and not isinstance(value, bool) and math.isfinite(value):
                self.values[key].append(value)
        self.pressure[sample.get("memory_pressure", "unknown")] += 1
        if sample.get("assistant_main_process_count") is not None:
            self.process_samples += 1
            self.assistant_present_samples += sample["assistant_main_process_count"] > 0
            self.feishu_present_samples += sample["feishu_process_count"] > 0
        self.system_error_samples += bool(sample.get("system_metric_errors"))
        elapsed = sample["elapsed_s"]
        if self.last_elapsed is not None:
            self.max_gap = max(self.max_gap, elapsed - self.last_elapsed)
        else:
            self.first_elapsed = elapsed
        self.last_elapsed = elapsed

    def summary(self):
        return {"samples": self.count, "valid_process_samples": self.process_samples,
                "assistant_present_samples": self.assistant_present_samples,
                "feishu_present_samples": self.feishu_present_samples,
                "system_error_samples": self.system_error_samples,
                "sample_span_s": round(self.last_elapsed - self.first_elapsed, 2) if self.count else 0,
                "max_sample_gap_s": round(self.max_gap, 2),
                "pressure_sample_counts": dict(self.pressure),
                "metrics": {key: distribution(values, self.count) for key, values in self.values.items()}}


def compare_baseline(baseline, current):
    """Descriptive deltas only. Never infer call quality or app-caused swap."""
    problems = []
    if baseline.get("schema_version") != 2 or baseline.get("phase") != "baseline":
        problems.append("baseline_format_or_phase_invalid")
    if current.get("phase") != "assistant":
        problems.append("current_phase_not_assistant")
    for key in ("architecture", "macos", "memory_bytes", "boot_epoch_s", "chip"):
        before = baseline.get("machine", {}).get(key)
        after = current.get("machine", {}).get(key)
        if not before or not after or before != after:
            problems.append(f"machine_or_boot_mismatch:{key}")
    for label, report in (("baseline", baseline), ("assistant", current)):
        stats = report.get("resources", {})
        count = stats.get("samples", 0)
        if not count or stats.get("valid_process_samples") != count:
            problems.append(f"{label}_process_samples_incomplete")
        if not count or stats.get("feishu_present_samples") != count:
            problems.append(f"{label}_feishu_not_always_observed")
        if label == "baseline" and stats.get("assistant_present_samples", 0):
            problems.append("baseline_contains_assistant")
    result = {"status": "unavailable" if problems else "descriptive_only", "problems": problems,
              "acceptance": "not_evaluated", "deltas": {}}
    if problems:
        return result
    for key in ("feishu_cpu_pct", "feishu_rss_mib", "swap_used_mib", "compressor_mib"):
        before = baseline["resources"].get("metrics", {}).get(key, {})
        after = current["resources"].get("metrics", {}).get(key, {})
        values = {}
        for statistic in ("median", "p95", "change"):
            a, b = before.get(statistic), after.get(statistic)
            values[statistic] = round(b - a, 3) if isinstance(a, (int, float)) and isinstance(b, (int, float)) else None
        result["deltas"][key] = {"assistant_minus_baseline": values,
                                  "baseline_valid_samples": before.get("valid_samples", 0),
                                  "assistant_valid_samples": after.get("valid_samples", 0)}
    return result


def valid_baseline(value):
    if not isinstance(value, dict) or value.get("schema_version") != 2 or value.get("phase") != "baseline":
        return False
    machine, stats = value.get("machine"), value.get("resources")
    if not isinstance(machine, dict) or not isinstance(stats, dict) or not isinstance(stats.get("metrics"), dict):
        return False
    for key in ("samples", "valid_process_samples", "feishu_present_samples", "assistant_present_samples"):
        count = stats.get(key)
        if not isinstance(count, int) or isinstance(count, bool) or count < 0:
            return False
    if any(stats[key] > stats["samples"] for key in ("valid_process_samples", "feishu_present_samples", "assistant_present_samples")):
        return False
    for key in METRICS:
        metric = stats["metrics"].get(key)
        if not isinstance(metric, dict):
            return False
        for label in ("valid_samples", "missing_samples"):
            count = metric.get(label)
            if not isinstance(count, int) or isinstance(count, bool) or count < 0:
                return False
        if metric["valid_samples"] + metric["missing_samples"] != stats["samples"]:
            return False
        for label in ("median", "p95", "change"):
            number = metric.get(label)
            if number is not None and (isinstance(number, bool) or not isinstance(number, (int, float)) or not math.isfinite(number)):
                return False
            if metric["valid_samples"] == 0 and number is not None:
                return False
    return True
