# MCP + tmux = mmux

mmux is a Rust MCP server for controlling tmux-backed local and remote
execution nodes. It lets agents inspect sessions, drive interactive shells,
operate coding CLIs, read and write files, and route terminal work to sandboxed
environments.

The core idea is simple:

- `mmux controller` exposes the MCP HTTP endpoint and the controller/node wire
  endpoint.
- `mmux node` owns tmux and filesystem access in the environment where it runs.
- MCP tools target the built-in local node by default, or a registered remote
  node when the tool accepts a `node` argument.
- Coder profiles describe how to launch and drive CLIs such as `codex`,
  `opencode`, `aider`, and `kimi`.

## Prerequisites

| Dependency | Required for | Notes |
| ---------- | ------------ | ----- |
| Rust and Cargo | Build, test, run | Install with rustup or your system package manager. |
| tmux | Local and node runtime | `mmux-node` shells out to the system `tmux` binary. |
| OpenSSL CLI | Token examples | Any secure token generator is fine. |
| `libcap-ng` development package | Microsandbox backend | Needed so the Microsandbox backend can link `libcap-ng.so.0`. |

Wire source generation is optional. `make wire-generate` additionally needs
`buf`, `protoc-gen-buffa`, `protoc-gen-buffa-packaging`, and the
`protoc-gen-connect-rust` generator that matches the pinned connect-rust
revision in `Cargo.toml`.

## Quick Start

### Local backend

```bash
make run-local
```

This starts the controller with the built-in local node enabled and loads
`example-backends/local/mmux.toml`.

The MCP endpoint is:

```text
http://<controller-host>:3000/mcp
```

If you are calling the MCP endpoint directly, include both accepted response
types:

```bash
curl -X POST "http://<controller-host>:3000/mcp" \
  -H "Accept: application/json, text/event-stream" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $MMUX_TOKEN" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list"}'
```

### Microsandbox backend

Start a controller that remote nodes can reach:

```bash
export MMUX_TOKEN=<token>
make run-controller CONTROLLER_ARGS="--host <bind-host> --port 3000 --token $MMUX_TOKEN"
```

In another shell, launch the Microsandbox node:

```bash
cd example-backends/microsandbox
export MMUX_TOKEN=<same-token>
make build
make launch CONTROLLER_URL="http://<controller-host>:3000"
```

Use a hostname for `<controller-host>` that resolves from inside the sandbox.
The `allowed_host` setting in `example-backends/microsandbox/mmux.toml` must
match that hostname when the token is injected as a Microsandbox secret.

## CLI Entrypoints

| Command | Purpose |
| ------- | ------- |
| `mmux controller` | Runs the MCP control plane and node registry. |
| `mmux node` | Registers to a controller and executes node-side tmux/file commands. |

`src/main.rs` dispatches to the controller by default, so `cargo run --` and
`cargo run -- controller` both start the controller entrypoint.

Important controller flags:

| Flag | Default | Purpose |
| ---- | ------- | ------- |
| `--node-config` | `mmux.toml` in the current directory, then built-ins | Loads coder profiles. |
| `--host` | loopback interface | Bind host for the HTTP server. |
| `--port` | `3000` | Bind port. |
| `--token` | none | Bearer token for MCP and wire requests. |
| `--token-file` | none | Reads the bearer token from a file. |
| `--token-env` | `MMUX_TOKEN` | Env var used when token flags are omitted. |
| `--security-mode` | `local` | One of `open`, `local`, `workspace`, `attached`, or `readonly`. |
| `--workspace-root` | none | Required in `workspace` mode. |
| `--enable-local-node` | false | Starts the built-in local tmux node in-process. |

Important node flags:

| Flag | Default | Purpose |
| ---- | ------- | ------- |
| `--node-id` | `local` | Node identifier advertised to the controller. |
| `--controller-url` | none | Controller URL to register with. |
| `--node-name` | generated | Human-readable node name. |
| `--controller-token` | `MMUX_CONTROLLER_TOKEN` env fallback | Bearer token for controller wire endpoints. |
| `--poll-interval-ms` | `500` | Command polling interval. |
| `--node-config` | none | Profile TOML loaded by the node process. |

## Make Targets

```bash
make build
make check
make test
make lint
make release
make generate-build
make run-local
make run-controller
make run-node
make wire-check-tools
make wire-generate
```

Pass entrypoint flags through the target variables:

```bash
make run-local LOCAL_ARGS="--node-config example-backends/local/mmux.toml"
make run-controller CONTROLLER_ARGS="--token $MMUX_TOKEN"
make run-node NODE_ARGS="--controller-url http://<controller-host>:3000"
```

## Coder Profiles

Coder profiles live under `[coder_profile.<name>]` in TOML. The controller
loads profiles from `--node-config`, then from `mmux.toml` in the current
directory, then falls back to built-in profiles.

Example:

```toml
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
key = "Down Enter"
triggers = ["Update available!", "Press enter to continue"]

[coder_profile.claude]
name = "claude"
cmd = "claude"
prompt_indicator = "❯"
busy_indicators = ["Thinking", "Working", "Running"]
approve_keys = "y Enter"
reject_keys = "n Enter"
cancel_keys = "C-c"
escape_keys = "Escape"

[coder_profile.claude.startup_dismiss]
key = "Escape"
triggers = [
    "Update available",
    "Claude Code Update Available",
    "A new version of Claude Code is available",
]
```

Profiles can also be loaded at runtime with the `load_profile` MCP tool.

## MCP Surface

Session and node tools:

| Tool | Purpose |
| ---- | ------- |
| `list_nodes` | List registered execution nodes. |
| `node.info` | Describe one execution node. |
| `list_sessions` | List tmux sessions. |
| `kill_session` | Kill a tmux session. |
| `session_info` | Show panes, windows, dimensions, and running commands. |
| `list_panes` | List panes in a session. |
| `resize_pane` | Resize a pane for TUI applications. |

Interaction tools:

| Tool | Purpose |
| ---- | ------- |
| `send_input` | Send text to a session. |
| `send_key` | Send a key such as `C-c`, `Escape`, or `Enter`. |
| `capture_output` | Capture visible output or full scrollback. |
| `wait_for` | Wait for stable output, a sentinel string, or a prompt marker. |
| `interact` | Send input and wait for stable output in one call. |
| `exec` | Run a shell command in a session and return cleaned output. |

Profile-aware coding tools:

| Tool | Purpose |
| ---- | ------- |
| `list_coder_profiles` | List loaded coder profiles. |
| `start_coding_session` | Start a CLI from its profile command. |
| `coding_send` | Send a prompt to a coding CLI. |
| `coding_wait_ready` | Wait until the CLI is at prompt and not busy. |
| `coding_read` | Read recent CLI output. |
| `coding_action` | Send `approve`, `reject`, `cancel`, `escape`, or `dismiss`. |
| `check_state` | Non-blocking JSON readiness check. |
| `load_profile` | Load a profile from inline TOML or a file. |

File tools:

| Tool | Purpose |
| ---- | ------- |
| `read_file` | Reads a file with UTF-8/base64 encoding detection, compression sniffing, and MIME type. |
| `save_file` | Writes UTF-8 or base64 content and creates parent directories. |

## Resources and Prompts

Resources:

- `profile://{name}` returns a loaded profile as JSON.
- `session://{session_name}/output` returns recent pane output.
- `session://{session_name}/info` returns tmux metadata.
- `session://{session_name}/scrollback` returns full pane scrollback.

Prompts:

- `drive-coding-cli` gives the recommended workflow for operating a coding CLI.
- `debug-session` gives a diagnostic checklist for stuck or garbled sessions.

## Security

mmux is a terminal controller. Giving a client access to a writable mmux server
is equivalent to giving that client shell access as the server user, plus any
filesystem access allowed by the selected security mode.

The default bind host is loopback-only. A non-loopback unauthenticated bind is
rejected unless you deliberately pass `--allow-remote-without-token`. For any
non-local exposure, use a bearer token:

```bash
export MMUX_TOKEN="$(openssl rand -hex 32)"
make run-controller CONTROLLER_ARGS="--host <bind-host> --token $MMUX_TOKEN"
```

Authenticated requests must include:

```text
Authorization: Bearer <token>
```

Security modes:

| Mode | Intended use | Allows |
| ---- | ------------ | ------ |
| `open` | Explicit trusted deployments | Full terminal and file operations. |
| `local` | Default local development | Full functionality; remote binds require auth. |
| `workspace` | Shared workspace confinement | Full terminal control with path APIs confined to `--workspace-root`. |
| `attached` | Drive existing sessions | Input/capture/coding tools without process launch, file APIs, or session killing. |
| `readonly` | Monitoring and diagnostics | Read-only session/profile inspection. |

Request and output limits are configurable with `--max-read-bytes`,
`--max-write-bytes`, `--max-timeout-seconds`, `--max-request-bytes`, and
`--max-capture-bytes`.

## Architecture

```text
.
├── src/main.rs
├── crates/
│   ├── mmux-controller/       # MCP server, auth, policy, node registry
│   ├── mmux-node/             # tmux/filesystem adapter and profile loader
│   ├── mmux-shared/           # shared profile and file DTOs
│   ├── mmux-wire/             # ConnectRPC/Buffa wire schema and generated code
│   └── backends/microsandbox/ # Microsandbox launcher crate
├── example-backends/
│   ├── local/                 # local profile config
│   └── microsandbox/          # sandbox config, scripts, assets, README
└── Makefile
```

Runtime flow for remote nodes:

1. `mmux controller` starts MCP and ConnectRPC HTTP routes.
2. A node starts with `mmux node --controller-url ...`.
3. The node registers, sends heartbeats, and polls for commands.
4. MCP tool calls enqueue node commands.
5. The node executes tmux/file work locally and submits results.

The canonical controller/node wire schema lives in
`crates/mmux-wire/proto/mmux/wire/v1/mmux_node.proto`.

## Backend Layout

- `example-backends/local/mmux.toml` defines local coder profiles.
- `example-backends/microsandbox/mmux.toml` defines Microsandbox runtime,
  assets, network policy, secrets, volumes, mounts, patches, and coder profiles.

The Microsandbox backend:

- installs `mmux` from the git ref declared in `mmux.toml`;
- loads shared scripts from `mmux_sources/scripts`;
- loads per-profile setup from `profile_sources/<profile>/scripts`;
- copies `mmux_sources/assets/tmux.conf` into the guest and sources it from
  `/etc/tmux.conf`;
- can snapshot/export/import prepared sandboxes for faster relaunch.

## Health Check

```bash
curl "http://<controller-host>:3000/health"
```

The response body is:

```text
OK
```

## License

Apache-2.0 - see [LICENSE](LICENSE).
