---
name: mmux-developer
description: Use when changing mmux source code, tests, controller/node/runtime behavior, MCP tools, orchestration state, local or Microsandbox backends, profile handling, packaging, docs, or smoke tests in the mmux repository.
---

# mmux Developer

## Skill Definition

Use this skill when implementing or reviewing mmux code. mmux is an MCP
controller plus execution-node system around tmux, coder CLIs, durable
orchestration state, and local/remote backends.

Primary use cases:

- Change mmux source code, tests, CLI behavior, MCP tools, prompt templates, or
  orchestration state.
- Change local, Microsandbox, or distributed node backend behavior.
- Change coder profile send/read/wait behavior for codex, opencode, kimi, or
  claude.
- Update user-facing docs, skills, recipes, or smoke tests
  for changed mmux behavior.

## Catalog

Code areas:

- `crates/mmux-controller-core`: runtime-neutral orchestration and auth model.
- `crates/mmux-controller`: MCP server, tool schemas/handlers, local runtime
  integration, actor orchestration, prompt templates, store glue.
- `crates/mmux-node`: execution backends, tmux process control, local and
  Microsandbox node behavior.
- `crates/mmux-shared`: profile/config shapes shared across controller/node.
- `crates/mmux-wire`: generated wire RPC bindings.
- `src/main.rs`: root CLI commands and local tmux proxy helpers.
- `crates/mmux-controller/src/prompts`: compile-time prompt templates used by
  MCP tools.

Primary commands:

- `rg`: inspect existing structs, helpers, tests, and docs before editing.
- `cargo fmt --check`, `cargo fmt`: check or apply Rust formatting.
- `git diff --check`: catch whitespace problems.
- `cargo test -p mmux-controller <filter>`: focused controller tests.
- `cargo test -p mmux-controller-core <filter>`: focused orchestration/core
  tests.
- `cargo test -p mmux-node <filter>`: focused node/backend tests.
- `cargo test --workspace`: full workspace test suite.
- `cargo run -- ...`: isolated controller smoke tests.

User-facing surfaces to keep aligned:

- MCP tool schemas and handlers.
- CLI flags and proxy helpers.
- README and bundled Codex skills.
- `.codex/skills/mmux-*` skills and `references/mcp-recipes.md`.
- Prompt templates under `crates/mmux-controller/src/prompts`.

## Bootstrap

1. Run `git status --short` and preserve unrelated user changes.
2. Use `rg` to inspect the current implementation, tests, docs, and prompt
   templates before designing changes.
3. Identify the affected boundary: controller core, controller MCP/runtime,
   node backend, shared profile config, wire bindings, CLI, docs, or skills.
4. Pick the smallest change that preserves the current module boundary.
5. Choose focused tests before editing so validation matches the behavioral
   risk.

## Development Rules

- Read the current code before designing. mmux has moved quickly; do not assume
  old README text or prior conversation state matches the implementation.
- Prefer canonical behavior only. Do not add legacy aliases, compatibility
  fallbacks, or parallel old/new paths unless the user explicitly asks.
- Keep runtime-neutral contracts out of local-runtime code. Shared task,
  project, auth, wire, and DTO logic belongs in core/shared crates; local tmux,
  Microsandbox, and process details belong behind node/backend/runtime layers.
- Treat node/backend filesystem paths as backend-owned strings. Do not
  canonicalize remote or sandbox paths in the controller.
- Preserve actor boundaries. Long-running node commands should go through the
  node execution actor path and use bounded timeouts; do not block quick
  inspection tools behind session startup or wait jobs.
- Be careful with global session visibility. Normal session discovery is
  project-scoped; raw node/tmux discovery belongs in explicit admin/debug
  tools.
- Keep task orchestration simple in v1. Prefer strings for descriptive role,
  kind, and skill metadata unless the value controls runtime authority.
- When adding MCP tools, update schema, handler, tests, README, and relevant
  Codex skills/recipes in the same change.
- When changing coder profile behavior, consider codex, claude, kimi, and
  opencode profiles. Profile-specific behavior should live in
  canonical built-in profile modules, not scattered special cases.

## Implementation Checklist

1. Add or update focused tests for the behavior, not only parsing.
2. Update docs and skills when the user-facing MCP surface, CLI flags, session
   semantics, profiles, or orchestration behavior changes.
3. Run focused tests first, then workspace tests.
4. Smoke test real MCP behavior when the change touches tool schemas, tool
   handlers, sessions, tmux/node behavior, profile send/read/wait paths, or
   persistent orchestration state.

## Test Commands

Use narrow tests while iterating:

```bash
cargo test -p mmux-controller <test-name-or-filter>
cargo test -p mmux-controller-core <test-name-or-filter>
cargo test -p mmux-node <test-name-or-filter>
```

Before finishing, run:

```bash
cargo fmt --check
git diff --check
cargo test --workspace
```

If formatting fails, run `cargo fmt`, then rerun `cargo fmt --check`.

## MCP Smoke Tests

Do a real MCP smoke test for changes involving:

- tool schemas or arguments;
- session listing, starting, recording, killing, reading, sending, waiting, or
  cleanup;
- task/project orchestration state;
- node/backend behavior;
- profile text submission/readiness behavior;
- auth or token flag behavior.

Use an isolated store and port:

```bash
rm -rf /tmp/mmux-smoke
env -u MMUX_MCP_TOKEN -u MMUX_WIRE_TOKEN \
  cargo run -- \
  --allow-remote-without-mcp-token \
  --allow-unauthenticated-node-wire \
  --enable-local-node \
  --port 3197 \
  --store-path /tmp/mmux-smoke
```

Then call `http://127.0.0.1:3197/mcp` with JSON-RPC. Include:

- `project_create`, `plan_create`, and `task_create` when testing
  project/plan/task-scoped behavior.
- `start_coding_session` or `exec` plus `session_record` when testing recorded
  sessions.
- The changed tool with both positive and negative cases.
- Cleanup of any created tmux sessions.

For project-scoped session listing, verify:

- `list_sessions` without `project_id` fails.
- `list_sessions(project_id)` accepts either project UUID id or globally unique
  slug and returns only durable recorded sessions attached to tasks in that
  project.
- Cross-project recorded sessions appear for each relevant project.
- `admin_list_node_sessions` shows raw live node sessions when admin/debug
  visibility is expected.

For coder prompt paths, verify:

- complex text containing quotes, apostrophes, backticks, and fenced code is
  submitted correctly;
- profile-specific submit behavior works for codex and claude when
  relevant;
- `coding_send` returns quickly and long work is tracked with
  `wait_start(kind="coding-ready")`, `wait_status`, and `coding_read`.
- `coding_read` compact output removes profile chrome while `raw=true` still
  returns the full pane when needed.
- blocking confirmation screens, such as claude bypass-permissions prompts, are
  not reported as promptable or turn-idle.

Always stop the smoke controller and remove the temporary store afterwards.

## Final Report

Report:

- source files changed;
- docs/skills updated;
- focused tests run;
- workspace test result;
- smoke test result or why smoke was not applicable.
