# Local Backend Example

This directory holds optional overrides for the built-in local backend. The
local backend runs tmux on the same host as the controller, so it is the fastest
path for development and for agents that should operate your current machine
directly.

## Run

```bash
make run-local
```

Equivalent direct command from the repository root:

```bash
cargo run -- controller --enable-local-node
```

The controller exposes MCP at:

```text
http://<controller-host>:3000/mcp
```

For local development, `<controller-host>` is usually the loopback hostname
used by your MCP client. If you bind the controller to another interface, use
bearer-token authentication.

## Profiles

Local mode uses built-in profiles by default:

- `codex`
- `opencode`
- `kimi`
- `claude`
- `generic`

Each profile declares a command, prompt marker, busy markers, and the keys used
for approve/reject/cancel/escape actions. `mmux.toml` is optional and should be
used only for local overlays. When overriding a built-in profile, set only the
fields you intentionally want to change. See `../../mmux.toml.example`.

## Typical MCP Workflow

1. Call `list_coder_profiles` to confirm the profile is loaded.
2. Call `start_coding_session` with `profile`, `session`, and optional `cwd`.
3. Send work with `coding_send`.
4. Wait with `coding_wait_ready`.
5. Read output with `coding_read`.

The Microsandbox example backend lives in `../microsandbox/`.
