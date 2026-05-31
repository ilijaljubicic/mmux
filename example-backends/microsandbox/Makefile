.DEFAULT_GOAL := help

SANDBOX ?= mmux-node
NODE_ID ?= msb-mmux-1
NODE_NAME ?= Microsandbox mmux node
CONTROLLER_URL ?= http://host.microsandbox.internal:3000
NODE_CONFIG ?= mmux.toml
SNAPSHOT_NAME ?= $(SANDBOX)-seed
BUNDLE ?= .artifacts/$(SANDBOX)-seed.tar.zst
MMUX_MICROSANDBOX ?= mmux-microsandbox-node
MMUX_MICROSANDBOX_DEV = cargo run -p mmux-microsandbox-node --
MMUX_DEV_TARGET ?= x86_64-unknown-linux-musl
MMUX_DEV_BINARY ?= ../../target/$(MMUX_DEV_TARGET)/release/mmux
.PHONY: help build-dev dev-build dev-prepare dev-launch dev-bundle-launch dev-bundle-export dev-status dev-logs dev-resume dev-stop prepare-dev launch-dev bundle-launch-dev prepare launch bundle-launch bundle-export status logs stop resume

help:
	@printf 'Microsandbox mmux backend example\n'
	@printf '\n'
	@printf '  make prepare     Prepare sandbox from NODE_CONFIG without starting mmux node\n'
	@printf '  make launch      Launch mmux node from NODE_CONFIG\n'
	@printf '  make bundle-launch  Import a bundle and launch from that snapshot\n'
	@printf '  make bundle-export  Stop, snapshot, and export a bundle\n'
	@printf '  make status      Show sandbox status\n'
	@printf '  make logs        Show sandbox logs\n'
	@printf '  make resume      Start an existing sandbox\n'
	@printf '  make stop        Stop the sandbox\n'
	@printf '\n'
	@printf '  make build-dev   Build the local development launcher\n'
	@printf '  make dev-prepare Run prepare through cargo for local development\n'
	@printf '  make dev-launch  Run launch through cargo for local development\n'
	@printf '  make dev-bundle-launch  Run bundle-launch through cargo for local development\n'

build-dev:
	cd ../.. && cargo build -p mmux-microsandbox-node
	cd ../.. && rustup target add $(MMUX_DEV_TARGET)
	cd ../.. && cargo build --release --target $(MMUX_DEV_TARGET) --bin mmux

dev-build: build-dev

dev-prepare:
	$(MAKE) prepare MMUX_MICROSANDBOX="$(MMUX_MICROSANDBOX_DEV)" PREPARE_ARGS="--mmux-binary $(MMUX_DEV_BINARY)"

dev-launch:
	$(MAKE) launch MMUX_MICROSANDBOX="$(MMUX_MICROSANDBOX_DEV)"

dev-bundle-launch:
	$(MAKE) bundle-launch MMUX_MICROSANDBOX="$(MMUX_MICROSANDBOX_DEV)"

dev-bundle-export:
	$(MAKE) bundle-export MMUX_MICROSANDBOX="$(MMUX_MICROSANDBOX_DEV)"

dev-status:
	$(MAKE) status MMUX_MICROSANDBOX="$(MMUX_MICROSANDBOX_DEV)"

dev-logs:
	$(MAKE) logs MMUX_MICROSANDBOX="$(MMUX_MICROSANDBOX_DEV)"

dev-resume:
	$(MAKE) resume MMUX_MICROSANDBOX="$(MMUX_MICROSANDBOX_DEV)"

dev-stop:
	$(MAKE) stop MMUX_MICROSANDBOX="$(MMUX_MICROSANDBOX_DEV)"

prepare-dev: dev-prepare

launch-dev: dev-launch

bundle-launch-dev: dev-bundle-launch

prepare:
	$(MMUX_MICROSANDBOX) prepare \
		--name $(SANDBOX) \
		--node-config $(NODE_CONFIG) \
		$(PREPARE_ARGS)

launch:
	$(MMUX_MICROSANDBOX) launch \
		--name $(SANDBOX) \
		--node-config $(NODE_CONFIG) \
		--controller-url $(CONTROLLER_URL) \
		--node-id $(NODE_ID) \
		--node-name "$(NODE_NAME)"

bundle-launch:
	@set -e; \
	imported="$$($(MMUX_MICROSANDBOX) snapshot-import --bundle $(BUNDLE) | sed -n 's/.*"path": "\(.*\)",/\1/p' | head -n 1)"; \
	if [ -z "$$imported" ]; then \
		echo "failed to import snapshot bundle $(BUNDLE)" >&2; \
		exit 1; \
	fi; \
	$(MMUX_MICROSANDBOX) launch \
		--name $(SANDBOX) \
		--node-config $(NODE_CONFIG) \
		--snapshot "$$imported" \
		--controller-url $(CONTROLLER_URL) \
		--node-id $(NODE_ID) \
		--node-name "$(NODE_NAME)"

bundle-export:
	@set -e; \
	if [ "$(origin SANDBOX)" != "command line" ] && [ "$(origin SANDBOX)" != "environment" ] && [ "$(origin SANDBOX)" != "environment override" ]; then \
		echo "SANDBOX is required for bundle-export because the snapshot is tied to a sandbox instance." >&2; \
		echo "Example:" >&2; \
		echo "  make bundle-export SANDBOX=mmux-node SNAPSHOT_NAME=mmux-node-seed BUNDLE=.artifacts/mmux-node-seed.tar.zst" >&2; \
		exit 1; \
	fi; \
	$(MMUX_MICROSANDBOX) snapshot-export \
		--name $(SANDBOX) \
		--snapshot-name $(SNAPSHOT_NAME) \
		--bundle $(BUNDLE)

status:
	$(MMUX_MICROSANDBOX) status \
		--name $(SANDBOX)

logs:
	$(MMUX_MICROSANDBOX) logs \
		--name $(SANDBOX)

resume:
	$(MMUX_MICROSANDBOX) resume \
		--name $(SANDBOX)

stop:
	$(MMUX_MICROSANDBOX) stop \
		--name $(SANDBOX)
