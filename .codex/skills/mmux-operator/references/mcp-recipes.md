# mmux MCP Recipes

Use these request bodies against `http://127.0.0.1:3000/mcp` unless the user
names another endpoint.

## Headers

```bash
-H 'Accept: application/json, text/event-stream'
-H 'Content-Type: application/json'
```

If MCP bearer auth is enabled, send the token explicitly and never print token
values:

```bash
-H "Authorization: Bearer $MMUX_MCP_TOKEN"
```

When calling MCP directly, check both JSON-RPC `error` and tool-level
`isError` before parsing the result as a success payload. Tool failures are
returned clearly inside the MCP envelope. For task-aware `start_coding_session`,
`node` is mandatory when `task_id` is present; pass `node: "local"` for the
embedded local node.

## Discovery

```json
{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}
```

```json
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"list_coder_profiles","arguments":{}}}
```

```json
{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"orchestration_status","arguments":{}}}
```

## Projects

MCP `project_create` is advertised and callable only when the controller starts
with `--enable-admin-tools`. It requires `title` and `description`. For offline
local store setup, use
`mmux create-project <title> --description <text> [--slug <slug>]`.

```json
{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"project_create","arguments":{"title":"mmux orchestration","description":"Tasks for mmux orchestration work"}}}
```

```json
{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"project_list","arguments":{}}}
```

## Plans

```json
{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"plan_create","arguments":{"project_id":"mmux","title":"Update orchestration docs plan","brief":"Document the orchestration workflow, update operator recipes, and validate examples. Tasks should cover README, AGENTS.md, and skill recipe updates."}}}
```

## Tasks

```json
{"jsonrpc":"2.0","id":10,"method":"tools/call","params":{"name":"task_create","arguments":{"plan_id":"plan-0001","title":"Update orchestration docs","objective":"Document the orchestration workflow.","include_paths":["README.md","AGENTS.md"],"exclude_paths":["target"],"notes":"Documentation-only task.","gates":["Docs updated","No unrelated files changed"]}}}
```

```json
{"jsonrpc":"2.0","id":11,"method":"tools/call","params":{"name":"task_update","arguments":{"task_id":"task-0001","title":"Update orchestration docs and recipes","notes":"Docs task expanded to include recipes.","gates":["Docs mention current workflow","Examples are usable"]}}}
```

```json
{"jsonrpc":"2.0","id":13,"method":"tools/call","params":{"name":"task_edge_add","arguments":{"from_task_id":"task-0001","to_task_id":"task-0002","kind":"DependsOn","note":"Validation should run after docs are patched."}}}
```

```json
{"jsonrpc":"2.0","id":14,"method":"tools/call","params":{"name":"task_edge_remove","arguments":{"from_task_id":"task-0001","to_task_id":"task-0002","kind":"DependsOn"}}}
```

## Sessions

```json
{"jsonrpc":"2.0","id":20,"method":"tools/call","params":{"name":"start_coding_session","arguments":{"node":"local","profile":"codex","bypass_permissions":false,"task_id":"task-0001","role":"editable-worker","kind":"codex","skills":["docs","mmux"],"workspace_path":"/mnt/Radni/mmux","generate_session_name":true}}}
```

Use the returned session name for `coding_task_send`, follow-up `coding_send`,
wait tools, `coding_read`, and task updates.

```json
{"jsonrpc":"2.0","id":21,"method":"tools/call","params":{"name":"session_record","arguments":{"node_id":"local","session":"codex-docs-worker","profile":"codex","workspace_path":"/mnt/Radni/mmux","bypass_permissions":false,"task_id":"task-0001","role":"editable-worker","kind":"codex","skills":["docs","mmux"]}}}
```

To change coder/session for a task, start or adopt the new session with the
same `task_id`. A different `node_id`/`session` replaces the task's recorded
session and mmux stops the previous live session with `tmux kill-session`
through the previous session's recorded node when it still exists. Send an
explicit handoff prompt to the new session after replacement.

```json
{"jsonrpc":"2.0","id":22,"method":"tools/call","params":{"name":"start_coding_session","arguments":{"node":"local","profile":"claude","session":"claude-docs-worker","workspace_path":"/mnt/Radni/mmux","bypass_permissions":false,"task_id":"task-0001","role":"implementation-worker","kind":"claude","skills":["docs","mmux"]}}}
```

## Send Work

```json
{"jsonrpc":"2.0","id":30,"method":"tools/call","params":{"name":"coding_task_send","arguments":{"node":"local","profile":"codex","session":"mmux-docs-worker","task_id_or_slug":"task-0001","template":"task","prompt":"Implement only this task. Run focused validation. Report: outcome, changed_files, validations_run, blockers, unresolved_questions. Do not mutate mmux task state directly."}}}
```

Use `template:"quality-guard"` when the worker should check project/operator
quality preferences rather than validate gates or perform a general review:

```json
{"jsonrpc":"2.0","id":30,"method":"tools/call","params":{"name":"coding_task_send","arguments":{"node":"local","profile":"codex","session":"mmux-docs-worker","task_id_or_slug":"task-0001","template":"quality-guard","prompt":"Check for over-generalization, hidden runtime assumptions, unclear ownership, obsolete fallback paths, and abstractions that do not reduce complexity. Report proceed|revise|escalate with evidence and recommended corrections."}}}
```

For validation of a task set, pass the reviewed task ids as `context_task_ids`.
This makes mmux render field-complete operator task cards into the validator
prompt. Do not ask the validator session to call mmux to discover prior task
results.

```json
{"jsonrpc":"2.0","id":30,"method":"tools/call","params":{"name":"coding_task_send","arguments":{"node":"local","profile":"codex","session":"mmux-validator","task_id_or_slug":"task-0009","template":"validate","context_task_ids":["task-0001","task-0002","task-0003"],"prompt":"Validate the supplied task-card bundle. For each card, check id, status, gates, outcome/evidence, scope, blockers, edges, and session. Report findings first, then field_coverage_table, gate_results, evidence, commands_or_checks_run, residual caveats, and recommended_status. Do not call mmux internally."}}}
```

```json
{"jsonrpc":"2.0","id":31,"method":"tools/call","params":{"name":"wait_start","arguments":{"node":"local","profile":"codex","session":"mmux-docs-worker","kind":"coding-ready","timeout_seconds":120,"poll_seconds":0.5}}}
```

```json
{"jsonrpc":"2.0","id":32,"method":"tools/call","params":{"name":"wait_status","arguments":{"wait_id":"wait-..."}}}
```

```json
{"jsonrpc":"2.0","id":33,"method":"tools/call","params":{"name":"coding_read","arguments":{"node":"local","session":"mmux-docs-worker","lines":80}}}
```

## Status

```json
{"jsonrpc":"2.0","id":40,"method":"tools/call","params":{"name":"task_status_update","arguments":{"task_id":"task-0001","status":"Running","outcome":"Docs worker session started with scoped prompt."}}}
```

```json
{"jsonrpc":"2.0","id":41,"method":"tools/call","params":{"name":"task_status_update","arguments":{"task_id":"task-0001","status":"Blocked","outcome":"Validation cannot complete because the docs checker command is missing.","blockers":["No markdown checker target found"]}}}
```

```json
{"jsonrpc":"2.0","id":42,"method":"tools/call","params":{"name":"task_status_update","arguments":{"task_id":"task-0001","status":"Passed","outcome":"Validation passed and scoped diff reviewed."}}}
```

```json
{"jsonrpc":"2.0","id":43,"method":"tools/call","params":{"name":"task_status_update","arguments":{"task_id":"task-0001","status":"Delivered","outcome":"Delivered after validation passed and no unrelated changes were found."}}}
```

## Cleanup

Start with dry-run cleanup:

```json
{"jsonrpc":"2.0","id":50,"method":"tools/call","params":{"name":"orchestration_cleanup_zombies","arguments":{}}}
```

Run explicit cleanup only after reviewing the dry-run result:

```json
{"jsonrpc":"2.0","id":51,"method":"tools/call","params":{"name":"orchestration_cleanup_zombies","arguments":{"node":"local","dry_run":false,"older_than_seconds":300}}}
```

Inspect state again after cleanup:

```json
{"jsonrpc":"2.0","id":52,"method":"tools/call","params":{"name":"orchestration_status","arguments":{"include_completed":true}}}
```

## Troubleshooting

```json
{"jsonrpc":"2.0","id":60,"method":"tools/call","params":{"name":"list_sessions","arguments":{"node":"local","project_id":"mmux"}}}
```

Use raw node visibility only for admin/debug:

```json
{"jsonrpc":"2.0","id":60,"method":"tools/call","params":{"name":"admin_list_node_sessions","arguments":{"node":"local"}}}
```

```json
{"jsonrpc":"2.0","id":61,"method":"tools/call","params":{"name":"session_info","arguments":{"node":"local","session":"mmux-docs-worker"}}}
```

```json
{"jsonrpc":"2.0","id":62,"method":"tools/call","params":{"name":"check_state","arguments":{"node":"local","profile":"codex","session":"mmux-docs-worker"}}}
```

```json
{"jsonrpc":"2.0","id":63,"method":"tools/call","params":{"name":"coding_action","arguments":{"node":"local","profile":"codex","session":"mmux-docs-worker","action":"approve"}}}
```
