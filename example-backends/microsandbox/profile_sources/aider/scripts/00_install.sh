#!/bin/sh
set -eu
export DEBIAN_FRONTEND=noninteractive
export PATH="/root/.cargo/bin:/root/.local/bin:$PATH"

uv_bin="$(command -v uv || true)"

if [ -z "$uv_bin" ] && [ -x /root/.local/bin/uv ]; then
  uv_bin=/root/.local/bin/uv
fi

"$uv_bin" tool install --force --python python3.12 --with pip aider-chat@latest
