# MCP + tmux = mmux

mmux is a Rust MCP server that lets agents operate coding harnesses and other
tmux-backed terminal workflows across local or remote execution nodes. Agents
can inspect sessions, drive interactive shells and coding CLIs, read and write
files, and route terminal work to sandboxed environments.

The core idea is simple:

- `mmux controller` exposes the MCP HTTP endpoint and the controller/node wire
  endpoint.
- `mmux node` owns tmux and filesystem access in the environment where it runs.
- mmux can run as one process for local convenience, or as a distributed
  controller plus one or more node processes. In single-process mode, the
  controller embeds a node backend and exposes it as the reserved `local` node.
  In distributed mode, each `mmux node` registers with the controller over the
  node wire RPC API.
- Node-aware MCP tools accept a `node` argument. Omitted `node` means `local`.
  To target a distributed node, pass the node id registered by `mmux node`.
- Built-in coder profiles describe how to launch and drive CLIs such as
  `codex`, `opencode`, `kimi`, and `claude`.

## Project Status

mmux is in early development. The project aims to provide a secure control
plane for terminal automation, especially when paired with sandboxed backends,
but interfaces, configuration, and backend behavior may still change in
breaking ways. Security guarantees cannot be made at this stage. Review the
configuration for your environment and use mmux at your own risk.

## Prerequisites

| Dependency | Required for | Notes |
| ---------- | ------------ | ----- |
| Node.js and npm | `npx @mmux/mmux` quick start | The npm package downloads and runs the native `mmux` binary for the current platform. |
| Rust and Cargo | Build, test, run | Install with rustup or your system package manager. |
| tmux | Local and node runtime | `mmux-node` shells out to the system `tmux` binary. |
| Microsandbox | Microsandbox backend | Required only when running the Microsandbox backend. |
| `libcap-ng` development package | Microsandbox backend | Needed so the Microsandbox backend can link `libcap-ng.so.0`. |

Wire source generation is only needed when editing the protobuf schema. The
generated Rust sources are checked in, so normal builds do not require `buf` or
the protobuf generators. If you change files under `crates/mmux-wire/proto`,
run `make wire-generate`, which additionally needs `buf`, `protoc-gen-buffa`,
`protoc-gen-buffa-packaging`, and the `protoc-gen-connect-rust` generator that
matches the pinned connect-rust revision in `Cargo.toml`.

## Quick Start

### Run from npm

For a local loopback-only MCP server with the built-in local tmux backend:

```bash
env -u MMUX_MCP_TOKEN -u MMUX_WIRE_TOKEN \
  npx --yes @mmux/mmux controller --enable-local-node
```

The MCP endpoint is:

```text
http://127.0.0.1:3000/mcp
```

Register that HTTP MCP server with Codex:

```bash
codex mcp add mmux --url http://127.0.0.1:3000/mcp
```

Register it with Claude Code:

```bash
claude mcp add --transport http mmux http://127.0.0.1:3000/mcp
```

For authenticated local setup:

```bash
export MMUX_MCP_TOKEN="$(openssl rand -hex 32)"
npx --yes @mmux/mmux controller --enable-local-node --mcp-token-env MMUX_MCP_TOKEN
codex mcp add mmux --url http://127.0.0.1:3000/mcp --bearer-token-env-var MMUX_MCP_TOKEN
claude mcp add --transport http mmux http://127.0.0.1:3000/mcp --header "Authorization: Bearer $MMUX_MCP_TOKEN"
```

For local Microsandbox mode, prepare the sandbox with `msb` and run the same
npm package with the embedded Microsandbox node:

```bash
cd example-backends/microsandbox
make sandbox-prepare
cd ../..
npx --yes @mmux/mmux controller --enable-microsandbox-node --sandbox-name mmux-node
```

### Install native binary

Install the latest released `mmux` binary:

```bash
curl -fsSL https://raw.githubusercontent.com/ilijaljubicic/mmux/main/scripts/install.sh | bash
```

Linux release archives include one `mmux` binary. On Linux, that binary
includes the `mmux node --backend microsandbox` connector.

Pin a specific release:

```bash
VERSION=v0.1.0 curl -fsSL https://raw.githubusercontent.com/ilijaljubicic/mmux/main/scripts/install.sh | bash
```

Then run a local controller:

```bash
mmux controller --enable-local-node
```

That is the single-binary local mode: the controller and local tmux backend run
in one process.

### Local backend

From a repo checkout, the equivalent development command is:

```bash
make run-local
```

Both commands start the controller with the built-in local node enabled and use
the built-in coder profiles.

Warning: the local backend is not sandboxed. Tools run tmux commands and file
operations on the same host as the controller, with the controller process
user's permissions. Use it only for trusted clients and trusted workspaces.

If you are calling the MCP endpoint directly, include both accepted response
types:

```bash
curl -X POST "http://<controller-host>:3000/mcp" \
  -H "Accept: application/json, text/event-stream" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $MMUX_MCP_TOKEN" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list"}'
```

### Microsandbox backend

For local development with an existing Microsandbox, mmux can also run in
single-binary mode:

```bash
cd example-backends/microsandbox
make sandbox-prepare
cd ../..
mmux controller --enable-microsandbox-node --sandbox-name mmux-node
```

This embeds a host-side Microsandbox connector in the controller process and
exposes it as node `local`. mmux still does not create or own Microsandbox
lifecycle; `msb` owns create, start, stop, snapshot, import, and export.

Embedded modes do not need a node wire token for the embedded node. Configure
`--wire-token`, `--wire-mtls`, or `--allow-unauthenticated-node-wire` only when
you also want distributed `mmux node` processes to register with the same
controller.

Distributed mode keeps controller and node separate:

Start a controller that remote nodes can reach:

```bash
export MMUX_MCP_TOKEN=<mcp-token>
export MMUX_WIRE_TOKEN=<wire-token>
make run-controller CONTROLLER_ARGS="--host <bind-host> --port 3000 --mcp-token $MMUX_MCP_TOKEN --wire-token $MMUX_WIRE_TOKEN"
```

In another shell, use `msb` to create or start the sandbox, then run the
host-side node connector:

```bash
cd example-backends/microsandbox
export MMUX_WIRE_TOKEN=<same-wire-token>
make launch
```

The Microsandbox example uses `http://127.0.0.1:3000` by default because the
node connector runs on the host and attaches to an existing sandbox by name.
Override `CONTROLLER_URL` only when the controller runs elsewhere. mmux does not
provide a Microsandbox lifecycle command; use `msb` directly for create, start,
stop, snapshot, import, and export.

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
| `--mcp-token` | `MMUX_MCP_TOKEN` | Bearer token for public MCP requests. |
| `--mcp-token-file` | none | Reads the MCP bearer token from a file. |
| `--mcp-token-env` | `MMUX_MCP_TOKEN` | Env var used when MCP token flags are omitted. |
| `--allow-remote-without-mcp-token` | false | Allows MCP without bearer auth and ignores `MMUX_MCP_TOKEN`; mutually exclusive with explicit MCP token flags. |
| `--wire-token` | `MMUX_WIRE_TOKEN` | Bearer token for node wire RPC requests. |
| `--wire-mtls` | false | Enables native TLS termination and requires mTLS node identity for wire RPC; mutually exclusive with explicit wire token flags. |
| `--tls-cert` | none | PEM server certificate chain used when `--wire-mtls` is set. |
| `--tls-key` | none | PEM server private key used when `--wire-mtls` is set. |
| `--wire-client-ca` | none | PEM CA certificate(s) used to verify node client certificates. |
| `--wire-token-file` | none | Reads the node wire bearer token from a file. |
| `--wire-token-env` | `MMUX_WIRE_TOKEN` | Env var used when wire token flags are omitted. |
| `--allow-unauthenticated-node-wire` | false | Allows node wire RPC without bearer auth and ignores `MMUX_WIRE_TOKEN`; mutually exclusive with explicit wire token flags. |
| `--workspace-root` | none | Optional root used to confine local `read_file` / `save_file` path APIs. |
| `--enable-local-node` | false | Starts the built-in local tmux node in-process. |
| `--enable-microsandbox-node` | false | Starts an embedded Microsandbox node in-process. Requires `--sandbox-name`. |
| `--sandbox-name` | none | Existing Microsandbox sandbox name used with `--enable-microsandbox-node`. |

Important node flags:

| Flag | Default | Purpose |
| ---- | ------- | ------- |
| `--backend` | `local` | Execution backend: `local` or `microsandbox`. |
| `--sandbox-name` | none | Microsandbox sandbox name used with `--backend microsandbox`. |
| `--node-id` | `local` | Node identifier advertised to the controller. |
| `--controller-url` | none | Controller URL to register with. |
| `--node-name` | generated | Human-readable node name. |
| `--wire-token` | `MMUX_WIRE_TOKEN` env fallback | Bearer token for controller wire endpoints. |
| `--controller-ca` | public WebPKI roots | PEM CA certificate(s) used to verify the HTTPS controller. |
| `--client-cert` | none | PEM certificate chain to present for node wire mTLS. |
| `--client-key` | none | PEM private key to present for node wire mTLS. |
| `--poll-interval-ms` | `500` | Command polling interval. |
| `--node-config` | none | Profile TOML loaded by the node process. |

## Make Targets

```bash
make build
make check
make test
make lint
make release
make run-local
make run-controller
make run-node
make wire-check-tools
make wire-generate
```

Pass entrypoint flags through the target variables:

```bash
make run-local LOCAL_ARGS="--port 3001"
make run-controller CONTROLLER_ARGS="--mcp-token $MMUX_MCP_TOKEN --wire-token $MMUX_WIRE_TOKEN"
make run-node NODE_ARGS="--controller-url http://<controller-host>:3000 --wire-token $MMUX_WIRE_TOKEN"
```

Release publishing uses git tags. The release version comes from
`[workspace.package].version` in top-level `Cargo.toml`; all crates inherit it
with `version.workspace = true`. Bump that version with `make update-patch`,
`make update-minor`, or `make update-major`, merge the version change to
`main`, then run `make release-tag` from a clean `main` checkout. The
`v<version>` tag triggers GitHub Actions to build and attach platform archives
used by `scripts/install.sh`, then publish the npm package with all supported
platform archives.

### Inspect the npm package locally

The `npm/mmux` package provides an `npx`/`yarn dlx` wrapper around the native
`mmux` binary. To inspect the package from this workstation:

```bash
make npm-pack-dry-run
```

`make npm-package` builds the current platform, writes
`npm/mmux/artifacts/mmux-<platform>.tar.gz`, and syncs the npm package version
from `[workspace.package].version`. The local package only contains the current
platform archive; public npm publishing is done by the GitHub release workflow
so the package contains all supported platform archives.

`make npm-pack` creates a `.tgz` without publishing. Set `NPM_CACHE=/path/to/cache`
if npm should use a cache directory other than `/tmp/mmux-npm-cache`.

The published package can be run with:

```bash
npx @mmux/mmux controller --enable-local-node
npx @mmux/mmux controller --enable-microsandbox-node --sandbox-name mmux-node
yarn dlx @mmux/mmux controller --enable-local-node
yarn dlx @mmux/mmux controller --enable-microsandbox-node --sandbox-name mmux-node
```

## Node Wire Authentication

Node wire RPC supports separate auth from the public MCP endpoint. Bearer token
auth requires every node request to present the configured shared secret.

```bash
mmux controller --wire-token "$MMUX_WIRE_TOKEN"
mmux node --controller-url https://<controller-host>:3000 --wire-token "$MMUX_WIRE_TOKEN"
```

The controller resolves a single node wire auth policy at startup: bearer token,
mTLS identity, or explicit `unauthenticated` development mode. Mixed
configuration is rejected when conflicting modes are explicit. If `--wire-mtls`
is set, explicit wire token flags/files must not also be set; the default
`MMUX_WIRE_TOKEN` env fallback is ignored. If
`--allow-unauthenticated-node-wire` is set, `MMUX_WIRE_TOKEN` is also ignored.
If no wire auth is configured, a distributed-only controller refuses to start
unless `--allow-unauthenticated-node-wire` is explicit. An embedded-node
controller can start without node wire credentials; in that case the embedded
node is usable as `local`, while unauthenticated distributed node wire requests
are rejected.

mTLS is the zero-trust node identity mode. A verified mTLS identity is
normalized to a node id before it reaches the registry, and the controller
rejects requests where that identity tries to act as a different `node_id`.
This is intentionally runtime-neutral: the native runtime or a future
Cloudflare Worker/Durable Object runtime can perform certificate verification
and pass the verified identity into the same core policy.

Native local runtime mTLS uses controller-side TLS termination:

```bash
mmux controller \
  --wire-mtls \
  --tls-cert ./certs/controller.pem \
  --tls-key ./certs/controller-key.pem \
  --wire-client-ca ./certs/node-ca.pem
```

The node can present a client certificate when calling an HTTPS controller:

```bash
mmux node \
  --controller-url https://<controller-host>:3000 \
  --node-id msb-1 \
  --controller-ca ./controller-ca.pem \
  --client-cert ./node.pem \
  --client-key ./node-key.pem
```

The native runtime requires node identity in URI SAN `mmux:node:<node-id>` or
`spiffe://mmux/node/<node-id>`. DNS SAN and CN are ignored for node identity.
Use `--controller-ca` when the controller uses a private or self-signed CA.
See `MTSL.md` for OpenSSL commands.

## Coder Profiles

Coder profiles are built into mmux for common coding CLIs, so local mode works
without a profile TOML file. If you provide `[coder_profile.<name>]` sections
through `--node-config` or `mmux.toml` in the current directory, those sections
overlay the built-ins. Omitted fields keep their built-in values.

Use TOML only for intentional changes:

- backend-agnostic profile tweaks, such as changing a command or prompt marker;
- new custom profiles.

Nested tables merge with the built-in profile. Scalar and list fields replace
the built-in value, so do not copy full profile definitions unless you want to
own every copied field. See `mmux.toml.example` and
the backend examples for copyable examples.

The fields shown here are the backend-agnostic profile shape: they describe how
mmux launches and drives a CLI once a node has the CLI available.

Backend-agnostic override example:

```toml
# Only this field changes. The rest of the built-in codex profile remains
# unchanged.
[coder_profile.codex]
cmd = "codex --model gpt-5"
permission_bypass_cmd = "codex --model gpt-5 --dangerously-bypass-approvals-and-sandbox"
```

Most profiles use the default direct launch strategy, which starts the CLI as
the tmux session command. TUIs that need to be started from an interactive shell
can set `launch_strategy = "shell_send"`; mmux then starts `bash` and sends
`cmd` followed by Enter before waiting for the prompt marker.

`permission_bypass_cmd` is optional and only used when `start_coding_session`
receives `bypass_permissions = true`. Normal sessions always use `cmd`. This
keeps approval/sandbox bypass modes an explicit per-session choice. Built-in
profiles define `permission_bypass_cmd` only for CLIs whose local help exposes a
clear bypass flag.

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

The default bind host is loopback-only. A non-loopback MCP bind without a token
is rejected unless you deliberately pass `--allow-remote-without-mcp-token`.
That flag also ignores the default `MMUX_MCP_TOKEN` env fallback and is
mutually exclusive with explicit MCP token flags/files.
Node wire RPC is rejected unless you configure `--wire-token`, configure
`--wire-mtls`, or deliberately pass `--allow-unauthenticated-node-wire`.
Embedded modes do not need node wire credentials for the embedded node, but the
public MCP endpoint still needs either MCP bearer auth or an explicit
unauthenticated bind decision for non-local exposure. For distributed
bearer-token mode, use separate MCP and wire tokens:

```bash
export MMUX_MCP_TOKEN="$(openssl rand -hex 32)"
export MMUX_WIRE_TOKEN="$(openssl rand -hex 32)"
make run-controller CONTROLLER_ARGS="--host <bind-host> --mcp-token $MMUX_MCP_TOKEN --wire-token $MMUX_WIRE_TOKEN"
```

Authenticated requests must include:

```text
Authorization: Bearer <token>
```

`--workspace-root` is a file API guardrail for MCP operations that target
`node=local`: when it is set, local `read_file`, `save_file`, and
coding-session `cwd` resolution are confined under that root. Distributed node
ids receive paths unchanged. This is not a sandbox for terminal commands.

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
│   └── mmux-wire/             # ConnectRPC/Buffa wire schema and generated code
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
- `example-backends/microsandbox/` shows how to prepare a local Microsandbox
  runtime and attach mmux to it.

Microsandbox lifecycle belongs to `msb`. mmux does not create, launch, stop,
snapshot, import, or export Microsandbox runtimes. For single-binary local
mode, the controller starts an embedded host-side connector with
`--enable-microsandbox-node --sandbox-name <name>` and exposes it as node
`local`. For distributed mode, run `mmux node --backend microsandbox
--sandbox-name <name>` and register it to a controller. In both modes the
connector runs on the host and attaches to an existing sandbox by name, so
controller credentials and node private keys stay out of sandbox config and
sandbox files.

Workspace persistence is handled by Microsandbox or the surrounding deployment
system, not by mmux. The local example Makefile creates a sandbox with the
repo-local `workspace/` mounted read-write at `/workspace` and setup assets
mounted read-only at `/mmux-setup`. For other deployments, configure mounts
with `msb`, Kubernetes, or your image/runtime tooling before starting mmux.

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
