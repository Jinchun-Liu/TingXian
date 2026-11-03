#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")"
source scripts/interview-env.sh
export RUST_LOG=warn
INTERVIEW_APP="$INTERVIEW_ROOT/target/debug/bundle/macos/Meetily Interview.app/Contents/MacOS/meetily"
if [ ! -x "$INTERVIEW_APP" ]; then
  echo '尚未构建应用。请先运行 bash scripts/build-interview.sh。'
  exit 1
fi
exec "$INTERVIEW_APP"
