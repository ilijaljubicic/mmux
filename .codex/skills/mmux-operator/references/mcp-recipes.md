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
`node` is mandatory when `task_ids` is present; pass `node: "local"` for the
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

```json
{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"project_create","arguments":{"title":"mmux orchestration","description":"Tasks for mmux orchestration work"}}}
```

```json
{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"project_list","arguments":{}}}
```

## Tasks

```json
{"jsonrpc":"2.0","id":10,"method":"tools/call","params":{"name":"task_create","arguments":{"project_id":"project-1","title":"Update orchestration docs","objective":"Document the orchestration workflow.","include_paths":["README.md","AGENTS.md"],"exclude_paths":["target"],"notes":"Documentation-only task.","agents":[{"kind":"codex","role":"editable-worker","skills":["docs","mmux"],"workspace_path":"/mnt/Radni/mmux","objective":"Patch only scoped docs.","prompt":"Implement the docs update only. Report changed files, validation commands, blockers, and unresolved questions."}],"gates":["Docs updated","No unrelated files changed"]}}}
```

```json
{"jsonrpc":"2.0","id":11,"method":"tools/call","params":{"name":"task_update","arguments":{"task_id":"task-0001","title":"Update orchestration docs and recipes","notes":"Docs task expanded to include recipes.","gates":["Docs mention current workflow","Examples are usable"]}}}
```

```json
{"jsonrpc":"2.0","id":12,"method":"tools/call","params":{"name":"task_assign","arguments":{"task_id":"task-0001","node_id":"local","session":"mmux-docs-worker","profile":"codex","role":"editable-worker","kind":"codex","skills":["docs","mmux"]}}}
```

```json
{"jsonrpc":"2.0","id":13,"method":"tools/call","params":{"name":"task_edge_add","arguments":{"from_task_id":"task-0001","to_task_id":"task-0002","kind":"DependsOn","note":"Validation should run after docs are patched."}}}
```

```json
{"jsonrpc":"2.0","id":14,"method":"tools/call","params":{"name":"task_edge_remove","arguments":{"from_task_id":"task-0001","to_task_id":"task-0002","kind":"DependsOn"}}}
```

## Sessions

```json
{"jsonrpc":"2.0","id":20,"method":"tools/call","params":{"name":"start_coding_session","arguments":{"node":"local","profile":"codex","bypass_permissions":false,"task_ids":["task-0001"],"role":"editable-worker","kind":"codex","skills":["docs","mmux"],"workspace_path":"/mnt/Radni/mmux","objective":"Patch docs and report validation evidence.","generate_session_name":true}}}
```

Use the returned session name for `coding_task_send`, follow-up `coding_send`,
wait tools, `coding_read`, and task updates.

```json
{"jsonrpc":"2.0","id":21,"method":"tools/call","params":{"name":"session_record","arguments":{"node_id":"local","session":"codex-docs-worker","profile":"codex","workspace_path":"/mnt/Radni/mmux","bypass_permissions":false,"task_ids":["task-0001"],"role":"editable-worker","kind":"codex","skills":["docs","mmux"],"objective":"Patch docs and report validation evidence."}}}
```

## Send Work

```json
{"jsonrpc":"2.0","id":30,"method":"tools/call","params":{"name":"coding_task_send","arguments":{"node":"local","profile":"codex","session":"mmux-docs-worker","task_id_or_slug":"task-0001","template":"task","prompt":"Implement only this task. Run focused validation. Report: summary, changed_files, validations_run, blockers, unresolved_questions. Do not mutate mmux task state directly."}}}
```

Use `template:"quality-guard"` when the worker should check project/operator
quality preferences rather than validate gates or perform a general review:

```json
{"jsonrpc":"2.0","id":30,"method":"tools/call","params":{"name":"coding_task_send","arguments":{"node":"local","profile":"codex","session":"mmux-docs-worker","task_id_or_slug":"task-0001","template":"quality-guard","prompt":"Check for over-generalization, hidden runtime assumptions, unclear ownership, compatibility fallbacks, and abstractions that do not reduce complexity. Report proceed|revise|escalate with evidence and recommended corrections."}}}
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
{"jsonrpc":"2.0","id":40,"method":"tools/call","params":{"name":"task_status_update","arguments":{"task_id":"task-0001","status":"Running","summary":"Docs worker session started with scoped prompt."}}}
```

```json
{"jsonrpc":"2.0","id":41,"method":"tools/call","params":{"name":"task_status_update","arguments":{"task_id":"task-0001","status":"Blocked","summary":"Validation cannot complete because the docs checker command is missing.","blockers":["No markdown checker target found"]}}}
```

```json
{"jsonrpc":"2.0","id":42,"method":"tools/call","params":{"name":"task_status_update","arguments":{"task_id":"task-0001","status":"Passed","summary":"Validation passed and scoped diff reviewed."}}}
```

```json
{"jsonrpc":"2.0","id":43,"method":"tools/call","params":{"name":"task_status_update","arguments":{"task_id":"task-0001","status":"Delivered","summary":"Delivered after validation passed and no unrelated changes were found."}}}
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
{"jsonrpc":"2.0","id":60,"method":"tools/call","params":{"name":"list_sessions","arguments":{"node":"local","project_id":"project-1"}}}
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
