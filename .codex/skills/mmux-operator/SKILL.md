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
- Use `mmux attach <session>` or `mmux tmux -- <tmux args>` only as a manual
  inspection escape hatch when MCP output is not enough.
- Use `mmux list-projects` and `mmux tmux -- list-sessions --project <project>`
  when you need to find project-scoped sessions from the CLI.
- Keep coder prompts specific: objective, constraints, expected output, and stop
  condition.
- Treat `coding_send` as submit-only. Coding prompts can run for minutes or
  hours; do not wait for completion inside `coding_send`. Track progress with
  `wait_start(kind = "coding-ready")`, poll `wait_status`, use `coding_read`
  for output, and `wait_cancel`/`coding_action`/`send_key` only when steering is
  needed.
- Be patient with work that is already running under a wait job. Long-running
  coder work should be supervised by polling `wait_status` and reading the
  session, not abandoned just because it takes minutes. Since
  `start_coding_session` returns after create/adopt, readiness delays belong in
  wait jobs, not in session creation.
- After each significant coder action, capture output and check state before
  sending the next instruction.
- In `check_state`, `promptable=true` means the CLI can accept text; it does
  not mean the current turn is finished. Use `turn_idle=true` or a completed
  `coding-ready` wait when you need foreground work to have settled.
- Do not leave accidental test sessions running. Keep intentional coder sessions
  alive when the user asks to wait for further instructions.
- Worker sessions report findings, evidence, blockers, and proposed changes.
  The operator records accepted task mutations unless authority is explicitly
  delegated outside this skill.

## Default Workflow

1. Confirm the mmux server:
   `GET /health`, then `list_coder_profiles` and project-scoped
   `list_sessions(project_id)`. Use `admin_list_node_sessions` only for raw
   node/tmux admin debugging.
2. Start or reuse the right coder session:
   `start_coding_session(profile, session, workspace_path, objective)`.
   This creates or adopts the tmux session and returns without waiting for the
   coding CLI to become ready.
3. Wait for readiness:
   `wait_start` with `kind = "coding-ready"` and `profile`, then `wait_status`;
   for quick checks use `check_state` or `capture_output`.
   If `check_state` shows `promptable=true` and `turn_idle=false`, you may send
   steering text, but the previous foreground turn is still active.
4. Delegate initial task work with `coding_task_send`; use `coding_send` for
   follow-up steering or non-task prompts.
5. Wait, read, and steer:
   `wait_start`, `wait_status`, `wait_cancel`, `coding_read`,
   `coding_action`, `capture_output`.
   Do not use blocking readiness calls for long-running agent work unless the
   user explicitly wants a synchronous call.
6. Verify with targeted commands or ask the coder for a short verification
   report.
7. Kill only disposable sessions. Leave requested long-lived sessions running.

## Orchestration V1 Workflow

Use this flow when coordinating tasks through the orchestration tools:

1. Discover the current MCP surface with `tools/list` or equivalent client
   discovery, then call `list_coder_profiles`.
2. Inspect current orchestration state with `orchestration_status`; use it for projects, task
   graph, task summaries, blockers, owner/session summaries, cleanup
   candidates, warnings, and runtime states instead of scraping tmux output.
   Pass `include_completed=true` when delivered, canceled, or failed tasks
   matter.
3. Create or select a project with `project_create`/`project_list`.
4. Create tasks with `task_create`: required `project_id`, `title`,
   `objective`, `agents`, `include_paths`, `exclude_paths`, `notes`, optional
   `gates`. The response is the created task object directly; read `id` from
   the top level. Created tasks start in `Backlog`; move them to `Planned` only
   when dependencies and scope are ready.
5. Correct mutable task metadata with `task_update`.
7. Assign owners with `task_assign` using actual runtime choices: `task_id`,
   `node_id`, `session`, `profile`, `role`, `kind`, and `skills`.
8. Maintain dependency edges with `task_edge_add` and `task_edge_remove`.
9. Start task-aware coder sessions with `start_coding_session`. Provide
   explicit `node`, `profile`, `workspace_path`, boolean `bypass_permissions`,
   `task_ids`, `role`, `kind`, `skills`, and `objective`. Provide `session`, or
   request `generate_session_name = true`.
   When `task_ids` is present, `node` is mandatory. Pass the selected runtime
   node explicitly, such as `node = "local"` for the embedded local node.
   `TaskAgent` metadata is planning intent only; mmux does not infer node,
   profile, workspace, or bypass settings from it.
10. Do not assume non-`mmux-*` sessions are orchestration-owned.
11. Record existing or manually adopted coder sessions with `session_record`.
12. Use `coding_task_send` for initial task delegation. Pass `task_id_or_slug`
    and a concrete instruction; mmux builds deterministic task context from
    orchestration state and appends your instruction before sending. Use
    `template = "task"` for implementation/delegation, `template = "validate"`
    for gate validation, `template = "review"` for bug/risk review, and
    `template = "quality-guard"` for maintainability, architecture fit, naming,
    boundaries, lifecycle, API shape, and operator/project quality preferences.
    Use `coding_send` only for follow-up prompts, steering, corrections, or
    non-task sessions.
13. Update task state with `task_status_update`. Include `status` and a concise
    `summary`; include `blockers` when blocked. For gated moves to `Passed` or
    `Delivered`, include evidence in the summary.

Task-owned `gates` are validation checks. Moving a gated task to `Passed` or
`Delivered` requires an operator-recorded summary.

`coding_task_send` templates answer different orchestration questions:

- `template = "task"` answers: "What work should this agent perform for this
  task?" Use it for the initial implementation/delegation prompt. Put concrete
  execution instructions in `prompt`: scope, constraints, expected report, and
  what not to mutate.
- `template = "validate"` answers: "Does the result satisfy the task gates and
  objective?" Use it for validator sessions. Put the validation focus in
  `prompt`: which gates, evidence, commands, files, or reports to inspect.
- `template = "review"` answers: "Are there correctness risks, regressions,
  missing tests, contract breaks, or scope drift?" Use it for reviewer/auditor
  sessions. Put the changed files, suspected risks, or review angle in
  `prompt`.
- `template = "quality-guard"` answers: "Does the change conform to the
  project/operator quality bar?" Use it for maintainability and design-quality
  checks. Put operator-specific guard points in `prompt`, such as architecture
  boundaries, naming preferences, lifecycle clarity, API shape, compatibility
  policy, or abstraction discipline.

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

Do not send empty prompts or placeholder prompt text such as `null` or
`undefined`; these indicate a bad extraction/parsing path.

Use cancellable runtime wait jobs as the canonical orchestration wait API:
`wait_start`, `wait_status`, and `wait_cancel`. Supported wait kinds are
`stable`, `sentinel`, `prompt`, and `coding-ready`; `coding-ready` requires a
profile. If a wait job remains pending, inspect with `check_state` and
`coding_read`; cancel the wait job with `wait_cancel` before interrupting the
CLI.

Use `orchestration_cleanup_zombies` only when intentionally cleaning
orchestration-owned sessions. Start with dry-run behavior.

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
Selected TaskAgent intent: <kind/role/skills/objective/prompt>
Create a concise execution plan. Report: summary, proposed_plan,
proposed_tasks, dependencies, gates, blockers, unresolved_questions.
Do not edit files or mutate mmux task state.
```

```text
Role: task-manager
Task: <task_id> - <title>
Current state: <status, owner/session, dependencies, blockers, gates>
Selected TaskAgent intent: <kind/role/skills/objective/prompt>
Decide the next orchestration action. Report: summary,
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
dependencies, and validation gates. Report: summary, proposed_tasks,
scope_paths, gates, dependencies, blockers, unresolved_questions.
```

```text
Role: editable-worker
Task: <task_id> - <title>
Objective: <objective>
Task summary and current status: <summary/status/blockers>
Scope: include <paths>; exclude <paths>; workspace_path <workspace_path>
Dependencies and gates: <dependency status; gate list>
Selected TaskAgent intent: <kind/role/skills/objective/prompt>
Implement only this task. Run focused verification. Report: summary,
changed_files, tests_run, blockers, unresolved_questions, proposed_tasks,
needs_planner, needs_task_writer, needs_validator, needs_auditor, blocked_on.
```

```text
Role: validator
Task context:
- <task_id>: <title>; status <status>; summary <summary>; blockers <blockers>
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
- <task_id/session>: <title/objective>; status <status>; summary <summary>
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
Extract only facts relevant to <question>. Report: summary, extracted_facts,
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
