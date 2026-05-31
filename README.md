# MCP + tmux = mmux

mmux is a Rust MCP server for controlling tmux-backed local and remote
execution nodes. It lets agents inspect sessions, drive interactive shells,
operate coding CLIs, read and write files, and route terminal work to sandboxed
environments.

The core idea is simple:

- `mmux controller` exposes the MCP HTTP endpoint and the controller/node wire
  endpoint.
- `mmux node` owns tmux and filesystem access in the environment where it runs.
- Node-aware MCP tools default to `node = "local"`, which uses the built-in
  local node when the controller is started with `--enable-local-node`. Pass a
  `node_id` registered by `mmux node` as the tool's `node` argument to target a
  remote node.
- Built-in coder profiles describe how to launch and drive CLIs such as
  `codex`, `opencode`, `kimi`, and `claude`.

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

### Install Controller

Install the latest released `mmux` binary:

```bash
curl -fsSL https://raw.githubusercontent.com/ilijaljubicic/mmux/main/scripts/install.sh | bash
```

Pin a specific release:

```bash
VERSION=v0.1.0 curl -fsSL https://raw.githubusercontent.com/ilijaljubicic/mmux/main/scripts/install.sh | bash
```

Then run a local controller:

```bash
mmux controller --enable-local-node
```

### Local backend

```bash
make run-local
```

This starts the controller with the built-in local node enabled and uses the
built-in coder profiles.

Warning: the local backend is not sandboxed. Tools run tmux commands and file
operations on the same host as the controller, with the controller process
user's permissions. Use it only for trusted clients and trusted workspaces.

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
make launch NODE_CONFIG=mmux-setup.toml
```

The Microsandbox example uses `http://host.microsandbox.internal:3000` by
default. That is the Microsandbox-provided host alias for reaching the laptop
or host machine from inside the sandbox. When `CONTROLLER_URL` targets that
alias, the launcher automatically allows exactly that host TCP port for node
registration and command polling. The controller token secret is scoped to
`allowed_host = "host.microsandbox.internal"`.

`mmux-setup.toml` is for first-time preparation: it opens DNS plus
public HTTP/HTTPS egress so apt/curl/git/npm/cargo installers can run. Export a
snapshot after setup, then relaunch that snapshot with `mmux.toml` for the
controller-only runtime policy.

If you already have a prepared Docker/OCI image with `tmux`, `mmux`, and the
coder CLIs installed, create an image config from the template and launch with
that config:

```bash
cp mmux-image.toml.example mmux-image.toml
# Edit [microsandbox.runtime].image in mmux-image.toml.
make launch NODE_CONFIG=mmux-image.toml
```

That config has no setup script sections, so launch does not open installer
network access. It only applies the controller URL allow rule and starts the
node from the prepared image.

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
| `--node-config` | `mmux.toml` in the current directory, then built-ins | Loads coder profile overlays. |
| `--host` | loopback interface | Bind host for the HTTP server. |
| `--port` | `3000` | Bind port. |
| `--token` | none | Bearer token for MCP and wire requests. |
| `--token-file` | none | Reads the bearer token from a file. |
| `--token-env` | `MMUX_TOKEN` | Env var used when token flags are omitted. |
| `--workspace-root` | none | Optional root used to confine local `read_file` / `save_file` path APIs. |
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
make run-local LOCAL_ARGS="--port 3001"
make run-controller CONTROLLER_ARGS="--token $MMUX_TOKEN"
make run-node NODE_ARGS="--controller-url http://<controller-host>:3000"
```

Release publishing uses git tags. The release version comes from
`[workspace.package].version` in top-level `Cargo.toml`; all crates inherit it
with `version.workspace = true`. Bump that version with `make update-patch`,
`make update-minor`, or `make update-major`, merge the version change to
`main`, then run `make release-tag` from a clean `main` checkout. The
`v<version>` tag triggers GitHub Actions to build and attach platform archives
used by `scripts/install.sh`.

## Coder Profiles

Coder profiles are built into mmux for common coding CLIs, so local mode works
without a profile TOML file. If you provide `[coder_profile.<name>]` sections
through `--node-config` or `mmux.toml` in the current directory, those sections
overlay the built-ins. Omitted fields keep their built-in values.

Use TOML only for intentional changes:

- backend-agnostic profile tweaks, such as changing a command or prompt marker;
- backend-specific extensions, such as Microsandbox launch scripts/assets;
- new custom profiles.

Nested tables merge with the built-in profile. Scalar and list fields replace
the built-in value, so do not copy full profile definitions unless you want to
own every copied field. See `mmux.toml.example` and
`mmux-microsandbox.toml.example` for copyable examples.

The fields shown here are the backend-agnostic profile shape: they describe how
mmux launches and drives a CLI once a node has the CLI available. Individual
backends may add extra nested sections for environment preparation, assets, or
secrets.

Backend-agnostic override example:

```toml
# Only this field changes. The rest of the built-in codex profile remains
# unchanged.
[coder_profile.codex]
cmd = "codex --model gpt-5"
permission_bypass_cmd = "codex --model gpt-5 --dangerously-bypass-approvals-and-sandbox"
```

`permission_bypass_cmd` is optional and only used when `start_coding_session`
receives `bypass_permissions = true`. Normal sessions always use `cmd`. This
keeps approval/sandbox bypass modes an explicit per-session choice. Built-in
profiles define `permission_bypass_cmd` only for CLIs whose local help exposes a
clear bypass flag.

Microsandbox extension example:

```toml
# This extends the built-in codex profile with backend launch assets. It does
# not override cmd, prompt_indicator, busy_indicators, or action keys.
[coder_profile.codex.launch]
scripts_dir = "./profile_sources/codex/scripts"
assets_dir = "./profile_sources/codex/assets"
```

Profiles can also be loaded at runtime with the `load_profile` MCP tool.

## Sessions

mmux works with tmux sessions. A session name identifies a running terminal on
one node. Node-aware tools accept `node`; if omitted, they target `local`.

There are two common session patterns:

| Session type | Created by | Used with | Meaning |
| ------------ | ---------- | --------- | ------- |
| Shell session | `exec` when needed, or an existing tmux session | `send_input`, `send_key`, `capture_output`, `wait_for`, `session_info`, `list_panes`, `resize_pane` | Generic terminal session with no profile-specific readiness rules. |
| Coder session | `start_coding_session` | `coding_send`, `coding_wait_ready`, `coding_read`, `coding_action`, `check_state` | A tmux session running a coding CLI and interpreted through a coder profile. |

A coder session can also carry one human-readable `objective`, stored as a
tmux session option. Use it to tell agents what that long-lived coding session
is about when several sessions exist.

A coder session is not a separate storage object. It is identified by:

- `node`: where the tmux session lives;
- `session`: the tmux session name;
- `profile`: the CLI interaction rules used to launch/read/drive it.
- `objective`: optional short description of the session's task or intent.

Example:

```json
{
  "node": "msb-mmux-1",
  "session": "codex-main",
  "profile": "codex",
  "objective": "work on mmux release docs"
}
```

`list_sessions` and `session_info` show the objective when it is set. The same
tmux session can be inspected with generic session tools, but coding tools need
the profile so mmux can detect prompts, busy states, startup/update prompts,
and approval actions correctly.

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
| `start_coding_session` | Start a CLI from its profile command, or from `permission_bypass_cmd` when `bypass_permissions = true`. |
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
is equivalent to giving that client shell access as the server user. The local
backend is not sandboxed. Use trusted clients only, or run work through a
sandboxed backend such as Microsandbox.

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

`--workspace-root` is a file API guardrail for the built-in local node: when it
is set, local `read_file`, `save_file`, and local coding-session `cwd`
resolution are confined under that root. It is not a sandbox for terminal
commands.

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

- Local mode uses built-in coder profiles by default.
- `mmux.toml.example` shows optional local/node profile overlays.
- `mmux-microsandbox.toml.example` shows Microsandbox config plus profile
  launch extensions.
- `example-backends/microsandbox/mmux.toml` defines controller-only runtime
  policy for prepared sandboxes.
- `example-backends/microsandbox/mmux-setup.toml` defines first-time
  setup policy with explicit installer egress.
- `example-backends/microsandbox/mmux-image.toml.example` defines runtime from
  a prepared Docker/OCI image without setup scripts; copy it to a real TOML and
  set `[microsandbox.runtime].image`.

The Microsandbox backend:

- can install `mmux` from the configured git ref during setup, or use a
  prepared image/snapshot that already contains `/usr/local/bin/mmux`;
- loads shared scripts from `mmux_sources/scripts` when the selected config
  declares them;
- loads per-profile setup from `profile_sources/<profile>/scripts` when the
  selected config declares them;
- copies `mmux_sources/assets/tmux.conf` into the guest and sources it from
  `/etc/tmux.conf`;
- automatically allows the `host.microsandbox.internal:<port>` controller URL
  as launcher plumbing, without requiring that rule in user TOML;
- can snapshot/export/import prepared sandboxes for faster relaunch.

The checked-in Microsandbox config uses `default_egress = "deny"` and
`default_ingress = "deny"`. Use `[[microsandbox.network.egress]]` rules only
for extra runtime access the sandbox should have beyond controller
communication. Do not mutate the runtime config back and forth for setup. Use
`mmux-setup.toml` for one-time preparation, then create a snapshot and
launch that snapshot with `mmux.toml`.

There are two supported ways to avoid installer egress at runtime:
use a prepared snapshot with `mmux.toml`, or use a prepared Docker/OCI image
with `mmux-image.toml.example`.

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
