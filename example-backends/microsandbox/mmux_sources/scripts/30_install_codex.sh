#!/bin/sh
set -eu
export DEBIAN_FRONTEND=noninteractive
export PATH="/root/.cargo/bin:/root/.local/bin:$PATH"

if ! command -v bwrap >/dev/null 2>&1; then
  apt-get update
  apt-get install -y --no-install-recommends bubblewrap
fi

if ! command -v codex >/dev/null 2>&1; then
  npm install -g @openai/codex
fi
