#!/bin/sh
set -eu
export DEBIAN_FRONTEND=noninteractive
export PATH="/root/.cargo/bin:/root/.local/bin:/root/.kimi-code/bin:$PATH"

if ! command -v kimi >/dev/null 2>&1; then
  curl -fsSL https://code.kimi.com/kimi-code/install.sh | bash
fi

if ! command -v kimi >/dev/null 2>&1; then
  echo "kimi install completed, but kimi is not on PATH" >&2
  exit 1
fi
