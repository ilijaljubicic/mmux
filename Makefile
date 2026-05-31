.DEFAULT_GOAL := help

.PHONY: help build check test clean lint release generate-build run-local run-controller run-node wire-check-tools wire-generate

LOCAL_ARGS ?= --node-config example-backends/local/mmux.toml
CONTROLLER_ARGS ?=
NODE_ARGS ?=

help:
	@printf 'mmux make targets\n'
	@printf '\nBuild/test:\n'
	@printf '  make build             Debug-build the full Cargo workspace\n'
	@printf '  make check             Type-check the full Cargo workspace\n'
	@printf '  make test              Run workspace tests\n'
	@printf '  make lint              Run clippy across workspace targets\n'
	@printf '  make release           Release-build the full Cargo workspace\n'
	@printf '  make generate-build    Generate wire sources, then debug-build\n'
	@printf '\nRun locally:\n'
	@printf '  make run-local         Run mmux controller with the built-in local node enabled\n'
	@printf '  make run-controller    Run mmux controller\n'
	@printf '  make run-node          Run mmux node\n'
	@printf '\nWire protocol:\n'
	@printf '  make wire-check-tools  Verify buf/buffa/connect-rust generators\n'
	@printf '  make wire-generate     Generate crates/mmux-wire sources\n'
	@printf '\nSandbox backend assets live under example-backends/; core make targets do not create sandboxes.\n'

# Default workspace build (debug)
build:
	cargo build --workspace

# Check the full workspace without producing binaries
check:
	cargo check --workspace

# Release build
release:
	cargo build --workspace --release

# Generate wire sources, then build the full workspace
generate-build: wire-generate build

# Run workspace tests
test:
	cargo test --workspace

# Run clippy across workspace targets
lint:
	cargo clippy --workspace --all-targets

# Run the controller with the built-in local node enabled
run-local:
	cargo run -- controller --enable-local-node $(LOCAL_ARGS)

# Run only the MCP controller entrypoint
run-controller:
	cargo run -- controller $(CONTROLLER_ARGS)

# Run the node scaffold entrypoint
run-node:
	cargo run -- node $(NODE_ARGS)

# Verify the local wire-protocol generator toolchain used by crates/mmux-wire
wire-check-tools:
	which buf
	which protoc-gen-buffa
	which protoc-gen-buffa-packaging
	which protoc-gen-connect-rust

# Generate ConnectRPC/Buffa wire sources from crates/mmux-wire/proto
wire-generate: wire-check-tools
	cd crates/mmux-wire && buf generate

# Clean build artifacts
clean:
	cargo clean
