---
name: mmux-operator
description: Use when operating mmux MCP/tmux coder sessions, delegating work to opencode/codex/kimi/claude through mmux, preserving the primary agent context budget, or coordinating expensive/secret-bearing local model and provider workflows.
---

# mmux Operator

## Skill Definition

Use this skill to operate mmux as the controller, not the worker. Create and
supervise coder sessions, assign focused tasks, read concise results, and
intervene only when needed.

Primary use cases:

- Drive profile-aware coder CLIs through mmux MCP tools.
- Coordinate durable plans, tasks, sessions, gates, and status inside
  a selected project boundary.
- Preserve this agent's context by delegating broad exploration or long-running
  implementation to worker sessions.
- Inspect or recover tmux-backed sessions without bypassing mmux state.

## Catalog

Core MCP endpoint:

- Default URL: `http://127.0.0.1:3000/mcp`.
- Health check: `GET /health`.
- If MCP bearer auth is enabled, pass `Authorization: Bearer $MMUX_MCP_TOKEN`
  and never print token values.

Discovery and state tools:

- `tools/list`: discover the current MCP surface.
- `list_coder_profiles`: inspect enabled built-in coder profiles.
- `orchestration_status`: inspect projects, plans, tasks, outcomes, blockers,
  task sessions, warnings, cleanup candidates, and runtime state.
- `project_list`, `list_sessions(project_id)`, `admin_list_node_sessions`,
  `session_info`, `check_state`: inspect durable and live runtime state.

Project, plan, and task tools:

- `project_create`: create a project boundary through MCP.
- `plan_create`, `plan_list`, `plan_update`, `plan_status_update`: manage
  plan work-package documents and status.
- `task_create`, `task_update`, `task_status_update`: manage
  task metadata, task session, and state.
- `task_edge_add`, `task_edge_remove`: maintain task relationships.

Session and coder tools:

- `start_coding_session`: create or adopt a profile-driven coder session.
- `session_record`: attach an existing session to durable task state.
- `coding_task_send`: send initial task-aware work using rendered task context.
- `coding_send`: send follow-up steering or non-task prompts.
- `wait_start`, `wait_status`, `wait_cancel`: supervise waits.
- `coding_read`, `capture_output`: read compact or raw session output.
- `coding_action`, `send_key`, `kill_session`: handle prompts, interrupts, and
  cleanup.

Profile selection:

- `list_coder_profiles` reports only profiles enabled for the running
  controller.
- If a tool omits `profile`, mmux uses `--default-coder-profile` when
  configured, otherwise the first enabled built-in profile in canonical order:
  `codex`, `opencode`, `kimi`, then `claude`.

Read `references/mcp-recipes.md` when exact JSON-RPC request bodies, headers,
or troubleshooting examples are needed.

## Bootstrap

1. Confirm the controller is reachable with `GET /health`.
2. Discover the loaded tool surface with `tools/list`.
3. Call `list_coder_profiles`; only use profiles enabled by this controller.
4. Inspect durable state with `orchestration_status`.
5. For project-scoped sessions, call `list_sessions(project_id)` with a project
   UUID id or globally unique slug. Use `admin_list_node_sessions` only for
   raw node/tmux admin debugging.
6. For task-aware starts, choose the actual runtime values up front:
   `node`, `profile`, `workspace_path`, `bypass_permissions`, `task_id`,
   `role`, `kind`, `skills`.

## Core Rules

- Treat this agent's context as expensive. Do not spend it reading broad file
  trees or long logs when a coder session can inspect and summarize.
- Keep secrets out of prompts, transcripts, and final answers. Refer to secret
  env vars by name only. Never ask a coder to print API keys or tokens.
- Prefer mmux MCP tools over direct terminal driving for interactive coder CLIs.
- Use `mmux attach <session>` or `mmux tmux -- <tmux args>` only as a manual
  inspection escape hatch when MCP output is not enough.
- Use `mmux create-project` for offline project setup. Use
  `mmux list-projects` and `mmux tmux -- list-sessions --project <project>`
  when you need to find project-scoped sessions from the CLI.
- Keep coder prompts specific: objective, constraints, expected output, and stop
  condition.
- Treat `coding_send` as submit-only. Coding prompts can run for minutes or
  hours; do not wait for completion inside `coding_send`. Track progress with
  `wait_start(kind = "coding-ready")`, poll `wait_status`, use `coding_read`
  for compact profile-aware output, and `wait_cancel`/`coding_action`/`send_key`
  only when steering is needed.
- Be patient with work that is already running under a wait job. Long-running
  coder work should be supervised by polling `wait_status` and reading the
  session, not abandoned just because it takes minutes. Since
  `start_coding_session` returns after create/adopt, readiness delays belong in
  wait jobs, not in session creation.
- After each significant coder action, capture output and check state before
  sending the next instruction.
- In `check_state`, `promptable=true` means the CLI can accept text; use it for
  steering the active turn only. It does not mean the current turn is finished.
- Use `turn_idle=true` or a completed `coding-ready` wait before sending a new
  independent prompt, new task prompt, validator prompt, review prompt, or
  quality-guard prompt.
- Do not leave accidental test sessions running. Keep intentional coder sessions
  alive when the user asks to wait for further instructions.
- Worker sessions report findings, evidence, blockers, and proposed changes.
  The operator records accepted task mutations unless authority is explicitly
  delegated outside this skill.

## Default Workflow

1. Start or reuse the right coder session:
   `start_coding_session(node, profile, session, workspace_path)`.
   This creates or adopts the tmux session and returns without waiting for the
   coding CLI to become ready.
2. Wait for readiness:
   `wait_start` with `kind = "coding-ready"` and `profile`, then `wait_status`;
   for quick checks use `check_state` or `capture_output`.
   If `check_state` shows `promptable=true` and `turn_idle=false`, you may send
   steering text related to the current active turn. Do not send a new task or
   unrelated prompt until `turn_idle=true` or the `coding-ready` wait completes.
3. Delegate initial task work with `coding_task_send`; use `coding_send` for
   follow-up steering or non-task prompts.
4. Wait, read, and steer:
   `wait_start`, `wait_status`, `wait_cancel`, `coding_read`,
   `coding_action`, `capture_output`.
   Do not use blocking readiness calls for long-running agent work unless the
   user explicitly wants a synchronous call.
5. Verify with targeted commands or ask the coder for a short verification
   report.
6. Kill only disposable sessions. Leave requested long-lived sessions running.

## Orchestration V1 Workflow

Use this flow when coordinating tasks through the orchestration tools:

1. Discover the current MCP surface with `tools/list` or equivalent client
   discovery, then call `list_coder_profiles`.
2. Inspect current orchestration state with `orchestration_status`; use it for
   projects, plans, task graph, task outcomes, blockers, task session
   summaries, cleanup candidates, warnings, and runtime states instead of
   scraping tmux output. Pass `include_completed=true` when delivered,
   canceled, or failed tasks matter.
3. Create or select a project with CLI
   `mmux create-project <title> --description <text>` for offline setup or MCP
   `project_create`/`project_list` through a running controller. Projects have
   required descriptions. MCP `project_create` and `project_status_update` are
   only advertised and callable when the controller starts with
   `--enable-admin-tools`; `project_list` is always available.
4. Create a plan with `plan_create`: required `project_id`, `title`, and
   Markdown `brief` with enough context and detail to derive tasks. Create
   tasks with `task_create`: required `plan_id`, `title`, `objective`,
   `include_paths`, `exclude_paths`, `notes`, optional `gates`. The
   response is the created task object directly; read `id` from the top level.
   Created tasks start in `Backlog`; move them to `Planned` only when
   dependencies and scope are ready.
5. Correct mutable task metadata with `task_update`.
6. Maintain dependency edges with `task_edge_add` and `task_edge_remove`.
7. Start task-aware coder sessions with `start_coding_session`. Provide
   explicit `node`, `profile`, `workspace_path`, boolean `bypass_permissions`,
   `task_id`, `role`, `kind`, `skills`. Provide `session`, or
   request `generate_session_name = true`.
   When `task_id` is present, `node` is mandatory. Pass the selected runtime
   node explicitly, such as `node = "local"` for the embedded local node.
   Treat `workspace_path` as session start/adoption placement; do not recreate
   a live session only because its current working directory changed.
   Starting or recording a different `node_id`/`session` for the same task
   replaces the task session canonically and stops the previous live session
   with `tmux kill-session` through the previous session's recorded node when
   it still exists. Recording the same `node_id`/`session` only refreshes
   metadata.
8. Do not assume non-`mmux-*` sessions are orchestration-owned.
9. Record existing or manually adopted coder sessions with `session_record`.
10. Use `coding_task_send` for initial task delegation. Pass `task_id_or_slug`
    and a concrete instruction; mmux builds deterministic task context from
    orchestration state and appends your instruction before sending. Use
    `template = "task"` for implementation/delegation, `template = "validate"`
    for gate validation, `template = "review"` for bug/risk review, and
    `template = "quality-guard"` for maintainability, architecture fit, naming,
    boundaries, lifecycle, API shape, and operator/project quality preferences.
    For validation or review of a task set, pass `context_task_ids` with every
    task id/slug whose result is in scope. This is the required operator-side
    task-card export path: mmux renders each card with status, gates,
    outcome/evidence, scope, blockers, edges, and session. Do
    not rely on the validator's primary task context, local artifacts alone, or
    worker-side mmux calls to recover prior task results.
    Use `coding_send` only for follow-up prompts, steering, corrections, or
    non-task sessions.
11. Update task state with `task_status_update`. Include `status` and a concise
    `outcome`; include `blockers` when blocked. For gated moves to `Passed` or
    `Delivered`, include evidence in the outcome.

Task-owned `gates` are validation checks. Moving a gated task to `Passed` or
`Delivered` requires an operator-recorded outcome.

`coding_task_send` templates answer different orchestration questions:

- `template = "task"` answers: "What work should this agent perform for this
  task?" Use it for the initial implementation/delegation prompt. Put concrete
  execution instructions in `prompt`: scope, constraints, expected report, and
  what not to mutate.
- `template = "validate"` answers: "Does the result satisfy the task gates and
  objective?" Use it for validator sessions. Put the validation focus in
  `prompt`: which gates, evidence, commands, files, or reports to inspect. For
  multi-task validation, include `context_task_ids` and require a
  `field_coverage_table`; if a task card, gate, outcome/evidence, or session
  record is missing, the worker must report the result as inconclusive.
- `template = "review"` answers: "Are there correctness risks, regressions,
  missing tests, contract breaks, or scope drift?" Use it for reviewer/auditor
  sessions. Put the changed files, suspected risks, or review angle in
  `prompt`.
- `template = "quality-guard"` answers: "Does the change conform to the
  project/operator quality bar?" Use it for maintainability and design-quality
  checks. Put operator-specific guard points in `prompt`, such as architecture
  boundaries, naming preferences, lifecycle clarity, API shape,
  canonical-only policy, or abstraction discipline.

Templates provide the operating mode and task context. The `prompt` provides
the specific assignment. Do not rely on a task id alone; include the concrete
focus the worker should act on.

For validation, prefer `coding_task_send` with `template = "validate"`, for
example:

```text
Validate this task against its gates. Report pass/fail findings, evidence
references, blockers, unresolved questions, and recommended status. Do not
mutate mmux task state directly.
```

For validation that spans prior tasks, the operator must pass the prior ids in
`context_task_ids`, for example `["task-71", "task-72", "task-73"]`, and the
instruction must tell the validator to check every card field. A separate
export file is acceptable only when the prompt names the path and carries the
same checklist. Never tell a worker session to query mmux for missing task
cards; missing cards are a prompt/context defect.

Do not send empty prompts or placeholder prompt text such as `null` or
`undefined`; these indicate a bad extraction/parsing path.

Use cancellable runtime wait jobs as the canonical orchestration wait API:
`wait_start`, `wait_status`, and `wait_cancel`. Supported wait kinds are
`stable`, `sentinel`, `prompt`, and `coding-ready`; `coding-ready` requires a
profile. If a wait job remains pending, inspect with `check_state` and
`coding_read`; cancel the wait job with `wait_cancel` before interrupting the
CLI.

`coding_read` is compact by default. It strips common dashboard, startup,
update, and status chrome from supported coder CLIs to reduce token waste. Use
`raw = true` only when compact output is insufficient and you need the exact
tmux pane text.

Use `orchestration_cleanup_zombies` only when intentionally cleaning
orchestration-owned sessions. Start with dry-run behavior.

## Delegation Prompts

Good prompts are short and bounded:

```text
Inspect this repo for where <feature> is implemented. Do not edit files.
Return: 3-6 bullet findings, exact files/functions, and one recommended next step.
Do not print secrets or full file contents.
```

```text
Implement <change>. Keep edits minimal and aligned with existing patterns.
Run the narrowest relevant tests. Report changed files, tests run, and blockers.
Do not touch unrelated worktree changes.
```

## Orchestration Role Prompt Examples

These are copy/edit examples for the `prompt` field of `coding_task_send` or
follow-up `coding_send`; they are not MCP prompts and not controller runtime
features. With `coding_task_send`, mmux supplies the task context. Worker
sessions should propose and report; the operator records accepted task
mutations.

```text
Role: planner
Task: <task_id> - <title>
Objective: <objective>
Context: <scope, dependencies, gates, blockers, session hints>
Task session: <node/session/profile/workspace_path/role/kind/skills>
Create a concise execution plan. Report: planning_outcome, proposed_plan,
proposed_tasks, dependencies, gates, blockers, unresolved_questions.
Do not edit files or mutate mmux task state.
```

```text
Role: task-manager
Task: <task_id> - <title>
Current state: <status, task session, dependencies, blockers, gates>
Task session: <node/session/profile/workspace_path/role/kind/skills>
Decide the next orchestration action. Report: decision_outcome,
proposed_status_changes, proposed_tasks, needs_planner, needs_task_writer,
needs_validator, needs_auditor, blocked_on, unresolved_questions.
Do not call mmux task tools unless explicitly instructed.
```

```text
Role: task-writer
Task goal: <goal or capability>
Known context: <facts, scope paths, dependencies, constraints>
Session hints: <node/profile/workspace_path if already chosen>
Draft concrete orchestration tasks with clear objectives, allowed paths,
dependencies, and validation gates. Report: task_writing_outcome, proposed_tasks,
scope_paths, gates, dependencies, blockers, unresolved_questions.
```

```text
Role: editable-worker
Task: <task_id> - <title>
Objective: <objective>
Task outcome and current status: <outcome/status/blockers>
Scope: include <paths>; exclude <paths>; workspace_path <workspace_path>
Dependencies and gates: <dependency status; gate list>
Task session: <node/session/profile/workspace_path/role/kind/skills>
Implement only this task. Run focused verification. Report: implementation_outcome,
changed_files, tests_run, blockers, unresolved_questions, proposed_tasks,
needs_planner, needs_task_writer, needs_validator, needs_auditor, blocked_on.
```

```text
Role: validator
Task context:
- <task_id>: <title>; status <status>; outcome <outcome>; blockers <blockers>
Changed-file groups: <paths grouped by purpose>
Gates to validate: <gate list with expected outcome>
Evidence to inspect: <tests, files, session reports copied or summarized here>
Review the current implementation against each gate. Report: verdict
pass|fail, gate_results, findings with severity and evidence reference,
changed_files reviewed, blockers, unresolved_questions.
Do not edit files.
```

```text
Role: auditor
Task/session context:
- <task_id/session>: <title/objective>; status <status>; outcome <outcome>
Scope, changed-file groups, dependencies, and gates: <copied compact details>
Audit for correctness, safety boundaries, missing evidence, and cleanup risk.
Report: verdict pass|fail, gate_results if gates exist, findings with severity
and evidence reference, changed_files reviewed, blockers, unresolved_questions.
Do not edit files.
```

```text
Role: extractor
Source context: <files, logs, session output, task graph>
Question and scope: <question, include/exclude paths, dependencies>
Extract only facts relevant to <question>. Report: extraction_outcome, extracted_facts,
evidence_refs, proposed_tasks, blockers, unresolved_questions.
Do not infer beyond the evidence.
```

```text
Role: reviewer
Change under review: <files/commit/session/task with copied summaries>
Changed-file groups: <paths grouped by purpose>
Gates and expected behavior: <gate list, acceptance criteria, expected outcomes>
Review for regressions, contract breaks, stale docs, and missing tests. Report:
verdict pass|fail, findings with severity and evidence reference,
changed_files reviewed, blockers, unresolved_questions.
Do not edit files.
```

```text
Role: quality-guard
Task/change context: <task, files, summaries, operator preferences>
Guard points: <project/operator-specific quality concerns>
Check maintainability, architecture fit, naming, boundaries, lifecycle, state
ownership, API coherence, and project conventions. Report: overall
recommendation proceed|revise|escalate, relevant built-in heuristic concerns,
operator guard point results, evidence refs, recommended corrections, blockers.
Do not edit files.
```

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
