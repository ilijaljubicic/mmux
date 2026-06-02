.DEFAULT_GOAL := help

CARGO_TOML := Cargo.toml

define get_version
$(shell awk '/^\[workspace.package\]/{in_workspace_package=1; next} /^\[/{in_workspace_package=0} in_workspace_package && /^version = /{gsub(/"/, "", $$3); print $$3; exit}' $(CARGO_TOML))
endef

define update_version
	perl -0pi -e 's/(\[workspace\.package\]\nversion = ")[^"]+(")/$${1}$(1)$${2}/' $(CARGO_TOML)
endef

.PHONY: help build check test clean lint release release-tag npm-package npm-pack-dry-run npm-pack update-patch update-minor update-major run-local run-controller run-node wire-check-tools wire-generate

LOCAL_ARGS ?=
CONTROLLER_ARGS ?=
NODE_ARGS ?=
NPM_CACHE ?= /tmp/mmux-npm-cache

help:
	@printf 'mmux make targets\n'
	@printf '\nBuild/test:\n'
	@printf '  make build             Debug-build the full Cargo workspace\n'
	@printf '  make check             Type-check the full Cargo workspace\n'
	@printf '  make test              Run workspace tests\n'
	@printf '  make lint              Run clippy across workspace targets\n'
	@printf '  make release           Release-build the full Cargo workspace\n'
	@printf '\nRelease publishing:\n'
	@printf '  make update-patch      Bump workspace patch version\n'
	@printf '  make update-minor      Bump workspace minor version\n'
	@printf '  make update-major      Bump workspace major version\n'
	@printf '  make release-tag       Tag v$$(workspace.package.version) and push it to trigger GitHub release\n'
	@printf '  make npm-package       Build current-platform npm archive under npm/mmux/artifacts\n'
	@printf '  make npm-pack-dry-run  Build package, then inspect npm pack contents\n'
	@printf '  make npm-pack          Build package, then run npm pack in npm/mmux\n'
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

update-patch:
	@echo "Updating patch version..."
	$(eval CURRENT_VERSION := $(call get_version))
	$(eval NEW_VERSION := $(shell echo $(CURRENT_VERSION) | awk -F. '{$$3=$$3+1} 1' OFS=.))
	$(call update_version,$(NEW_VERSION))
	@echo "Version updated from $(CURRENT_VERSION) to $(NEW_VERSION)"

update-minor:
	@echo "Updating minor version..."
	$(eval CURRENT_VERSION := $(call get_version))
	$(eval NEW_VERSION := $(shell echo $(CURRENT_VERSION) | awk -F. '{$$2=$$2+1; $$3=0} 1' OFS=.))
	$(call update_version,$(NEW_VERSION))
	@echo "Version updated from $(CURRENT_VERSION) to $(NEW_VERSION)"

update-major:
	@echo "Updating major version..."
	$(eval CURRENT_VERSION := $(call get_version))
	$(eval NEW_VERSION := $(shell echo $(CURRENT_VERSION) | awk -F. '{$$1=$$1+1; $$2=0; $$3=0} 1' OFS=.))
	$(call update_version,$(NEW_VERSION))
	@echo "Version updated from $(CURRENT_VERSION) to $(NEW_VERSION)"

release-tag:
	@echo "Creating release tag from current branch..."
	$(eval VERSION := $(call get_version))
	$(eval CURRENT_BRANCH := $(shell git branch --show-current))
	@if [ "$(CURRENT_BRANCH)" != "main" ]; then \
		echo "ERROR: release-tag must be run from main. Current branch: $(CURRENT_BRANCH)"; \
		exit 1; \
	fi
	@if ! git diff-index --quiet HEAD --; then \
		echo "ERROR: working directory has uncommitted changes."; \
		exit 1; \
	fi
	cargo test --workspace
	cargo build --release --bin mmux
	git tag -a v$(VERSION) -m "Release version v$(VERSION)"
	git push origin v$(VERSION)
	@echo "Release v$(VERSION) tagged. GitHub Actions will build and publish artifacts."

npm-package:
	./scripts/npm-package.sh

npm-pack-dry-run: npm-package
	cd npm/mmux && npm --cache $(NPM_CACHE) pack --dry-run

npm-pack: npm-package
	cd npm/mmux && npm --cache $(NPM_CACHE) pack

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
