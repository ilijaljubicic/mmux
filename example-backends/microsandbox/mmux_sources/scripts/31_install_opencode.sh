#!/bin/sh
set -eu
export DEBIAN_FRONTEND=noninteractive
export PATH="/root/.cargo/bin:/root/.local/bin:$PATH"

if ! command -v tsc >/dev/null 2>&1; then
  npm install -g typescript
fi

if ! command -v opencode >/dev/null 2>&1; then
  npm install -g opencode-ai
fi
