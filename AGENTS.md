# Agent Guide: Using mmux

This document is for AI agents that need to operate terminals, coding CLIs, or interactive TUI applications through mmux.

## What mmux Does

mmux is an MCP server that controls tmux sessions. It lets you:
- Inspect and manage tmux sessions
- Send text and key sequences to any session
- Capture output (visible lines or full scrollback)
- Wait for conditions (stable output, sentinel strings, prompt markers)
- Read and save files on the host filesystem
- Drive coding CLIs (codex, opencode, kimi, claude, etc.) with profile-aware actions

## How to Connect

mmux exposes an MCP HTTP server. Before using any tools, the server must be running:

```
http://127.0.0.1:3000/mcp
```

If the server requires a bearer token, every request must include:
```
Authorization: Bearer <token>
```

## Quick Decision Tree

```
Do you need to run a single shell command and get its output?
  └─ Use exec → session, command, lines

Do you need to run a command in a terminal?
  └─ Use exec for one-shot commands, or start_coding_session / send_input / send_key → capture_output for interactive work

Do you need to drive a coding CLI (codex, opencode, kimi, claude)?
  └─ Use list_coder_profiles → start_coding_session → coding_send or coding_task_send → wait_start(kind=coding-ready) → wait_status → coding_read → coding_action

Do you need to coordinate task work across coder sessions?
  └─ Use tools/list → orchestration_status → project_create/project_list → task_create/task_update/task_assign/edges → start_coding_session or session_record → coding_task_send → task_status_update

Do you need to check what's happening without waiting?
  └─ Use check_state or capture_output

Do you need to transfer a file to or from a backend node?
  └─ Use read_file or save_file with the selected node (no session needed)
```

## Session Lifecycle Best Practices

mmux has generic tmux sessions and profile-driven coder sessions. A coder
session is still a tmux session; it is identified by `node`, `session`, the
`profile` used by coding tools, and an optional `objective` describing what the
session is about.

For the built-in local backend, mmux uses its own tmux server socket. The socket
is a deterministic short runtime path derived from the store path, not a file
inside the store directory. Do not use plain `tmux` to inspect local mmux
sessions. Use MCP tools, or use the CLI proxy:

```
mmux list-projects
mmux prune-store --dry-run
mmux prune-store --sessions-only --older-than-days 7
mmux tmux -- list-sessions
mmux tmux -- list-sessions --project <project-id-or-slug>
mmux tmux -- capture-pane -t codex -p
mmux attach codex
```

Pass `--tmux-config <path>` with `--enable-local-node` to make the embedded
local backend start tmux with an explicit config file such as
`./tmux.local.conf`. For a distributed local node, pass `--tmux-config` to
`mmux node --backend local`. Do not use this flag with Microsandbox; sandboxed
backends own tmux config inside their runtime.

If the controller was started with a custom store path, use the same path with
the proxy:

```
mmux --store-path /tmp/mmux-dev list-projects
mmux --store-path /tmp/mmux-dev prune-store --dry-run
mmux --store-path /tmp/mmux-dev tmux -- list-sessions
mmux --store-path /tmp/mmux-dev attach codex
```

Plain `tmux` talks to the user's default tmux server, not mmux's local-node
tmux server.
For orchestration work, use `mmux list-projects` to discover project ids/slugs,
then `mmux tmux -- list-sessions --project <project-id-or-slug>` to filter live
local sessions recorded against tasks in that project.

If a running controller was upgraded in place and new orchestration tools are
missing from discovery, restart `mmux controller` and run `tools/list` again.
An old controller process cannot expose tools compiled into a newer binary.

### 1. Always check what is already running
```
tools/list     → discover the current MCP surface
list_sessions(project_id) → see recorded sessions for one project
admin_list_node_sessions  → raw node/tmux session discovery for admin/debug
session_info   → deep inspect a specific session
check_state    → quick JSON check (has_prompt, promptable, busy, turn_idle)
```

### 2. Start profile-driven sessions explicitly
```
start_coding_session(profile="codex", session="codex", node="msb-mmux-1", objective="work on release docs")
```

The public MCP surface does not expose raw generic session creation. Use
`start_coding_session` for coding CLIs. Use `exec` for one-shot shell work; it
creates its shell session if needed.

### 3. Capture output after every significant action
```
send_input(session="myapp", text="cargo test")
wait_start(session="myapp", kind="stable", timeout_seconds=60)
wait_status(wait_id="<returned wait_id>")
capture_output(session="myapp", lines=40)
```

### 4. Detach without killing (keep session running in background)
When inside a tmux session via `sandbox-tmux-attach` or direct tmux:
```
Ctrl+a  then  d      → detach, session keeps running
```
Or run from inside the session:
```
tmux detach
```

This is useful when you want to leave a coding CLI running and come back to it later.

### 5. Clean up when done
```
kill_session(session="myapp")
```

For orchestration-owned cleanup, prefer `orchestration_cleanup_zombies` first
with its default dry-run behavior. Only pass `dry_run=false` when the reported
candidates are correct.

## Orchestration Workflow

Operators mutate orchestration state; worker sessions report findings,
blockers, evidence, and proposed changes. Do not let worker prompts assume they
can update task state unless the operator explicitly delegates that authority.

1. Discover the loaded surface with `tools/list`, then call
   `list_coder_profiles`.
2. Inspect durable state with `orchestration_status`; use it for compact
   project, task, summary, blocker, owner/session, cleanup candidate, warning,
   task-agent, and runtime-state data. Task summaries include `agents`, so the
   selected `TaskAgent.prompt` can be copied from status when needed. Pass
   `include_completed=true` when delivered, canceled, or failed tasks matter.
3. Create or select a project with `project_create`/`project_list`. Projects
   are required boundaries and do not own workspace paths, nodes, or profiles.
   Project summaries include total, active, and `task_status_counts` entries
   for every task status, including zero counts.
4. Create tasks with `task_create` and required `project_id`. Each `TaskAgent`
   may contain only `kind`, `role`, `skills`, `workspace_path`, `objective`, and
   `prompt`; do not put `count`, `profile`, `node`, `node_id`, or
   `bypass_permissions` there. `task_create` returns the created task object
   directly; read its id from the top-level `id` field. Created tasks start in
   `Backlog`; move them to `Planned` when scope/dependencies are ready.
5. Correct mutable task metadata with `task_update`: `title`, `objective`,
   scope fields (`include_paths`, `exclude_paths`, `notes`), `agents`,
   and `gates`. Scalar and scope fields are partial updates; `agents` and
   `gates` replace the whole list. Do not use `task_update` for project
   membership, status, edges, session runtime metadata, task id, or completion
   time.
6. Assign owners with `task_assign` using actual runtime choices:
   `task_id`, `node_id`, `session`, `profile`, `role`, `kind`, and `skills`.
7. Add or remove relationships with `task_edge_add` and `task_edge_remove`
   using `from_task_id`, `to_task_id`, `kind`, and optional `note` on add.
8. Start a task-aware coder with `start_coding_session`, or adopt an existing
   coder with `session_record`. Task-aware starts require explicit `node`,
   `profile`, `workspace_path`, boolean `bypass_permissions`, `task_ids`,
   `role`, `kind`, `skills`, and `objective`; use `session` or
   `generate_session_name=true`.
   If `task_ids` is present, do not rely on `TaskAgent` metadata for runtime
   placement. Pass the selected node explicitly, for example `node="local"` for
   the embedded local node. The controller intentionally errors with
   `task-aware start_coding_session requires explicit node` when this is
   omitted.
   For task-attached `session_record` calls, the controller validates that the
   selected node is reachable and the tmux session already exists before it
   writes durable state.
   Treat `workspace_path` as the backend-owned workspace/start directory for
   the selected node/backend. Pass the explicit string through without
   controller-side canonicalization.
8. For initial task delegation, use `coding_task_send` with `task_id_or_slug`
   and a concrete instruction. It builds deterministic task context from
   orchestration state and sends it with profile-aware coding behavior. Use
   `template="task"` for implementation/delegation, `template="validate"` for
   gate validation, `template="review"` for bug/risk review, and
   `template="quality-guard"` for maintainability, architecture fit, naming,
   boundaries, lifecycle, API shape, and operator/project quality preferences.
   Use `coding_send` only for follow-up prompts, steering, corrections, or
   non-task sessions.
   Never send a prompt that is empty or the literal placeholder text `null` or
   `undefined`; mmux rejects these, and callers should treat that as a script
   extraction/parsing bug.

   Template selection answers different orchestration questions:
   - `task`: "What work should this agent perform for this task?" Use for
     initial implementation/delegation.
   - `validate`: "Does the result satisfy the task gates and objective?" Use
     for validator sessions.
   - `review`: "Are there correctness risks, regressions, missing tests,
     contract breaks, or scope drift?" Use for reviewer/auditor sessions.
   - `quality-guard`: "Does the change conform to the project/operator quality
     bar?" Use for maintainability and design-quality checks.

   The template supplies the operating mode and task context. The prompt must
   supply the concrete focus, such as scope, evidence to inspect, review angle,
   or operator-specific guard points.
9. Record accepted progress with `task_status_update`. Include a concise
   `summary`; include `blockers` when blocked. For gated moves to `Passed` or
   `Delivered`, the summary must include validation or review evidence.
10. Before destructive cleanup, call `orchestration_cleanup_zombies` without
   arguments. Explicit cleanup requires `dry_run=false` and can kill only live
   local `mmux-*` sessions absent from durable session records.
   Cleanup candidates report tmux creation time as `created_at_ms` when
   available; this is distinct from durable `SessionRecord.last_seen_ms`.
   Use `orchestration_prune_store` for online durable stale session records
   through the running controller; use `mmux prune-store --dry-run` for offline
   local SQLite maintenance. Pruning only removes missing local `SessionRecord`s
   whose attached tasks are all finished; it does not remove projects, tasks,
   active task sessions, remote sessions, or live local sessions.

The `mmux-*` prefix is reserved for mmux-owned orchestration sessions generated
from task slug, agent kind, and a short suffix. Never treat arbitrary tmux
sessions or non-`mmux-*` sessions as orchestration cleanup candidates.

Startup reconciliation loads durable `SessionRecord`s and compares them with
live local `mmux-*` sessions. It may recreate missing active stored sessions
only from recorded runtime choices. Unrecorded task-agent metadata does not
start sessions. Missing individual sessions can produce reconciliation
warnings; missing `tmux` with `--enable-local-node` fails local backend startup
early.

## Driving a Coding CLI (Profile-Aware Workflow)

### Step 1: Ensure the session exists
```
list_coder_profiles
start_coding_session(profile="codex", session="codex", node="msb-mmux-1", workspace_path="/path/to/project", objective="fix tests for project X")
```
`start_coding_session` creates or adopts the tmux session and returns without
waiting for the coding CLI to become ready.

### Step 2: Wait for the CLI to be ready
```
wait_start(session="codex", kind="coding-ready", profile="codex", timeout_seconds=120)
wait_status(wait_id="<returned wait_id>")
```

If the CLI shows startup noise or an update prompt, the profile's
`startup_dismiss` config can handle it before proceeding. Codex update prompts
use a policy: `skip-update` by default, or `update-now` when explicitly
configured in profile TOML.

### Step 3: Send your prompt
```
coding_send(session="codex", profile="codex", prompt="refactor auth module")
```

For initial orchestration task delegation, prefer:

```
coding_task_send(session="codex", profile="codex", task_id_or_slug="task-29", template="task", prompt="Implement this task. Report changed files, validation commands, blockers, and unresolved questions.")
```

### Step 4: Wait for it to finish processing
```
wait_start(session="codex", kind="coding-ready", profile="codex", timeout_seconds=120)
wait_status(wait_id="<returned wait_id>")
```

### Step 5: Read the output
```
coding_read(session="codex", lines=40)
```

### Step 6: Handle approval prompts
If the CLI asks for approval, use the appropriate action:
```
coding_action(session="codex", profile="codex", action="approve")
coding_action(session="codex", profile="codex", action="reject")
coding_action(session="codex", profile="codex", action="cancel")
```

### Full Example Sequence
```
1. list_coder_profiles
2. start_coding_session → profile="codex", session="codex"
3. coding_send or coding_task_send → prompt="implement fibonacci"
4. wait_start → kind="coding-ready", profile="codex"
5. wait_status → until completed, failed, or canceled
6. coding_read → lines=40
7. (if approval needed) coding_action → action="approve"
8. (repeat 4-7 as needed)
9. kill_session → session="codex"
```

## Profile System

Profiles define how to interact with a specific CLI. Common coder profiles are
built into mmux so local mode works without a profile TOML file. Optional
`[coder_profile.<name>]` sections in the active config overlay those built-ins:
omitted fields keep their built-in values, nested tables merge, and scalar/list
fields replace the built-in value. For this repo, backend examples live under
`example-backends/`:

- `example-backends/local/mmux.toml` for local overrides
- `mmux.toml.example` for root examples

The canonical profile section name is `coder_profile`.

Key fields:

| Field | Meaning |
|-------|---------|
| `cmd` | Command to launch the CLI session |
| `launch_strategy` | Launch mode: omitted/`direct` starts `cmd` as the tmux command; `shell_send` starts `bash` and sends `cmd Enter` |
| `text_mode` | Prompt text input mode for `coding_send`: `paste-buffer` or `literal-keys` |
| `submit_keys` | Real tmux keys sent after prompt text when `submit_after_text` is true, e.g. `Enter` |
| `submit_after_text` | Whether `coding_send` submits after inserting prompt text |
| `prompt_indicator` | Substring that means "the CLI is ready for input" |
| `busy_indicators` | Substrings that mean "the CLI is still processing" |
| `approve_keys` | Keys to send for approval (e.g., `y Enter`) |
| `reject_keys` | Keys to send for rejection (e.g., `n Enter`) |
| `cancel_keys` | Keys to send to cancel (e.g., `C-c`) |
| `escape_keys` | Keys to send to escape/dismiss (e.g., `Escape`) |
| `startup_dismiss` | Auto-dismiss startup noise if detected; update prompts use `policy = "skip-update"` or `policy = "update-now"` |

Use `list_coder_profiles` to inspect the loaded runtime profiles.

If a profile is missing, use `load_profile` to add it at runtime:
```
load_profile(toml="[coder_profile.custom]\nname = \"custom\"\ncmd = \"custom\"\n...")
```

Or load from a file:
```
load_profile(path="/path/to/custom.toml")
```

## When to Use Which Tool

### `send_input` vs `coding_send`
- `send_input` — raw text, no profile logic. Use for generic shells, REPLs, scripts.
- `coding_send` — profile-aware. Auto-dismisses startup noise and uses the profile's text/submit strategy. Use only for coding CLIs. Claude Code uses `text_mode = "literal-keys"` so `Enter` is sent as a real keypress rather than pasted text.
- `coding_task_send` — task-aware initial delegation. It renders task context
  from orchestration state and appends your instruction before using the same
  profile-aware send behavior. Use `template="quality-guard"` when the worker
  should check project/operator quality preferences rather than validate gates
  or perform a general review. Use `template="validate"` for gate/objective
  validation and `template="review"` for correctness/risk review.

### `capture_output` vs `coding_read`
- `capture_output` — any session. Can capture scrollback.
- `coding_read` — convenience wrapper around `capture_output` with default 40 lines. Use for coding CLIs.

### `wait_start` / `wait_status` / `wait_cancel`
- `wait_start` — starts a runtime-only wait job. Supports `stable`, `sentinel`, `prompt`, and `coding-ready`.
- `wait_status` — reports `pending`, `completed`, `failed`, or `canceled` with result details.
- `wait_cancel` — cancels a pending wait without killing or interrupting the tmux session.

Use `kind="coding-ready"` with `profile` for profile-aware CLI readiness. Wait
jobs are the canonical orchestration wait API. Wait jobs can target any
reachable execution node that supports mmux tmux command primitives.
Omitted/default `node` targets the embedded `local` node, which may be local
tmux or embedded Microsandbox.

`check_state` returns `has_prompt`, `promptable`, `busy`, and `turn_idle`.
`promptable=true` means the CLI can accept text, not that the current turn is
finished. For Codex, a prompt can be visible while `busy=true` and
`turn_idle=false`; use `turn_idle=true` or a completed `coding-ready` wait when
you need foreground work to have settled.

### `interact` vs `exec` vs manual send + wait
- `exec` — one-shot shell command. Creates session if needed, runs command, waits for completion, returns clean output (no prompt/command line). Best for ad-hoc commands.
- `interact` — sends text then waits for stable output. One-shot convenience for already-running sessions.
- Manual send + wait — gives you full control over the wait mode and parameters.

## Backend Node File Operations

### read_file
```
read_file(node="local", path="./src/main.rs", limit=1000)
```
Returns:
- `content` — text or base64-encoded bytes
- `encoding` — `"utf-8"` or `"base64"`
- `mime_type` — detected from magic bytes or extension
- `compression` — `"gzip"`, `"zstd"`, etc. if compressed
- `size_bytes` — total file size
- `read_bytes` — how many bytes were actually read

Always check `encoding` before interpreting `content`.

### save_file
```
save_file(node="local", path="./output.txt", content="hello", encoding="utf-8")
save_file(node="local", path="./image.png", content="<base64>", encoding="base64")
```
Creates parent directories on the selected backend node. Paths are interpreted
in that backend's filesystem namespace.

## Common Gotchas

### 1. Session does not exist
If you get `"Session 'X' does not exist"` for a coding CLI, call
`start_coding_session` first. For shell work, use `exec`; it creates its shell
session if needed.

### 2. Output is truncated
- `capture_output` defaults to visible pane only. Use `scrollback: true` for full history.
- `read_file` defaults to 4 MiB limit. Use `limit` to read more.

### 3. TUI is garbled
Resize the pane:
```
resize_pane(session="myapp", width=120, height=40)
```

### 4. Prompt not detected
Verify the profile's `prompt_indicator` actually matches the CLI's prompt string. Use `capture_output` to inspect what the CLI is showing.

### 5. CLI stuck / hung
```
send_key(session="myapp", key="C-c")   # Ctrl+C
send_key(session="myapp", key="Escape") # Escape
```

### 6. Token authentication required
If the server returns `401 Unauthorized`, you must include:
```
Authorization: Bearer <token>
```

## MCP Resources

Use these for discovery without calling tools:

- `resources/list` → see loaded profiles as `profile://{name}`
- `resources/templates/list` → see `session://{name}/output`, `info`, `scrollback`
- `resources/read` with `uri: "profile://opencode"` → inspect a profile's config

## MCP Prompts

- `prompts/get` with `name: "drive-coding-cli"` → full workflow guide
- `prompts/get` with `name: "debug-session"` → diagnostic checklist

Both accept optional `profile` and `session` arguments.

## Architecture Notes for Agents

- mmux shells out to the system `tmux` binary. With `--enable-local-node`,
  startup fails early if the local tmux backend is unavailable.
- The server is single-process but async-concurrent. Multiple agents can call tools simultaneously.
- `thread::sleep` has been replaced with `tokio::time::sleep`. Canonical wait jobs run outside the fast tmux actor path so pending waits do not block quick inspection requests.
- The controller does not start an embedded execution backend by default. Use
  `--enable-local-node` for the built-in local tmux backend, or
  `--enable-microsandbox-node --sandbox-name <name>` to attach the host-side
  Microsandbox connector to an existing running sandbox as node `local`.
- The local backend does not need a TOML file unless you want profile overlays.
- The local backend uses a private tmux socket. Use `--tmux-config <path>` only
  for local tmux backends when an explicit tmux config is needed. Use
  `mmux tmux -- <args>` or `mmux attach <session>` for manual debugging.
- Microsandbox lifecycle is managed by `msb`, not mmux. Use embedded
  Microsandbox mode for a single controller process, or use
  `mmux node --backend microsandbox --sandbox-name <name>` to attach an
  existing running sandbox to a distributed controller.
- Pass `--node-config` to override the default profile config file if needed.
