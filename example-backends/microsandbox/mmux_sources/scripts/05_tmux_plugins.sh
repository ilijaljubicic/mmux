#!/bin/sh
set -eu

plugin_root=/mmux/tmux/plugins
mkdir -p "$plugin_root"

if [ ! -d "$plugin_root/tpm/.git" ]; then
  git clone --depth 1 https://github.com/tmux-plugins/tpm "$plugin_root/tpm"
fi

if [ ! -d "$plugin_root/tmux-resurrect/.git" ]; then
  git clone --depth 1 https://github.com/tmux-plugins/tmux-resurrect "$plugin_root/tmux-resurrect"
fi

if [ ! -d "$plugin_root/tmux-continuum/.git" ]; then
  git clone --depth 1 https://github.com/tmux-plugins/tmux-continuum "$plugin_root/tmux-continuum"
fi
