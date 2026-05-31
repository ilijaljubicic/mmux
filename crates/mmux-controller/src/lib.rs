use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use clap::{Parser, ValueEnum};
use connectrpc::{
    ConnectError, ConnectRpcService, RequestContext as ConnectRequestContext,
    Response as ConnectResponse, ServiceResult,
};
use mmux_node::ProfileRegistry;
use mmux_shared::{CliProfile, ReadFileResult, SaveFileResult};
use mmux_wire::connect::mmux::wire::v1::{
    MmuxNodeRegistryService, MmuxNodeRegistryServiceServer, OwnedHeartbeatRequestView,
    OwnedPullCommandsRequestView, OwnedRegisterNodeRequestView,
    OwnedSubmitCommandResultRequestView,
};
use mmux_wire::proto::mmux::wire::v1 as wire_proto;
use mmux_wire::{
    heartbeat_request_from_proto, heartbeat_response_to_proto, pull_commands_request_from_proto,
    pull_commands_response_to_proto, register_node_request_from_proto,
    register_node_response_to_proto, submit_command_result_request_from_proto,
    submit_command_result_response_to_proto, NodeCommand, NodeCommandKind, NodeCommandResult,
    NodeDescriptor, NodeStatus, PullCommandsResponse, RegisterNodeResponse,
    SubmitCommandResultResponse,
};
use serde_json::{json, Map, Value};
use std::collections::{HashMap, VecDeque};
use std::ffi::OsString;
use std::net::SocketAddr;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::{
    body::Body,
    extract::DefaultBodyLimit,
    http::{Request, StatusCode},
    middleware::{self, Next},
    response::Response,
};
use ractor::{rpc::CallResult, Actor, ActorProcessingErr, ActorRef, RpcReplyPort};
use tower_http::cors::{Any, CorsLayer};

// ═══════════════════════════════════════════════════════════════════════════════
//  rmcp imports (MCP HTTP server)
// ═══════════════════════════════════════════════════════════════════════════════

use rmcp::{
    handler::server::ServerHandler,
    model::{
        // Resources
        AnnotateAble,
        CallToolRequestParams,
        CallToolResult,
        Content,
        GetPromptRequestParams,
        GetPromptResult,
        Implementation,
        // Prompts
        ListPromptsResult,
        ListResourceTemplatesResult,
        ListResourcesResult,
        ListToolsResult,
        PaginatedRequestParams,
        Prompt,
        PromptArgument,
        PromptMessage,
        PromptMessageContent,
        PromptMessageRole,
        RawResource,
        RawResourceTemplate,
        ReadResourceRequestParams,
        ReadResourceResult,
        Resource,
        ResourceContents,
        ServerCapabilities,
        ServerInfo,
        Tool,
    },
    service::{RequestContext, RoleServer},
    transport::streamable_http_server::{
        session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
    },
    ErrorData as McpError,
};

// ═══════════════════════════════════════════════════════════════════════════════
//  Config
// ═══════════════════════════════════════════════════════════════════════════════

#[derive(Parser)]
#[command(name = "mmux")]
#[command(
    about = "Tmux remote control over MCP — operate terminals and coding harnesses with AI agents"
)]
struct Cli {
    #[arg(long, help = "Path to node profile TOML file")]
    node_config: Option<String>,
    #[arg(long, hide = true, help = "Deprecated alias for --node-config")]
    config: Option<String>,
    #[arg(
        long,
        default_value = "127.0.0.1",
        help = "Host to bind the MCP HTTP server"
    )]
    host: String,
    #[arg(
        long,
        default_value_t = 3000,
        help = "Port to bind the MCP HTTP server"
    )]
    port: u16,
    #[arg(
        long,
        help = "Bearer token for API authentication. If set, all requests must include 'Authorization: Bearer <token>'."
    )]
    token: Option<String>,
    #[arg(
        long,
        help = "Path to a file containing the bearer token. Prefer /run/secrets paths in containers."
    )]
    token_file: Option<String>,
    #[arg(
        long,
        default_value = "MMUX_TOKEN",
        help = "Environment variable to read the bearer token from when --token/--token-file are not set."
    )]
    token_env: String,
    #[arg(long, value_enum, default_value_t = SecurityMode::Local, help = "Security mode: open, local, workspace, attached, or readonly.")]
    security_mode: SecurityMode,
    #[arg(
        long,
        help = "Workspace root used to confine path-based APIs in workspace mode."
    )]
    workspace_root: Option<String>,
    #[arg(
        long,
        help = "Permit a non-loopback unauthenticated bind. Intended only behind localhost-only port forwarding."
    )]
    allow_remote_without_token: bool,
    #[arg(long, default_value_t = 4 * 1024 * 1024, help = "Maximum bytes returned by read_file.")]
    max_read_bytes: usize,
    #[arg(long, default_value_t = 4 * 1024 * 1024, help = "Maximum decoded bytes accepted by save_file.")]
    max_write_bytes: usize,
    #[arg(
        long,
        default_value_t = 60.0,
        help = "Maximum wait timeout accepted by wait tools."
    )]
    max_timeout_seconds: f64,
    #[arg(long, default_value_t = 2 * 1024 * 1024, help = "Maximum MCP HTTP request body size.")]
    max_request_bytes: usize,
    #[arg(long, default_value_t = 2 * 1024 * 1024, help = "Maximum bytes returned by terminal capture tools.")]
    max_capture_bytes: usize,
    #[arg(
        long,
        help = "Start the built-in local tmux node inside the controller process."
    )]
    enable_local_node: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum SecurityMode {
    /// Current trust-the-client behavior. Intended only for explicitly trusted deployments.
    Open,
    /// Full functionality for loopback use; remote binds require authentication.
    Local,
    /// Full terminal control, with path-based APIs confined to --workspace-root.
    Workspace,
    /// Drive existing sessions only. No process launch, file APIs, or session killing.
    Attached,
    /// Inspection only. No input, writes, process launch, or mutable profile changes.
    Readonly,
}

#[derive(Clone, Debug)]
struct SecurityPolicy {
    mode: SecurityMode,
    workspace_root: Option<PathBuf>,
    max_read_bytes: usize,
    max_write_bytes: usize,
    max_timeout_seconds: f64,
    max_request_bytes: usize,
    max_capture_bytes: usize,
}

impl SecurityPolicy {
    fn new(cli: &Cli) -> Result<Self, String> {
        let workspace_root =
            if cli.security_mode == SecurityMode::Workspace {
                let root = cli
                    .workspace_root
                    .as_deref()
                    .ok_or("--workspace-root is required in workspace mode")?;
                Some(std::fs::canonicalize(root).map_err(|e| {
                    format!("failed to canonicalize workspace root '{}': {}", root, e)
                })?)
            } else {
                match cli.workspace_root.as_deref() {
                    Some(root) => Some(std::fs::canonicalize(root).map_err(|e| {
                        format!("failed to canonicalize workspace root '{}': {}", root, e)
                    })?),
                    None => None,
                }
            };

        Ok(Self {
            mode: cli.security_mode,
            workspace_root,
            max_read_bytes: cli.max_read_bytes,
            max_write_bytes: cli.max_write_bytes,
            max_timeout_seconds: cli.max_timeout_seconds,
            max_request_bytes: cli.max_request_bytes,
            max_capture_bytes: cli.max_capture_bytes,
        })
    }

    fn deny(&self, action: &str) -> String {
        format!("Denied by {:?} security mode: {}", self.mode, action)
    }

    fn can_create_session(&self) -> bool {
        matches!(
            self.mode,
            SecurityMode::Open | SecurityMode::Local | SecurityMode::Workspace
        )
    }

    fn can_kill_session(&self) -> bool {
        matches!(
            self.mode,
            SecurityMode::Open | SecurityMode::Local | SecurityMode::Workspace
        )
    }

    fn can_send_input(&self) -> bool {
        !matches!(self.mode, SecurityMode::Readonly)
    }

    fn can_exec(&self) -> bool {
        matches!(
            self.mode,
            SecurityMode::Open | SecurityMode::Local | SecurityMode::Workspace
        )
    }

    fn can_read_files(&self) -> bool {
        matches!(
            self.mode,
            SecurityMode::Open | SecurityMode::Local | SecurityMode::Workspace
        )
    }

    fn can_write_files(&self) -> bool {
        matches!(
            self.mode,
            SecurityMode::Open | SecurityMode::Local | SecurityMode::Workspace
        )
    }

    fn can_mutate_profiles(&self) -> bool {
        !matches!(self.mode, SecurityMode::Readonly)
    }

    fn can_load_profile_from_path(&self) -> bool {
        matches!(
            self.mode,
            SecurityMode::Open | SecurityMode::Local | SecurityMode::Workspace
        )
    }

    fn can_resize(&self) -> bool {
        !matches!(self.mode, SecurityMode::Readonly)
    }

    fn clamp_timeout(&self, requested: f64) -> Result<f64, String> {
        if !requested.is_finite() || requested <= 0.0 {
            return Err("timeout_seconds must be a positive finite number".into());
        }
        Ok(requested.min(self.max_timeout_seconds))
    }

    fn limit_capture_output(&self, mut output: String) -> String {
        if output.len() <= self.max_capture_bytes {
            return output;
        }
        let mut keep_from = output.len().saturating_sub(self.max_capture_bytes);
        while keep_from < output.len() && !output.is_char_boundary(keep_from) {
            keep_from += 1;
        }
        let suffix = output.split_off(keep_from);
        format!(
            "[mmux truncated capture to last {} bytes]\n{}",
            self.max_capture_bytes, suffix
        )
    }

    fn resolve_read_path(&self, user_path: &str) -> Result<PathBuf, String> {
        match self.workspace_root.as_ref() {
            Some(root) if self.mode == SecurityMode::Workspace => {
                let candidate = self.workspace_candidate(root, user_path)?;
                let real = std::fs::canonicalize(&candidate).map_err(|e| {
                    format!("failed to canonicalize '{}': {}", candidate.display(), e)
                })?;
                if !real.starts_with(root) {
                    return Err(format!("path escapes workspace root: {}", user_path));
                }
                Ok(real)
            }
            _ => Ok(PathBuf::from(user_path)),
        }
    }

    fn resolve_write_path(&self, user_path: &str) -> Result<PathBuf, String> {
        match self.workspace_root.as_ref() {
            Some(root) if self.mode == SecurityMode::Workspace => {
                let candidate = self.workspace_candidate(root, user_path)?;
                let parent = candidate
                    .parent()
                    .ok_or_else(|| format!("path has no parent: {}", user_path))?;
                let existing_ancestor = nearest_existing_ancestor(parent);
                let real_ancestor = std::fs::canonicalize(&existing_ancestor).map_err(|e| {
                    format!(
                        "failed to canonicalize ancestor '{}': {}",
                        existing_ancestor.display(),
                        e
                    )
                })?;
                if !real_ancestor.starts_with(root) {
                    return Err(format!(
                        "path ancestor escapes workspace root: {}",
                        user_path
                    ));
                }
                std::fs::create_dir_all(parent).map_err(|e| {
                    format!("failed to create parent '{}': {}", parent.display(), e)
                })?;

                if candidate.exists() {
                    let real = std::fs::canonicalize(&candidate).map_err(|e| {
                        format!("failed to canonicalize '{}': {}", candidate.display(), e)
                    })?;
                    if !real.starts_with(root) {
                        return Err(format!("path escapes workspace root: {}", user_path));
                    }
                    return Ok(real);
                }

                let real_parent = std::fs::canonicalize(parent).map_err(|e| {
                    format!(
                        "failed to canonicalize parent '{}': {}",
                        parent.display(),
                        e
                    )
                })?;
                if !real_parent.starts_with(root) {
                    return Err(format!("path parent escapes workspace root: {}", user_path));
                }
                let file_name = candidate
                    .file_name()
                    .ok_or_else(|| format!("path has no file name: {}", user_path))?;
                Ok(real_parent.join(file_name))
            }
            _ => Ok(PathBuf::from(user_path)),
        }
    }

    fn workspace_candidate(&self, root: &Path, user_path: &str) -> Result<PathBuf, String> {
        let path = Path::new(user_path);
        if path.components().any(|c| matches!(c, Component::ParentDir)) {
            return Err(format!(
                "parent directory components are not allowed: {}",
                user_path
            ));
        }
        let candidate = if path.is_absolute() {
            path.to_path_buf()
        } else {
            root.join(path)
        };
        if !candidate.starts_with(root) {
            return Err(format!("path is outside workspace root: {}", user_path));
        }
        Ok(candidate)
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Tmux operations
// ═══════════════════════════════════════════════════════════════════════════════

fn tmux(args: &[&str]) -> Result<String, String> {
    mmux_node::tmux(args)
}

fn session_exists(session: &str) -> bool {
    mmux_node::session_exists(session)
}

async fn session_create(session: &str, command: &str, cwd: Option<&str>) -> Result<String, String> {
    if session_exists(session) {
        return Ok(format!("Session '{}' already exists", session));
    }
    let mut args = vec!["new-session", "-d", "-s", session];
    if let Some(dir) = cwd {
        args.push("-c");
        args.push(dir);
    }
    args.push(command);
    tmux(&args)?;
    tokio::time::sleep(Duration::from_millis(300)).await;
    Ok(format!(
        "Created session '{}' with command '{}'",
        session, command
    ))
}

fn session_kill(session: &str) -> Result<String, String> {
    if !session_exists(session) {
        return Ok(format!("Session '{}' not found", session));
    }
    tmux(&["kill-session", "-t", session])?;
    Ok(format!("Killed session '{}'", session))
}

fn session_list() -> Result<String, String> {
    match tmux(&[
        "list-sessions",
        "-F",
        "#{session_name}: #{session_windows} windows (#{session_attached} attached)",
    ]) {
        Ok(output) => Ok(output),
        Err(_) => Ok("No tmux sessions running".into()),
    }
}

async fn session_send(session: &str, text: &str, enter: bool) -> Result<String, String> {
    if !session_exists(session) {
        return Err(format!("Session '{}' does not exist", session));
    }
    tmux(&["send-keys", "-l", "-t", session, text])?;
    if enter {
        tokio::time::sleep(Duration::from_millis(200)).await;
        tmux(&["send-keys", "-t", session, "Enter"])?;
    }
    Ok(format!("Sent to {}: {}", session, text))
}

fn session_key(session: &str, key: &str) -> Result<String, String> {
    if !session_exists(session) {
        return Err(format!("Session '{}' does not exist", session));
    }
    tmux(&["send-keys", "-t", session, key])?;
    Ok(format!("Sent key '{}' to {}", key, session))
}

fn session_capture(
    session: &str,
    lines: Option<usize>,
    scrollback: bool,
) -> Result<String, String> {
    if !session_exists(session) {
        return Err(format!("Session '{}' does not exist", session));
    }
    if scrollback {
        tmux(&["capture-pane", "-t", session, "-p", "-S", "-"])
    } else if let Some(n) = lines {
        let s = format!("-{}", n);
        tmux(&["capture-pane", "-t", session, "-p", "-S", &s])
    } else {
        tmux(&["capture-pane", "-t", session, "-p"])
    }
}

async fn wait_for(
    session: &str,
    mode: &str,
    sentinel: Option<&str>,
    prompt: Option<&str>,
    timeout: f64,
    poll: f64,
    stability: f64,
) -> Result<String, String> {
    if !session_exists(session) {
        return Err(format!("Session '{}' does not exist", session));
    }
    let deadline = Instant::now() + Duration::from_secs_f64(timeout);
    let poll_dur = Duration::from_secs_f64(poll);

    match mode {
        "sentinel" => {
            let s = sentinel.ok_or("sentinel required for sentinel mode")?;
            while Instant::now() < deadline {
                let output = session_capture(session, Some(200), false)?;
                if output.contains(s) {
                    return Ok(format!("Sentinel '{}' found", s));
                }
                tokio::time::sleep(poll_dur).await;
            }
            Err(format!(
                "Timeout after {}s waiting for sentinel '{}'",
                timeout, s
            ))
        }
        "prompt" => {
            let p = prompt.ok_or("prompt required for prompt mode")?;
            while Instant::now() < deadline {
                let output = session_capture(session, Some(200), false)?;
                if output.contains(p) {
                    return Ok(format!("Prompt '{}' found", p));
                }
                tokio::time::sleep(poll_dur).await;
            }
            Err(format!(
                "Timeout after {}s waiting for prompt '{}'",
                timeout, p
            ))
        }
        _ => {
            // stable mode (default)
            let stable_needed = (stability / poll).max(1.0) as usize;
            let mut last_output = String::new();
            let mut stable_count = 0;
            while Instant::now() < deadline {
                let output = session_capture(session, Some(200), false)?;
                if output == last_output {
                    stable_count += 1;
                    if stable_count >= stable_needed {
                        return Ok(format!("Output stable for {}s", stability));
                    }
                } else {
                    stable_count = 0;
                    last_output = output;
                }
                tokio::time::sleep(poll_dur).await;
            }
            Err(format!("Timeout after {}s", timeout))
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  File operations (read / save)
// ═══════════════════════════════════════════════════════════════════════════════

#[cfg(test)]
fn detect_compression(bytes: &[u8]) -> Option<String> {
    mmux_node::detect_compression(bytes)
}

#[cfg(test)]
fn detect_mime_type(path: &Path, bytes: &[u8]) -> String {
    mmux_node::detect_mime_type(path, bytes)
}

fn read_file_impl(
    path: &str,
    offset: Option<u64>,
    limit: Option<usize>,
) -> Result<ReadFileResult, String> {
    mmux_node::read_file_impl(path, offset, limit)
}

fn save_file_impl(
    path: &str,
    content: &str,
    encoding: &str,
    append: bool,
    max_bytes: Option<usize>,
) -> Result<SaveFileResult, String> {
    mmux_node::save_file_impl(path, content, encoding, append, max_bytes)
}

// ═══════════════════════════════════════════════════════════════════════════════
//  CLI Profiles — extensible behavior for different terminal applications
// ═══════════════════════════════════════════════════════════════════════════════

fn load_profile_from_toml(text: &str) -> Result<CliProfile, String> {
    mmux_node::load_profile_from_toml(text)
}

async fn session_exec(
    session: &str,
    command: &str,
    cwd: Option<&str>,
    timeout: f64,
    max_lines: usize,
) -> Result<String, String> {
    if !session_exists(session) {
        session_create(session, "bash", cwd).await?;
    }
    // Use a sentinel to isolate this command's output from scrollback history.
    let sentinel = format!(
        "__MMUX_{}__",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
    session_send(session, &format!("echo '{}'", sentinel), true).await?;
    tokio::time::sleep(Duration::from_millis(200)).await;
    session_send(session, command, true).await?;
    wait_for(session, "stable", None, None, timeout, 0.5, 1.0).await?;
    let output = session_capture(session, None, true)?; // full scrollback
    let all_lines: Vec<&str> = output.lines().collect();
    // Find the last occurrence of the sentinel output line
    let mut sentinel_idx = None;
    for (i, line) in all_lines.iter().enumerate() {
        if line.trim() == sentinel {
            sentinel_idx = Some(i);
        }
    }
    let result_lines: Vec<&str> = if let Some(idx) = sentinel_idx {
        all_lines.iter().skip(idx + 1).copied().collect()
    } else {
        let start = all_lines.len().saturating_sub(max_lines);
        all_lines[start..].to_vec()
    };
    Ok(clean_exec_output(result_lines))
}

fn clean_exec_output(lines: Vec<&str>) -> String {
    let mut result = lines;
    // Skip the command line (first non-empty line after sentinel is the shell command)
    let mut i = 0;
    while i < result.len() && result[i].trim().is_empty() {
        i += 1;
    }
    if i < result.len() {
        i += 1; // skip the command line
    }
    result = result[i..].to_vec();
    // Trim trailing empty lines and prompt-like lines
    let trimmed: Vec<&str> = result
        .into_iter()
        .rev()
        .skip_while(|l| {
            let t = l.trim();
            t.is_empty() || t.ends_with('$') || t.ends_with('#') || t.ends_with('>')
        })
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    trimmed.join("\n")
}

async fn coding_send_with_profile(
    session: &str,
    prompt: &str,
    profile: &CliProfile,
) -> Result<String, String> {
    if !session_exists(session) {
        return Err(format!("Session '{}' does not exist", session));
    }
    let pane = session_first_pane(session)?;

    // Dismiss startup noise if configured
    if let Some(ref dismiss) = profile.startup_dismiss {
        let buf = tmux(&["capture-pane", "-t", &pane, "-p"]).unwrap_or_default();
        if dismiss.triggers.iter().any(|t| buf.contains(t)) {
            tmux(&["send-keys", "-t", &pane, &dismiss.key]).map_err(|e| e.to_string())?;
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
    }

    tmux(&["send-keys", "-l", "-t", &pane, prompt]).map_err(|e| e.to_string())?;
    tokio::time::sleep(Duration::from_millis(200)).await;
    tmux(&["send-keys", "-t", &pane, "Enter"]).map_err(|e| e.to_string())?;
    Ok(format!(
        "Sent to {} (profile: {}): {}",
        session, profile.name, prompt
    ))
}

async fn coding_wait_ready_with_profile(
    session: &str,
    timeout: u64,
    profile: &CliProfile,
) -> Result<String, String> {
    if !session_exists(session) {
        return Err(format!("Session '{}' does not exist", session));
    }
    let pane = session_first_pane(session)?;
    let deadline = Instant::now() + Duration::from_secs(timeout);
    loop {
        let buf = tmux(&["capture-pane", "-t", &pane, "-p"]).unwrap_or_default();
        let has_prompt = buf.contains(&profile.prompt_indicator);
        let busy = profile
            .busy_indicators
            .iter()
            .any(|marker| buf.contains(marker));
        if has_prompt && !busy {
            return Ok(format!("{} is ready (profile: {})", session, profile.name));
        }
        if Instant::now() > deadline {
            return Err(format!(
                "Timeout waiting for {} to be ready (profile: {})",
                session, profile.name
            ));
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

fn coding_action_with_profile(
    session: &str,
    action: &str,
    profile: &CliProfile,
) -> Result<String, String> {
    if !session_exists(session) {
        return Err(format!("Session '{}' does not exist", session));
    }
    let pane = session_first_pane(session)?;
    let keys = match action {
        "approve" => &profile.approve_keys,
        "reject" => &profile.reject_keys,
        "cancel" => &profile.cancel_keys,
        "escape" | "dismiss" => &profile.escape_keys,
        other => return Err(format!("Unknown action: {}", other)),
    };
    tmux(&["send-keys", "-t", &pane, keys])?;
    Ok(format!(
        "Sent action '{}' to {} (profile: {})",
        action, session, profile.name
    ))
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Extended tmux operations for agents
// ═══════════════════════════════════════════════════════════════════════════════

fn session_info(session: &str) -> Result<String, String> {
    if !session_exists(session) {
        return Err(format!("Session '{}' does not exist", session));
    }
    let info = tmux(&[
        "list-panes",
        "-t", session,
        "-F", "pane_id=#{pane_id} index=#{pane_index} width=#{pane_width} height=#{pane_height} command=#{pane_current_command} title=#{pane_title}",
    ])?;
    let windows = tmux(&[
        "list-windows",
        "-t",
        session,
        "-F",
        "window_id=#{window_id} index=#{window_index} name=#{window_name} active=#{window_active}",
    ])?;
    Ok(format!(
        "Session: {}\nPanes:\n{}\nWindows:\n{}",
        session, info, windows
    ))
}

fn list_panes(session: &str) -> Result<String, String> {
    if !session_exists(session) {
        return Err(format!("Session '{}' does not exist", session));
    }
    tmux(&[
        "list-panes",
        "-t",
        session,
        "-F",
        "#{pane_index}\t#{pane_width}x#{pane_height}\t#{pane_current_command}\t#{pane_title}",
    ])
}

fn check_state(session: &str, profile: &CliProfile) -> Result<String, String> {
    if !session_exists(session) {
        return Err(format!("Session '{}' does not exist", session));
    }
    let pane = session_first_pane(session)?;
    let buf = tmux(&["capture-pane", "-t", &pane, "-p"]).unwrap_or_default();
    let has_prompt = buf.contains(&profile.prompt_indicator);
    let busy = profile.busy_indicators.iter().any(|m| buf.contains(m));
    Ok(format!(
        "{{\"session\":\"{}\",\"has_prompt\":{},\"busy\":{},\"profile\":\"{}\"}}",
        session, has_prompt, busy, profile.name
    ))
}

fn resize_pane(session: &str, width: Option<u32>, height: Option<u32>) -> Result<String, String> {
    if !session_exists(session) {
        return Err(format!("Session '{}' does not exist", session));
    }
    let pane = session_first_pane(session)?;
    if let Some(w) = width {
        tmux(&["resize-pane", "-t", &pane, "-x", &w.to_string()])?;
    }
    if let Some(h) = height {
        tmux(&["resize-pane", "-t", &pane, "-y", &h.to_string()])?;
    }
    Ok(format!("Resized pane {}", pane))
}

fn session_first_pane(session: &str) -> Result<String, String> {
    let panes = tmux(&["list-panes", "-t", session, "-F", "#{pane_id}"])?;
    panes
        .lines()
        .find(|line| !line.trim().is_empty())
        .map(|line| line.trim().to_string())
        .ok_or_else(|| format!("Session '{}' has no panes", session))
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Registered node actor
// ═══════════════════════════════════════════════════════════════════════════════

struct NodeRegistryActor;

struct RegisteredNode {
    descriptor: NodeDescriptor,
    status: NodeStatus,
    last_seen: Instant,
}

struct NodeRegistryState {
    nodes: HashMap<String, RegisteredNode>,
    queues: HashMap<String, VecDeque<NodeCommand>>,
    pending: HashMap<String, RpcReplyPort<Result<NodeCommandResult, String>>>,
    next_command_id: u64,
    local_enabled: bool,
}

enum NodeRegistryMessage {
    Register {
        descriptor: NodeDescriptor,
        reply: RpcReplyPort<Result<String, String>>,
    },
    Heartbeat {
        node_id: String,
        status: NodeStatus,
        reply: RpcReplyPort<Result<(), String>>,
    },
    Pull {
        node_id: String,
        reply: RpcReplyPort<Result<Vec<NodeCommand>, String>>,
    },
    SubmitResult {
        node_id: String,
        command_id: String,
        result: NodeCommandResult,
        reply: RpcReplyPort<Result<(), String>>,
    },
    Dispatch {
        node_id: String,
        kind: NodeCommandKind,
        reply: RpcReplyPort<Result<NodeCommandResult, String>>,
    },
    ListNodes {
        reply: RpcReplyPort<Result<String, String>>,
    },
    NodeInfo {
        node_id: String,
        reply: RpcReplyPort<Result<String, String>>,
    },
}

impl Actor for NodeRegistryActor {
    type Msg = NodeRegistryMessage;
    type State = NodeRegistryState;
    type Arguments = bool;

    async fn pre_start(
        &self,
        _myself: ActorRef<Self::Msg>,
        args: Self::Arguments,
    ) -> Result<Self::State, ActorProcessingErr> {
        Ok(NodeRegistryState {
            nodes: HashMap::new(),
            queues: HashMap::new(),
            pending: HashMap::new(),
            next_command_id: 1,
            local_enabled: args,
        })
    }

    async fn handle(
        &self,
        _myself: ActorRef<Self::Msg>,
        message: Self::Msg,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        match message {
            NodeRegistryMessage::Register { descriptor, reply } => {
                if descriptor.node_id.trim().is_empty() {
                    let _ = reply.send(Err("node_id must not be empty".into()));
                    return Ok(());
                }
                if descriptor.node_id == "local" {
                    let _ = reply.send(Err("'local' is reserved for the built-in node".into()));
                    return Ok(());
                }
                let node_id = descriptor.node_id.clone();
                state.nodes.insert(
                    node_id.clone(),
                    RegisteredNode {
                        descriptor,
                        status: NodeStatus::Ready,
                        last_seen: Instant::now(),
                    },
                );
                state.queues.entry(node_id.clone()).or_default();
                let _ = reply.send(Ok(format!("registered node '{}'", node_id)));
            }
            NodeRegistryMessage::Heartbeat {
                node_id,
                status,
                reply,
            } => match state.nodes.get_mut(&node_id) {
                Some(node) => {
                    node.status = status;
                    node.last_seen = Instant::now();
                    let _ = reply.send(Ok(()));
                }
                None => {
                    let _ = reply.send(Err(format!("node '{}' is not registered", node_id)));
                }
            },
            NodeRegistryMessage::Pull { node_id, reply } => {
                if !state.nodes.contains_key(&node_id) {
                    let _ = reply.send(Err(format!("node '{}' is not registered", node_id)));
                    return Ok(());
                }
                if let Some(node) = state.nodes.get_mut(&node_id) {
                    node.last_seen = Instant::now();
                }
                let commands = state.queues.entry(node_id).or_default().drain(..).collect();
                let _ = reply.send(Ok(commands));
            }
            NodeRegistryMessage::SubmitResult {
                node_id,
                command_id,
                result,
                reply,
            } => {
                if let Some(node) = state.nodes.get_mut(&node_id) {
                    node.last_seen = Instant::now();
                }
                if let Some(waiter) = state.pending.remove(&command_id) {
                    let _ = waiter.send(Ok(result));
                    let _ = reply.send(Ok(()));
                } else {
                    let _ = reply.send(Err(format!("command '{}' is not pending", command_id)));
                }
            }
            NodeRegistryMessage::Dispatch {
                node_id,
                kind,
                reply,
            } => {
                if !state.nodes.contains_key(&node_id) {
                    let _ = reply.send(Err(format!("node '{}' is not registered", node_id)));
                    return Ok(());
                }
                let command_id = format!("cmd-{}", state.next_command_id);
                state.next_command_id += 1;
                state.pending.insert(command_id.clone(), reply);
                state
                    .queues
                    .entry(node_id)
                    .or_default()
                    .push_back(NodeCommand { command_id, kind });
            }
            NodeRegistryMessage::ListNodes { reply } => {
                let mut nodes = Vec::new();
                if state.local_enabled {
                    nodes.push(json!({
                        "node_id": "local",
                        "display_name": "Local tmux node",
                        "status": "ready",
                        "last_seen_ms_ago": 0
                    }));
                }
                for node in state.nodes.values() {
                    nodes.push(json!({
                        "node_id": node.descriptor.node_id,
                        "display_name": node.descriptor.display_name,
                        "status": format!("{:?}", node.status),
                        "last_seen_ms_ago": node.last_seen.elapsed().as_millis()
                    }));
                }
                let text = serde_json::to_string_pretty(&nodes)
                    .unwrap_or_else(|error| format!("{{\"error\":\"{}\"}}", error));
                let _ = reply.send(Ok(text));
            }
            NodeRegistryMessage::NodeInfo { node_id, reply } => {
                if node_id == "local" {
                    let text = serde_json::to_string_pretty(&json!({
                        "node_id": "local",
                        "display_name": "Local tmux node",
                        "status": if state.local_enabled { "ready" } else { "disabled" },
                        "last_seen_ms_ago": 0,
                    }))
                    .unwrap_or_else(|error| format!("{{\"error\":\"{}\"}}", error));
                    let _ = reply.send(Ok(text));
                    return Ok(());
                }
                match state.nodes.get(&node_id) {
                    Some(node) => {
                        let text = serde_json::to_string_pretty(&json!({
                            "node_id": node.descriptor.node_id,
                            "display_name": node.descriptor.display_name,
                            "status": format!("{:?}", node.status),
                            "last_seen_ms_ago": node.last_seen.elapsed().as_millis()
                        }))
                        .unwrap_or_else(|error| format!("{{\"error\":\"{}\"}}", error));
                        let _ = reply.send(Ok(text));
                    }
                    None => {
                        let _ = reply.send(Err(format!("Node '{}' not found", node_id)));
                    }
                }
            }
        }
        Ok(())
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Local node actor
// ═══════════════════════════════════════════════════════════════════════════════

struct LocalNodeActor;

enum LocalNodeMessage {
    CreateSession {
        session: String,
        command: String,
        cwd: Option<String>,
        reply: RpcReplyPort<Result<String, String>>,
    },
    KillSession {
        session: String,
        reply: RpcReplyPort<Result<String, String>>,
    },
    ListSessions {
        reply: RpcReplyPort<Result<String, String>>,
    },
    SendInput {
        session: String,
        text: String,
        enter: bool,
        reply: RpcReplyPort<Result<String, String>>,
    },
    SendKey {
        session: String,
        key: String,
        reply: RpcReplyPort<Result<String, String>>,
    },
    CaptureOutput {
        session: String,
        lines: Option<usize>,
        scrollback: bool,
        reply: RpcReplyPort<Result<String, String>>,
    },
    WaitFor {
        session: String,
        mode: String,
        sentinel: Option<String>,
        prompt: Option<String>,
        timeout: f64,
        poll: f64,
        stability: f64,
        reply: RpcReplyPort<Result<String, String>>,
    },
    Exec {
        session: String,
        command: String,
        cwd: Option<String>,
        timeout: f64,
        max_lines: usize,
        reply: RpcReplyPort<Result<String, String>>,
    },
    ReadFile {
        path: String,
        offset: Option<u64>,
        limit: Option<usize>,
        reply: RpcReplyPort<Result<ReadFileResult, String>>,
    },
    SaveFile {
        path: String,
        content: String,
        encoding: String,
        append: bool,
        max_bytes: Option<usize>,
        reply: RpcReplyPort<Result<SaveFileResult, String>>,
    },
    CodingSend {
        session: String,
        prompt: String,
        profile: CliProfile,
        reply: RpcReplyPort<Result<String, String>>,
    },
    CodingWaitReady {
        session: String,
        timeout: u64,
        profile: CliProfile,
        reply: RpcReplyPort<Result<String, String>>,
    },
    CodingAction {
        session: String,
        action: String,
        profile: CliProfile,
        reply: RpcReplyPort<Result<String, String>>,
    },
    SessionInfo {
        session: String,
        reply: RpcReplyPort<Result<String, String>>,
    },
    ListPanes {
        session: String,
        reply: RpcReplyPort<Result<String, String>>,
    },
    CheckState {
        session: String,
        profile: CliProfile,
        reply: RpcReplyPort<Result<String, String>>,
    },
    ResizePane {
        session: String,
        width: Option<u32>,
        height: Option<u32>,
        reply: RpcReplyPort<Result<String, String>>,
    },
}

impl Actor for LocalNodeActor {
    type Msg = LocalNodeMessage;
    type State = ();
    type Arguments = ();

    async fn pre_start(
        &self,
        _myself: ActorRef<Self::Msg>,
        _args: Self::Arguments,
    ) -> Result<Self::State, ActorProcessingErr> {
        Ok(())
    }

    async fn handle(
        &self,
        _myself: ActorRef<Self::Msg>,
        message: Self::Msg,
        _state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        match message {
            LocalNodeMessage::CreateSession {
                session,
                command,
                cwd,
                reply,
            } => {
                let _ = reply.send(session_create(&session, &command, cwd.as_deref()).await);
            }
            LocalNodeMessage::KillSession { session, reply } => {
                let _ = reply.send(session_kill(&session));
            }
            LocalNodeMessage::ListSessions { reply } => {
                let _ = reply.send(session_list());
            }
            LocalNodeMessage::SendInput {
                session,
                text,
                enter,
                reply,
            } => {
                let _ = reply.send(session_send(&session, &text, enter).await);
            }
            LocalNodeMessage::SendKey {
                session,
                key,
                reply,
            } => {
                let _ = reply.send(session_key(&session, &key));
            }
            LocalNodeMessage::CaptureOutput {
                session,
                lines,
                scrollback,
                reply,
            } => {
                let _ = reply.send(session_capture(&session, lines, scrollback));
            }
            LocalNodeMessage::WaitFor {
                session,
                mode,
                sentinel,
                prompt,
                timeout,
                poll,
                stability,
                reply,
            } => {
                let _ = reply.send(
                    wait_for(
                        &session,
                        &mode,
                        sentinel.as_deref(),
                        prompt.as_deref(),
                        timeout,
                        poll,
                        stability,
                    )
                    .await,
                );
            }
            LocalNodeMessage::Exec {
                session,
                command,
                cwd,
                timeout,
                max_lines,
                reply,
            } => {
                let _ = reply.send(
                    session_exec(&session, &command, cwd.as_deref(), timeout, max_lines).await,
                );
            }
            LocalNodeMessage::ReadFile {
                path,
                offset,
                limit,
                reply,
            } => {
                let _ = reply.send(read_file_impl(&path, offset, limit));
            }
            LocalNodeMessage::SaveFile {
                path,
                content,
                encoding,
                append,
                max_bytes,
                reply,
            } => {
                let _ = reply.send(save_file_impl(
                    &path, &content, &encoding, append, max_bytes,
                ));
            }
            LocalNodeMessage::CodingSend {
                session,
                prompt,
                profile,
                reply,
            } => {
                let _ = reply.send(coding_send_with_profile(&session, &prompt, &profile).await);
            }
            LocalNodeMessage::CodingWaitReady {
                session,
                timeout,
                profile,
                reply,
            } => {
                let _ =
                    reply.send(coding_wait_ready_with_profile(&session, timeout, &profile).await);
            }
            LocalNodeMessage::CodingAction {
                session,
                action,
                profile,
                reply,
            } => {
                let _ = reply.send(coding_action_with_profile(&session, &action, &profile));
            }
            LocalNodeMessage::SessionInfo { session, reply } => {
                let _ = reply.send(session_info(&session));
            }
            LocalNodeMessage::ListPanes { session, reply } => {
                let _ = reply.send(list_panes(&session));
            }
            LocalNodeMessage::CheckState {
                session,
                profile,
                reply,
            } => {
                let _ = reply.send(check_state(&session, &profile));
            }
            LocalNodeMessage::ResizePane {
                session,
                width,
                height,
                reply,
            } => {
                let _ = reply.send(resize_pane(&session, width, height));
            }
        }
        Ok(())
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  MCP HTTP Server Mode (rmcp)
// ═══════════════════════════════════════════════════════════════════════════════

#[derive(Clone)]
struct TmuxMcpServer {
    profiles: ProfileRegistry,
    policy: SecurityPolicy,
    local_node: Option<ActorRef<LocalNodeMessage>>,
    registry: ActorRef<NodeRegistryMessage>,
}

impl TmuxMcpServer {
    fn new(
        profiles: ProfileRegistry,
        policy: SecurityPolicy,
        local_node: Option<ActorRef<LocalNodeMessage>>,
        registry: ActorRef<NodeRegistryMessage>,
    ) -> Self {
        Self {
            profiles,
            policy,
            local_node,
            registry,
        }
    }

    fn text_result(text: impl Into<String>) -> CallToolResult {
        CallToolResult::success(vec![Content::text(text)])
    }

    fn error_result(text: impl Into<String>) -> CallToolResult {
        CallToolResult::error(vec![Content::text(text)])
    }

    fn resolve_profile(&self, name: Option<&str>) -> Option<CliProfile> {
        let name = name.unwrap_or("opencode");
        mmux_node::get_profile(&self.profiles, name)
    }

    fn resolve_launch_secret_value(value_from: &str) -> Result<String, String> {
        let env_name = value_from
            .strip_prefix("host.")
            .ok_or_else(|| {
                format!(
                    "unsupported launch secret source '{}': expected host.ENV_VAR",
                    value_from
                )
            })?;
        std::env::var(env_name)
            .map_err(|error| format!("missing host env var {} for launch secret: {}", env_name, error))
    }

    fn profile_launch_envs(profile: &CliProfile) -> Result<Vec<(String, String)>, String> {
        let mut envs = Vec::new();
        if let Some(launch) = profile.launch.as_ref() {
            for (key, value) in &launch.env {
                envs.push((key.clone(), value.clone()));
            }
            for secret in &launch.secrets {
                let value = Self::resolve_launch_secret_value(&secret.value_from)?;
                envs.push((secret.env.clone(), value));
            }
        }
        Ok(envs)
    }

    async fn apply_launch_envs(
        &self,
        node: &str,
        envs: &[(String, String)],
    ) -> Result<(), String> {
        for (key, value) in envs {
            if node != "local" {
                self.remote_tmux(
                    node,
                    vec![
                        "set-environment".into(),
                        "-g".into(),
                        key.clone(),
                        value.clone(),
                    ],
                    Duration::from_secs(20),
                )
                .await?;
            } else {
                tmux(&["set-environment", "-g", key, value])?;
            }
        }
        Ok(())
    }

    async fn create_session_with_command(
        &self,
        node: &str,
        session: &str,
        cmd: &str,
        cwd: Option<&str>,
    ) -> Result<String, String> {
        if node != "local" {
            if self.node_session_exists(node, session).await {
                return Ok(format!(
                    "Session '{}' already exists on node '{}'",
                    session, node
                ));
            }
            let mut tmux_args = vec![
                "new-session".into(),
                "-d".into(),
                "-s".into(),
                session.into(),
            ];
            if let Some(cwd) = cwd {
                tmux_args.push("-c".into());
                tmux_args.push(cwd.into());
            }
            tmux_args.push(cmd.into());
            return match self
                .remote_tmux(node, tmux_args, Duration::from_secs(30))
                .await
            {
                Ok(_) => {
                    tokio::time::sleep(Duration::from_millis(300)).await;
                    Ok(format!(
                        "Created session '{}' with command '{}' on node '{}'",
                        session, cmd, node
                    ))
                }
                Err(e) => Err(e),
            };
        }

        self.local_node_call(|reply| LocalNodeMessage::CreateSession {
            session: session.to_owned(),
            command: cmd.to_owned(),
            cwd: cwd.map(|value| value.to_owned()),
            reply,
        })
        .await
    }

    async fn wait_coding_session_ready(
        &self,
        node: &str,
        session: &str,
        profile: &CliProfile,
        timeout: u64,
    ) -> Result<String, String> {
        if node != "local" {
            let deadline = Instant::now() + Duration::from_secs(timeout);
            while Instant::now() <= deadline {
                let buf = self
                    .remote_session_capture(node, session, None, false)
                    .await
                    .unwrap_or_default();
                let has_prompt = buf.contains(&profile.prompt_indicator);
                let busy = profile
                    .busy_indicators
                    .iter()
                    .any(|marker| buf.contains(marker));
                if has_prompt && !busy {
                    return Ok(format!(
                        "{} is ready on node {} (profile: {})",
                        session, node, profile.name
                    ));
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            return Err(format!(
                "Timeout waiting for {} on node {} to be ready (profile: {})",
                session, node, profile.name
            ));
        }

        self.local_node_call(|reply| LocalNodeMessage::CodingWaitReady {
            session: session.to_owned(),
            timeout,
            profile: profile.clone(),
            reply,
        })
        .await
    }

    fn deny(&self, action: &str) -> CallToolResult {
        Self::error_result(self.policy.deny(action))
    }

    async fn local_node_call<T>(
        &self,
        build: impl FnOnce(RpcReplyPort<Result<T, String>>) -> LocalNodeMessage,
    ) -> Result<T, String>
    where
        T: Send + 'static,
    {
        let Some(local_node) = self.local_node.as_ref() else {
            return Err("local node is disabled".into());
        };
        match local_node
            .call(build, None)
            .await
            .map_err(|error| format!("local node actor call failed: {}", error))?
        {
            CallResult::Success(result) => result,
            CallResult::Timeout => Err("local node actor call timed out".into()),
            CallResult::SenderError => Err("local node actor reply channel closed".into()),
        }
    }

    async fn registry_call<T>(
        &self,
        build: impl FnOnce(RpcReplyPort<Result<T, String>>) -> NodeRegistryMessage,
        timeout: Option<Duration>,
    ) -> Result<T, String>
    where
        T: Send + 'static,
    {
        registry_call(&self.registry, build, timeout).await
    }

    async fn remote_node_call(
        &self,
        node_id: &str,
        kind: NodeCommandKind,
        timeout: Duration,
    ) -> Result<NodeCommandResult, String> {
        self.registry_call(
            |reply| NodeRegistryMessage::Dispatch {
                node_id: node_id.to_owned(),
                kind,
                reply,
            },
            Some(timeout),
        )
        .await
    }

    async fn remote_tmux(
        &self,
        node_id: &str,
        args: Vec<String>,
        timeout: Duration,
    ) -> Result<String, String> {
        match self
            .remote_node_call(node_id, NodeCommandKind::Tmux { args }, timeout)
            .await?
        {
            NodeCommandResult::TmuxOutput(output) => Ok(output),
            NodeCommandResult::Error { message } => Err(message),
            other => Err(format!("unexpected tmux command result: {:?}", other)),
        }
    }

    async fn node_session_exists(&self, node_id: &str, session: &str) -> bool {
        if node_id == "local" {
            return session_exists(session);
        }
        self.remote_tmux(
            node_id,
            vec!["has-session".into(), "-t".into(), session.into()],
            Duration::from_secs(10),
        )
        .await
        .is_ok()
    }

    async fn remote_session_capture(
        &self,
        node_id: &str,
        session: &str,
        lines: Option<usize>,
        scrollback: bool,
    ) -> Result<String, String> {
        let args = if scrollback {
            vec![
                "capture-pane".into(),
                "-t".into(),
                session.into(),
                "-p".into(),
                "-S".into(),
                "-".into(),
            ]
        } else if let Some(lines) = lines {
            vec![
                "capture-pane".into(),
                "-t".into(),
                session.into(),
                "-p".into(),
                "-S".into(),
                format!("-{}", lines),
            ]
        } else {
            vec![
                "capture-pane".into(),
                "-t".into(),
                session.into(),
                "-p".into(),
            ]
        };
        self.remote_tmux(node_id, args, Duration::from_secs(20))
            .await
    }

    async fn remote_session_first_pane(
        &self,
        node_id: &str,
        session: &str,
    ) -> Result<String, String> {
        let panes = self
            .remote_tmux(
                node_id,
                vec![
                    "list-panes".into(),
                    "-t".into(),
                    session.into(),
                    "-F".into(),
                    "#{pane_id}".into(),
                ],
                Duration::from_secs(20),
            )
            .await?;
        panes
            .lines()
            .find(|line| !line.trim().is_empty())
            .map(|line| line.trim().to_string())
            .ok_or_else(|| format!("Session '{}' has no panes on node '{}'", session, node_id))
    }

    async fn remote_wait_for(
        &self,
        node_id: &str,
        session: &str,
        mode: &str,
        sentinel: Option<&str>,
        prompt: Option<&str>,
        timeout: f64,
        poll: f64,
        stability: f64,
    ) -> Result<String, String> {
        let deadline = Instant::now() + Duration::from_secs_f64(timeout);
        let poll_dur = Duration::from_secs_f64(poll);
        match mode {
            "sentinel" => {
                let sentinel = sentinel.ok_or("sentinel required for sentinel mode")?;
                while Instant::now() < deadline {
                    let output = self
                        .remote_session_capture(node_id, session, Some(200), false)
                        .await?;
                    if output.contains(sentinel) {
                        return Ok(format!("Sentinel '{}' found", sentinel));
                    }
                    tokio::time::sleep(poll_dur).await;
                }
                Err(format!(
                    "Timeout after {}s waiting for sentinel '{}'",
                    timeout, sentinel
                ))
            }
            "prompt" => {
                let prompt = prompt.ok_or("prompt required for prompt mode")?;
                while Instant::now() < deadline {
                    let output = self
                        .remote_session_capture(node_id, session, Some(200), false)
                        .await?;
                    if output.contains(prompt) {
                        return Ok(format!("Prompt '{}' found", prompt));
                    }
                    tokio::time::sleep(poll_dur).await;
                }
                Err(format!(
                    "Timeout after {}s waiting for prompt '{}'",
                    timeout, prompt
                ))
            }
            _ => {
                let stable_needed = (stability / poll).max(1.0) as usize;
                let mut last_output = String::new();
                let mut stable_count = 0;
                while Instant::now() < deadline {
                    let output = self
                        .remote_session_capture(node_id, session, Some(200), false)
                        .await?;
                    if output == last_output {
                        stable_count += 1;
                        if stable_count >= stable_needed {
                            return Ok(format!("Output stable for {}s", stability));
                        }
                    } else {
                        stable_count = 0;
                        last_output = output;
                    }
                    tokio::time::sleep(poll_dur).await;
                }
                Err(format!("Timeout after {}s", timeout))
            }
        }
    }
}

async fn registry_call<T>(
    registry: &ActorRef<NodeRegistryMessage>,
    build: impl FnOnce(RpcReplyPort<Result<T, String>>) -> NodeRegistryMessage,
    timeout: Option<Duration>,
) -> Result<T, String>
where
    T: Send + 'static,
{
    match registry
        .call(build, timeout)
        .await
        .map_err(|error| format!("node registry actor call failed: {}", error))?
    {
        CallResult::Success(result) => result,
        CallResult::Timeout => Err("node registry actor call timed out".into()),
        CallResult::SenderError => Err("node registry actor reply channel closed".into()),
    }
}

impl ServerHandler for TmuxMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .enable_prompts()
                .build(),
        )
        .with_server_info(Implementation::new("mmux", env!("CARGO_PKG_VERSION")))
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        let tools = vec![
                // ── Universal session management ──
                Tool::new(
                    "kill_session",
                    "Kill a tmux session",
                    Arc::new(tool_schema(json!({
                        "node": { "type": "string", "description": "Execution node id (default: local)" },
                        "session": { "type": "string" }
                    }), None)),
                ),
                Tool::new(
                    "list_sessions",
                    "List all tmux sessions",
                    Arc::new(tool_schema(json!({
                        "node": { "type": "string", "description": "Execution node id (default: local)" }
                    }), None)),
                ),
                Tool::new(
                    "list_nodes",
                    "List registered execution nodes",
                    Arc::new(tool_schema(json!({}), None)),
                ),
                Tool::new(
                    "list_coder_profiles",
                    "List loaded coder profiles",
                    Arc::new(tool_schema(json!({}), None)),
                ),
                Tool::new(
                    "node.info",
                    "Describe the selected execution node",
                    Arc::new(tool_schema(json!({
                        "node": { "type": "string", "description": "Node id (default: local)" }
                    }), None)),
                ),
                // ── Universal interaction ──
                Tool::new(
                    "send_input",
                    "Send text input to any tmux session",
                    Arc::new(tool_schema(json!({
                        "session": { "type": "string" },
                        "text": { "type": "string", "description": "Text to send" },
                        "enter": { "type": "boolean", "description": "Send Enter after text (default: true)" }
                    }), Some(vec!["text"]))),
                ),
                Tool::new(
                    "send_key",
                    "Send a special key (C-c, C-d, Escape, Enter, etc.)",
                    Arc::new(tool_schema(json!({
                        "session": { "type": "string" },
                        "key": { "type": "string", "description": "Key sequence (e.g. C-c, Escape, Enter)" }
                    }), Some(vec!["key"]))),
                ),
                Tool::new(
                    "capture_output",
                    "Capture pane output from a session",
                    Arc::new(tool_schema(json!({
                        "session": { "type": "string" },
                        "lines": { "type": "integer", "description": "Number of lines to capture" },
                        "scrollback": { "type": "boolean", "description": "Capture full scrollback" }
                    }), None)),
                ),
                Tool::new(
                    "wait_for",
                    "Wait for a condition in session output",
                    Arc::new(tool_schema(json!({
                        "session": { "type": "string" },
                        "mode": { "type": "string", "enum": ["stable", "sentinel", "prompt"], "description": "stable: output stops changing; sentinel: text appears; prompt: prompt marker appears" },
                        "sentinel": { "type": "string", "description": "Text to wait for (sentinel mode)" },
                        "prompt": { "type": "string", "description": "Prompt marker to wait for (prompt mode)" },
                        "timeout_seconds": { "type": "number", "description": "Max seconds to wait (default: 30)" },
                        "poll_seconds": { "type": "number", "description": "Poll interval (default: 0.5)" },
                        "stability_seconds": { "type": "number", "description": "Seconds of stability required (default: 1.0)" }
                    }), None)),
                ),
                Tool::new(
                    "interact",
                    "Send input and wait for output in one call",
                    Arc::new(tool_schema(json!({
                        "session": { "type": "string" },
                        "text": { "type": "string" },
                        "timeout_seconds": { "type": "number", "description": "Max seconds to wait (default: 30)" }
                    }), Some(vec!["text"]))),
                ),
                Tool::new(
                    "exec",
                    "Execute a shell command in a session and return the output. Creates the session if it does not exist.",
                    Arc::new(tool_schema(json!({
                        "session": { "type": "string", "description": "Session name (default: mmux_shell)" },
                        "command": { "type": "string", "description": "Shell command to execute" },
                        "cwd": { "type": "string", "description": "Working directory (only used when creating session)" },
                        "timeout_seconds": { "type": "number", "description": "Max seconds to wait for output (default: 30)" },
                        "lines": { "type": "integer", "description": "Lines of output to capture (default: 40)" }
                    }), Some(vec!["command"]))),
                ),
                Tool::new(
                    "start_coding_session",
                    "Start a coding CLI session from a profile-defined command, then wait until it is ready.",
                    Arc::new(tool_schema(json!({
                        "profile": { "type": "string", "description": "CLI profile name (default: opencode)" },
                        "session": { "type": "string", "description": "Session name (default: profile name)" },
                        "node": { "type": "string", "description": "Execution node id (default: local)" },
                        "cwd": { "type": "string", "description": "Working directory" },
                        "timeout_seconds": { "type": "integer", "description": "Max seconds to wait for readiness (default: 30)" }
                    }), None)),
                ),
                // ── Session introspection ──
                Tool::new(
                    "session_info",
                    "Get detailed info about a tmux session: panes, windows, dimensions, running commands",
                    Arc::new(tool_schema(json!({
                        "session": { "type": "string", "description": "Session name" }
                    }), None)),
                ),
                Tool::new(
                    "list_panes",
                    "List all panes in a tmux session with dimensions and running commands",
                    Arc::new(tool_schema(json!({
                        "session": { "type": "string", "description": "Session name" }
                    }), None)),
                ),
                Tool::new(
                    "check_state",
                    "Quick non-blocking check: is the session at a prompt and not busy? Returns JSON.",
                    Arc::new(tool_schema(json!({
                        "session": { "type": "string" },
                        "profile": { "type": "string", "description": "CLI profile name (default: opencode)" }
                    }), None)),
                ),
                Tool::new(
                    "resize_pane",
                    "Resize the main pane in a tmux session. Useful for TUI apps.",
                    Arc::new(tool_schema(json!({
                        "session": { "type": "string" },
                        "width": { "type": "integer", "description": "New width in columns" },
                        "height": { "type": "integer", "description": "New height in rows" }
                    }), None)),
                ),
                // ── File operations ──
                Tool::new(
                    "read_file",
                    "Read a file from disk. Returns 'content' + 'encoding' (utf-8 or base64), compression detection, and mime_type.",
                    Arc::new(tool_schema(json!({
                        "path": { "type": "string", "description": "Absolute or relative file path" },
                        "offset": { "type": "integer", "description": "Optional byte offset" },
                        "limit": { "type": "integer", "description": "Optional max bytes (default 4 MiB)" }
                    }), Some(vec!["path"]))),
                ),
                Tool::new(
                    "save_file",
                    "Save a file to disk. Accepts content + encoding (utf-8 or base64). Creates parent dirs.",
                    Arc::new(tool_schema(json!({
                        "path": { "type": "string", "description": "File path to write" },
                        "content": { "type": "string", "description": "File content" },
                        "encoding": { "type": "string", "enum": ["base64", "utf-8"], "description": "Use base64 for binary" },
                        "mime_type": { "type": "string", "description": "Optional mime type hint" },
                        "append": { "type": "boolean", "description": "Append instead of overwrite" }
                    }), Some(vec!["path", "content"]))),
                ),
                // ── Coding CLI adapters (profile-aware) ──
                Tool::new(
                    "coding_send",
                    "Send a prompt to a coding CLI with profile-specific preprocessing (dismiss startup, etc.)",
                    Arc::new(tool_schema(json!({
                        "session": { "type": "string" },
                        "prompt": { "type": "string" },
                        "profile": { "type": "string", "description": "CLI profile name (default: opencode)" }
                    }), Some(vec!["prompt"]))),
                ),
                Tool::new(
                    "coding_read",
                    "Capture the last N lines from a coding CLI pane",
                    Arc::new(tool_schema(json!({
                        "session": { "type": "string" },
                        "lines": { "type": "integer", "description": "Lines to capture (default: 40)" }
                    }), None)),
                ),
                Tool::new(
                    "coding_action",
                    "Send a profile-aware action to a coding CLI (approve, reject, cancel, escape, dismiss)",
                    Arc::new(tool_schema(json!({
                        "session": { "type": "string" },
                        "action": { "type": "string", "enum": ["approve", "reject", "cancel", "escape", "dismiss"], "description": "Action to perform" },
                        "profile": { "type": "string", "description": "CLI profile name (default: opencode)" }
                    }), Some(vec!["action"]))),
                ),
                Tool::new(
                    "coding_wait_ready",
                    "Wait until a coding CLI is at a prompt and not busy",
                    Arc::new(tool_schema(json!({
                        "session": { "type": "string" },
                        "timeout_seconds": { "type": "integer", "description": "Max seconds to wait (default: 30)" },
                        "profile": { "type": "string", "description": "CLI profile name (default: opencode)" }
                    }), None)),
                ),
                Tool::new(
                    "load_profile",
                    "Load a new CLI profile from inline TOML or a file path. Adds it to the running server's profile registry.",
                    Arc::new(tool_schema(json!({
                        "toml": { "type": "string", "description": "Inline TOML profile definition" },
                        "path": { "type": "string", "description": "Path to TOML file containing profile definition" }
                    }), None)),
                ),
            ];
        Ok(ListToolsResult::with_all_items(tools))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let args = request.arguments.unwrap_or_default();
        let session = args
            .get("session")
            .and_then(|v| v.as_str())
            .unwrap_or("kimi_codex");
        let node = args.get("node").and_then(|v| v.as_str()).unwrap_or("local");

        let result = match request.name.as_ref() {
            // ── Universal session management ──
            "kill_session" => {
                if !self.policy.can_kill_session() {
                    return Ok(self.deny("kill_session"));
                }
                if node != "local" {
                    if !self.node_session_exists(node, session).await {
                        return Ok(Self::text_result(format!(
                            "Session '{}' not found on node '{}'",
                            session, node
                        )));
                    }
                    return match self
                        .remote_tmux(
                            node,
                            vec!["kill-session".into(), "-t".into(), session.into()],
                            Duration::from_secs(20),
                        )
                        .await
                    {
                        Ok(_) => Ok(Self::text_result(format!(
                            "Killed session '{}' on node '{}'",
                            session, node
                        ))),
                        Err(e) => Ok(Self::error_result(e)),
                    };
                }
                match self
                    .local_node_call(|reply| LocalNodeMessage::KillSession {
                        session: session.to_owned(),
                        reply,
                    })
                    .await
                {
                    Ok(msg) => Ok(Self::text_result(msg)),
                    Err(e) => Ok(Self::error_result(e)),
                }
            }
            "list_sessions" => {
                if node != "local" {
                    return match self
                        .remote_tmux(
                            node,
                            vec![
                                "list-sessions".into(),
                                "-F".into(),
                                "#{session_name}: #{session_windows} windows (#{session_attached} attached)".into(),
                            ],
                            Duration::from_secs(20),
                        )
                        .await
                    {
                        Ok(output) => Ok(Self::text_result(output)),
                        Err(_) => Ok(Self::text_result(format!(
                            "No tmux sessions running on node '{}'",
                            node
                        ))),
                    };
                }
                match self
                    .local_node_call(|reply| LocalNodeMessage::ListSessions { reply })
                    .await
                {
                    Ok(msg) => Ok(Self::text_result(msg)),
                    Err(e) => Ok(Self::error_result(e)),
                }
            }
            "list_nodes" => match self
                .registry_call(|reply| NodeRegistryMessage::ListNodes { reply }, None)
                .await
            {
                Ok(msg) => Ok(Self::text_result(msg)),
                Err(e) => Ok(Self::error_result(e)),
            },
            "list_coder_profiles" => {
                let mut profiles: Vec<_> = self
                    .profiles
                    .read()
                    .unwrap()
                    .values()
                    .cloned()
                    .collect();
                profiles.sort_by(|a, b| a.name.cmp(&b.name));
                let json = serde_json::to_string_pretty(
                    &profiles
                        .into_iter()
                        .map(|profile| {
                            json!({
                                "name": profile.name,
                                "cmd": profile.cmd,
                                "prompt_indicator": profile.prompt_indicator,
                                "busy_indicators": profile.busy_indicators,
                                "startup_dismiss": profile.startup_dismiss,
                            })
                        })
                        .collect::<Vec<_>>(),
                )
                .unwrap_or_else(|e| format!("{{\"error\":\"{}\"}}", e));
                Ok(Self::text_result(json))
            }
            "node.info" => match self
                .registry_call(
                    |reply| NodeRegistryMessage::NodeInfo {
                        node_id: node.to_owned(),
                        reply,
                    },
                    None,
                )
                .await
            {
                Ok(msg) => Ok(Self::text_result(msg)),
                Err(e) => Ok(Self::error_result(e)),
            },
            // ── Universal interaction ──
            "send_input" => {
                if !self.policy.can_send_input() {
                    return Ok(self.deny("send_input"));
                }
                let text = args
                    .get("text")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| McpError::invalid_request("missing text", None))?;
                let enter = args.get("enter").and_then(|v| v.as_bool()).unwrap_or(true);
                if node != "local" {
                    let target = session.to_owned();
                    if let Err(error) = self
                        .remote_tmux(
                            node,
                            vec![
                                "send-keys".into(),
                                "-l".into(),
                                "-t".into(),
                                target,
                                text.into(),
                            ],
                            Duration::from_secs(20),
                        )
                        .await
                    {
                        return Ok(Self::error_result(error));
                    }
                    if enter {
                        tokio::time::sleep(Duration::from_millis(200)).await;
                        if let Err(error) = self
                            .remote_tmux(
                                node,
                                vec![
                                    "send-keys".into(),
                                    "-t".into(),
                                    session.into(),
                                    "Enter".into(),
                                ],
                                Duration::from_secs(20),
                            )
                            .await
                        {
                            return Ok(Self::error_result(error));
                        }
                    }
                    return Ok(Self::text_result(format!(
                        "Sent to {} on node {}: {}",
                        session, node, text
                    )));
                }
                match self
                    .local_node_call(|reply| LocalNodeMessage::SendInput {
                        session: session.to_owned(),
                        text: text.to_owned(),
                        enter,
                        reply,
                    })
                    .await
                {
                    Ok(msg) => Ok(Self::text_result(msg)),
                    Err(e) => Ok(Self::error_result(e)),
                }
            }
            "send_key" => {
                if !self.policy.can_send_input() {
                    return Ok(self.deny("send_key"));
                }
                let key = args
                    .get("key")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| McpError::invalid_request("missing key", None))?;
                if node != "local" {
                    return match self
                        .remote_tmux(
                            node,
                            vec!["send-keys".into(), "-t".into(), session.into(), key.into()],
                            Duration::from_secs(20),
                        )
                        .await
                    {
                        Ok(_) => Ok(Self::text_result(format!(
                            "Sent key '{}' to {} on node {}",
                            key, session, node
                        ))),
                        Err(e) => Ok(Self::error_result(e)),
                    };
                }
                match self
                    .local_node_call(|reply| LocalNodeMessage::SendKey {
                        session: session.to_owned(),
                        key: key.to_owned(),
                        reply,
                    })
                    .await
                {
                    Ok(msg) => Ok(Self::text_result(msg)),
                    Err(e) => Ok(Self::error_result(e)),
                }
            }
            "capture_output" => {
                let lines = args
                    .get("lines")
                    .and_then(|v| v.as_u64())
                    .map(|n| n as usize);
                let scrollback = args
                    .get("scrollback")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                if node != "local" {
                    return match self
                        .remote_session_capture(node, session, lines, scrollback)
                        .await
                    {
                        Ok(output) => {
                            Ok(Self::text_result(self.policy.limit_capture_output(output)))
                        }
                        Err(e) => Ok(Self::error_result(e)),
                    };
                }
                match self
                    .local_node_call(|reply| LocalNodeMessage::CaptureOutput {
                        session: session.to_owned(),
                        lines,
                        scrollback,
                        reply,
                    })
                    .await
                {
                    Ok(output) => Ok(Self::text_result(self.policy.limit_capture_output(output))),
                    Err(e) => Ok(Self::error_result(e)),
                }
            }
            "wait_for" => {
                let mode = args
                    .get("mode")
                    .and_then(|v| v.as_str())
                    .unwrap_or("stable");
                let sentinel = args.get("sentinel").and_then(|v| v.as_str());
                let prompt = args.get("prompt").and_then(|v| v.as_str());
                let timeout = self
                    .policy
                    .clamp_timeout(
                        args.get("timeout_seconds")
                            .and_then(|v| v.as_f64())
                            .unwrap_or(30.0),
                    )
                    .map_err(|e| McpError::invalid_request(e, None))?;
                let poll = args
                    .get("poll_seconds")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.5);
                let stability = args
                    .get("stability_seconds")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(1.0);
                if !poll.is_finite() || poll <= 0.0 {
                    return Err(McpError::invalid_request(
                        "poll_seconds must be a positive finite number",
                        None,
                    ));
                }
                if !stability.is_finite() || stability < 0.0 {
                    return Err(McpError::invalid_request(
                        "stability_seconds must be a non-negative finite number",
                        None,
                    ));
                }
                if node != "local" {
                    return match self
                        .remote_wait_for(
                            node, session, mode, sentinel, prompt, timeout, poll, stability,
                        )
                        .await
                    {
                        Ok(msg) => Ok(Self::text_result(msg)),
                        Err(e) => Ok(Self::error_result(e)),
                    };
                }
                match self
                    .local_node_call(|reply| LocalNodeMessage::WaitFor {
                        session: session.to_owned(),
                        mode: mode.to_owned(),
                        sentinel: sentinel.map(ToOwned::to_owned),
                        prompt: prompt.map(ToOwned::to_owned),
                        timeout,
                        poll,
                        stability,
                        reply,
                    })
                    .await
                {
                    Ok(msg) => Ok(Self::text_result(msg)),
                    Err(e) => Ok(Self::error_result(e)),
                }
            }
            "interact" => {
                if !self.policy.can_send_input() {
                    return Ok(self.deny("interact"));
                }
                let text = args
                    .get("text")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| McpError::invalid_request("missing text", None))?;
                let timeout = self
                    .policy
                    .clamp_timeout(
                        args.get("timeout_seconds")
                            .and_then(|v| v.as_f64())
                            .unwrap_or(30.0),
                    )
                    .map_err(|e| McpError::invalid_request(e, None))?;
                if node != "local" {
                    if let Err(error) = self
                        .remote_tmux(
                            node,
                            vec![
                                "send-keys".into(),
                                "-l".into(),
                                "-t".into(),
                                session.into(),
                                text.into(),
                            ],
                            Duration::from_secs(20),
                        )
                        .await
                    {
                        return Ok(Self::error_result(error));
                    }
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    if let Err(error) = self
                        .remote_tmux(
                            node,
                            vec![
                                "send-keys".into(),
                                "-t".into(),
                                session.into(),
                                "Enter".into(),
                            ],
                            Duration::from_secs(20),
                        )
                        .await
                    {
                        return Ok(Self::error_result(error));
                    }
                    return match self
                        .remote_wait_for(node, session, "stable", None, None, timeout, 0.5, 1.0)
                        .await
                    {
                        Ok(msg) => Ok(Self::text_result(msg)),
                        Err(e) => Ok(Self::error_result(e)),
                    };
                }
                match self
                    .local_node_call(|reply| LocalNodeMessage::SendInput {
                        session: session.to_owned(),
                        text: text.to_owned(),
                        enter: true,
                        reply,
                    })
                    .await
                {
                    Ok(_) => match self
                        .local_node_call(|reply| LocalNodeMessage::WaitFor {
                            session: session.to_owned(),
                            mode: "stable".to_owned(),
                            sentinel: None,
                            prompt: None,
                            timeout,
                            poll: 0.5,
                            stability: 1.0,
                            reply,
                        })
                        .await
                    {
                        Ok(msg) => Ok(Self::text_result(msg)),
                        Err(e) => Ok(Self::error_result(e)),
                    },
                    Err(e) => Ok(Self::error_result(e)),
                }
            }
            "exec" => {
                if !self.policy.can_exec() {
                    return Ok(self.deny("exec"));
                }
                let command = args
                    .get("command")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| McpError::invalid_request("missing command", None))?;
                let session = args
                    .get("session")
                    .and_then(|v| v.as_str())
                    .unwrap_or("mmux_shell");
                let cwd = if node == "local" {
                    if let Some(cwd) = args.get("cwd").and_then(|v| v.as_str()) {
                        Some(
                            self.policy
                                .resolve_read_path(cwd)
                                .map_err(|e| McpError::invalid_request(e, None))?,
                        )
                    } else {
                        None
                    }
                } else {
                    args.get("cwd").and_then(|v| v.as_str()).map(PathBuf::from)
                };
                let cwd_text = cwd.as_ref().map(|p| p.to_string_lossy().into_owned());
                let timeout = self
                    .policy
                    .clamp_timeout(
                        args.get("timeout_seconds")
                            .and_then(|v| v.as_f64())
                            .unwrap_or(30.0),
                    )
                    .map_err(|e| McpError::invalid_request(e, None))?;
                let lines = args.get("lines").and_then(|v| v.as_u64()).unwrap_or(40) as usize;
                if node != "local" {
                    if !self.node_session_exists(node, session).await {
                        let mut tmux_args = vec![
                            "new-session".into(),
                            "-d".into(),
                            "-s".into(),
                            session.into(),
                        ];
                        if let Some(cwd) = cwd_text.as_ref() {
                            tmux_args.push("-c".into());
                            tmux_args.push(cwd.clone());
                        }
                        tmux_args.push("bash".into());
                        if let Err(error) = self
                            .remote_tmux(node, tmux_args, Duration::from_secs(30))
                            .await
                        {
                            return Ok(Self::error_result(error));
                        }
                        tokio::time::sleep(Duration::from_millis(300)).await;
                    }
                    let sentinel = format!(
                        "__MMUX_{}__",
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_nanos()
                    );
                    for text in [format!("echo '{}'", sentinel), command.to_owned()] {
                        if let Err(error) = self
                            .remote_tmux(
                                node,
                                vec![
                                    "send-keys".into(),
                                    "-l".into(),
                                    "-t".into(),
                                    session.into(),
                                    text,
                                ],
                                Duration::from_secs(20),
                            )
                            .await
                        {
                            return Ok(Self::error_result(error));
                        }
                        tokio::time::sleep(Duration::from_millis(200)).await;
                        if let Err(error) = self
                            .remote_tmux(
                                node,
                                vec![
                                    "send-keys".into(),
                                    "-t".into(),
                                    session.into(),
                                    "Enter".into(),
                                ],
                                Duration::from_secs(20),
                            )
                            .await
                        {
                            return Ok(Self::error_result(error));
                        }
                    }
                    if let Err(error) = self
                        .remote_wait_for(node, session, "stable", None, None, timeout, 0.5, 1.0)
                        .await
                    {
                        return Ok(Self::error_result(error));
                    }
                    return match self.remote_session_capture(node, session, None, true).await {
                        Ok(output) => {
                            let all_lines: Vec<&str> = output.lines().collect();
                            let sentinel_idx = all_lines
                                .iter()
                                .enumerate()
                                .filter_map(|(i, line)| {
                                    if line.trim() == sentinel {
                                        Some(i)
                                    } else {
                                        None
                                    }
                                })
                                .last();
                            let result_lines: Vec<&str> = if let Some(idx) = sentinel_idx {
                                all_lines.iter().skip(idx + 1).copied().collect()
                            } else {
                                let start = all_lines.len().saturating_sub(lines);
                                all_lines[start..].to_vec()
                            };
                            Ok(Self::text_result(
                                self.policy
                                    .limit_capture_output(clean_exec_output(result_lines)),
                            ))
                        }
                        Err(e) => Ok(Self::error_result(e)),
                    };
                }
                match self
                    .local_node_call(|reply| LocalNodeMessage::Exec {
                        session: session.to_owned(),
                        command: command.to_owned(),
                        cwd: cwd_text.clone(),
                        timeout,
                        max_lines: lines,
                        reply,
                    })
                    .await
                {
                    Ok(output) => Ok(Self::text_result(self.policy.limit_capture_output(output))),
                    Err(e) => Ok(Self::error_result(e)),
                }
            }
            // ── File operations ──
            "read_file" => {
                if !self.policy.can_read_files() {
                    return Ok(self.deny("read_file"));
                }
                let path = args
                    .get("path")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| McpError::invalid_request("missing path", None))?;
                let offset = args.get("offset").and_then(|v| v.as_u64());
                let limit = args
                    .get("limit")
                    .and_then(|v| v.as_u64())
                    .map(|n| n as usize)
                    .unwrap_or(self.policy.max_read_bytes);
                if limit > self.policy.max_read_bytes {
                    return Ok(Self::error_result(format!(
                        "read limit too large: {} bytes exceeds limit of {}",
                        limit, self.policy.max_read_bytes
                    )));
                }
                let resolved_text = if node == "local" {
                    self.policy
                        .resolve_read_path(path)
                        .map_err(|e| McpError::invalid_request(e, None))?
                        .to_string_lossy()
                        .into_owned()
                } else {
                    path.to_owned()
                };
                if node != "local" {
                    return match self
                        .remote_node_call(
                            node,
                            NodeCommandKind::ReadFile {
                                path: resolved_text.clone(),
                                offset,
                                limit,
                            },
                            Duration::from_secs(60),
                        )
                        .await
                    {
                        Ok(NodeCommandResult::FileContent { content_base64 }) => {
                            match BASE64.decode(content_base64.as_bytes()) {
                                Ok(bytes) => {
                                    let encoding = if std::str::from_utf8(&bytes).is_ok() {
                                        "utf-8"
                                    } else {
                                        "base64"
                                    };
                                    let content = if encoding == "utf-8" {
                                        String::from_utf8_lossy(&bytes).into_owned()
                                    } else {
                                        BASE64.encode(&bytes)
                                    };
                                    let result = ReadFileResult {
                                        path: resolved_text.clone(),
                                        content,
                                        encoding: encoding.into(),
                                        mime_type: mmux_node::detect_mime_type(
                                            Path::new(&resolved_text),
                                            &bytes,
                                        ),
                                        compression: mmux_node::detect_compression(&bytes),
                                        size_bytes: bytes.len() as u64,
                                        read_bytes: bytes.len(),
                                    };
                                    let json = serde_json::to_string_pretty(&result)
                                        .unwrap_or_else(|e| format!("{{\"error\":\"{}\"}}", e));
                                    Ok(Self::text_result(json))
                                }
                                Err(e) => Ok(Self::error_result(format!(
                                    "remote node returned invalid base64: {}",
                                    e
                                ))),
                            }
                        }
                        Ok(NodeCommandResult::Error { message }) => Ok(Self::error_result(message)),
                        Ok(other) => Ok(Self::error_result(format!(
                            "unexpected read_file command result: {:?}",
                            other
                        ))),
                        Err(e) => Ok(Self::error_result(e)),
                    };
                }
                match self
                    .local_node_call(|reply| LocalNodeMessage::ReadFile {
                        path: resolved_text.clone(),
                        offset,
                        limit: Some(limit),
                        reply,
                    })
                    .await
                {
                    Ok(result) => {
                        let json = serde_json::to_string_pretty(&result)
                            .unwrap_or_else(|e| format!("{{\"error\":\"{}\"}}", e));
                        Ok(Self::text_result(json))
                    }
                    Err(e) => Ok(Self::error_result(e)),
                }
            }
            "save_file" => {
                if !self.policy.can_write_files() {
                    return Ok(self.deny("save_file"));
                }
                let path = args
                    .get("path")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| McpError::invalid_request("missing path", None))?;
                let content = args
                    .get("content")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| McpError::invalid_request("missing content", None))?;
                let encoding = args
                    .get("encoding")
                    .and_then(|v| v.as_str())
                    .unwrap_or("base64");
                let append = args
                    .get("append")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let resolved_text = if node == "local" {
                    self.policy
                        .resolve_write_path(path)
                        .map_err(|e| McpError::invalid_request(e, None))?
                        .to_string_lossy()
                        .into_owned()
                } else {
                    path.to_owned()
                };
                if node == "local" && self.policy.mode != SecurityMode::Workspace {
                    if let Some(parent) = std::path::Path::new(&resolved_text).parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                }
                if node != "local" {
                    let bytes = match encoding {
                        "base64" => match BASE64.decode(content.as_bytes()) {
                            Ok(bytes) => bytes,
                            Err(e) => {
                                return Ok(Self::error_result(format!(
                                    "base64 decode error: {}",
                                    e
                                )))
                            }
                        },
                        "utf-8" => content.as_bytes().to_vec(),
                        other => {
                            return Ok(Self::error_result(format!(
                                "unsupported encoding: {}",
                                other
                            )))
                        }
                    };
                    if bytes.len() > self.policy.max_write_bytes {
                        return Ok(Self::error_result(format!(
                            "write too large: {} bytes exceeds limit of {}",
                            bytes.len(),
                            self.policy.max_write_bytes
                        )));
                    }
                    return match self
                        .remote_node_call(
                            node,
                            NodeCommandKind::WriteFile {
                                path: resolved_text.clone(),
                                content_base64: BASE64.encode(&bytes),
                                append,
                            },
                            Duration::from_secs(60),
                        )
                        .await
                    {
                        Ok(NodeCommandResult::WriteComplete { bytes_written }) => {
                            let result = SaveFileResult {
                                path: resolved_text,
                                bytes_written,
                                mime_type: None,
                            };
                            let json = serde_json::to_string_pretty(&result)
                                .unwrap_or_else(|e| format!("{{\"error\":\"{}\"}}", e));
                            Ok(Self::text_result(json))
                        }
                        Ok(NodeCommandResult::Error { message }) => Ok(Self::error_result(message)),
                        Ok(other) => Ok(Self::error_result(format!(
                            "unexpected save_file command result: {:?}",
                            other
                        ))),
                        Err(e) => Ok(Self::error_result(e)),
                    };
                }
                match self
                    .local_node_call(|reply| LocalNodeMessage::SaveFile {
                        path: resolved_text.clone(),
                        content: content.to_owned(),
                        encoding: encoding.to_owned(),
                        append,
                        max_bytes: Some(self.policy.max_write_bytes),
                        reply,
                    })
                    .await
                {
                    Ok(result) => {
                        let json = serde_json::to_string_pretty(&result)
                            .unwrap_or_else(|e| format!("{{\"error\":\"{}\"}}", e));
                        Ok(Self::text_result(json))
                    }
                    Err(e) => Ok(Self::error_result(e)),
                }
            }
            // ── Coding CLI adapters ──
            "coding_send" => {
                if !self.policy.can_send_input() {
                    return Ok(self.deny("coding_send"));
                }
                let prompt = args
                    .get("prompt")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| McpError::invalid_request("missing prompt", None))?;
                let profile = self
                    .resolve_profile(args.get("profile").and_then(|v| v.as_str()))
                    .ok_or_else(|| McpError::invalid_request("unknown profile", None))?;
                if node != "local" {
                    let pane = match self.remote_session_first_pane(node, session).await {
                        Ok(pane) => pane,
                        Err(error) => return Ok(Self::error_result(error)),
                    };
                    if let Some(dismiss) = profile.startup_dismiss.as_ref() {
                        let buf = self
                            .remote_tmux(
                                node,
                                vec![
                                    "capture-pane".into(),
                                    "-t".into(),
                                    pane.clone(),
                                    "-p".into(),
                                ],
                                Duration::from_secs(20),
                            )
                            .await
                            .unwrap_or_default();
                        if dismiss.triggers.iter().any(|trigger| buf.contains(trigger)) {
                            let _ = self
                                .remote_tmux(
                                    node,
                                    vec![
                                        "send-keys".into(),
                                        "-t".into(),
                                        pane.clone(),
                                        dismiss.key.clone(),
                                    ],
                                    Duration::from_secs(20),
                                )
                                .await;
                            tokio::time::sleep(Duration::from_millis(300)).await;
                        }
                    }
                    if let Err(error) = self
                        .remote_tmux(
                            node,
                            vec![
                                "send-keys".into(),
                                "-l".into(),
                                "-t".into(),
                                pane.clone(),
                                prompt.into(),
                            ],
                            Duration::from_secs(20),
                        )
                        .await
                    {
                        return Ok(Self::error_result(error));
                    }
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    return match self
                        .remote_tmux(
                            node,
                            vec!["send-keys".into(), "-t".into(), pane, "Enter".into()],
                            Duration::from_secs(20),
                        )
                        .await
                    {
                        Ok(_) => Ok(Self::text_result(format!(
                            "Sent to {} on node {} (profile: {}): {}",
                            session, node, profile.name, prompt
                        ))),
                        Err(e) => Ok(Self::error_result(e)),
                    };
                }
                match self
                    .local_node_call(|reply| LocalNodeMessage::CodingSend {
                        session: session.to_owned(),
                        prompt: prompt.to_owned(),
                        profile,
                        reply,
                    })
                    .await
                {
                    Ok(msg) => Ok(Self::text_result(msg)),
                    Err(e) => Ok(Self::error_result(e)),
                }
            }
            "start_coding_session" => {
                if !self.policy.can_create_session() {
                    return Ok(self.deny("start_coding_session"));
                }
                let profile = self
                    .resolve_profile(args.get("profile").and_then(|v| v.as_str()))
                    .ok_or_else(|| McpError::invalid_request("unknown profile", None))?;
                let node = args.get("node").and_then(|v| v.as_str()).unwrap_or("local");
                let session_name = args
                    .get("session")
                    .and_then(|v| v.as_str())
                    .unwrap_or(profile.name.as_str());
                let cwd = if node == "local" {
                    if let Some(cwd) = args.get("cwd").and_then(|v| v.as_str()) {
                        Some(
                            self.policy
                                .resolve_read_path(cwd)
                                .map_err(|e| McpError::invalid_request(e, None))?,
                        )
                    } else {
                        None
                    }
                } else {
                    args.get("cwd").and_then(|v| v.as_str()).map(PathBuf::from)
                };
                let cwd_text = cwd.as_ref().map(|p| p.to_string_lossy().into_owned());
                let timeout = self
                    .policy
                    .clamp_timeout(
                        args.get("timeout_seconds")
                            .and_then(|v| v.as_u64())
                            .map(|v| v as f64)
                            .unwrap_or(30.0),
                    )
                    .map_err(|e| McpError::invalid_request(e, None))? as u64;
                let cmd = profile.cmd.as_deref().ok_or_else(|| {
                    McpError::invalid_request(
                        format!("profile '{}' does not define a launch cmd", profile.name),
                        None,
                    )
                })?;
                let launch_envs = Self::profile_launch_envs(&profile)
                    .map_err(|error| McpError::invalid_request(error, None))?;
                if let Err(error) = self.apply_launch_envs(node, &launch_envs).await {
                    return Ok(Self::error_result(error));
                }
                match self
                    .create_session_with_command(node, session_name, cmd, cwd_text.as_deref())
                    .await
                {
                    Ok(_) => match self
                        .wait_coding_session_ready(node, session_name, &profile, timeout)
                        .await
                    {
                        Ok(msg) => Ok(Self::text_result(msg)),
                        Err(e) => Ok(Self::error_result(e)),
                    },
                    Err(e) => Ok(Self::error_result(e)),
                }
            }
            "coding_read" => {
                let lines = args.get("lines").and_then(|v| v.as_u64()).unwrap_or(40) as usize;
                if node != "local" {
                    return match self
                        .remote_session_capture(node, session, Some(lines), false)
                        .await
                    {
                        Ok(output) => {
                            Ok(Self::text_result(self.policy.limit_capture_output(output)))
                        }
                        Err(e) => Ok(Self::error_result(e)),
                    };
                }
                match self
                    .local_node_call(|reply| LocalNodeMessage::CaptureOutput {
                        session: session.to_owned(),
                        lines: Some(lines),
                        scrollback: false,
                        reply,
                    })
                    .await
                {
                    Ok(output) => Ok(Self::text_result(self.policy.limit_capture_output(output))),
                    Err(e) => Ok(Self::error_result(e)),
                }
            }
            "coding_action" => {
                if !self.policy.can_send_input() {
                    return Ok(self.deny("coding_action"));
                }
                let action = args
                    .get("action")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| McpError::invalid_request("missing action", None))?;
                let profile = self
                    .resolve_profile(args.get("profile").and_then(|v| v.as_str()))
                    .ok_or_else(|| McpError::invalid_request("unknown profile", None))?;
                if node != "local" {
                    let pane = match self.remote_session_first_pane(node, session).await {
                        Ok(pane) => pane,
                        Err(error) => return Ok(Self::error_result(error)),
                    };
                    let keys = match action {
                        "approve" => profile.approve_keys,
                        "reject" => profile.reject_keys,
                        "cancel" => profile.cancel_keys,
                        "escape" | "dismiss" => profile.escape_keys,
                        other => {
                            return Ok(Self::error_result(format!("Unknown action: {}", other)))
                        }
                    };
                    return match self
                        .remote_tmux(
                            node,
                            vec![
                                "send-keys".into(),
                                "-t".into(),
                                pane,
                                keys,
                            ],
                            Duration::from_secs(20),
                        )
                        .await
                    {
                        Ok(_) => Ok(Self::text_result(format!(
                            "Sent action '{}' to {} on node {}",
                            action, session, node
                        ))),
                        Err(e) => Ok(Self::error_result(e)),
                    };
                }
                match self
                    .local_node_call(|reply| LocalNodeMessage::CodingAction {
                        session: session.to_owned(),
                        action: action.to_owned(),
                        profile,
                        reply,
                    })
                    .await
                {
                    Ok(msg) => Ok(Self::text_result(msg)),
                    Err(e) => Ok(Self::error_result(e)),
                }
            }
            "coding_wait_ready" => {
                let timeout = args
                    .get("timeout_seconds")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(30);
                let profile = self
                    .resolve_profile(args.get("profile").and_then(|v| v.as_str()))
                    .ok_or_else(|| McpError::invalid_request("unknown profile", None))?;
                if node != "local" {
                    let deadline = Instant::now() + Duration::from_secs(timeout);
                    while Instant::now() <= deadline {
                        let buf = self
                            .remote_session_capture(node, session, None, false)
                            .await
                            .unwrap_or_default();
                        let has_prompt = buf.contains(&profile.prompt_indicator);
                        let busy = profile
                            .busy_indicators
                            .iter()
                            .any(|marker| buf.contains(marker));
                        if has_prompt && !busy {
                            return Ok(Self::text_result(format!(
                                "{} is ready on node {} (profile: {})",
                                session, node, profile.name
                            )));
                        }
                        tokio::time::sleep(Duration::from_millis(500)).await;
                    }
                    return Ok(Self::error_result(format!(
                        "Timeout waiting for {} on node {} to be ready (profile: {})",
                        session, node, profile.name
                    )));
                }
                match self
                    .local_node_call(|reply| LocalNodeMessage::CodingWaitReady {
                        session: session.to_owned(),
                        timeout,
                        profile,
                        reply,
                    })
                    .await
                {
                    Ok(msg) => Ok(Self::text_result(msg)),
                    Err(e) => Ok(Self::error_result(e)),
                }
            }
            "load_profile" => {
                if !self.policy.can_mutate_profiles() {
                    return Ok(self.deny("load_profile"));
                }
                let toml_text = if let Some(path) = args.get("path").and_then(|v| v.as_str()) {
                    if !self.policy.can_load_profile_from_path() {
                        return Ok(self.deny("load_profile path"));
                    }
                    let resolved = self
                        .policy
                        .resolve_read_path(path)
                        .map_err(|e| McpError::invalid_request(e, None))?;
                    std::fs::read_to_string(&resolved).map_err(|e| {
                        McpError::invalid_request(format!("read error: {}", e), None)
                    })?
                } else if let Some(text) = args.get("toml").and_then(|v| v.as_str()) {
                    text.to_owned()
                } else {
                    return Err(McpError::invalid_request("provide 'toml' or 'path'", None));
                };
                match load_profile_from_toml(&toml_text) {
                    Ok(profile) => {
                        let name = profile.name.clone();
                        self.profiles.write().unwrap().insert(name.clone(), profile);
                        Ok(Self::text_result(format!("Loaded profile '{}'", name)))
                    }
                    Err(e) => Ok(Self::error_result(e)),
                }
            }
            "session_info" => {
                if node != "local" {
                    let panes = self
                        .remote_tmux(
                            node,
                            vec![
                                "list-panes".into(),
                                "-t".into(),
                                session.into(),
                                "-F".into(),
                                "pane_id=#{pane_id} index=#{pane_index} width=#{pane_width} height=#{pane_height} command=#{pane_current_command} title=#{pane_title}".into(),
                            ],
                            Duration::from_secs(20),
                        )
                        .await;
                    let windows = self
                        .remote_tmux(
                            node,
                            vec![
                                "list-windows".into(),
                                "-t".into(),
                                session.into(),
                                "-F".into(),
                                "window_id=#{window_id} index=#{window_index} name=#{window_name} active=#{window_active}".into(),
                            ],
                            Duration::from_secs(20),
                        )
                        .await;
                    return match (panes, windows) {
                        (Ok(panes), Ok(windows)) => Ok(Self::text_result(format!(
                            "Node: {}\nSession: {}\nPanes:\n{}\nWindows:\n{}",
                            node, session, panes, windows
                        ))),
                        (Err(e), _) | (_, Err(e)) => Ok(Self::error_result(e)),
                    };
                }
                match self
                    .local_node_call(|reply| LocalNodeMessage::SessionInfo {
                        session: session.to_owned(),
                        reply,
                    })
                    .await
                {
                    Ok(msg) => Ok(Self::text_result(msg)),
                    Err(e) => Ok(Self::error_result(e)),
                }
            }
            "list_panes" => {
                if node != "local" {
                    return match self
                        .remote_tmux(
                            node,
                            vec![
                                "list-panes".into(),
                                "-t".into(),
                                session.into(),
                                "-F".into(),
                                "#{pane_index}\t#{pane_width}x#{pane_height}\t#{pane_current_command}\t#{pane_title}".into(),
                            ],
                            Duration::from_secs(20),
                        )
                        .await
                    {
                        Ok(msg) => Ok(Self::text_result(msg)),
                        Err(e) => Ok(Self::error_result(e)),
                    };
                }
                match self
                    .local_node_call(|reply| LocalNodeMessage::ListPanes {
                        session: session.to_owned(),
                        reply,
                    })
                    .await
                {
                    Ok(msg) => Ok(Self::text_result(msg)),
                    Err(e) => Ok(Self::error_result(e)),
                }
            }
            "check_state" => {
                let profile = self
                    .resolve_profile(args.get("profile").and_then(|v| v.as_str()))
                    .ok_or_else(|| McpError::invalid_request("unknown profile", None))?;
                if node != "local" {
                    let buf = self
                        .remote_session_capture(node, session, None, false)
                        .await
                        .unwrap_or_default();
                    let has_prompt = buf.contains(&profile.prompt_indicator);
                    let busy = profile
                        .busy_indicators
                        .iter()
                        .any(|marker| buf.contains(marker));
                    return Ok(Self::text_result(format!(
                        "{{\"node\":\"{}\",\"session\":\"{}\",\"has_prompt\":{},\"busy\":{},\"profile\":\"{}\"}}",
                        node, session, has_prompt, busy, profile.name
                    )));
                }
                match self
                    .local_node_call(|reply| LocalNodeMessage::CheckState {
                        session: session.to_owned(),
                        profile,
                        reply,
                    })
                    .await
                {
                    Ok(msg) => Ok(Self::text_result(msg)),
                    Err(e) => Ok(Self::error_result(e)),
                }
            }
            "resize_pane" => {
                if !self.policy.can_resize() {
                    return Ok(self.deny("resize_pane"));
                }
                let width = args.get("width").and_then(|v| v.as_u64()).map(|n| n as u32);
                let height = args
                    .get("height")
                    .and_then(|v| v.as_u64())
                    .map(|n| n as u32);
                if node != "local" {
                    if let Some(width) = width {
                        let pane = match self.remote_session_first_pane(node, session).await {
                            Ok(pane) => pane,
                            Err(error) => return Ok(Self::error_result(error)),
                        };
                        if let Err(error) = self
                            .remote_tmux(
                                node,
                                vec![
                                    "resize-pane".into(),
                                    "-t".into(),
                                    pane,
                                    "-x".into(),
                                    width.to_string(),
                                ],
                                Duration::from_secs(20),
                            )
                            .await
                        {
                            return Ok(Self::error_result(error));
                        }
                    }
                    if let Some(height) = height {
                        let pane = match self.remote_session_first_pane(node, session).await {
                            Ok(pane) => pane,
                            Err(error) => return Ok(Self::error_result(error)),
                        };
                        if let Err(error) = self
                            .remote_tmux(
                                node,
                                vec![
                                    "resize-pane".into(),
                                    "-t".into(),
                                    pane,
                                    "-y".into(),
                                    height.to_string(),
                                ],
                                Duration::from_secs(20),
                            )
                            .await
                        {
                            return Ok(Self::error_result(error));
                        }
                    }
                    return Ok(Self::text_result(format!(
                        "Resized pane {} on node {}",
                        session, node
                    )));
                }
                match self
                    .local_node_call(|reply| LocalNodeMessage::ResizePane {
                        session: session.to_owned(),
                        width,
                        height,
                        reply,
                    })
                    .await
                {
                    Ok(msg) => Ok(Self::text_result(msg)),
                    Err(e) => Ok(Self::error_result(e)),
                }
            }
            _ => Err(McpError::invalid_request(
                format!("unknown tool: {}", request.name.as_ref()),
                None,
            )),
        };
        result
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        let profiles = self.profiles.read().unwrap();
        let resources: Vec<Resource> = profiles
            .keys()
            .map(|name| {
                RawResource {
                    uri: format!("profile://{}", name),
                    name: format!("Profile: {}", name),
                    title: Some(format!("CLI profile '{}'", name)),
                    description: Some(format!(
                        "Configuration for driving '{}' coding sessions",
                        name
                    )),
                    mime_type: Some("application/json".into()),
                    size: None,
                    icons: None,
                    meta: None,
                }
                .no_annotation()
            })
            .collect();
        Ok(ListResourcesResult::with_all_items(resources))
    }

    async fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, McpError> {
        let templates = vec![
            RawResourceTemplate {
                uri_template: "session://{session_name}/output".into(),
                name: "Session Output".into(),
                title: Some("Live session pane output".into()),
                description: Some("Read the current output of a tmux session pane".into()),
                mime_type: Some("text/plain".into()),
                icons: None,
            }
            .no_annotation(),
            RawResourceTemplate {
                uri_template: "session://{session_name}/info".into(),
                name: "Session Info".into(),
                title: Some("Tmux session metadata".into()),
                description: Some("Panes, windows, dimensions, and running commands".into()),
                mime_type: Some("text/plain".into()),
                icons: None,
            }
            .no_annotation(),
            RawResourceTemplate {
                uri_template: "session://{session_name}/scrollback".into(),
                name: "Session Scrollback".into(),
                title: Some("Full pane scrollback".into()),
                description: Some("Complete scrollback history of a tmux session pane".into()),
                mime_type: Some("text/plain".into()),
                icons: None,
            }
            .no_annotation(),
        ];
        Ok(ListResourceTemplatesResult::with_all_items(templates))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResult, McpError> {
        let uri = request.uri;
        if uri.starts_with("profile://") {
            let name = uri.trim_start_matches("profile://");
            let profiles = self.profiles.read().unwrap();
            if let Some(profile) = profiles.get(name) {
                let json = serde_json::to_string_pretty(profile)
                    .unwrap_or_else(|e| format!("{{\"error\":\"{}\"}}", e));
                Ok(ReadResourceResult::new(vec![
                    ResourceContents::TextResourceContents {
                        uri: uri.clone(),
                        mime_type: Some("application/json".into()),
                        text: json,
                        meta: None,
                    },
                ]))
            } else {
                Err(McpError::invalid_request(
                    format!("Profile '{}' not found", name),
                    None,
                ))
            }
        } else if uri.starts_with("session://") {
            let rest = uri.trim_start_matches("session://");
            let (session_name, resource_type) = rest.split_once('/').unwrap_or((rest, "output"));
            match resource_type {
                "info" => match self
                    .local_node_call(|reply| LocalNodeMessage::SessionInfo {
                        session: session_name.to_owned(),
                        reply,
                    })
                    .await
                {
                    Ok(text) => Ok(ReadResourceResult::new(vec![
                        ResourceContents::TextResourceContents {
                            uri: uri.clone(),
                            mime_type: Some("text/plain".into()),
                            text,
                            meta: None,
                        },
                    ])),
                    Err(e) => Err(McpError::invalid_request(e, None)),
                },
                "scrollback" => match self
                    .local_node_call(|reply| LocalNodeMessage::CaptureOutput {
                        session: session_name.to_owned(),
                        lines: None,
                        scrollback: true,
                        reply,
                    })
                    .await
                {
                    Ok(text) => Ok(ReadResourceResult::new(vec![
                        ResourceContents::TextResourceContents {
                            uri: uri.clone(),
                            mime_type: Some("text/plain".into()),
                            text: self.policy.limit_capture_output(text),
                            meta: None,
                        },
                    ])),
                    Err(e) => Err(McpError::invalid_request(e, None)),
                },
                _ => match self
                    .local_node_call(|reply| LocalNodeMessage::CaptureOutput {
                        session: session_name.to_owned(),
                        lines: Some(200),
                        scrollback: false,
                        reply,
                    })
                    .await
                {
                    Ok(text) => Ok(ReadResourceResult::new(vec![
                        ResourceContents::TextResourceContents {
                            uri: uri.clone(),
                            mime_type: Some("text/plain".into()),
                            text: self.policy.limit_capture_output(text),
                            meta: None,
                        },
                    ])),
                    Err(e) => Err(McpError::invalid_request(e, None)),
                },
            }
        } else {
            Err(McpError::invalid_request(
                format!("Unknown resource URI: {}", uri),
                None,
            ))
        }
    }

    async fn list_prompts(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListPromptsResult, McpError> {
        let prompts = vec![
            Prompt::new(
                "drive-coding-cli",
                Some("Best practices for driving a coding CLI through mmux"),
                Some(vec![
                    PromptArgument::new("profile")
                        .with_description("CLI profile to use (e.g. opencode, aider, codex)"),
                    PromptArgument::new("session").with_description("Tmux session name"),
                ]),
            ),
            Prompt::new(
                "debug-session",
                Some("How to diagnose and debug a tmux session"),
                Some(vec![
                    PromptArgument::new("session").with_description("Tmux session name")
                ]),
            ),
        ];
        Ok(ListPromptsResult::with_all_items(prompts))
    }

    async fn get_prompt(
        &self,
        request: GetPromptRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<GetPromptResult, McpError> {
        let messages = match request.name.as_str() {
            "drive-coding-cli" => {
                let profile = request
                    .arguments
                    .as_ref()
                    .and_then(|a| a.get("profile"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("opencode");
                let session = request
                    .arguments
                    .as_ref()
                    .and_then(|a| a.get("session"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("kimi_codex");
                vec![
                        PromptMessage::new(
                            PromptMessageRole::User,
                            PromptMessageContent::Text {
                                text: format!(
                                    "You are driving a coding CLI via mmux.\n\nProfile: {}\nSession: {}\n\nWorkflow:\n1. Start the session with start_coding_session using the profile-defined command\n2. Use coding_send to submit a prompt\n3. Use coding_wait_ready to wait for the CLI to finish processing\n4. Use coding_read to capture the output\n5. Use coding_action (approve/reject/cancel/escape) to interact\n\nTips:\n- check_state is a quick non-blocking way to see if the CLI is ready\n- resize_pane can help if the TUI layout is broken\n- capture_output with scrollback:true gets full history\n- Use wait_for with sentinel mode to detect specific output strings",
                                    profile, session
                                ),
                            },
                        ),
                    ]
            }
            "debug-session" => {
                let session = request
                    .arguments
                    .as_ref()
                    .and_then(|a| a.get("session"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("kimi_codex");
                vec![
                        PromptMessage::new(
                            PromptMessageRole::User,
                            PromptMessageContent::Text {
                                text: format!(
                                    "Debug tmux session '{}'. Follow this checklist:\n\n1. session_info — check if session exists, see panes/dimensions/commands\n2. capture_output with scrollback:true — see full history\n3. check_state with appropriate profile — is it busy or at a prompt?\n4. If stuck: send_key C-c (cancel), or send_key Escape\n5. If TUI is garbled: resize_pane to a reasonable size (e.g. 120x40)\n6. If the CLI crashed: kill_session then start_coding_session again\n\nCommon issues:\n- 'Session does not exist' → start_coding_session first\n- Output truncated → use scrollback:true or increase lines\n- Prompt not detected → verify profile.prompt_indicator matches the CLI",
                                    session
                                ),
                            },
                        ),
                    ]
            }
            _ => {
                return Err(McpError::invalid_request(
                    format!("Unknown prompt: {}", request.name),
                    None,
                ))
            }
        };
        Ok(GetPromptResult::new(messages))
    }
}

fn tool_schema(props: Value, required: Option<Vec<&str>>) -> Map<String, Value> {
    let mut map = Map::new();
    map.insert("type".into(), "object".into());
    map.insert("properties".into(), props);
    if let Some(req) = required {
        map.insert("required".into(), json!(req));
    }
    map
}

async fn security_middleware(request: Request<Body>, next: Next) -> Response {
    // Block cross-site browser requests via Sec-Fetch-Site
    if let Some(site) = request
        .headers()
        .get("sec-fetch-site")
        .and_then(|v| v.to_str().ok())
    {
        if site != "none" && site != "same-origin" {
            return Response::builder()
                .status(StatusCode::FORBIDDEN)
                .body(Body::from("Forbidden: cross-site request"))
                .unwrap();
        }
    }

    // Reject browser-ish Content-Type for POSTs
    if request.method() == axum::http::Method::POST {
        let ct = request
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        let allowed = ct.starts_with("application/json")
            || ct.starts_with("text/event-stream")
            || ct.starts_with("application/proto")
            || ct.starts_with("application/connect+")
            || ct.starts_with("application/grpc");
        if !allowed {
            return Response::builder()
                .status(StatusCode::UNSUPPORTED_MEDIA_TYPE)
                .body(Body::from(
                    "Unsupported Media Type: expected MCP JSON or ConnectRPC content type",
                ))
                .unwrap();
        }
    }

    next.run(request).await
}

async fn auth_middleware(
    request: Request<Body>,
    next: Next,
    token: Option<Arc<String>>,
) -> Response {
    if let Some(ref expected) = token {
        let auth = request
            .headers()
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        let expected_auth = format!("Bearer {}", expected);
        if !constant_time_eq(auth.as_bytes(), expected_auth.as_bytes()) {
            return Response::builder()
                .status(StatusCode::UNAUTHORIZED)
                .body(Body::from("Unauthorized"))
                .unwrap();
        }
    }
    next.run(request).await
}

struct NodeRegistryConnectService {
    registry: ActorRef<NodeRegistryMessage>,
}

fn invalid_argument(error: impl ToString) -> ConnectError {
    ConnectError::invalid_argument(error.to_string())
}

fn internal_error(error: impl ToString) -> ConnectError {
    ConnectError::internal(error.to_string())
}

#[allow(refining_impl_trait)]
impl MmuxNodeRegistryService for NodeRegistryConnectService {
    async fn register_node(
        &self,
        _ctx: ConnectRequestContext,
        request: OwnedRegisterNodeRequestView,
    ) -> ServiceResult<wire_proto::RegisterNodeResponse> {
        let request = register_node_request_from_proto(request.to_owned_message())
            .map_err(invalid_argument)?;
        let response = match registry_call(
            &self.registry,
            |reply| NodeRegistryMessage::Register {
                descriptor: request.descriptor,
                reply,
            },
            None,
        )
        .await
        {
            Ok(message) => RegisterNodeResponse {
                accepted: true,
                message,
            },
            Err(message) => RegisterNodeResponse {
                accepted: false,
                message,
            },
        };
        ConnectResponse::ok(register_node_response_to_proto(response))
    }

    async fn pull_commands(
        &self,
        _ctx: ConnectRequestContext,
        request: OwnedPullCommandsRequestView,
    ) -> ServiceResult<wire_proto::PullCommandsResponse> {
        let request = pull_commands_request_from_proto(request.to_owned_message());
        let commands = registry_call(
            &self.registry,
            |reply| NodeRegistryMessage::Pull {
                node_id: request.node_id,
                reply,
            },
            None,
        )
        .await
        .map_err(internal_error)?;
        ConnectResponse::ok(pull_commands_response_to_proto(PullCommandsResponse {
            commands,
        }))
    }

    async fn submit_command_result(
        &self,
        _ctx: ConnectRequestContext,
        request: OwnedSubmitCommandResultRequestView,
    ) -> ServiceResult<wire_proto::SubmitCommandResultResponse> {
        let request = submit_command_result_request_from_proto(request.to_owned_message())
            .map_err(invalid_argument)?;
        let accepted = registry_call(
            &self.registry,
            |reply| NodeRegistryMessage::SubmitResult {
                node_id: request.node_id,
                command_id: request.command_id,
                result: request.result,
                reply,
            },
            None,
        )
        .await
        .is_ok();
        ConnectResponse::ok(submit_command_result_response_to_proto(
            SubmitCommandResultResponse { accepted },
        ))
    }

    async fn heartbeat(
        &self,
        _ctx: ConnectRequestContext,
        request: OwnedHeartbeatRequestView,
    ) -> ServiceResult<wire_proto::HeartbeatResponse> {
        let request =
            heartbeat_request_from_proto(request.to_owned_message()).map_err(invalid_argument)?;
        let accepted = registry_call(
            &self.registry,
            |reply| NodeRegistryMessage::Heartbeat {
                node_id: request.node_id,
                status: request.status,
                reply,
            },
            None,
        )
        .await
        .is_ok();
        ConnectResponse::ok(heartbeat_response_to_proto(mmux_wire::HeartbeatResponse {
            accepted,
        }))
    }
}

async fn run_mcp_http_server(
    bind: SocketAddr,
    profiles: ProfileRegistry,
    policy: SecurityPolicy,
    token: Option<String>,
    enable_local_node: bool,
) -> Result<(), String> {
    let local_node = if enable_local_node {
        let (local_node, _local_node_handle) = Actor::spawn(None, LocalNodeActor, ())
            .await
            .map_err(|error| format!("failed to start local node actor: {}", error))?;
        Some(local_node)
    } else {
        None
    };
    let (registry, _registry_handle) = Actor::spawn(None, NodeRegistryActor, enable_local_node)
        .await
        .map_err(|error| format!("failed to start node registry actor: {}", error))?;
    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .map_err(|e| format!("failed to bind: {}", e))?;

    let service_policy = policy.clone();
    let request_body_limit = policy.max_request_bytes;
    let service_registry = registry.clone();
    let service: StreamableHttpService<TmuxMcpServer, LocalSessionManager> =
        StreamableHttpService::new(
            move || {
                Ok(TmuxMcpServer::new(
                    profiles.clone(),
                    service_policy.clone(),
                    local_node.clone(),
                    service_registry.clone(),
                ))
            },
            LocalSessionManager::default().into(),
            StreamableHttpServerConfig::default()
                .with_allowed_origins(loopback_allowed_origins())
                .with_stateful_mode(false)
                .with_json_response(true)
                .disable_allowed_hosts(),
        );

    let token_arc = token.map(Arc::new);
    let has_token = token_arc.is_some();
    let api_token_arc = token_arc.clone();
    let wire_token_arc = token_arc.clone();
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let api_router = axum::Router::new()
        .nest_service("/mcp", service)
        .layer(DefaultBodyLimit::max(request_body_limit))
        .layer(cors)
        .layer(middleware::from_fn(move |req, next| {
            let t = api_token_arc.clone();
            auth_middleware(req, next, t)
        }))
        .layer(middleware::from_fn(security_middleware));

    let wire_service = ConnectRpcService::new(MmuxNodeRegistryServiceServer::new(
        NodeRegistryConnectService {
            registry: registry.clone(),
        },
    ));
    let wire_router = axum::Router::new()
        .fallback_service(wire_service)
        .layer(DefaultBodyLimit::max(request_body_limit))
        .layer(middleware::from_fn(move |req, next| {
            let t = wire_token_arc.clone();
            auth_middleware(req, next, t)
        }))
        .layer(middleware::from_fn(security_middleware));

    let health_router = axum::Router::new().route("/health", axum::routing::get(|| async { "OK" }));

    let router = health_router.merge(api_router).merge(wire_router);

    println!("mmux MCP HTTP server listening on http://{}/mcp", bind);
    println!("  Security mode: {:?}", policy.mode);
    if let Some(root) = policy.workspace_root.as_ref() {
        println!("  Workspace root: {}", root.display());
    }
    if has_token {
        println!("  Bearer token authentication enabled");
    } else {
        println!("  Warning: no bearer token set. Use --token to prevent unauthorized access.");
    }

    axum::serve(listener, router)
        .await
        .map_err(|e| format!("server error: {}", e))
}

fn loopback_allowed_origins() -> [&'static str; 6] {
    [
        "http://localhost",
        "https://localhost",
        "http://127.0.0.1",
        "https://127.0.0.1",
        "http://[::1]",
        "https://[::1]",
    ]
}

fn is_loopback_bind(bind: SocketAddr) -> bool {
    bind.ip().is_loopback()
}

fn nearest_existing_ancestor(path: &Path) -> PathBuf {
    let mut current = path;
    loop {
        if current.exists() {
            return current.to_path_buf();
        }
        match current.parent() {
            Some(parent) => current = parent,
            None => return PathBuf::from("."),
        }
    }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (left, right) in a.iter().zip(b.iter()) {
        diff |= left ^ right;
    }
    diff == 0
}

fn resolve_token(cli: &Cli, policy: &SecurityPolicy) -> Result<Option<String>, String> {
    if let Some(token) = cli.token.as_ref() {
        if token.is_empty() {
            return Err("--token must not be empty".into());
        }
        return Ok(Some(token.clone()));
    }

    if let Some(path) = cli.token_file.as_ref() {
        let token_path = Path::new(path);
        let real_token_path = std::fs::canonicalize(token_path)
            .map_err(|e| format!("failed to canonicalize token file '{}': {}", path, e))?;
        if let Some(root) = policy.workspace_root.as_ref() {
            if real_token_path.starts_with(root) {
                return Err(format!(
                    "token file '{}' is inside the workspace root; place it under /run/secrets or another non-workspace path",
                    path
                ));
            }
        }
        warn_if_token_file_permissions_are_loose(&real_token_path);
        let token = std::fs::read_to_string(&real_token_path)
            .map_err(|e| format!("failed to read token file '{}': {}", path, e))?
            .trim()
            .to_owned();
        if token.is_empty() {
            return Err(format!("token file '{}' is empty", path));
        }
        return Ok(Some(token));
    }

    match std::env::var(&cli.token_env) {
        Ok(token) if !token.is_empty() => Ok(Some(token)),
        Ok(_) => Err(format!("{} is set but empty", cli.token_env)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(e) => Err(format!("failed to read {}: {}", cli.token_env, e)),
    }
}

#[cfg(unix)]
fn warn_if_token_file_permissions_are_loose(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(meta) = std::fs::metadata(path) {
        let mode = meta.permissions().mode();
        if mode & 0o077 != 0 {
            eprintln!(
                "Warning: token file '{}' is readable or writable by group/other; prefer mode 0400 or 0440.",
                path.display()
            );
        }
    }
}

#[cfg(not(unix))]
fn warn_if_token_file_permissions_are_loose(_path: &Path) {}

fn validate_startup_security(
    bind: SocketAddr,
    token: Option<&String>,
    policy: &SecurityPolicy,
    allow_remote_without_token: bool,
) -> Result<(), String> {
    if is_loopback_bind(bind) || token.is_some() {
        return Ok(());
    }
    if allow_remote_without_token {
        eprintln!(
            "Warning: mmux is bound to {} without authentication in {:?} mode. Only use this behind localhost-only port forwarding or another trusted network boundary.",
            bind, policy.mode
        );
        return Ok(());
    }
    Err(format!(
        "refusing to bind unauthenticated mmux to {} in {:?} mode; set --token, --token-file, or MMUX_TOKEN, or deliberately use --allow-remote-without-token behind a trusted network boundary",
        bind, policy.mode
    ))
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Entry point
// ═══════════════════════════════════════════════════════════════════════════════

pub fn main_entry() {
    main_entry_from(std::env::args_os());
}

pub fn main_entry_from<I, T>(args: I)
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let cli = Cli::parse_from(args);
    let policy = SecurityPolicy::new(&cli).unwrap_or_else(|e| {
        eprintln!("Security policy error: {}", e);
        std::process::exit(1);
    });
    let token = resolve_token(&cli, &policy).unwrap_or_else(|e| {
        eprintln!("Token error: {}", e);
        std::process::exit(1);
    });

    // Resolve node profile config path: --node-config/--config > mmux.toml in cwd > built-in defaults
    let node_config = cli.node_config.clone().or_else(|| cli.config.clone());
    let profiles = if let Some(path) = node_config {
        mmux_node::load_profiles_from_config(&path).unwrap_or_else(|e| {
            eprintln!("Node profile config error: {}", e);
            std::process::exit(1);
        })
    } else if let Some(path) = mmux_node::default_profile_config_in_cwd() {
        mmux_node::load_profiles_from_config(&path).unwrap_or_else(|e| {
            eprintln!("Node profile config error: {}", e);
            std::process::exit(1);
        })
    } else {
        println!(
            "No node profile config found. Using built-in profiles (opencode, aider, codex, generic)."
        );
        mmux_node::default_profiles()
    };

    // MCP HTTP server mode
    let bind: SocketAddr = format!("{}:{}", cli.host, cli.port)
        .parse()
        .unwrap_or_else(|_| "127.0.0.1:3000".parse().unwrap());
    validate_startup_security(
        bind,
        token.as_ref(),
        &policy,
        cli.allow_remote_without_token,
    )
    .unwrap_or_else(|e| {
        eprintln!("Security policy error: {}", e);
        std::process::exit(1);
    });

    let rt = tokio::runtime::Runtime::new().expect("failed to create tokio runtime");
    if let Err(e) = rt.block_on(run_mcp_http_server(
        bind,
        profiles,
        policy,
        token,
        cli.enable_local_node,
    )) {
        eprintln!("MCP server error: {}", e);
        std::process::exit(1);
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Tests
// ═══════════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
    use std::fs;

    #[test]
    fn test_load_profile_from_toml() {
        let toml = r#"
name = "test"
prompt_indicator = ">"
busy_indicators = ["Loading"]
approve_keys = "y"
reject_keys = "n"
cancel_keys = "C-c"
escape_keys = "Escape"
"#;
        let profile = load_profile_from_toml(toml).unwrap();
        assert_eq!(profile.name, "test");
        assert_eq!(profile.prompt_indicator, ">");
        assert_eq!(profile.busy_indicators, vec!["Loading"]);
        assert_eq!(profile.approve_keys, "y");
    }

    #[test]
    fn test_load_profile_with_startup_dismiss() {
        let toml = r#"
name = "test"
prompt_indicator = ">"
busy_indicators = []
approve_keys = "y"
reject_keys = "n"
cancel_keys = "C-c"
escape_keys = "Escape"

[startup_dismiss]
key = "Escape"
triggers = ["Starting MCP servers"]
"#;
        let profile = load_profile_from_toml(toml).unwrap();
        assert!(profile.startup_dismiss.is_some());
        let dismiss = profile.startup_dismiss.unwrap();
        assert_eq!(dismiss.key, "Escape");
        assert_eq!(dismiss.triggers, vec!["Starting MCP servers"]);
    }

    #[test]
    fn test_load_profile_invalid_toml() {
        let toml = "not valid toml ::";
        let result = load_profile_from_toml(toml);
        assert!(result.is_err());
    }

    #[test]
    fn test_detect_compression_gzip() {
        let bytes = vec![0x1f, 0x8b, 0x08];
        assert_eq!(detect_compression(&bytes), Some("gzip".into()));
    }

    #[test]
    fn test_detect_compression_zstd() {
        let bytes = vec![0x28, 0xb5, 0x2f, 0xfd];
        assert_eq!(detect_compression(&bytes), Some("zstd".into()));
    }

    #[test]
    fn test_detect_compression_none() {
        let bytes = b"hello world".to_vec();
        assert_eq!(detect_compression(&bytes), None);
    }

    #[test]
    fn test_detect_mime_type_by_extension() {
        assert_eq!(
            detect_mime_type(Path::new("test.rs"), b""),
            "text/x-rustsrc"
        );
        assert_eq!(
            detect_mime_type(Path::new("test.json"), b""),
            "application/json"
        );
        assert_eq!(detect_mime_type(Path::new("test.png"), b""), "image/png");
    }

    #[test]
    fn test_detect_mime_type_by_magic() {
        let png = vec![0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
        assert_eq!(detect_mime_type(Path::new("unknown"), &png), "image/png");
    }

    #[test]
    fn test_read_save_file_roundtrip() {
        let tmp = std::env::temp_dir().join("mmux_test_roundtrip.txt");
        let content = "Hello, mmux!";

        // Save as utf-8
        let result = save_file_impl(tmp.to_str().unwrap(), content, "utf-8", false, None).unwrap();
        assert_eq!(result.bytes_written, content.len());

        // Read back
        let read = read_file_impl(tmp.to_str().unwrap(), None, None).unwrap();
        assert_eq!(read.content, content);
        assert_eq!(read.encoding, "utf-8");
        assert_eq!(read.mime_type, "text/plain");

        // Clean up
        let _ = fs::remove_file(&tmp);
    }

    #[test]
    fn test_read_save_file_base64() {
        let tmp = std::env::temp_dir().join("mmux_test_base64.bin");
        let bytes = vec![0x00, 0x01, 0x02, 0xff];
        let b64 = BASE64.encode(&bytes);

        // Save as base64
        save_file_impl(tmp.to_str().unwrap(), &b64, "base64", false, None).unwrap();

        // Read back
        let read = read_file_impl(tmp.to_str().unwrap(), None, None).unwrap();
        assert_eq!(read.encoding, "base64");

        // Clean up
        let _ = fs::remove_file(&tmp);
    }

    #[test]
    fn test_tool_schema_required_fields() {
        let schema = tool_schema(
            json!({
                "session": { "type": "string" },
                "text": { "type": "string" }
            }),
            Some(vec!["text"]),
        );
        assert!(schema.contains_key("required"));
        let req = schema.get("required").unwrap();
        assert_eq!(req, &json!(["text"]));
    }

    #[test]
    fn test_tool_schema_no_required() {
        let schema = tool_schema(
            json!({
                "session": { "type": "string" }
            }),
            None,
        );
        assert!(!schema.contains_key("required"));
    }

    #[test]
    fn test_clean_exec_output() {
        // Typical shell output: command line, output, empty lines, prompt
        let lines = vec!["user@host:~$ echo hello", "hello", "", "user@host:~$ "];
        assert_eq!(clean_exec_output(lines), "hello");

        // Output with multiple lines
        let lines = vec![
            "user@host:~$ ls -la",
            "total 100",
            "drwxr-xr-x  5 user user  4096 May 28 21:19 .",
            "user@host:~$ ",
        ];
        assert_eq!(
            clean_exec_output(lines),
            "total 100\ndrwxr-xr-x  5 user user  4096 May 28 21:19 ."
        );

        // Empty output (just prompt)
        let lines = vec!["user@host:~$ pwd", "user@host:~$ "];
        assert_eq!(clean_exec_output(lines), "");

        // No command line (edge case) — first non-empty line is still treated as command
        let lines = vec!["just output", "user@host:~$ "];
        assert_eq!(clean_exec_output(lines), "");
    }
}
