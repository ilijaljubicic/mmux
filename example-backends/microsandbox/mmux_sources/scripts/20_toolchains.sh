#!/bin/sh
set -eu
export DEBIAN_FRONTEND=noninteractive

if ! command -v uv >/dev/null 2>&1; then
  curl -LsSf https://astral.sh/uv/install.sh | sh
fi

if ! command -v rustc >/dev/null 2>&1 || ! command -v cargo >/dev/null 2>&1; then
  curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal
fi

export PATH="/root/.cargo/bin:/root/.local/bin:$PATH"

uv_bin="$(command -v uv)"
rustup_bin="$(command -v rustup)"

mkdir -p /etc/profile.d
cat >/etc/profile.d/20-mmux-toolchains.sh <<'EOF'
export PATH="/root/.cargo/bin:/root/.local/bin:$PATH"
EOF

if [ -n "$rustup_bin" ]; then
  "$rustup_bin" toolchain install stable
  "$rustup_bin" component add rustfmt clippy
fi

if [ -n "$uv_bin" ]; then
  "$uv_bin" python install 3.12
fi
