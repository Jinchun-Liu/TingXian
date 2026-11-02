#!/usr/bin/env python3
"""Read-only live test observer. Writes metadata/metrics, never transcript text.

Does not restart the app, capture additional audio, invoke AI, or edit history.
CPU/RSS are process observations, not whole-machine or GPU acceptance metrics.
"""
import argparse
import datetime as dt
import hashlib
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import time


def utc():
    return dt.datetime.now(dt.timezone.utc).isoformat()


def processes(pid):
    result = subprocess.run(
        ["ps", "-axo", "pid=,ppid=,%cpu=,rss=,comm="],
        check=True, capture_output=True, text=True, timeout=5,
    )
    rows = {}
    feishu = []
    for line in result.stdout.splitlines():
        fields = line.strip().split(None, 4)
        if len(fields) != 5:
            continue
        ident, parent, cpu, rss, command = fields
        row = {"pid": int(ident), "parent": int(parent), "cpu_pct": float(cpu), "rss_mib": int(rss) / 1024}
        rows[row["pid"]] = row
        if "/Feishu.app/" in command or "/Lark.app/" in command:
            feishu.append(row)
    selected = {pid}
    while True:
        children = {p for p, row in rows.items() if row["parent"] in selected}
        new = children - selected
        if not new:
            break
        selected |= new
    related = [rows[p] for p in selected if p in rows]
    return {
        "app_alive": pid in rows,
        "app_main": rows.get(pid),
        "app_descendants_rss_mib": round(sum(r["rss_mib"] for r in related), 2),
        "app_descendants_cpu_pct": round(sum(r["cpu_pct"] for r in related), 2),
        "app_process_count": len(related),
        "feishu_process_count": len(feishu),
        "feishu_rss_mib": round(sum(r["rss_mib"] for r in feishu), 2),
        "feishu_cpu_pct": round(sum(r["cpu_pct"] for r in feishu), 2),
        "resource_scope": "main PID and descendants; XPC WebKit helpers may be outside this tree; summed RSS may double-count shared pages",
    }


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--pid", type=int, required=True)
    parser.add_argument("--log", type=Path, required=True)
    parser.add_argument("--recordings", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--seconds", type=int, default=3600)
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=False)
    running = True

    def stop(*_):
        nonlocal running
        running = False

    signal.signal(signal.SIGTERM, stop)
    signal.signal(signal.SIGINT, stop)
    started = time.monotonic()
    log_offset = 0
    known = {}
    warnings = set()
    peak_rss = peak_cpu = 0
    last_status = 0
    total_segments = 0
    first_speech = None
    reason = "duration_limit"
    errors = re.compile(r"error|failed|panic|overflow|retract|out of memory|chunks? lost", re.I)
    metadata = {"started_at": utc(), "pid": args.pid, "seconds": args.seconds,
        "log": str(args.log), "recordings": str(args.recordings),
        "limits": ["No AI answer text or request timing available in current log", "Transcripts file omits capture source", "Feishu process presence does not prove active video call", "No additional audio capture; no transcript body copied"]}
    (args.out / "run.json").write_text(json.dumps(metadata, ensure_ascii=False, indent=2))
    print(json.dumps({"event": "monitor_started", "pid": args.pid, "output": str(args.out)}, ensure_ascii=False), flush=True)
    with (args.out / "samples.ndjson").open("w") as samples, (args.out / "events.ndjson").open("w") as events:
        def event(value):
            events.write(json.dumps({"observed_at": utc(), **value}, ensure_ascii=False) + "\n")
            events.flush()

        while running and time.monotonic() - started < args.seconds:
            observation = {"observed_at": utc(), "elapsed_s": round(time.monotonic() - started, 1)}
            try:
                observation.update(processes(args.pid))
            except (subprocess.SubprocessError, OSError) as error:
                event({"type": "process_observation_error", "error": type(error).__name__})
                observation["app_alive"] = None
            if observation["app_alive"] is False:
                reason = "app_process_exited"
                event({"type": reason})
                break
            peak_rss = max(peak_rss, observation.get("app_descendants_rss_mib", 0))
            peak_cpu = max(peak_cpu, observation.get("app_descendants_cpu_pct", 0))
            try:
                size = args.log.stat().st_size
                if size < log_offset:
                    log_offset = 0
                    event({"type": "application_log_rotated"})
                with args.log.open("r", errors="replace") as log:
                    log.seek(log_offset)
                    for line in log:
                        match = errors.search(line)
                        if match:
                            digest = hashlib.sha256(line.encode()).hexdigest()
                            if digest not in warnings:
                                warnings.add(digest)
                                event({"type": "log_error_candidate", "category": match.group().lower(), "line_sha256": digest})
                    log_offset = log.tell()
            except OSError as error:
                event({"type": "log_observation_error", "error": type(error).__name__})
            current_total = 0
            new_segments = 0
            for folder in args.recordings.glob("Interview practice_*"):
                path = folder / "transcripts.json"
                try:
                    document = json.loads(path.read_text())
                    segments = document.get("segments", [])
                    current_total += len(segments)
                    for segment in segments:
                        key = (str(path), segment.get("sequence_id"))
                        text = segment.get("text", "")
                        digest = hashlib.sha256(text.encode()).hexdigest()
                        if known.get(key) == digest:
                            continue
                        if first_speech is None:
                            first_speech = utc()
                        changed = key in known
                        known[key] = digest
                        new_segments += 1
                        event({"type": "transcript_revision" if changed else "transcript_segment",
                            "file": str(path), "sequence_id": segment.get("sequence_id"),
                            "audio_start_s": segment.get("audio_start_time"), "audio_end_s": segment.get("audio_end_time"),
                            "duration_s": segment.get("duration"), "character_count": len(text), "text_sha256": digest})
                except FileNotFoundError:
                    continue
                except (OSError, json.JSONDecodeError) as error:
                    event({"type": "transcript_read_retry", "error": type(error).__name__})
            total_segments = current_total
            observation.update({"transcript_segments": total_segments, "new_or_changed_segments": new_segments,
                "log_error_candidates": len(warnings), "application_log_bytes": log_offset})
            samples.write(json.dumps(observation, ensure_ascii=False) + "\n")
            samples.flush()
            if time.monotonic() - last_status >= 30:
                print(json.dumps({"event": "status", "elapsed_s": observation["elapsed_s"],
                    "transcript_segments": total_segments, "new_segments": new_segments,
                    "rss_mib": observation.get("app_descendants_rss_mib"),
                    "cpu_pct": observation.get("app_descendants_cpu_pct"),
                    "log_error_candidates": len(warnings)}, ensure_ascii=False), flush=True)
                last_status = time.monotonic()
            time.sleep(2)
    if not running:
        reason = "observer_stopped"
    summary = {"ended_at": utc(), "reason": reason, "observed_seconds": round(time.monotonic() - started, 1),
        "first_transcript_seen_at": first_speech, "last_transcript_segment_count": total_segments,
        "peak_observed_process_tree_rss_mib": peak_rss, "peak_observed_process_tree_cpu_pct": peak_cpu,
        "log_error_candidates": len(warnings), "limits": metadata["limits"]}
    (args.out / "summary.json").write_text(json.dumps(summary, ensure_ascii=False, indent=2))
    print(json.dumps({"event": "monitor_finished", **summary}, ensure_ascii=False), flush=True)


if __name__ == "__main__":
    main()
