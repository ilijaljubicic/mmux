#!/bin/sh
set -eu

if [ "${MMUX_SKIP_RELEASE_INSTALL:-}" = "1" ]; then
  exit 0
fi

if [ -z "${MMUX_VERSION:-}" ]; then
  exit 0
fi

repo="${MMUX_REPO:-ilijaljubicic/mmux}"
desired_stamp="${repo}@${MMUX_VERSION}"
stamp_file=/mmux/mmux-release.stamp

if [ -x /usr/local/bin/mmux ] && [ -f "$stamp_file" ] && [ "$(cat "$stamp_file" 2>/dev/null || true)" = "$desired_stamp" ]; then
  exit 0
fi

arch_raw="$(uname -m)"
case "$arch_raw" in
  x86_64|amd64) arch="x86_64" ;;
  aarch64|arm64) arch="arm64" ;;
  *) echo "unsupported architecture for mmux release install: $arch_raw" >&2; exit 1 ;;
esac

archive="mmux-linux-${arch}.tar.gz"
url="https://github.com/${repo}/releases/download/${MMUX_VERSION}/${archive}"
tmpdir="$(mktemp -d)"
trap 'rm -rf "$tmpdir"' EXIT

curl -fsSL "$url" -o "$tmpdir/$archive"
tar -xzf "$tmpdir/$archive" -C "$tmpdir"

binary=""
for candidate in "$tmpdir/mmux" "$tmpdir/mmux-linux-${arch}"; do
  if [ -f "$candidate" ]; then
    binary="$candidate"
    break
  fi
done

if [ -z "$binary" ]; then
  echo "archive did not contain an mmux binary" >&2
  exit 1
fi

install -m 0755 "$binary" /usr/local/bin/mmux
printf '%s\n' "$desired_stamp" >"$stamp_file"
