#!/usr/bin/env python3
"""Read-only live test observer. Writes metadata/metrics, never transcript text.

Does not restart the app, capture additional audio, invoke AI, or edit history.
CPU/RSS are process observations, not whole-machine or GPU acceptance metrics.
"""
import argparse
import datetime as dt
import hashlib
import json
from pathlib import Path
import re
import signal
import subprocess
import time

from interview_monitor_metrics import ResourceStats, compare_baseline, machine_info, read_command, system_metrics, valid_baseline


def utc():
    return dt.datetime.now(dt.timezone.utc).isoformat()


def processes(pid, reader=read_command):
    output = reader(["/bin/ps", "-axo", "pid=,ppid=,%cpu=,rss=,lstart=,comm="])
    rows = {}
    feishu = []
    assistant_count = 0
    for line in output.splitlines():
        fields = line.strip().split(None, 9)
        if len(fields) != 10:
            continue
        ident, parent, cpu, rss = fields[:4]
        start, command = " ".join(fields[4:9]), fields[9]
        row = {"pid": int(ident), "parent": int(parent), "cpu_pct": float(cpu), "rss_mib": int(rss) / 1024}
        row["identity"] = hashlib.sha256(f"{start}\n{command}".encode()).hexdigest()
        row["is_assistant"] = Path(command).name == "meetily"
        rows[row["pid"]] = row
        if "/Feishu.app/" in command or "/Lark.app/" in command:
            feishu.append(row)
        if row["is_assistant"]:
            assistant_count += 1
    selected = {pid} if pid else set()
    while True:
        children = {p for p, row in rows.items() if row["parent"] in selected}
        new = children - selected
        if not new:
            break
        selected |= new
    related = [rows[p] for p in selected if p in rows]
    return {
        "app_alive": pid in rows if pid else None,
        "app_main": rows.get(pid),
        "app_descendants_rss_mib": round(sum(r["rss_mib"] for r in related), 2) if related else None,
        "app_descendants_cpu_pct": round(sum(r["cpu_pct"] for r in related), 2) if related else None,
        "app_process_count": len(related),
        "assistant_main_process_count": assistant_count,
        "feishu_process_count": len(feishu),
        "feishu_rss_mib": round(sum(r["rss_mib"] for r in feishu), 2),
        "feishu_cpu_pct": round(sum(r["cpu_pct"] for r in feishu), 2),
        "resource_scope": "main PID and descendants; XPC WebKit helpers may be outside this tree; summed RSS may double-count shared pages",
    }


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--phase", choices=("baseline", "assistant"), default="assistant")
    parser.add_argument("--pid", type=int)
    parser.add_argument("--log", type=Path)
    parser.add_argument("--recordings", type=Path)
    parser.add_argument("--baseline", type=Path, help="completed baseline run directory (optional)")
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--seconds", type=int, default=3600)
    args = parser.parse_args()
    if not 1 <= args.seconds <= 86400:
        parser.error("--seconds must be between 1 and 86400")
    if args.phase == "assistant" and (args.pid is None or args.pid <= 0):
        parser.error("assistant phase requires a positive --pid")
    if args.phase == "baseline" and (args.pid is not None or any((args.log, args.recordings, args.baseline))):
        parser.error("baseline phase accepts no app PID, log, recordings or baseline input")
    baseline = None
    if args.baseline:
        try:
            path = args.baseline / "summary.json"
            if path.stat().st_size > 1024 * 1024:
                raise ValueError("baseline summary exceeds 1 MiB")
            baseline = json.loads(path.read_text())
            if not valid_baseline(baseline):
                raise ValueError("expected a version 2 baseline summary")
        except (OSError, ValueError):
            parser.error("cannot read a valid baseline summary; output has not been created")
    args.out.mkdir(parents=True, exist_ok=False)
    running = True

    def stop(*_):
        nonlocal running
        running = False

    signal.signal(signal.SIGTERM, stop)
    signal.signal(signal.SIGINT, stop)
    log_offset = 0
    known = {}
    warnings = set()
    resources = ResourceStats()
    identity = None
    last_status = 0
    total_segments = 0
    first_speech = None
    reason = "duration_limit"
    errors = re.compile(r"error|failed|panic|overflow|retract|out of memory|chunks? lost", re.I)
    metadata = {"schema_version": 2, "started_at": utc(), "phase": args.phase, "pid": args.pid,
        "seconds": args.seconds, "machine": machine_info(),
        "log": str(args.log) if args.log else None, "recordings": str(args.recordings) if args.recordings else None,
        "baseline": str(args.baseline) if args.baseline else None,
        "limits": ["No AI answer text or request timing available in current log",
            "Legacy transcripts omit capture source; absent/unknown source is never inferred",
            "Feishu process presence does not prove active video call or absence of audiovisual stalls",
            "No additional audio capture; no transcript body copied; existing transcripts may predate this run",
            "CPU uses macOS ps estimates (single core 100%), not interval CPU or GPU utilization",
            "Summed RSS may double-count shared pages; XPC WebKit helpers may be outside the process tree",
            "Pressure is a sampled dispatch level, not Activity Monitor graph or a continuous duration",
            "Swap and compression are system-wide; differences cannot be attributed to this app",
            "Missing/failed samples are unknown, not zero; no automatic acceptance decision",
            "Model identity and comparable video-call/camera/workload conditions require operator annotation"]}
    (args.out / "run.json").write_text(json.dumps(metadata, ensure_ascii=False, indent=2))
    print(json.dumps({"event": "monitor_started", "pid": args.pid, "output": str(args.out)}, ensure_ascii=False), flush=True)
    started = time.monotonic()
    with (args.out / "samples.ndjson").open("w") as samples, (args.out / "events.ndjson").open("w") as events:
        def event(value):
            events.write(json.dumps({"observed_at": utc(), **value}, ensure_ascii=False) + "\n")
            events.flush()

        while running and time.monotonic() - started < args.seconds:
            observation = {"observed_at": utc(), "elapsed_s": round(time.monotonic() - started, 1)}
            try:
                observation.update(processes(args.pid))
            except (subprocess.SubprocessError, OSError, ValueError) as error:
                event({"type": "process_observation_error", "error": type(error).__name__})
                observation["app_alive"] = None
            if observation["app_alive"] is False:
                reason = "app_process_exited"
                event({"type": reason})
                break
            if observation.get("app_main"):
                if not observation["app_main"]["is_assistant"]:
                    reason = "pid_is_not_assistant"
                    event({"type": reason})
                    break
                current_identity = observation["app_main"]["identity"]
                if identity is not None and identity != current_identity:
                    reason = "app_pid_reused"
                    event({"type": reason})
                    break
                identity = current_identity
            observation.update(system_metrics())
            resources.add(observation)
            try:
                if args.log:
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
            for folder in args.recordings.glob("Interview practice_*") if args.recordings else ():
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
                            "source": segment.get("source") if segment.get("source") in ("System", "Microphone") else "unknown",
                            "utterance_id": segment.get("utterance_id"), "microphone_epoch": segment.get("microphone_epoch"),
                            "replaces_utterance": segment.get("replaces_utterance") is True,
                            "audio_start_s": segment.get("audio_start_time"), "audio_end_s": segment.get("audio_end_time"),
                            "duration_s": segment.get("duration"), "character_count": len(text), "text_sha256": digest})
                except FileNotFoundError:
                    continue
                except (OSError, json.JSONDecodeError) as error:
                    event({"type": "transcript_read_retry", "error": type(error).__name__})
            total_segments = current_total
            observation.update({"transcript_segments": total_segments if args.recordings else None,
                "new_or_changed_segments": new_segments if args.recordings else None,
                "log_error_candidates": len(warnings) if args.log else None,
                "application_log_bytes": log_offset if args.log else None})
            samples.write(json.dumps(observation, ensure_ascii=False) + "\n")
            samples.flush()
            if time.monotonic() - last_status >= 30:
                print(json.dumps({"event": "status", "elapsed_s": observation["elapsed_s"],
                    "transcript_segments": observation["transcript_segments"],
                    "new_segments": observation["new_or_changed_segments"],
                    "rss_mib": observation.get("app_descendants_rss_mib"),
                    "cpu_pct": observation.get("app_descendants_cpu_pct"),
                    "log_error_candidates": observation["log_error_candidates"]}, ensure_ascii=False), flush=True)
                last_status = time.monotonic()
            time.sleep(min(2, max(0, args.seconds - (time.monotonic() - started))))
    if not running:
        reason = "observer_stopped"
    stats = resources.summary()
    summary = {"schema_version": 2, "phase": args.phase, "machine": metadata["machine"],
        "started_at": metadata["started_at"], "ended_at": utc(), "reason": reason,
        "observed_seconds": round(time.monotonic() - started, 1), "resources": stats,
        "first_transcript_seen_at": first_speech,
        "last_transcript_segment_count": total_segments if args.recordings else None,
        "peak_observed_process_tree_rss_mib": stats["metrics"]["app_descendants_rss_mib"]["max"],
        "peak_observed_process_tree_cpu_pct": stats["metrics"]["app_descendants_cpu_pct"]["max"],
        "log_error_candidates": len(warnings) if args.log else None, "limits": metadata["limits"]}
    if baseline is not None:
        summary["baseline_comparison"] = compare_baseline(baseline, summary)
    (args.out / "summary.json").write_text(json.dumps(summary, ensure_ascii=False, indent=2))
    print(json.dumps({"event": "monitor_finished", **summary}, ensure_ascii=False), flush=True)


if __name__ == "__main__":
    main()
