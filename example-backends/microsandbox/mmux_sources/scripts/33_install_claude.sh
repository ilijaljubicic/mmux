#!/usr/bin/env bash
set -euo pipefail

curl -fsSL https://claude.ai/install.sh | bash

mkdir -p /etc/profile.d
cat >/etc/profile.d/33-mmux-claude-path.sh <<'EOF'
export PATH="$HOME/.local/bin:$PATH"
EOF
