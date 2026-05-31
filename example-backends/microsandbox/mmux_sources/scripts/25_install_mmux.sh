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

case "$MMUX_SOURCE_REF" in
  refs/heads/*)
    ref_name="${MMUX_SOURCE_REF#refs/heads/}"
    cargo install --locked --force --git "$MMUX_SOURCE_REPO" --branch "$ref_name" --bin mmux --root /usr/local mmux
    ;;
  refs/tags/*)
    ref_name="${MMUX_SOURCE_REF#refs/tags/}"
    cargo install --locked --force --git "$MMUX_SOURCE_REPO" --tag "$ref_name" --bin mmux --root /usr/local mmux
    ;;
  v[0-9]*)
    cargo install --locked --force --git "$MMUX_SOURCE_REPO" --tag "$MMUX_SOURCE_REF" --bin mmux --root /usr/local mmux
    ;;
  *)
    if printf '%s\n' "$MMUX_SOURCE_REF" | grep -Eq '^[0-9a-fA-F]{7,40}$'; then
      cargo install --locked --force --git "$MMUX_SOURCE_REPO" --rev "$MMUX_SOURCE_REF" --bin mmux --root /usr/local mmux
    else
      cargo install --locked --force --git "$MMUX_SOURCE_REPO" --branch "$MMUX_SOURCE_REF" --bin mmux --root /usr/local mmux
    fi
    ;;
esac
printf '%s\n' "$desired_stamp" >"$stamp_file"
