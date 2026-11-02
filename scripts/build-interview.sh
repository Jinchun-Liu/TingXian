#!/bin/bash
set -euo pipefail
source "$(dirname "$0")/interview-env.sh"
cd "$INTERVIEW_ROOT/frontend"
pnpm exec tauri build --debug --bundles app
