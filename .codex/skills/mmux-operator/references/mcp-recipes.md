# mmux MCP Recipes

Use these request shapes against `http://127.0.0.1:3000/mcp` unless the user
names another port.

## Common Headers

```bash
-H 'Accept: application/json, text/event-stream'
-H 'Content-Type: application/json'
```

If MCP bearer auth is enabled, the server reads its expected token from
`--mcp-token`, `--mcp-token-file`, or `MMUX_MCP_TOKEN`. HTTP clients must still
send that value explicitly:

```bash
-H "Authorization: Bearer $MMUX_MCP_TOKEN"
```

Never print token values.

## Profile And Session Discovery

```json
{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"list_coder_profiles","arguments":{}}}
```

```json
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"list_sessions","arguments":{"node":"local"}}}
```

```json
{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"session_info","arguments":{"node":"local","session":"opencode"}}}
```

## Start A Coder

```json
{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"start_coding_session","arguments":{"node":"local","profile":"opencode","session":"opencode","cwd":"/path/to/repo","objective":"await further coding instructions","timeout_seconds":30}}}
```

If `start_coding_session` says a session already exists, inspect it first. If it
is stale and the user did not ask to preserve it, kill it and start clean.

## Drive A Coder

```json
{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"coding_send","arguments":{"node":"local","profile":"opencode","session":"opencode","prompt":"Inspect the repo and summarize the architecture in 8 bullets. Do not edit files."}}}
```

```json
{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"coding_wait_ready","arguments":{"node":"local","profile":"opencode","session":"opencode","timeout_seconds":120}}}
```

```json
{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"coding_read","arguments":{"node":"local","session":"opencode","lines":80}}}
```

## Check And Recover

```json
{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"check_state","arguments":{"node":"local","profile":"opencode","session":"opencode"}}}
```

```json
{"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"capture_output","arguments":{"node":"local","session":"opencode","lines":120,"scrollback":true}}}
```

```json
{"jsonrpc":"2.0","id":10,"method":"tools/call","params":{"name":"coding_action","arguments":{"node":"local","profile":"opencode","session":"opencode","action":"approve"}}}
```

Use `reject`, `cancel`, or `escape` for approval prompts, stuck operations, or
modal screens.

## Cleanup

```json
{"jsonrpc":"2.0","id":11,"method":"tools/call","params":{"name":"kill_session","arguments":{"node":"local","session":"temporary-test"}}}
```

Do not kill a long-lived coder session that the user asked to keep open.
