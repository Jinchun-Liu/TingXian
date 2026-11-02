#!/usr/bin/env python3
"""Check tracked release files without printing candidate secrets."""
import pathlib
import re
import subprocess
import sys

root = pathlib.Path(__file__).resolve().parent.parent
paths = subprocess.check_output(["git", "ls-files", "-z"], cwd=root).decode().split("\0")
patterns = {
    "API token": re.compile(rb"(?:sk-[A-Za-z0-9_-]{24,}|gh[pousr]_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{30,})"),
    "AWS access ID": re.compile(rb"AKIA[A-Z0-9]{16}"),
    "private key": re.compile(rb"-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----"),
}
failures = []
count = 0
for name in filter(None, paths):
    path = root / name
    if not path.is_file():
        continue
    count += 1
    if path.is_symlink():
        failures.append((name, "unexpected tracked symlink"))
        continue
    if (path.name == ".env" or path.suffix in {".env", ".sqlite", ".db", ".docx"}) and not name.endswith(".example"):
        failures.append((name, "local data or credential file"))
    if path.stat().st_size > 50 * 1024 * 1024:
        failures.append((name, "oversized generated artifact"))
    data = path.read_bytes()
    for label, pattern in patterns.items():
        if pattern.search(data):
            failures.append((name, label))

for name, label in failures:
    print(f"FAIL {name}: {label}")
if failures:
    sys.exit(1)
print(f"PASS: {count} tracked source files checked for credential patterns and private data artifacts")
print("Pattern checks do not constitute a complete security audit.")
