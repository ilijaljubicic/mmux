#!/bin/sh
set -eu

if [ -z "${MMUX_SOURCE_REPO:-}" ] || [ -z "${MMUX_SOURCE_REF:-}" ]; then
  exit 0
fi

desired_stamp="${MMUX_SOURCE_REPO}@${MMUX_SOURCE_REF}"
stamp_file=/mmux/mmux-source.stamp

if [ -x /usr/local/bin/mmux ] && [ -f "$stamp_file" ] && [ "$(cat "$stamp_file" 2>/dev/null || true)" = "$desired_stamp" ]; then
  exit 0
fi

export DEBIAN_FRONTEND=noninteractive
export PATH="/root/.cargo/bin:/root/.local/bin:/usr/local/bin:$PATH"

cargo install --locked --force --git "$MMUX_SOURCE_REPO" --rev "$MMUX_SOURCE_REF" --bin mmux --root /usr/local
printf '%s\n' "$desired_stamp" >"$stamp_file"
