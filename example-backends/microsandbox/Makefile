.DEFAULT_GOAL := help

IMAGE ?= debian:bookworm-slim
SANDBOX ?= mmux-node
NODE_ID ?= msb-mmux-1
NODE_NAME ?= Microsandbox mmux node
CONTROLLER_URL ?= http://10.0.2.2:3000
NODE_CONFIG ?= mmux.toml
SNAPSHOT_NAME ?= $(SANDBOX)-seed
BUNDLE ?= .artifacts/$(SANDBOX)-seed.tar.zst
ARTIFACT_DIR ?= .artifacts
MMUX_BINARY ?= $(ARTIFACT_DIR)/mmux
MMUX_TARGET ?= x86_64-unknown-linux-musl

.PHONY: help build launch bundle-launch bundle-export status logs stop resume

help:
	@printf 'Microsandbox mmux backend example\n'
	@printf '\n'
	@printf '  make build       Build and stage the workspace mmux binary\n'
	@printf '  make launch      Launch mmux node inside Microsandbox\n'
	@printf '  make bundle-launch  Import a bundle and launch from that snapshot\n'
	@printf '  make bundle-export  Stop, snapshot, and export a bundle\n'
	@printf '  make status      Show sandbox status\n'
	@printf '  make logs        Show sandbox logs\n'
	@printf '  make resume      Start an existing sandbox\n'
	@printf '  make stop        Stop the sandbox\n'

build:
	cd ../.. && cargo build --release --target $(MMUX_TARGET)
	mkdir -p $(ARTIFACT_DIR)
	cp ../../target/$(MMUX_TARGET)/release/mmux $(MMUX_BINARY)
launch: build
	cargo run -p mmux-microsandbox-node -- launch \
		--name $(SANDBOX) \
		--image $(IMAGE) \
		--node-config $(NODE_CONFIG) \
		--controller-url $(CONTROLLER_URL) \
		--node-id $(NODE_ID) \
		--node-name "$(NODE_NAME)"

bundle-launch: build
	@set -e; \
	imported="$$(cargo run -p mmux-microsandbox-node -- snapshot-import --bundle $(BUNDLE) | sed -n 's/.*"path": "\(.*\)",/\1/p' | head -n 1)"; \
	if [ -z "$$imported" ]; then \
		echo "failed to import snapshot bundle $(BUNDLE)" >&2; \
		exit 1; \
	fi; \
	cargo run -p mmux-microsandbox-node -- launch \
		--name $(SANDBOX) \
		--image $(IMAGE) \
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
	cargo run -p mmux-microsandbox-node -- snapshot-export \
		--name $(SANDBOX) \
		--image $(IMAGE) \
		--snapshot-name $(SNAPSHOT_NAME) \
		--bundle $(BUNDLE)

status:
	cargo run -p mmux-microsandbox-node -- status \
		--name $(SANDBOX) \
		--image $(IMAGE)

logs:
	cargo run -p mmux-microsandbox-node -- logs \
		--name $(SANDBOX) \
		--image $(IMAGE)

resume:
	cargo run -p mmux-microsandbox-node -- resume \
		--name $(SANDBOX) \
		--image $(IMAGE)

stop:
	cargo run -p mmux-microsandbox-node -- stop \
		--name $(SANDBOX) \
		--image $(IMAGE)
