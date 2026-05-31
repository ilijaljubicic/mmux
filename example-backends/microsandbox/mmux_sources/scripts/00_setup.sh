#!/bin/sh
set -eu
export DEBIAN_FRONTEND=noninteractive
if ! command -v tmux >/dev/null 2>&1 || ! command -v ps >/dev/null 2>&1; then
  apt-get update
  apt-get install -y --no-install-recommends \
    bash \
    ca-certificates \
    curl \
    git \
    procps \
    ripgrep \
    tmux
fi
