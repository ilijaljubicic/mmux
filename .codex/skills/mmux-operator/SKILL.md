---
name: mmux-operator
description: Use when operating mmux MCP/tmux coder sessions, delegating work to opencode/codex/kimi/claude through mmux, preserving the primary agent context budget, or coordinating expensive/secret-bearing local model and provider workflows.
---

# mmux Operator

Operate as the controller, not the worker. Use mmux to create and supervise coder
sessions, assign focused tasks, read their concise results, and intervene only
when needed.

## Core Rules

- Treat this agent's context as expensive. Do not spend it reading broad file
  trees or long logs when a coder session can inspect and summarize.
- Keep secrets out of prompts, transcripts, and final answers. Refer to secret
  env vars by name only. Never ask a coder to print API keys or tokens.
- Prefer mmux MCP tools over direct terminal driving for interactive coder CLIs.
- Do not use `mmux microsandbox`; mmux does not manage Microsandbox lifecycle.
  Use `msb` for sandbox lifecycle and `mmux node --backend microsandbox` only
  to attach an existing sandbox to a controller.
- Keep coder prompts specific: objective, constraints, expected output, and stop
  condition.
- After each significant coder action, capture output and check state before
  sending the next instruction.
- Do not leave accidental test sessions running. Keep intentional coder sessions
  alive when the user asks to wait for further instructions.

## Default Workflow

1. Confirm the mmux server:
   `GET /health`, then `list_coder_profiles` and `list_sessions`.
2. Start or reuse the right coder session:
   `start_coding_session(profile, session, cwd, objective)`.
3. Wait for readiness:
   `coding_wait_ready` or `check_state`; for TUIs, also `capture_output`.
4. Delegate the task with `coding_send`.
5. Wait, read, and steer:
   `coding_wait_ready`, `coding_read`, `coding_action`, `capture_output`.
6. Verify with targeted commands or ask the coder for a short verification
   report.
7. Kill only disposable sessions. Leave requested long-lived sessions running.

## Delegation Prompts

Good prompts are short and bounded:

```text
Inspect this repo for where <feature> is implemented. Do not edit files.
Return: 3-6 bullet summary, exact files/functions, and one recommended next step.
Do not print secrets or full file contents.
```

```text
Implement <change>. Keep edits minimal and aligned with existing patterns.
Run the narrowest relevant tests. Report changed files, tests run, and blockers.
Do not touch unrelated worktree changes.
```

## OpenCode Through mmux

OpenCode needs shell-mediated launch in this environment. The built-in profile
should use:

```toml
[coder_profile.opencode]
cmd = "opencode"
launch_strategy = "shell_send"
prompt_indicator = "ctrl+p commands"
busy_indicators = ["Thinking", "Working", "Running", "Processing", "Generating"]
```

`shell_send` means mmux starts `bash`, waits briefly for the shell pane, sends
`cmd`, then sends Enter. This avoids OpenCode `setRawMode failed with errno: 5`
from direct detached tmux launch.

## Context Budget Discipline

Use direct file reads only for:

- checking exact code before making a patch;
- validating a coder report;
- inspecting small config files;
- reading test failures or short diffs.

Use coder sessions for:

- repo exploration;
- tracing call graphs;
- broad search and summarization;
- implementation attempts that may take many tool calls;
- comparing alternatives before this agent patches.

Ask the coder to return concise summaries, not copied files.

## References

For exact MCP request examples and recovery commands, read
`references/mcp-recipes.md`.
