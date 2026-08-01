# MCP + tmux = mmux

mmux is a Rust MCP server for durable agent orchestration and tmux-backed
terminal control across local or remote execution nodes. It gives operators a
project -> plan -> task model for coordinating work, attaching coder sessions
to tasks, recording outcomes and blockers, and pruning finished orchestration
state. Agents can also inspect sessions, drive interactive shells and coding
CLIs, read and write files, and route terminal work to sandboxed environments.

The core idea is simple:

- `mmux controller` exposes the MCP HTTP endpoint and the controller/node wire
  endpoint.
- `mmux node` owns tmux and filesystem access in the environment where it runs.
- Durable orchestration state groups work as projects, Markdown plan briefs,
  and executable tasks with task-owned runtime sessions.
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
| tmux | Local and node runtime | `mmux-node` shells out to the system `tmux` binary; `--enable-local-node` fails early if `tmux` is unavailable. |
| Microsandbox CLI (`msb`) | Microsandbox backend | Required only when running the Microsandbox backend; mmux shells through `msb exec`. |

Wire source generation is only needed when editing the protobuf schema. The
generated Rust sources are checked in, so normal builds do not require `buf` or
the protobuf generators. If you change files under `crates/mmux-wire/proto`,
run `make wire-generate`, which additionally needs `buf`, `protoc-gen-buffa`,
`protoc-gen-buffa-packaging`, and the `protoc-gen-connect-rust` generator that
matches the pinned connect-rust revision in `Cargo.toml`.

## Quick Start

### Run from npm

For a local loopback-only MCP server with the built-in local tmux backend
(least secure version; run only in trusted environments):

```bash
npx --yes @mmux/mmux controller --enable-local-node --allow-remote-without-mcp-token
```

The MCP endpoint is:

```text
http://127.0.0.1:3000/mcp
```

Register that HTTP MCP server with codex:

```bash
codex mcp add mmux --url http://127.0.0.1:3000/mcp
```

Register it with claude code:

```bash
claude mcp add --transport http mmux http://127.0.0.1:3000/mcp
```

For authenticated local setup, start mmux with an MCP bearer token and register
the same token with each MCP client:

```bash
export MMUX_MCP_TOKEN="$(openssl rand -hex 32)"
npx --yes @mmux/mmux controller --enable-local-node --mcp-token-env MMUX_MCP_TOKEN
```

In another shell with `MMUX_MCP_TOKEN` set, register codex:

```bash
codex mcp add mmux \
  --url http://127.0.0.1:3000/mcp \
  --bearer-token-env-var MMUX_MCP_TOKEN
```

Register claude code by adding the bearer header:

```bash
claude mcp add --transport http mmux http://127.0.0.1:3000/mcp \
  --header "Authorization: Bearer $MMUX_MCP_TOKEN"
```

For local Microsandbox mode, prepare the sandbox with `msb` and run the same
npm package with the embedded Microsandbox node:

```bash
cd example-backends/microsandbox
make sandbox-prepare
cd ../..
npx --yes @mmux/mmux controller --enable-microsandbox-node --sandbox-name mmux-node --allow-remote-without-mcp-token
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
When `--enable-local-node` is set, controller startup checks the local tmux
backend and fails early if the system `tmux` binary is unavailable.

#### Local tmux access

The local backend uses a mmux-owned tmux server socket instead of the user's
default tmux server. This keeps mmux sessions separate from personal tmux
sessions. Use an explicit tmux config when you need deterministic local tmux
server settings.

Default local runtime paths:

```text
store path:  ~/.mmux
tmux socket: deterministic private runtime socket derived from the store path
```

With `--store-path <path>`, mmux uses that path for durable local runtime state
and derives a short private tmux socket path from it. The socket is intentionally
not stored under the store path because tmux/Unix socket path limits are short
on macOS and some CI environments.

By default tmux uses its normal config discovery for that private server. Pass
`--tmux-config <path>` with `--enable-local-node` to use an explicit local
backend config file, for example the repo's `tmux.local.conf`:

```bash
mmux controller --enable-local-node --tmux-config ./tmux.local.conf
```

For distributed local nodes, pass the same flag to the node process:

```bash
mmux node --backend local --tmux-config ./tmux.local.conf
```

`--tmux-config` is only valid for local tmux backends. Microsandbox and other
remote environments own their tmux config inside the backend image/runtime.
Restart the local tmux server for config changes to take effect.

Use MCP tools for normal operation:

```text
list_sessions(project_id)   # project UUID id or slug
start_coding_session
coding_send
coding_read
kill_session
```

Use the CLI proxy for manual tmux inspection or interactive attach:

```bash
mmux create-project "Release hardening" --slug release-hardening
mmux list-projects
mmux prune-store --dry-run
mmux prune-store --sessions-only --older-than-days 7
mmux tmux -- list-sessions
mmux tmux -- list-sessions --project <project-id-or-slug>
mmux tmux -- capture-pane -t codex -p
mmux tmux -- send-keys -t codex C-c
mmux attach --read-only codex
mmux attach codex
```

For a custom store path, pass the same path to both the controller and the
proxy:

```bash
mmux controller --enable-local-node --store-path /tmp/mmux-dev
mmux --store-path /tmp/mmux-dev create-project "Release hardening" --slug release-hardening
mmux --store-path /tmp/mmux-dev list-projects
mmux --store-path /tmp/mmux-dev prune-store --dry-run
mmux --store-path /tmp/mmux-dev tmux -- list-sessions
mmux --store-path /tmp/mmux-dev attach --read-only codex
mmux --store-path /tmp/mmux-dev attach codex
```

Plain `tmux` talks to your user's default tmux server and will not show mmux
local-node sessions. Use `mmux tmux -- ...` when inspecting mmux local sessions.
For orchestration work, project ids are UUIDs and project slugs are globally
unique aliases. MCP tools that accept `project_id` accept either the UUID id or
the slug. `list_sessions` requires `project_id` and returns durable sessions
recorded against tasks in that project. Use `admin_list_node_sessions` only for
raw node/tmux debugging. At the CLI layer,
`mmux tmux -- list-sessions --project <project-id-or-slug>` provides a similar
local-node filter for manual inspection.

Local node environment is managed outside task orchestration. Start mmux from a
shell, service, or container that already has the env needed by coder CLIs. If
the private tmux server does not exist yet, it inherits env from the mmux
process when the first local session starts.
For Codex specifically, mmux does not set a separate `CODEX_HOME`; local-node
Codex sessions use the `CODEX_HOME` inherited by the controller or node
process. If `CODEX_HOME` is unset, Codex uses its own default home, typically
`~/.codex`. To isolate mmux Codex state, start the controller or local node with
an explicit value, for example
`CODEX_HOME=/path/to/mmux-codex-home mmux controller --enable-local-node`.

If you are calling the MCP endpoint directly, include both accepted response
types:

```bash
curl -X POST "http://<controller-host>:3000/mcp" \
  -H "Accept: application/json, text/event-stream" \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list"}'
```

Raw MCP clients must check for JSON-RPC `error` and tool-level `isError`
before parsing a response as a successful tool result. Tool failures are clear
but still returned in the MCP response envelope. For example, a task-aware
`start_coding_session` without `node` returns an error such as
`task-aware start_coding_session requires explicit node`; client wrappers
should surface that error instead of parsing the missing success payload.

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
exposes it as node `local`. The connector verifies the sandbox by running
`msb exec <name> -- bash -lc true` during startup. mmux still does not create
or own Microsandbox lifecycle; `msb` owns create, start, stop, snapshot,
import, and export.

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
| `mmux create-project <title> --description <text>` | Creates a durable orchestration project in the local mmux store. Supports optional `--slug <slug>`. |
| `mmux delete-project <id-or-slug>` | Deletes a durable orchestration project from the local mmux store, including all contained plans, task cards, task sessions, and task edges. |
| `mmux list-projects` | Lists durable orchestration projects from the local mmux store so project ids/slugs are discoverable. |
| `mmux prune-store` | Prunes stale durable task sessions and finished plans from the local mmux store. Use `--dry-run` to preview, `--sessions-only` to skip plan pruning, and `--older-than-days <days>` to constrain by age. |
| `mmux tmux -- <args>` | Runs `tmux` against mmux's private local-node tmux socket. `list-sessions` accepts mmux's `--project <project-id-or-slug>` filter. |
| `mmux attach [--read-only|-r] <session>` | Attaches to a session in mmux's private local-node tmux server. Use read-only mode for inspection without sending input. |

`src/main.rs` dispatches to root help when no arguments are provided. Use
`mmux controller` to start the controller explicitly, or pass controller flags
directly such as `mmux --enable-local-node`.

Important controller flags:

| Flag | Default | Purpose |
| ---- | ------- | ------- |
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
| `--store-path` | `~/.mmux` | Local runtime state directory. The embedded local tmux socket is a deterministic short runtime path derived from this store path. |
| `--tmux-config` | tmux default discovery | Local tmux config file for `--enable-local-node`; invalid without the embedded local node. |
| `--enabled-coder-profiles` | all built-ins | Comma-separated MCP coder profile allowlist, for example `codex,claude`. |
| `--default-coder-profile` | first enabled built-in in canonical order | Default coder profile used when profile-aware tools omit `profile`. Must be enabled. |
| `--enable-local-node` | false | Starts the built-in local tmux node in-process. |
| `--enable-microsandbox-node` | false | Attaches an embedded Microsandbox node in-process. Requires `--sandbox-name` for an existing running sandbox. |
| `--sandbox-name` | none | Existing running Microsandbox sandbox name used with `--enable-microsandbox-node`. |

Important node flags:

| Flag | Default | Purpose |
| ---- | ------- | ------- |
| `--backend` | `local` | Execution backend: `local` or `microsandbox`. |
| `--sandbox-name` | none | Existing running Microsandbox sandbox name used with `--backend microsandbox`. |
| `--node-id` | `local` | Node identifier advertised to the controller. |
| `--controller-url` | none | Controller URL to register with. |
| `--node-name` | generated | Human-readable node name. |
| `--wire-token` | `MMUX_WIRE_TOKEN` env fallback | Bearer token for controller wire endpoints. |
| `--controller-ca` | public WebPKI roots | PEM CA certificate(s) used to verify the HTTPS controller. |
| `--client-cert` | none | PEM certificate chain to present for node wire mTLS. |
| `--client-key` | none | PEM private key to present for node wire mTLS. |
| `--poll-interval-ms` | `500` | Command polling interval. |
| `--store-path` | `~/.mmux` | Local backend runtime state directory. The local tmux socket is a deterministic short runtime path derived from this store path. |
| `--tmux-config` | tmux default discovery | Local tmux config file for `--backend local`; invalid with `--backend microsandbox`. |

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

Coder profiles are built into mmux as canonical Rust adapters. Supported
profiles are:

- `codex`
- `opencode`
- `kimi`
- `claude`

Each built-in profile owns its launch command, prompt submission mode,
readiness markers, startup/update handling, blocking-confirmation detection,
compact-read filtering, and approve/reject/cancel/escape keys. Use
`list_coder_profiles` to inspect the public metadata for the enabled built-ins.
By default every built-in profile is enabled; pass `--enabled-coder-profiles`
with a comma-separated list such as `codex,claude` to expose only that subset
through MCP profile listing, profile resources, and profile-aware tools. When a
profile-aware tool omits `profile`, mmux uses `--default-coder-profile` if set,
otherwise the first enabled built-in profile in canonical order: `codex`,
`opencode`, `kimi`, then `claude`.

Prompt submission is profile-aware. codex, kimi, and opencode sessions use tmux
paste buffers by default. claude uses literal key input followed by a real
Enter keypress because its Ink-based text box handles pasted newlines
differently from actual keypresses.

`permission_bypass_cmd` is only used when `start_coding_session` receives
`bypass_permissions = true`. Normal sessions always use the profile's normal
command. Built-in profiles define permission bypass only for CLIs whose local
help exposes a clear bypass flag.

## Sessions

mmux works with tmux sessions. A session name identifies a running terminal on
one node. Node-aware tools accept `node`; if omitted, they target `local`.

There are two common session patterns:

| Session type | Created by | Used with | Meaning |
| ------------ | ---------- | --------- | ------- |
| Shell session | `exec` when needed, or an existing tmux session | `send_input`, `send_key`, `capture_output`, `wait_start`, `wait_status`, `wait_cancel`, `session_info`, `list_panes`, `resize_pane` | Generic terminal session with no profile-specific readiness rules. |
| Coder session | `start_coding_session` | `coding_task_send` for initial task delegation, `coding_send` for follow-ups, `wait_start` with `kind = "coding-ready"`, `wait_status`, `coding_read`, `coding_action`, `check_state` | A tmux session running a coding CLI and interpreted through a coder profile. |

A coder session is not a separate storage object. It is identified by:

- `node`: where the tmux session lives;
- `session`: the tmux session name;
- `profile`: the CLI interaction rules used to launch/read/drive it.

Example:

```json
{
  "node": "msb-mmux-1",
  "session": "codex-main",
  "profile": "codex"
}
```

The same tmux session can be inspected with generic session tools, but coding
tools need the profile so mmux can detect prompts, busy states, startup/update
prompts, and approval actions correctly.

Task-aware `start_coding_session` uses the same create/adopt behavior as
ordinary coder sessions. It does not wait for the coding CLI to become ready;
start readiness tracking explicitly with `wait_start` using
`kind = "coding-ready"`. When you provide task metadata, the operator must also provide
explicit runtime choices: `node`, `profile`, `workspace_path`,
`bypass_permissions`, `task_id`, `role`, `kind`, and `skills`. Use either
`session` or `generate_session_name = true`. If `task_id` is present, `node` is
mandatory and must be supplied by the caller, for example `node = "local"` for
the embedded local node.
Generated orchestration-owned names use the `mmux-*` prefix and include the
task slug, session kind, and a short suffix; non-`mmux-*` sessions are never
treated as orchestration-owned cleanup targets.

`workspace_path` is the backend-owned workspace/start directory. The controller
stores and passes the selected string for the chosen node/backend without
canonicalizing it against the controller host filesystem. The backend that
launches tmux owns interpreting, validating, or failing the path in its own
environment. Reconciliation does not compare a live session's current working
directory to `TaskSession.workspace_path`; the recorded path is the session's
startup/adoption placement, and the session may change directories during work.
Task scope is separate from runtime placement: `include_paths` and
`exclude_paths` define the task work boundaries. Relative scope paths are
interpreted from the runtime workspace when a `run_spec` or recorded session
provides one. Prefer scope paths inside that workspace unless the operator
intentionally scopes external files.

## Orchestration

mmux includes a durable orchestration layer for coordinating agent work:
projects contain Markdown plan briefs, plans contain executable tasks, and each
task can own one recorded coder session. Projects are long-lived boundaries
with required descriptions. Plans carry enough context to derive tasks. Tasks
carry objective, scope, gates, outcome, blockers, dependency edges, and runtime
placement for the attached session. Tasks may also carry an optional
`run_spec` with `node_id`, `profile`, `workspace_path`, `bypass_permissions`,
`role`, `kind`, `skills`, prompt `template`, scheduler `instruction`, and
the top-level `auto_schedule` flag. `run_spec` describes how a task can be
started; `auto_schedule` separately controls whether explicit
`orchestration_next` runs may start it automatically.

Operators create or select a project, create a plan, derive tasks from that
plan, then start or record coder sessions against individual tasks. Initial
delegation uses `coding_task_send`, which renders deterministic task context
before sending the operator's instruction to the coding CLI. Follow-up steering
uses `coding_send`.

Scheduling is explicit. An operator or external MCP controller observes
`orchestration_status`, records worker and validator results with
`task_status_update` or `task_report`, optionally calls `orchestration_report`
to inspect ready/skipped/error task classifications, then calls
`orchestration_next` to start all currently ready tasks in that plan whose
`auto_schedule` is true. A task is startable only when a `run_spec` exists,
dependencies and required validations are ready, the task is `Backlog` or
`Planned`, its profile is enabled, and it has no live recorded session.
`orchestration_report` is read-only and never starts sessions.
`orchestration_next` starts deterministic `mmux-*` sessions, records them on
tasks, waits for coding readiness, sends task prompts, and marks tasks
`Running`. Use `task_start` to explicitly start one task from its `run_spec`
without requiring `auto_schedule=true`.

Task-owned `gates` are acceptance checks. `TaskEdgeKind::Validates` makes a
validator task operational: an edge from validator to target means downstream
tasks that `DependsOn` the target cannot start until that validator is `Passed`
or `Delivered`. A failed or blocked validator remains visible in
`orchestration_status` through `validation_blocked_by` and keeps dependent
auto-scheduled tasks from starting.

Supported task edges in v1 are `DependsOn`, `ParentOf`, `Validates`, `Audits`,
`Supersedes`, and `Related`. `DependsOn`, `ParentOf`, `Validates`, and
`Supersedes` have orchestration semantics. `Audits` is non-gating review
traceability; use `Validates` when an audit must approve gates before dependent
work can start. `Supersedes` marks the `to` task as replaced by the `from`
task, so the replaced task is skipped by scheduling and explicit starts.
`Related` is durable traceability/navigation only.

The orchestrator detects failed or blocked scheduler starts through
`orchestration_status`: startup/readiness/prompt failures are recorded as task
`Blocked` state with blockers. Worker-reported failure remains an operator
state update through `task_status_update`. Runtime detection of a long-running
task being "stuck" requires an additional timeout or heartbeat policy and is
not inferred from silence in v1.

Validation that must gate downstream work should be modeled as a task plus a
`Validates` edge. For example, the validation task should `DependsOn` the
implementation task, the validation task should `Validates` the implementation
task, and downstream work should `DependsOn` the implementation task. If
validation cannot run, mark the validator task `Blocked`; if validation rejects
the work, mark the validator task `Failed` with outcome evidence. The scheduler
will not start downstream `DependsOn` tasks while the dependency status is not
ready or while linked validators have not approved it.

`orchestration_status` is the compact source of truth for current project,
plan, task, edge, outcome, blocker, task-session, cleanup, warning, and runtime
state. Use `task_get` when an exporter or operator needs one full stored task
body, including objective, scope, gates, outcome/evidence, run spec, session,
and incoming/outgoing edges. Use `plan_get` for the analogous full plan record,
including the Markdown plan brief that the summary listings omit.
`orchestration_cleanup_zombies` and
`orchestration_prune_store` are dry-run by default; destructive
cleanup/pruning requires explicit opt-in.

For the full operator workflow, use the bundled `mmux-operator` skill.

## MCP Surface

Session and node tools:

| Tool | Purpose |
| ---- | ------- |
| `list_nodes` | List registered execution nodes. |
| `node.info` | Describe one execution node. |
| `list_sessions` | List durable task sessions in a required `project_id` selector (project UUID id or slug), enriched with live node metadata when available. |
| `admin_list_node_sessions` | Admin/debug tool that lists raw live tmux sessions on a node, including unrecorded sessions. |
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
| `wait_start` | Start a cancellable runtime wait job for `stable`, `sentinel`, `prompt`, or `coding-ready`. |
| `wait_status` | Inspect a wait job as `pending`, `completed`, `failed`, or `canceled`. |
| `wait_cancel` | Cancel a pending wait job without killing or interrupting the tmux session. |
| `interact` | Send input and wait for stable output in one call. |
| `exec` | Run a shell command in a session and return cleaned output. |

Profile-aware coding tools:

| Tool | Purpose |
| ---- | ------- |
| `list_coder_profiles` | List enabled built-in coder profiles. |
| `start_coding_session` | Create or adopt a CLI session from its profile command, or from `permission_bypass_cmd` when `bypass_permissions = true`; returns without waiting for readiness. Optional task metadata records one `TaskSession` on the task. |
| `coding_send` | Send a prompt to a coding CLI; rejects blank prompts and placeholder strings such as `null` or `undefined`. |
| `coding_task_send` | Send an initial task-scoped prompt by rendering task context from orchestration state with template `task`, `validate`, `review`, or `quality-guard`, optionally adding `context_task_ids` task cards for multi-task validation/review, then appending the provided instruction. The template selects the operating mode; the instruction supplies the concrete focus. |
| `coding_read` | Read recent CLI output through profile-aware compaction by default; pass `raw = true` for the full tmux pane text. |
| `coding_action` | Send `approve`, `reject`, `cancel`, `escape`, or `dismiss`. |
| `check_state` | Non-blocking JSON state check with `has_prompt`, `promptable`, `busy`, and `turn_idle`. |

`wait_start`, `wait_status`, and `wait_cancel` are the canonical orchestration
wait API. Wait jobs are runtime-only in v1 and are not persisted to SQLite.
Wait jobs can target any reachable execution node that supports mmux tmux
command primitives. Omitting `node` targets the embedded `local` node, which
may be local tmux or embedded Microsandbox. Embedded local wait jobs run outside
the local tmux actor and poll through short actor calls, so pending long waits
do not block `coding_read`, `check_state`, `capture_output`, or
`coding_action`. Use `kind = "coding-ready"` with `profile` to wait until the
current foreground turn is idle for the requested stability window.
`check_state.promptable=true` means the CLI can accept text; it does not prove
the current turn is finished. For codex, the prompt can be visible while
`busy=true` and `turn_idle=false`, because codex may accept steering text while
active work is still running.

Orchestration tools:

| Tool | Purpose |
| ---- | ------- |
| `project_create` | Create a durable orchestration project boundary with required `title` and `description`, a UUID id, and globally unique slug. Requires `--enable-admin-tools`. |
| `project_list` | List orchestration projects with total, active, and per-status plan/task counts. |
| `project_status_update` | Set project status to `Active` or `Archived`; `project_id` accepts UUID id or slug. Requires `--enable-admin-tools`. |
| `plan_create` | Create a durable plan brief under a project; `project_id` accepts UUID id or slug. |
| `plan_list` | List orchestration plans, optionally filtered by project. |
| `plan_update` | Update mutable plan metadata: title and brief. |
| `plan_status_update` | Update plan status with optional plan-level outcome. |
| `plan_get` | Return one full stored plan record by plan id or unique slug, including its Markdown brief. |
| `task_create` | Create a durable orchestration task inside a required `plan_id` selector (plan id or unique slug) with scope, notes, and gates; returns the created task object directly. |
| `task_update` | Update mutable task metadata: title, objective, scope fields, gates, `auto_schedule`, and optional `run_spec`. |
| `task_get` | Return one full stored task record by task id or unique slug, plus incoming and outgoing task edges. |
| `task_start` | Explicitly start one task using its `run_spec`; does not require `auto_schedule=true`. |
| `task_edge_add` | Add a task dependency or relationship edge. |
| `task_edge_remove` | Remove a task dependency or relationship edge. |
| `session_record` | Record durable runtime placement for an existing or manually started coder session; task-attached records require a live node/session. |
| `task_status_update` | Update task status with operator outcome and blockers. |
| `task_report` | Submit durable task result status, outcome, blockers, and evidence; intended for operator/external-controller result commits. |
| `orchestration_status` | Return compact project, plan, task, edge, task-session, cleanup, warning, and runtime-state summaries. |
| `orchestration_report` | Read-only report of tasks that are ready or not ready for automatic orchestration. Never starts sessions. |
| `orchestration_next` | Advance one plan by starting all currently ready tasks whose `auto_schedule` is true and `run_spec` exists. Requires `plan_id` id-or-slug. Executes by default; pass `dry_run=true` to preview. |
| `orchestration_cleanup_zombies` | Dry-run or explicitly clean live local `mmux-*` sessions missing from durable orchestration storage. |
| `orchestration_prune_store` | Dry-run or explicitly prune stale durable task sessions and finished plans whose contained tasks are all finished. |

Backend node file tools:

| Tool | Purpose |
| ---- | ------- |
| `read_file` | Reads a file from the selected backend node with UTF-8/base64 encoding detection, compression sniffing, and MIME type. |
| `save_file` | Writes UTF-8 or base64 content to the selected backend node and creates parent directories there. |

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

Backend node file tools (`read_file` and `save_file`) operate in the selected
node/backend filesystem namespace. They are separate from coder-session
`workspace_path`, session placement, and terminal command sandboxing.

Request and output limits are configurable with `--max-read-bytes`,
`--max-write-bytes`, `--max-timeout-seconds`, `--max-request-bytes`, and
`--max-capture-bytes`.

## Architecture

```text
.
├── src/main.rs
├── crates/
│   ├── mmux-controller/       # MCP server, auth, policy, node registry
│   ├── mmux-node/             # tmux/filesystem adapter and built-in profiles
│   ├── mmux-shared/           # shared profile and file DTOs
│   └── mmux-wire/             # ConnectRPC/Buffa wire schema and generated code
├── example-backends/
│   ├── local/                 # local backend README
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

- Local mode uses canonical built-in coder profiles.
- `example-backends/microsandbox/` shows how to prepare a local Microsandbox
  runtime and attach mmux to it.

Microsandbox lifecycle belongs to `msb`. mmux does not create, launch, stop,
snapshot, import, or export Microsandbox runtimes. For single-binary local
mode, the controller attaches an embedded host-side connector with
`--enable-microsandbox-node --sandbox-name <name>` and exposes it as node
`local`. For distributed mode, run `mmux node --backend microsandbox
--sandbox-name <name>` and register it to a controller. In both modes the
connector runs on the host and attaches to an existing sandbox by name, so
controller credentials and node private keys stay out of sandbox config and
sandbox files.

Workspace persistence is handled by Microsandbox or the surrounding deployment
system, not by mmux. The local example Makefile creates a sandbox with
`example-backends/microsandbox/workspace/` mounted read-write at `/workspace`
and setup assets mounted read-only at `/mmux-setup`. For other deployments,
configure mounts with `msb`, Kubernetes, or your image/runtime tooling before
starting mmux.

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
