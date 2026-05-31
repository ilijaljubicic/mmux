# Agent Guide: Using mmux

This document is for AI agents that need to operate terminals, coding CLIs, or interactive TUI applications through mmux.

## What mmux Does

mmux is an MCP server that controls tmux sessions. It lets you:
- Inspect and manage tmux sessions
- Send text and key sequences to any session
- Capture output (visible lines or full scrollback)
- Wait for conditions (stable output, sentinel strings, prompt markers)
- Read and save files on the host filesystem
- Drive coding CLIs (opencode, aider, codex, etc.) with profile-aware actions

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

Do you need to drive a coding CLI (opencode, aider, codex)?
  └─ Use list_coder_profiles → start_coding_session → coding_send → coding_wait_ready → coding_read → coding_action

Do you need to check what's happening without waiting?
  └─ Use check_state or capture_output

Do you need to read or write a file?
  └─ Use read_file or save_file (no session needed)
```

## Session Lifecycle Best Practices

### 1. Always check what is already running
```
list_sessions  → see what's running
session_info   → deep inspect a specific session
check_state    → quick JSON check (has_prompt, busy)
```

### 2. Start profile-driven sessions explicitly
```
start_coding_session(profile="codex", session="codex", node="msb-mmux-1")
```

The public MCP surface no longer exposes raw generic session creation. Use `start_coding_session` for coding CLIs and the debug/read tools for inspection.

### 3. Capture output after every significant action
```
send_input(session="myapp", text="cargo test")
wait_for(session="myapp", mode="stable", timeout_seconds=60)
capture_output(session="myapp", lines=40)
```

### 4. Detach without killing (keep session running in background)
When inside a tmux session via `sandbox-tmux-attach` or direct tmux:
```
Ctrl+b  then  d      → detach, session keeps running
```
Or run from inside the session:
```
tmux detach
```

This is useful when you want to leave a coding CLI (codex, aider, etc.) running and come back to it later.

### 5. Clean up when done
```
kill_session(session="myapp")
```

## Driving a Coding CLI (Profile-Aware Workflow)

### Step 1: Ensure the session exists
```
list_coder_profiles
start_coding_session(profile="codex", session="codex", node="msb-mmux-1", cwd="/path/to/project")
```

### Step 2: Wait for the CLI to be ready
```
coding_wait_ready(session="codex", profile="codex", timeout_seconds=30)
```

If the CLI shows startup noise (e.g., "Starting MCP servers"), the profile's `startup_dismiss` config will automatically send Escape before proceeding.

### Step 3: Send your prompt
```
coding_send(session="codex", profile="codex", prompt="refactor auth module")
```

### Step 4: Wait for it to finish processing
```
coding_wait_ready(session="codex", profile="codex", timeout_seconds=120)
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
3. coding_wait_ready → profile="codex"
4. coding_send → prompt="implement fibonacci"
5. coding_wait_ready → profile="codex"
6. coding_read → lines=40
7. (if approval needed) coding_action → action="approve"
8. (repeat 4-7 as needed)
9. kill_session → session="codex"
```

## Profile System

Profiles define how to interact with a specific CLI. They are loaded from the active config file at startup. For this repo, the canonical backend configs live under `example-backends/`:

- `example-backends/local/mmux.toml`
- `example-backends/microsandbox/mmux.toml`

The canonical profile section name is `coder_profile`.

Key fields:

| Field | Meaning |
|-------|---------|
| `cmd` | Command to launch the CLI session |
| `prompt_indicator` | Substring that means "the CLI is ready for input" |
| `busy_indicators` | Substrings that mean "the CLI is still processing" |
| `approve_keys` | Keys to send for approval (e.g., `y Enter`) |
| `reject_keys` | Keys to send for rejection (e.g., `n Enter`) |
| `cancel_keys` | Keys to send to cancel (e.g., `C-c`) |
| `escape_keys` | Keys to send to escape/dismiss (e.g., `Escape`) |
| `startup_dismiss` | Auto-dismiss startup noise if detected |

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
- `coding_send` — profile-aware. Auto-dismisses startup noise, handles prompt formatting. Use only for coding CLIs.

### `capture_output` vs `coding_read`
- `capture_output` — any session. Can capture scrollback.
- `coding_read` — convenience wrapper around `capture_output` with default 40 lines. Use for coding CLIs.

### `wait_for` vs `coding_wait_ready`
- `wait_for` — generic. Supports `stable` (output stops changing), `sentinel` (text appears), `prompt` (marker appears).
- `coding_wait_ready` — profile-aware. Combines "prompt visible + not busy" into one check. Use for coding CLIs.

### `interact` vs `exec` vs manual send + wait
- `exec` — one-shot shell command. Creates session if needed, runs command, waits for completion, returns clean output (no prompt/command line). Best for ad-hoc commands.
- `interact` — sends text then waits for stable output. One-shot convenience for already-running sessions.
- Manual send + wait — gives you full control over the wait mode and parameters.

## File Operations

### read_file
```
read_file(path="./src/main.rs", limit=1000)
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
save_file(path="./output.txt", content="hello", encoding="utf-8")
save_file(path="./image.png", content="<base64>", encoding="base64")
```
Creates parent directories automatically.

## Common Gotchas

### 1. Session does not exist
If you get `"Session 'X' does not exist"`, call `create_session` first.

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

- mmux shells out to the system `tmux` binary. If tmux is not installed, all tools fail.
- The server is single-process but async-concurrent. Multiple agents can call tools simultaneously.
- `thread::sleep` has been replaced with `tokio::time::sleep`. Long waits (e.g., `wait_for` with 60s timeout) do not block other requests.
- The controller does not start the local backend by default. Use `--enable-local-node` for the built-in local tmux backend.
- The local backend config lives at `example-backends/local/mmux.toml`.
- The Microsandbox backend config lives at `example-backends/microsandbox/mmux.toml`.
- Pass `--config` to override the default config file if needed.
