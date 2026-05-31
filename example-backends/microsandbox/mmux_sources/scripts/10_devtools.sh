#!/bin/sh
set -eu
export DEBIAN_FRONTEND=noninteractive

if ! command -v python3 >/dev/null 2>&1 || \
   ! command -v pip3 >/dev/null 2>&1 || \
   ! command -v node >/dev/null 2>&1 || \
   ! command -v npm >/dev/null 2>&1 || \
   ! command -v make >/dev/null 2>&1 || \
   ! command -v gcc >/dev/null 2>&1 || \
   ! command -v g++ >/dev/null 2>&1 || \
   ! command -v pkg-config >/dev/null 2>&1 || \
   ! command -v jq >/dev/null 2>&1 || \
   ! command -v less >/dev/null 2>&1; then
  apt-get update
  apt-get install -y --no-install-recommends \
    gcc \
    g++ \
    jq \
    less \
    make \
    nodejs \
    npm \
    pkg-config \
    python3
fi
