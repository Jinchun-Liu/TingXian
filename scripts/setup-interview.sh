#!/bin/bash
set -euo pipefail
source "$(dirname "$0")/interview-env.sh"
mkdir -p "$INTERVIEW_TOOLS"
if [ "$(uname -s)" != Darwin ] || [ "$(uname -m)" != arm64 ]; then
  echo '此本地构建脚本仅验证 Apple Silicon macOS。'
  exit 1
fi
if [ ! -x "$CARGO_HOME/bin/cargo" ]; then
  /usr/bin/curl --fail --location --proto '=https' --tlsv1.2 https://sh.rustup.rs -o "$INTERVIEW_TOOLS/rustup-init.sh"
  bash "$INTERVIEW_TOOLS/rustup-init.sh" -y --profile minimal --no-modify-path --default-toolchain 1.99.1
fi
if [ ! -x "$INTERVIEW_PYTHON" ]; then python3 -m venv "$INTERVIEW_TOOLS/python"; fi
"$INTERVIEW_PYTHON" -m pip install -r "$INTERVIEW_ROOT/interview/requirements.txt" 'cmake==3.31.6'
npm install --prefix "$INTERVIEW_TOOLS/node" pnpm@9.15.9 --no-audit --no-fund
cd "$INTERVIEW_ROOT/frontend"
pnpm install --frozen-lockfile --store-dir "$INTERVIEW_TOOLS/pnpm-store"
if [ "${1:-}" = '--with-model' ]; then
  INTERVIEW_MODEL_DIR="$HOME/Library/Application Support/io.github.jinchunliu.meetilyinterview/models"
  mkdir -p "$INTERVIEW_MODEL_DIR"
  if [ ! -f "$INTERVIEW_MODEL_DIR/ggml-base.bin" ]; then
    /usr/bin/curl --fail --location --retry 2 --connect-timeout 10 --max-time 600 \
      https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin \
      -o "$INTERVIEW_MODEL_DIR/ggml-base.bin.download"
    INTERVIEW_HASH="$(shasum -a 256 "$INTERVIEW_MODEL_DIR/ggml-base.bin.download" | cut -d ' ' -f 1)"
    if [ "$INTERVIEW_HASH" != '60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe' ]; then
      echo '模型校验不匹配，保留下载文件以供检查；未安装。'; exit 1
    fi
    mv "$INTERVIEW_MODEL_DIR/ggml-base.bin.download" "$INTERVIEW_MODEL_DIR/ggml-base.bin"
  fi
fi
echo '环境准备完毕。设置 DEEPSEEK_API_KEY 后，运行 bash scripts/build-interview.sh。'
