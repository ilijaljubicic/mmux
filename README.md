# mmux

Tmux remote control over MCP. Operate terminals and coding harnesses with AI agents.

Built in Rust on [`rmcp`](https://crates.io/crates/rmcp). Exposes tmux control as structured tools, resources, and prompts over HTTP stream.

## Prerequisites

mmux is a Cargo workspace. Building the Rust crates only requires Rust/Cargo. Running the local node also requires `tmux`, because the node layer shells out to the local `tmux` binary.
The Microsandbox backend additionally needs the host `libcap-ng` development package so Rust can link `libcap-ng.so.0` during build.

| Dependency | Required for | Install |
|------------|--------------|---------|
| Rust + Cargo | Build, test, run | [rustup.rs](https://rustup.rs) |
| tmux 3.0+ | Local/node runtime | `apt install tmux` / `brew install tmux` / `pacman -S tmux` |
| OpenSSL CLI | Token examples | Usually preinstalled; package name is often `openssl` |
| libcap-ng development files | Microsandbox backend build | `apt install libcap-ng-dev` / `apt install libcap-ng0` plus the matching `-dev` package / distro equivalent that provides `libcap-ng.so.0` |

Optional wire-protocol code generation uses the same Buf/Buffa/connect-rust toolchain as Cormilo Edge:

| Dependency | Required for | Install |
|------------|--------------|---------|
| `buf` | `make wire-generate` | Buf release for your platform |
| `protoc-gen-buffa` | `make wire-generate` | `cargo install --locked protoc-gen-buffa` |
| `protoc-gen-buffa-packaging` | `make wire-generate` | Installed with `protoc-gen-buffa` |
| `protoc-gen-connect-rust` | `make wire-generate` | Install `connectrpc-codegen` from the pinned connect-rust revision below |

The generator and runtime crate must be treated as one dependency pair. Use the same connect-rust revision in both `Cargo.toml` and the codegen install command:

```bash
cargo install \
  --git https://github.com/anthropics/connect-rust \
  --rev d0bee62d4a3f7316d092f9b3920fa694928c60ee \
  connectrpc-codegen \
  --locked

make wire-check-tools
```

## Features

- **21 MCP tools** — node discovery, session lifecycle, interaction, introspection, file I/O, shell execution, coding-CLI adapters
- **MCP resources** — discoverable `profile://` configs and `session://` live output/templates
- **MCP prompts** — pre-built agent guidance for driving coding CLIs and debugging sessions
- **Coder profile system** — TOML-driven behavior for different terminals (opencode, aider, codex, generic, …)
- **Zero hardcoding** — all coder profiles loaded from config; new ones added at runtime via `load_profile`
- **File ops** — read/save with automatic UTF-8/base64 detection, compression sniffing, mime typing

## Quick Start

```bash
cd /mnt/Radni/aitools/mmux
make release

# Start MCP HTTP server (default: 127.0.0.1:3000/mcp)
./target/release/mmux controller --node-config example-backends/local/mmux.toml

# Bind on a different host/port
./target/release/mmux controller --host 0.0.0.0 --port 8080

# Require bearer token for all requests
./target/release/mmux controller --token my-secret-token

# Workspace mode: full terminal control, path APIs fenced to a workspace
MMUX_TOKEN=$(openssl rand -hex 32) \
./target/release/mmux controller --host 0.0.0.0 --security-mode workspace --workspace-root "$PWD"
```

The coder-profile loader looks for `mmux.toml` in the current directory. The checked-in backend configs live under `example-backends/local/` and `example-backends/microsandbox/`. If no config is found, it falls back to built-in coder profiles (opencode, aider, codex, generic).

CLI entrypoints:

| Command | Purpose |
|---------|---------|
| `mmux controller` | Controller entrypoint for the public MCP/control plane |
| `mmux node` | Execution-side node entrypoint; registers to a controller and executes node wire commands |

Useful Make targets:

| Target | Purpose |
|--------|---------|
| `make build` | Debug-build the full workspace |
| `make check` | Type-check the full workspace |
| `make test` | Run workspace tests |
| `make lint` | Run clippy across workspace targets |
| `make release` | Release-build the full workspace |
| `make generate-build` | Generate wire sources, then debug-build the full workspace |
| `make run-controller` | Run `cargo run -- controller` |
| `make run-node` | Run `cargo run -- node` |
| `make wire-check-tools` | Verify Buf/Buffa/connect-rust generators are installed |
| `make wire-generate` | Generate `crates/mmux-wire` ConnectRPC/Buffa sources |

Pass extra CLI flags with the target-specific argument variables:

```bash
LOCAL_ARGS="--port 3333 --allow-remote-without-token" make run-local
NODE_ARGS="--controller-url http://127.0.0.1:3000" make run-node
```

## Coder Profile Configuration

Coder profile behavior is defined in config. Example:

```toml
[coder_profile.opencode]
name = "opencode"
cmd = "opencode"
prompt_indicator = ">"
busy_indicators = ["Processing", "Generating"]
approve_keys = "y Enter"
reject_keys = "n Enter"
cancel_keys = "C-c"
escape_keys = "Escape"

[coder_profile.opencode.launch]
scripts_dir = "./profile_sources/opencode/scripts"
assets_dir = "./profile_sources/opencode/assets"

[coder_profile.opencode.startup_dismiss]
key = "Escape"
triggers = ["Starting MCP servers"]

[coder_profile.aider]
name = "aider"
cmd = "aider"
prompt_indicator = "aider >"
busy_indicators = ["Generating"]
approve_keys = "y Enter"
reject_keys = "n Enter"
cancel_keys = "C-c"
escape_keys = "Escape"

[coder_profile.aider.launch]
scripts_dir = "./profile_sources/aider/scripts"
assets_dir = "./profile_sources/aider/assets"

[coder_profile.generic]
name = "generic"
prompt_indicator = "$"
busy_indicators = []
approve_keys = "y Enter"
reject_keys = "n Enter"
cancel_keys = "C-c"
escape_keys = "Escape"

[coder_profile.codex]
name = "codex"
cmd = "codex"
prompt_indicator = "›"
busy_indicators = ["• Working", "Starting MCP servers"]
approve_keys = "y Enter"
reject_keys = "n Enter"
cancel_keys = "C-c"
escape_keys = "Escape"

[coder_profile.codex.launch]
scripts_dir = "./profile_sources/codex/scripts"
assets_dir = "./profile_sources/codex/assets"

[coder_profile.codex.startup_dismiss]
key = "Escape"
triggers = ["Starting MCP servers"]
```

## MCP Tools

### Session Management
| Tool | Purpose |
|------|---------|
| `start_coding_session` | Start a coding CLI session using a profile-defined command and wait until it is ready |
| `list_coder_profiles` | List loaded coder profiles and their launch config |
| `kill_session` | Kill a tmux session |
| `list_sessions` | List all tmux sessions |
| `list_nodes` | List local and registered execution nodes |
| `node.info` | Describe an execution node |

### Interaction
| Tool | Purpose |
|------|---------|
| `send_input` | Send text input to any tmux session |
| `send_key` | Send a special key (C-c, C-d, Escape, Enter, etc.) |
| `capture_output` | Capture pane output (visible lines or full scrollback) |
| `wait_for` | Wait for stable output, a sentinel string, or a prompt marker |
| `interact` | Send input and wait for output in one call |
| `exec` | Execute a shell command in a session and return clean output (creates session if needed) |

### Session Introspection
| Tool | Purpose |
|------|---------|
| `session_info` | Get detailed info: panes, windows, dimensions, running commands |
| `list_panes` | List panes with dimensions and current commands |
| `check_state` | Quick non-blocking JSON check: is prompt visible? is busy? |
| `resize_pane` | Resize pane (width/height) — fixes garbled TUI layouts |

### File Operations
| Tool | Purpose |
|------|---------|
| `read_file` | Read a file. Returns `content` + `encoding` (utf-8/base64), compression, mime_type |
| `save_file` | Save a file. Accepts `content` + `encoding` (utf-8/base64). Creates parent dirs |

### Coding CLI Adapters (profile-aware)
| Tool | Purpose |
|------|---------|
| `coding_send` | Send a prompt with profile-specific preprocessing (startup dismiss, etc.) |
| `coding_read` | Capture last N lines from the coding CLI pane |
| `coding_action` | Send profile-aware actions: approve, reject, cancel, escape, dismiss |
| `coding_wait_ready` | Wait until the CLI is at a prompt and not busy |

### Profile Management
| Tool | Purpose |
|------|---------|
| `load_profile` | Load a new CLI profile from inline TOML or a file path at runtime |

## MCP Resources

### Static Resources
- `profile://{name}` — Read any loaded profile as JSON

### Resource Templates
- `session://{session_name}/output` — Current pane output (last 200 lines)
- `session://{session_name}/info` — Session metadata (panes, windows, commands)
- `session://{session_name}/scrollback` — Full scrollback history

## MCP Prompts

- `drive-coding-cli` — Best-practice workflow for driving a coding CLI through mmux
- `debug-session` — Diagnostic checklist for stuck or broken tmux sessions

## Security

mmux is a terminal controller. Exposing it to an untrusted client is equivalent to exposing shell access as the server user, plus any file access granted by the selected security mode.

By default mmux binds to `127.0.0.1` only and uses `local` security mode. Remote binds without authentication are refused unless you deliberately use `--allow-remote-without-token` behind localhost-only port forwarding or another trusted network boundary.

To prevent unauthorized access, use bearer token authentication:

```bash
mmux --token $(openssl rand -hex 32)
```

The server also reads `MMUX_TOKEN` by default, or a token file outside the workspace:

```bash
MMUX_TOKEN=$(openssl rand -hex 32) mmux
mmux --token-file /run/secrets/mmux_token
```

All authenticated requests must include the header:
```
Authorization: Bearer <token>
```

Security modes:

| Mode | Intended use | Allows |
|------|--------------|--------|
| `open` | Explicit compatibility mode for trusted networks only | All tools, including arbitrary commands and file paths |
| `local` | Default local development | All tools; non-loopback binds require auth |
| `workspace` | Shared workspace confinement | All terminal tools; path-based APIs and `cwd` are confined to `--workspace-root` |
| `attached` | Drive existing tmux/coding CLI sessions | Input/capture/coding tools; no process launch, file APIs, session killing, or profile file loading |
| `readonly` | Monitoring and diagnostics | Read-only session/profile inspection |

## CLI Options

| Flag | Default | Description |
|------|---------|-------------|
| `--node-config` | `mmux.toml` in cwd | Path to coder profile config TOML |
| `--host` | `127.0.0.1` | Host to bind |
| `--port` | `3000` | Port to bind |
| `--token` | none | Bearer token for request authentication |
| `--token-file` | none | Read bearer token from a file, preferably outside the workspace |
| `--token-env` | `MMUX_TOKEN` | Environment variable used when no token flag/file is set |
| `--security-mode` | `local` | `open`, `local`, `workspace`, `attached`, or `readonly` |
| `--workspace-root` | none | Required in `workspace` mode; confines path-based APIs |

## Health Check

A simple health endpoint is available at `GET /health` (no auth required):

```bash
curl http://localhost:3000/health
# → OK
```

## Sandbox Backends

Core Make targets do not create sandboxes. The Microsandbox backend lives in
`crates/backends/microsandbox/`, and example backend assets live under
`example-backends/microsandbox/`.

The first example is:

```text
example-backends/microsandbox/
```

It contains the backend example Makefile and node profile config used by the
Microsandbox Rust backend crate.

The intended runtime shape is:

- `mmux controller` runs the central MCP/control plane.
- Microsandbox environments run an mmux node-side component.
- Node-side components register back to the controller over outbound ConnectRPC.
- MCP tools accept an optional `node` argument. Omit it for the built-in local node.

The Microsandbox backend example stages `target/release/mmux` into
`example-backends/microsandbox/.artifacts/mmux` before launch. That staging step
keeps the sandbox injection path stable while leaving the actual copy into the
guest inside the Rust backend crate.

Current node wire service:

| RPC | Purpose |
|-----|---------|
| `MmuxNodeRegistryService.RegisterNode` | Register a node descriptor |
| `MmuxNodeRegistryService.Heartbeat` | Refresh node status |
| `MmuxNodeRegistryService.PullCommands` | Poll pending commands |
| `MmuxNodeRegistryService.SubmitCommandResult` | Submit command results |

The canonical proto schema lives in `crates/mmux-wire/proto`.

### MCP Endpoint Headers

rmcp's streamable HTTP transport validates request headers for security. When calling the endpoint directly (e.g., with `curl`), the `Accept` header must include **both** MIME types:

```bash
curl -X POST http://localhost:3000/mcp \
  -H "Accept: application/json, text/event-stream" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $MMUX_TOKEN" \
  -d '{"jsonrpc": "2.0", "id": 1, "method": "tools/list"}'
```

Requests with `Accept: */*` will be rejected by the streamable HTTP transport.

## Architecture

```
mmux/
├── src/main.rs          # Thin binary entrypoint
├── Cargo.toml           # Cargo workspace and root binary package
├── crates/
│   ├── mmux-controller/ # MCP server, auth, policy, and routing
│   ├── backends/
│   │   └── microsandbox/ # Microsandbox Rust backend crate
│   ├── mmux-node/       # Local tmux/filesystem adapter and node profile loading
│   ├── mmux-shared/     # Shared DTOs and profile types
│   └── mmux-wire/       # Controller/node wire DTOs and ConnectRPC proto schema
├── example-backends/
│   └── microsandbox/    # Example assets for the Microsandbox backend crate
├── example-backends/
│   └── microsandbox/
│   ├── local/
│   │   └── mmux.toml    # Local backend coder profile config
│   └── microsandbox/
│       └── mmux.toml    # Microsandbox example coder profile config
├── Makefile             # Build, test, run, and wire generation targets
└── README.md            # This file
```

- **Controller** owns the public MCP endpoint, auth, security modes, and request limits.
- **Node** owns tmux and filesystem execution for the environment it runs in.
- **Wire** owns the controller/node ConnectRPC contract. The checked-in proto currently defines node registration, command pull, command result, and heartbeat surfaces.
- **Node-owned profiles**. The controller consumes local-node profiles during compatibility mode; future registered nodes will own their runtime behavior.

## License

Apache-2.0 — see [LICENSE](LICENSE).
