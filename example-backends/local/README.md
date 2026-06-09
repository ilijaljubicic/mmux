# Local Backend Example

This directory documents the built-in local backend. The local backend runs
tmux on the same host as the controller, so it is the fastest path for
development and for agents that should operate your current machine directly.

## Run

```bash
make run-local
```

Equivalent direct command from the repository root:

```bash
cargo run -- controller --enable-local-node
```

Use an explicit tmux config for the mmux-owned local tmux server when needed:

```bash
cargo run -- controller --enable-local-node --tmux-config ./tmux.local.conf
```

`--tmux-config` is local-backend startup configuration. It is valid with
`--enable-local-node` on the controller or `mmux node --backend local` for a
distributed local node, and is not valid with Microsandbox.

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

Each profile is a canonical Rust adapter. mmux does not load local profile TOML
overlays and profiles cannot be added or overridden at runtime.
Use controller flags to select the public profile set: `--enabled-coder-profiles`
limits which built-ins are available, and `--default-coder-profile` selects the
profile used when profile-aware tools omit `profile`. Without an explicit
default, mmux uses the first enabled built-in in canonical order: `codex`,
`opencode`, `kimi`, then `claude`.

## Typical MCP Workflow

1. Call `list_coder_profiles` to confirm the built-in profile is available.
2. Call `start_coding_session` with `profile`, `session`, and optional `workspace_path`.
3. Send work with `coding_send`.
4. Start a canonical wait job with `wait_start` using `kind = "coding-ready"`
   and the same `profile`, then poll `wait_status` until it completes.
5. Use `wait_cancel` to cancel a pending wait job when needed.
6. Read output with `coding_read`.

The Microsandbox example backend lives in `../microsandbox/`.
