.DEFAULT_GOAL := sandbox-prepare

SANDBOX ?= mmux-node
SANDBOX_IMAGE ?= debian:bookworm-slim
SANDBOX_MEMORY ?= 4096M
SANDBOX_CPUS ?= 2
REPO_ROOT ?= $(abspath ../..)
WORKSPACE_ROOT ?= $(REPO_ROOT)/workspace
SETUP_ROOT ?= $(abspath .)
NODE_ID ?= msb-mmux-1
NODE_NAME ?= Microsandbox mmux node
CONTROLLER_URL ?= http://127.0.0.1:3000
MMUX ?= mmux
MMUX_DEV = cargo run --manifest-path ../../Cargo.toml --

.PHONY: help _sandbox-create _sandbox-setup _sandbox-tmux sandbox-prepare launch dev-launch

help:
	@printf 'Microsandbox mmux connector example\n'
	@printf '\n'
	@printf '  make sandbox-prepare  Create running sandbox and run setup assets\n'
	@printf '  make launch      Run host-side mmux node connector for an existing sandbox\n'
	@printf '  make dev-launch  Run connector through cargo for local development\n'
	@printf '\n'
	@printf 'Use msb directly for advanced lifecycle operations such as stop, snapshot, import, and export.\n'

_sandbox-create:
	msb create $(SANDBOX_IMAGE) \
		--name $(SANDBOX) \
		--replace \
		--memory $(SANDBOX_MEMORY) \
		--cpus $(SANDBOX_CPUS) \
		--volume $(WORKSPACE_ROOT):/workspace \
		--volume $(SETUP_ROOT):/mmux-setup:ro \
		--workdir /workspace \
		--shell /bin/bash $(MSB_CREATE_ARGS)

_sandbox-setup:
	msb exec $(SANDBOX) -- bash -lc 'set -eu; for script in /mmux-setup/mmux_sources/scripts/*.sh; do bash "$$script"; done'

_sandbox-tmux:
	msb exec $(SANDBOX) -- bash -lc 'set -eu; mkdir -p /mmux; cp /mmux-setup/mmux_sources/assets/tmux.conf /mmux/tmux.conf; printf "source-file /mmux/tmux.conf\n" > /etc/tmux.conf'

sandbox-prepare: _sandbox-create _sandbox-setup _sandbox-tmux

dev-launch:
	$(MAKE) launch MMUX="$(MMUX_DEV)"

launch:
	$(MMUX) node \
		--backend microsandbox \
		--sandbox-name $(SANDBOX) \
		--controller-url $(CONTROLLER_URL) \
		--node-id $(NODE_ID) \
		--node-name "$(NODE_NAME)" $(NODE_ARGS)
