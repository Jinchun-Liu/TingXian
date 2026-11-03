#!/bin/bash
# Source this file from a script; all tools/config stay outside the source tree.
INTERVIEW_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
INTERVIEW_TOOLS="${INTERVIEW_TOOLS:-$(dirname "$INTERVIEW_ROOT")/.meetily-interview-tools}"
export CARGO_HOME="$INTERVIEW_TOOLS/cargo"
export RUSTUP_HOME="$INTERVIEW_TOOLS/rustup"
export PATH="$CARGO_HOME/bin:$INTERVIEW_TOOLS/python/bin:$INTERVIEW_TOOLS/node/node_modules/.bin:$PATH"
export INTERVIEW_PYTHON="$INTERVIEW_TOOLS/python/bin/python"
export NEXT_TELEMETRY_DISABLED=1
export CARGO_BUILD_JOBS=4
export CIDRE_USE_CLT=1
export CLANG_MODULE_CACHE_PATH="${CLANG_MODULE_CACHE_PATH:-$INTERVIEW_TOOLS/clang-module-cache}"
export MACOSX_DEPLOYMENT_TARGET=14.4
if [ -z "${DEEPSEEK_API_KEY:-}" ] && [ -f "$INTERVIEW_TOOLS/deepseek.env" ]; then
  set -a
  source "$INTERVIEW_TOOLS/deepseek.env"
  set +a
fi
