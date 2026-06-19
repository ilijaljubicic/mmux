use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use clap::{CommandFactory, Parser};
use connectrpc::{
    ConnectError, ConnectRpcService, RequestContext as ConnectRequestContext,
    Response as ConnectResponse, ServiceResult,
};
use mmux_controller_core::{
    orchestration::{
        CreatePlan, CreateProject, CreateTask, CreateTaskEdge, NodeId, OrchestrationCounts,
        OrchestrationState, OrchestrationStatus, PlanId, PlanStatus, ProjectId, ProjectStatus,
        SessionCleanupCandidate, SessionId, Task, TaskEdge, TaskEdgeKind, TaskId, TaskScope,
        TaskSession, TaskStatus, UpdatePlan, UpdateTask, UpdateTaskScope,
    },
    NodeRegistry, NodeWireAuthContext, NodeWireAuthMode, NodeWireAuthPolicy, NodeWireIdentity,
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
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::io::BufReader;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::{
    body::Body,
    extract::{connect_info::Connected, ConnectInfo, DefaultBodyLimit},
    http::{Request, StatusCode},
    middleware::{self, Next},
    response::Response,
    serve::IncomingStream,
};
use ractor::{rpc::CallResult, Actor, ActorProcessingErr, ActorRef, ActorRuntime, RpcReplyPort};
use rustls::{
    pki_types::{CertificateDer, PrivateKeyDer},
    server::WebPkiClientVerifier,
    RootCertStore, ServerConfig,
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::TcpStream,
    task::JoinHandle,
};
use tokio_rustls::{server::TlsStream, TlsAcceptor};
use tower_http::cors::{Any, CorsLayer};
use x509_parser::{extensions::GeneralName, prelude::FromDer};

mod orchestration_actor;
mod runtime;
mod store;

const DEFAULT_CODING_READY_TIMEOUT_SECONDS: u64 = 120;
const DEFAULT_NODE_EXECUTION_ACTOR_CALL_TIMEOUT: Duration = Duration::from_secs(30);
const ORCHESTRATION_SESSION_PREFIX: &str = "mmux";
const MAX_ORCHESTRATION_TASK_SLUG_LEN: usize = 40;
const MAX_ORCHESTRATION_KIND_LEN: usize = 24;
const MAX_ORCHESTRATION_SUFFIX_LEN: usize = 8;
const CODING_TASK_SEND_PROMPT: &str = include_str!("prompts/coding_task_send.md");
const CODING_VALIDATE_SEND_PROMPT: &str = include_str!("prompts/coding_validate_send.md");
const CODING_REVIEW_SEND_PROMPT: &str = include_str!("prompts/coding_review_send.md");
const CODING_QUALITY_GUARD_SEND_PROMPT: &str = include_str!("prompts/coding_quality_guard_send.md");

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
    #[arg(long, help = "Bearer token protecting the public MCP endpoint.")]
    mcp_token: Option<String>,
    #[arg(
        long,
        help = "Path to a file containing the MCP bearer token. Prefer /run/secrets paths in containers."
    )]
    mcp_token_file: Option<String>,
    #[arg(
        long,
        default_value = "MMUX_MCP_TOKEN",
        help = "Environment variable to read the MCP bearer token from when --mcp-token/--mcp-token-file are not set."
    )]
    mcp_token_env: String,
    #[arg(long, help = "Bearer token protecting node wire RPC endpoints.")]
    wire_token: Option<String>,
    #[arg(
        long,
        help = "Require runtime-verified mTLS identity for node wire RPC. Mutually exclusive with explicit wire token flags."
    )]
    wire_mtls: bool,
    #[arg(
        long,
        help = "PEM server certificate chain used when --wire-mtls is set."
    )]
    tls_cert: Option<String>,
    #[arg(long, help = "PEM server private key used when --wire-mtls is set.")]
    tls_key: Option<String>,
    #[arg(
        long,
        help = "PEM CA certificate(s) that sign node client certificates."
    )]
    wire_client_ca: Option<String>,
    #[arg(
        long,
        help = "Path to a file containing the node wire bearer token. Prefer /run/secrets paths in containers."
    )]
    wire_token_file: Option<String>,
    #[arg(
        long,
        default_value = "MMUX_WIRE_TOKEN",
        help = "Environment variable to read the node wire bearer token from when --wire-token/--wire-token-file are not set."
    )]
    wire_token_env: String,
    #[arg(
        long,
        help = "Directory for local runtime state. The embedded local tmux socket is a deterministic short runtime path derived from this store path."
    )]
    store_path: Option<PathBuf>,
    #[arg(
        long,
        help = "Path to tmux.conf used by the embedded local node. Only valid with --enable-local-node."
    )]
    tmux_config: Option<PathBuf>,
    #[arg(
        long,
        help = "Permit MCP without bearer auth and ignore MMUX_MCP_TOKEN. Intended only behind localhost-only port forwarding."
    )]
    allow_remote_without_mcp_token: bool,
    #[arg(
        long,
        help = "Permit node wire RPC without bearer auth and ignore MMUX_WIRE_TOKEN. Intended only for development or trusted private tunnels."
    )]
    allow_unauthenticated_node_wire: bool,
    #[arg(
        long,
        help = "Enable admin-only MCP tools that create or change project boundaries."
    )]
    enable_admin_tools: bool,
    #[arg(long, default_value_t = 4 * 1024 * 1024, help = "Maximum bytes returned by read_file.")]
    max_read_bytes: usize,
    #[arg(long, default_value_t = 4 * 1024 * 1024, help = "Maximum decoded bytes accepted by save_file.")]
    max_write_bytes: usize,
    #[arg(
        long,
        default_value_t = 120.0,
        help = "Maximum wait timeout accepted by wait tools."
    )]
    max_timeout_seconds: f64,
    #[arg(long, default_value_t = 2 * 1024 * 1024, help = "Maximum MCP HTTP request body size.")]
    max_request_bytes: usize,
    #[arg(long, default_value_t = 2 * 1024 * 1024, help = "Maximum bytes returned by terminal capture tools.")]
    max_capture_bytes: usize,
    #[arg(
        long,
        value_name = "PROFILE[,PROFILE...]",
        help = "Comma-separated coder profiles enabled on the MCP surface. Defaults to all built-in profiles."
    )]
    enabled_coder_profiles: Option<String>,
    #[arg(
        long,
        value_name = "PROFILE",
        help = "Default coder profile for omitted profile arguments. Defaults to the first enabled built-in profile."
    )]
    default_coder_profile: Option<String>,
    #[arg(
        long,
        help = "Start the built-in local tmux node inside the controller process."
    )]
    enable_local_node: bool,
    #[arg(
        long,
        help = "Attach an embedded Microsandbox node inside the controller process. Requires --sandbox-name for an existing running sandbox."
    )]
    enable_microsandbox_node: bool,
    #[arg(
        long,
        help = "Existing running Microsandbox sandbox name used with --enable-microsandbox-node."
    )]
    sandbox_name: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) enum EmbeddedNodeConfig {
    Local {
        store_path: Option<PathBuf>,
        tmux_config: Option<PathBuf>,
    },
    Microsandbox {
        sandbox_name: String,
    },
}

impl EmbeddedNodeConfig {
    fn display_name(&self) -> String {
        match self {
            Self::Local { .. } => "Local tmux node".into(),
            Self::Microsandbox { sandbox_name } => {
                format!("Microsandbox node '{}'", sandbox_name)
            }
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ControllerPolicy {
    enable_admin_tools: bool,
    max_read_bytes: usize,
    max_write_bytes: usize,
    max_timeout_seconds: f64,
    max_request_bytes: usize,
    max_capture_bytes: usize,
}

impl ControllerPolicy {
    fn new(cli: &Cli) -> Result<Self, String> {
        Ok(Self {
            enable_admin_tools: cli.enable_admin_tools,
            max_read_bytes: cli.max_read_bytes,
            max_write_bytes: cli.max_write_bytes,
            max_timeout_seconds: cli.max_timeout_seconds,
            max_request_bytes: cli.max_request_bytes,
            max_capture_bytes: cli.max_capture_bytes,
        })
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

    fn ensure_admin_tools_enabled(&self, tool_name: &str) -> Result<(), McpError> {
        if self.enable_admin_tools {
            return Ok(());
        }
        Err(McpError::invalid_request(
            format!("{tool_name} requires controller flag --enable-admin-tools"),
            None,
        ))
    }
}

const SESSION_LIST_FORMAT: &str =
    "#{session_name}|#{session_windows}|#{session_attached}|#{session_created}";
const SESSION_INFO_LIST_FORMAT: &str = "#{session_name}|#{session_created}";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct SessionListEntry {
    node: String,
    session: String,
    windows: Option<u64>,
    attached: Option<u64>,
    created_at_seconds: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct LocalSessionInfo {
    session: String,
    created_at_seconds: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OrchestrationCleanupZombiesArgs {
    #[serde(default = "default_cleanup_dry_run")]
    dry_run: bool,
    older_than_seconds: Option<u64>,
    #[serde(default = "default_cleanup_node")]
    node: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OrchestrationPruneStoreArgs {
    #[serde(default = "default_cleanup_dry_run")]
    dry_run: bool,
    #[serde(default)]
    sessions_only: bool,
    older_than_days: Option<u64>,
    #[serde(default = "default_cleanup_node")]
    node: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum LocalStartupReconciliationAction {
    Recreate { record: TaskSession },
    Missing { key: String, reason: String },
    Historical { key: String },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct OrchestrationCleanupZombiesResult {
    node: String,
    dry_run: bool,
    candidates: Vec<SessionCleanupCandidate>,
    killed: Vec<String>,
    warnings: Vec<String>,
}

fn default_cleanup_dry_run() -> bool {
    true
}

fn default_cleanup_node() -> String {
    "local".into()
}

fn is_orchestration_owned_session(session: &str) -> bool {
    session.starts_with(&format!("{ORCHESTRATION_SESSION_PREFIX}-"))
}

fn runtime_session_key(node_id: &str, session: &str) -> String {
    format!("{node_id}:{session}")
}

fn cleanup_candidates_from_live_sessions(
    node_id: &str,
    live_sessions: &[LocalSessionInfo],
    durable_keys: &HashSet<String>,
    older_than_seconds: Option<u64>,
    now_seconds: u64,
) -> Vec<SessionCleanupCandidate> {
    let mut candidates = live_sessions
        .iter()
        .filter(|live| is_orchestration_owned_session(&live.session))
        .filter(|live| !durable_keys.contains(&runtime_session_key(node_id, &live.session)))
        .filter(|live| {
            older_than_seconds
                .map(|older_than_seconds| {
                    live.created_at_seconds
                        .and_then(|created| now_seconds.checked_sub(created))
                        .is_some_and(|age| age >= older_than_seconds)
                })
                .unwrap_or(true)
        })
        .map(|live| SessionCleanupCandidate {
            node_id: node_id.to_owned(),
            session: live.session.clone(),
            reason: "live mmux-* session is absent from durable task session storage".into(),
            created_at_ms: live
                .created_at_seconds
                .map(|seconds| seconds.saturating_mul(1000)),
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        left.node_id
            .cmp(&right.node_id)
            .then_with(|| left.session.cmp(&right.session))
    });
    candidates
}

fn is_zombie_cleanup_warning(warning: &str) -> bool {
    warning.starts_with("live local session '")
        && warning.ends_with("' is a zombie cleanup candidate")
}

fn safe_cleanup_kill_targets(
    candidates: &[SessionCleanupCandidate],
    durable_keys: &HashSet<String>,
) -> (Vec<String>, Vec<String>) {
    let mut targets = Vec::new();
    let mut warnings = Vec::new();
    for candidate in candidates {
        if !is_orchestration_owned_session(&candidate.session) {
            warnings.push(format!(
                "refusing to kill non-mmux session '{}'",
                candidate.session
            ));
            continue;
        }
        if durable_keys.contains(&runtime_session_key(&candidate.node_id, &candidate.session)) {
            warnings.push(format!(
                "refusing to kill durable session '{}'",
                candidate.session
            ));
            continue;
        }
        targets.push(candidate.session.clone());
    }
    targets.sort();
    targets.dedup();
    (targets, warnings)
}

fn decorate_orchestration_status_with_local_runtime(
    status: &mut OrchestrationStatus,
    live_sessions: &[LocalSessionInfo],
    node_id: &str,
    older_than_seconds: Option<u64>,
    now_seconds: u64,
) {
    let live_session_names = live_sessions
        .iter()
        .map(|live| live.session.as_str())
        .collect::<HashSet<_>>();
    let durable_keys = status
        .sessions
        .iter()
        .map(|session| runtime_session_key(&session.node_id, &session.session))
        .collect::<HashSet<_>>();

    for session in &mut status.sessions {
        session.runtime_state = Some(
            if session.node_id == node_id {
                if live_session_names.contains(session.session.as_str()) {
                    "live"
                } else {
                    "missing"
                }
            } else {
                "unknown"
            }
            .into(),
        );
    }

    status.cleanup_candidates = cleanup_candidates_from_live_sessions(
        node_id,
        live_sessions,
        &durable_keys,
        older_than_seconds,
        now_seconds,
    );
    for candidate in &status.cleanup_candidates {
        status.warnings.push(format!(
            "live local session '{}' is a zombie cleanup candidate",
            candidate.session
        ));
    }
    status.counts = summarize_orchestration_counts(status);
}

fn durable_session_keys(
    state: &mmux_controller_core::orchestration::OrchestrationState,
) -> HashSet<String> {
    state
        .tasks
        .values()
        .filter_map(|task| task.session.as_ref().map(TaskSession::key))
        .collect()
}

fn active_attached_task_exists(
    state: &mmux_controller_core::orchestration::OrchestrationState,
    task_id: &TaskId,
) -> bool {
    state
        .tasks
        .get(task_id)
        .is_some_and(|task| !task.status.is_finished())
}

fn plan_local_startup_reconciliation(
    state: &mmux_controller_core::orchestration::OrchestrationState,
    live_sessions: &[LocalSessionInfo],
    profiles: &ProfileRegistry,
) -> Vec<LocalStartupReconciliationAction> {
    let live_session_names = live_sessions
        .iter()
        .map(|live| live.session.as_str())
        .collect::<HashSet<_>>();
    let mut actions = Vec::new();

    for (task_id, record) in state
        .tasks
        .iter()
        .filter_map(|(task_id, task)| task.session.as_ref().map(|session| (task_id, session)))
    {
        if record.node_id.0 != "local" || live_session_names.contains(record.session.0.as_str()) {
            continue;
        }

        let key = record.key();
        if !active_attached_task_exists(state, task_id) {
            actions.push(LocalStartupReconciliationAction::Historical { key });
            continue;
        }

        let Some(profile) = mmux_node::get_profile(profiles, &record.profile) else {
            actions.push(LocalStartupReconciliationAction::Missing {
                key,
                reason: format!("profile '{}' is not loaded", record.profile),
            });
            continue;
        };
        if let Err(error) = profile_launch_command(&profile, record.bypass_permissions) {
            actions.push(LocalStartupReconciliationAction::Missing { key, reason: error });
            continue;
        }
        if let Err(error) = profile_launch_strategy(&profile) {
            actions.push(LocalStartupReconciliationAction::Missing { key, reason: error });
            continue;
        }

        actions.push(LocalStartupReconciliationAction::Recreate {
            record: record.clone(),
        });
    }

    actions.sort_by(|left, right| {
        let left_key = reconciliation_action_key(left);
        let right_key = reconciliation_action_key(right);
        left_key.cmp(&right_key)
    });
    actions
}

fn reconciliation_action_key(action: &LocalStartupReconciliationAction) -> String {
    match action {
        LocalStartupReconciliationAction::Recreate { record } => record.key(),
        LocalStartupReconciliationAction::Missing { key, .. }
        | LocalStartupReconciliationAction::Historical { key } => key.clone(),
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

#[cfg(test)]
fn read_file_impl(
    path: &str,
    offset: Option<u64>,
    limit: Option<usize>,
) -> Result<ReadFileResult, String> {
    mmux_node::read_file_impl(path, offset, limit)
}

#[cfg(test)]
fn save_file_impl(
    path: &str,
    content: &str,
    encoding: &str,
    append: bool,
    max_bytes: Option<usize>,
) -> Result<SaveFileResult, String> {
    mmux_node::save_file_impl(path, content, encoding, append, max_bytes)
}

fn profile_launch_command(profile: &CliProfile, bypass_permissions: bool) -> Result<&str, String> {
    mmux_node::profiles::launch_command(profile, bypass_permissions)
}

fn profile_launch_strategy(profile: &CliProfile) -> Result<&str, String> {
    mmux_node::profiles::launch_strategy(profile)
}

fn profile_text_mode(profile: &CliProfile) -> Result<&str, String> {
    mmux_node::profiles::text_mode(profile)
}

#[derive(Debug)]
struct ResolvedCoderProfiles {
    profiles: ProfileRegistry,
    default_profile: String,
}

fn first_enabled_builtin_profile(profiles: &ProfileRegistry) -> Option<String> {
    mmux_node::profiles::BuiltinProfile::all()
        .into_iter()
        .map(|profile| profile.name())
        .find(|name| profiles.contains_key(*name))
        .map(str::to_owned)
}

fn resolve_coder_profiles(cli: &Cli) -> Result<ResolvedCoderProfiles, String> {
    let profiles = mmux_node::default_profiles();
    let profiles = if let Some(raw_enabled) = cli.enabled_coder_profiles.as_deref() {
        let mut enabled = HashMap::new();
        for raw_name in raw_enabled.split(',') {
            let name = raw_name.trim();
            if name.is_empty() {
                return Err("--enabled-coder-profiles contains an empty profile name".into());
            }
            let profile = mmux_node::get_profile(&profiles, name).ok_or_else(|| {
                let mut available = profiles.keys().cloned().collect::<Vec<_>>();
                available.sort();
                format!(
                    "unknown coder profile '{}' in --enabled-coder-profiles; available profiles: {}",
                    name,
                    available.join(",")
                )
            })?;
            enabled.insert(profile.name.clone(), profile);
        }

        if enabled.is_empty() {
            return Err("--enabled-coder-profiles must enable at least one profile".into());
        }

        Arc::new(enabled)
    } else {
        profiles
    };

    let default_profile = if let Some(raw_name) = cli.default_coder_profile.as_deref() {
        let name = raw_name.trim();
        if name.is_empty() {
            return Err("--default-coder-profile must not be empty".into());
        }
        let profile = mmux_node::get_profile(&profiles, name).ok_or_else(|| {
            format!(
                "--default-coder-profile '{}' is not enabled; enable it with --enabled-coder-profiles or choose an enabled profile",
                name
            )
        })?;
        profile.name
    } else {
        first_enabled_builtin_profile(&profiles)
            .ok_or_else(|| "no enabled built-in coder profiles are available".to_string())?
    };

    Ok(ResolvedCoderProfiles {
        profiles,
        default_profile,
    })
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

fn profile_is_busy(output: &str, profile: &CliProfile) -> bool {
    mmux_node::profiles::is_busy(output, profile)
}

fn profile_has_prompt(output: &str, profile: &CliProfile) -> bool {
    mmux_node::profiles::has_prompt(output, profile)
}

fn profile_turn_idle(output: &str, profile: &CliProfile) -> bool {
    mmux_node::profiles::turn_idle(output, profile)
}

fn compact_coding_output(output: &str, profile: &CliProfile) -> String {
    mmux_node::profiles::compact_output(output, profile)
}

fn startup_dismiss_key(output: &str, profile: &CliProfile) -> Option<String> {
    mmux_node::profiles::startup_dismiss_key(output, profile)
}

fn key_sequence_parts(keys: &str) -> Vec<&str> {
    let parts = keys.split_whitespace().collect::<Vec<_>>();
    if parts.is_empty() {
        vec![keys]
    } else {
        parts
    }
}

fn coding_prompt_submit_delay(prompt: &str) -> Duration {
    let line_count = prompt.lines().count().saturating_sub(1) as u64;
    let char_count = prompt.chars().count() as u64;
    let millis = 200 + line_count.saturating_mul(80) + char_count / 3;
    Duration::from_millis(millis.clamp(200, 2_000))
}

fn tmux_buffer_name(prefix: &str, target: &str) -> String {
    let target = target
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect::<String>();
    format!("{prefix}-{target}-{}", now_ms())
}

fn tmux_set_buffer_args(buffer: &str, text: &str) -> Vec<String> {
    vec![
        "set-buffer".into(),
        "-b".into(),
        buffer.into(),
        "--".into(),
        text.into(),
    ]
}

fn tmux_paste_buffer_args(target: &str, buffer: &str) -> Vec<String> {
    vec![
        "paste-buffer".into(),
        "-d".into(),
        "-p".into(),
        "-b".into(),
        buffer.into(),
        "-t".into(),
        target.into(),
    ]
}

#[cfg(test)]
fn tmux_submit_args(target: &str) -> Vec<String> {
    node_send_key_args(target, "Enter")
}

fn tmux_submit_keys_args(target: &str, keys: &str) -> Vec<String> {
    node_send_key_args(target, keys)
}

fn tmux_literal_text_args(target: &str, text: &str) -> Vec<String> {
    vec![
        "send-keys".into(),
        "-t".into(),
        target.into(),
        "-l".into(),
        "--".into(),
        text.into(),
    ]
}

fn node_send_key_args(target: &str, keys: &str) -> Vec<String> {
    let mut args = vec!["send-keys".into(), "-t".into(), target.into()];
    args.extend(key_sequence_parts(keys).into_iter().map(ToOwned::to_owned));
    args
}

fn tmux_capture_output_args(target: &str, lines: Option<usize>, scrollback: bool) -> Vec<String> {
    if scrollback {
        vec![
            "capture-pane".into(),
            "-t".into(),
            target.into(),
            "-p".into(),
            "-S".into(),
            "-".into(),
        ]
    } else if let Some(lines) = lines {
        vec![
            "capture-pane".into(),
            "-t".into(),
            target.into(),
            "-p".into(),
            "-S".into(),
            format!("-{}", lines),
        ]
    } else {
        vec![
            "capture-pane".into(),
            "-t".into(),
            target.into(),
            "-p".into(),
        ]
    }
}

fn sanitize_orchestration_name_part(value: &str, fallback: &str, max_len: usize) -> String {
    let mut sanitized = String::new();
    let mut previous_was_dash = false;

    for ch in value.chars() {
        let next = if ch.is_ascii_alphanumeric() {
            Some(ch.to_ascii_lowercase())
        } else if ch == '-' || ch == '_' || ch.is_whitespace() || ch.is_ascii_punctuation() {
            Some('-')
        } else {
            None
        };

        let Some(next) = next else {
            continue;
        };
        if next == '-' {
            if sanitized.is_empty() || previous_was_dash {
                continue;
            }
            previous_was_dash = true;
            sanitized.push(next);
        } else {
            previous_was_dash = false;
            sanitized.push(next);
        }

        if sanitized.len() >= max_len {
            break;
        }
    }

    while sanitized.ends_with('-') {
        sanitized.pop();
    }
    if sanitized.is_empty() {
        fallback.to_owned()
    } else {
        sanitized
    }
}

fn generated_orchestration_session_name(task_slug: &str, kind: &str, suffix: &str) -> String {
    let task_slug =
        sanitize_orchestration_name_part(task_slug, "task", MAX_ORCHESTRATION_TASK_SLUG_LEN);
    let kind = sanitize_orchestration_name_part(kind, "agent", MAX_ORCHESTRATION_KIND_LEN);
    let suffix = sanitize_orchestration_name_part(suffix, "session", MAX_ORCHESTRATION_SUFFIX_LEN);
    format!("{ORCHESTRATION_SESSION_PREFIX}-{task_slug}-{kind}-{suffix}")
}

fn short_session_suffix() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let mut suffix = format!("{nanos:x}");
    if suffix.len() > MAX_ORCHESTRATION_SUFFIX_LEN {
        suffix = suffix[suffix.len() - MAX_ORCHESTRATION_SUFFIX_LEN..].to_owned();
    }
    suffix
}

fn string_vec_arg(args: &Map<String, Value>, field: &str) -> Result<Vec<String>, McpError> {
    let Some(value) = args.get(field) else {
        return Ok(Vec::new());
    };
    let Some(values) = value.as_array() else {
        return Err(McpError::invalid_request(
            format!("{field} must be an array of strings"),
            None,
        ));
    };
    values
        .iter()
        .map(|value| {
            value.as_str().map(str::to_owned).ok_or_else(|| {
                McpError::invalid_request(format!("{field} must be an array of strings"), None)
            })
        })
        .collect()
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Registered node actor
// ═══════════════════════════════════════════════════════════════════════════════

struct NodeRegistryActor;

struct NodeRegistryState {
    registry: NodeRegistry,
    pending: HashMap<String, RpcReplyPort<Result<NodeCommandResult, String>>>,
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
    type Arguments = Option<String>;

    async fn pre_start(
        &self,
        _myself: ActorRef<Self::Msg>,
        args: Self::Arguments,
    ) -> Result<Self::State, ActorProcessingErr> {
        Ok(NodeRegistryState {
            registry: NodeRegistry::new(args),
            pending: HashMap::new(),
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
                let _ = reply.send(state.registry.register(descriptor, now_ms()));
            }
            NodeRegistryMessage::Heartbeat {
                node_id,
                status,
                reply,
            } => {
                let _ = reply.send(state.registry.heartbeat(&node_id, status, now_ms()));
            }
            NodeRegistryMessage::Pull { node_id, reply } => {
                let _ = reply.send(state.registry.pull_commands(&node_id, now_ms()));
            }
            NodeRegistryMessage::SubmitResult {
                node_id,
                command_id,
                result,
                reply,
            } => {
                state.registry.note_result(&node_id, now_ms());
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
            } => match state.registry.dispatch(&node_id, kind) {
                Ok(command) => {
                    state.pending.insert(command.command_id, reply);
                }
                Err(error) => {
                    let _ = reply.send(Err(error));
                }
            },
            NodeRegistryMessage::ListNodes { reply } => {
                let nodes = state.registry.list_nodes(now_ms());
                let text = serde_json::to_string_pretty(&nodes)
                    .unwrap_or_else(|error| format!("{{\"error\":\"{}\"}}", error));
                let _ = reply.send(Ok(text));
            }
            NodeRegistryMessage::NodeInfo { node_id, reply } => {
                match state.registry.node_info(&node_id, now_ms()) {
                    Ok(summary) => {
                        let text = serde_json::to_string_pretty(&summary)
                            .unwrap_or_else(|error| format!("{{\"error\":\"{}\"}}", error));
                        let _ = reply.send(Ok(text));
                    }
                    Err(error) => {
                        let _ = reply.send(Err(error));
                    }
                }
            }
        }
        Ok(())
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Execution node actor
// ═══════════════════════════════════════════════════════════════════════════════

struct NodeExecutionActor;

enum NodeExecutionMessage {
    Command {
        node_id: String,
        kind: NodeCommandKind,
        timeout: Duration,
        reply: RpcReplyPort<Result<NodeCommandResult, String>>,
    },
}

#[derive(Clone)]
struct NodeExecutionWorkerState {
    embedded: Option<mmux_node::EmbeddedNodeBackend>,
    registry: ActorRef<NodeRegistryMessage>,
}

struct NodeExecutionJobActor;

impl Actor for NodeExecutionActor {
    type Msg = NodeExecutionMessage;
    type State = NodeExecutionWorkerState;
    type Arguments = NodeExecutionWorkerState;

    async fn pre_start(
        &self,
        _myself: ActorRef<Self::Msg>,
        state: Self::Arguments,
    ) -> Result<Self::State, ActorProcessingErr> {
        Ok(state)
    }

    async fn handle(
        &self,
        _myself: ActorRef<Self::Msg>,
        message: Self::Msg,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        match ActorRuntime::<NodeExecutionJobActor>::spawn_instant(
            None,
            NodeExecutionJobActor,
            state.clone(),
        ) {
            Ok((worker, _handle)) => {
                if let Err(error) = worker.send_message(message) {
                    match error {
                        ractor::MessagingErr::SendErr(message) => {
                            reply_node_execution_dispatch_error(
                                message,
                                "job actor stopped".into(),
                            );
                        }
                        other => {
                            let _ = other;
                        }
                    }
                }
            }
            Err(error) => {
                reply_node_execution_dispatch_error(message, error.to_string());
            }
        }
        Ok(())
    }
}

impl Actor for NodeExecutionJobActor {
    type Msg = NodeExecutionMessage;
    type State = NodeExecutionWorkerState;
    type Arguments = NodeExecutionWorkerState;

    async fn pre_start(
        &self,
        _myself: ActorRef<Self::Msg>,
        args: Self::Arguments,
    ) -> Result<Self::State, ActorProcessingErr> {
        Ok(args)
    }

    async fn handle(
        &self,
        myself: ActorRef<Self::Msg>,
        message: Self::Msg,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        run_node_execution_message(state, message).await;
        myself.stop(None);
        Ok(())
    }
}

fn reply_node_execution_dispatch_error(message: NodeExecutionMessage, error: String) {
    let NodeExecutionMessage::Command { reply, .. } = message;
    let _ = reply.send(Err(format!("failed to dispatch node operation: {error}")));
}

async fn run_node_execution_message(
    state: &mut NodeExecutionWorkerState,
    message: NodeExecutionMessage,
) {
    match message {
        NodeExecutionMessage::Command {
            node_id,
            kind,
            timeout,
            reply,
        } => {
            let result = execute_node_command(state, &node_id, kind, timeout).await;
            let _ = reply.send(result);
        }
    }
}

async fn execute_node_command(
    state: &mut NodeExecutionWorkerState,
    node_id: &str,
    kind: NodeCommandKind,
    timeout: Duration,
) -> Result<NodeCommandResult, String> {
    if node_id == "local" {
        if let Some(embedded) = state.embedded.as_mut() {
            return Ok(embedded.execute(kind).await);
        }
    }
    registry_call(
        &state.registry,
        |reply| NodeRegistryMessage::Dispatch {
            node_id: node_id.to_owned(),
            kind,
            reply,
        },
        Some(timeout),
    )
    .await
}

fn parse_session_list(node: &str, output: &str) -> Vec<SessionListEntry> {
    let mut sessions = output
        .lines()
        .filter_map(|line| {
            let line = line.trim_end();
            if line.trim().is_empty() || line.trim() == "No tmux sessions running" {
                return None;
            }
            let fields = line.splitn(4, '|').collect::<Vec<_>>();
            let session = fields.first().copied().unwrap_or("").trim();
            if session.is_empty() {
                return None;
            }
            Some(SessionListEntry {
                node: node.to_owned(),
                session: session.to_owned(),
                windows: fields
                    .get(1)
                    .and_then(|value| value.trim().parse::<u64>().ok()),
                attached: fields
                    .get(2)
                    .and_then(|value| value.trim().parse::<u64>().ok()),
                created_at_seconds: fields
                    .get(3)
                    .and_then(|value| value.trim().parse::<u64>().ok()),
            })
        })
        .collect::<Vec<_>>();
    sessions.sort_by(|left, right| left.session.cmp(&right.session));
    sessions
}

fn project_scoped_session_entries(
    state: &OrchestrationState,
    project_id: &ProjectId,
    node: &str,
    live_sessions: &[SessionListEntry],
) -> Result<Vec<ProjectSessionListEntry>, String> {
    if !state.projects.contains_key(project_id) {
        return Err(format!("project '{}' not found", project_id.0));
    }

    let live_by_session = live_sessions
        .iter()
        .map(|session| (session.session.as_str(), session))
        .collect::<std::collections::HashMap<_, _>>();

    let mut entries = state
        .tasks
        .values()
        .filter(|task| task_project_id(state, task).as_ref() == Some(project_id))
        .filter_map(|task| task.session.as_ref().map(|record| (task, record)))
        .filter(|(_, record)| record.node_id.0 == node)
        .map(|(task, record)| {
            let live = live_by_session.get(record.session.0.as_str()).copied();
            ProjectSessionListEntry {
                node: record.node_id.0.clone(),
                session: record.session.0.clone(),
                profile: record.profile.clone(),
                workspace_path: record.workspace_path.clone(),
                bypass_permissions: record.bypass_permissions,
                task_id: task.id.clone(),
                role: record.role.clone(),
                kind: record.kind.clone(),
                last_seen_ms: record.last_seen_ms,
                runtime_state: if live.is_some() {
                    "running".into()
                } else {
                    "missing".into()
                },
                windows: live.and_then(|session| session.windows),
                attached: live.and_then(|session| session.attached),
                created_at_seconds: live.and_then(|session| session.created_at_seconds),
            }
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| {
        left.node
            .cmp(&right.node)
            .then_with(|| left.session.cmp(&right.session))
    });
    Ok(entries)
}

fn resolve_project_id_or_slug(
    state: &OrchestrationState,
    project_id_or_slug: &str,
) -> Result<ProjectId, String> {
    let selector = project_id_or_slug.trim();
    if selector.is_empty() {
        return Err("project_id must not be empty".into());
    }
    if state.projects.contains_key(&ProjectId(selector.to_owned())) {
        return Ok(ProjectId(selector.to_owned()));
    }
    state
        .projects
        .values()
        .find(|project| project.slug == selector)
        .map(|project| project.id.clone())
        .ok_or_else(|| format!("project '{selector}' not found"))
}

fn resolve_plan_id_or_slug(
    state: &OrchestrationState,
    plan_id_or_slug: &str,
) -> Result<PlanId, String> {
    let selector = plan_id_or_slug.trim();
    if selector.is_empty() {
        return Err("plan_id must not be empty".into());
    }
    if state.plans.contains_key(&PlanId(selector.to_owned())) {
        return Ok(PlanId(selector.to_owned()));
    }
    let mut matches = state
        .plans
        .values()
        .filter(|plan| plan.slug == selector)
        .collect::<Vec<_>>();
    matches.sort_by(|left, right| left.id.0.cmp(&right.id.0));
    match matches.as_slice() {
        [plan] => Ok(plan.id.clone()),
        [] => Err(format!("plan '{selector}' not found")),
        plans => {
            let matches = plans
                .iter()
                .map(|plan| format!("{} in project {}", plan.id.0, plan.project_id.0))
                .collect::<Vec<_>>()
                .join(", ");
            Err(format!(
                "plan slug '{}' is ambiguous; matches: {}",
                selector, matches
            ))
        }
    }
}

fn task_project_id(state: &OrchestrationState, task: &Task) -> Option<ProjectId> {
    state
        .plans
        .get(&task.plan_id)
        .map(|plan| plan.project_id.clone())
}

fn is_no_tmux_sessions_error(error: &str) -> bool {
    let error = error.to_ascii_lowercase();
    error.contains("no server running")
        || error.contains("no sessions")
        || (error.contains("error connecting") && error.contains("no such file or directory"))
}

fn is_tmux_missing_session_error(error: &str) -> bool {
    let error = error.to_ascii_lowercase();
    is_no_tmux_sessions_error(&error)
        || error.trim() == "missing"
        || error.contains("can't find session")
        || error.contains("cannot find session")
        || error.contains("missing session")
}

fn parse_local_session_info_list(output: &str) -> Vec<LocalSessionInfo> {
    let mut sessions = output
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line == "No tmux sessions running" {
                return None;
            }
            let (session, created_at_seconds) = line.split_once('|').unwrap_or((line, ""));
            let session = session.trim();
            if session.is_empty() {
                return None;
            }
            Some(LocalSessionInfo {
                session: session.to_owned(),
                created_at_seconds: created_at_seconds.trim().parse::<u64>().ok(),
            })
        })
        .collect::<Vec<_>>();
    sessions.sort_by(|left, right| left.session.cmp(&right.session));
    sessions
}

// ═══════════════════════════════════════════════════════════════════════════════
//  MCP HTTP Server Mode (rmcp)
// ═══════════════════════════════════════════════════════════════════════════════

#[derive(Clone)]
struct TmuxMcpServer {
    profiles: ProfileRegistry,
    default_coder_profile: Option<String>,
    policy: ControllerPolicy,
    node_executor: ActorRef<NodeExecutionMessage>,
    registry: ActorRef<NodeRegistryMessage>,
    orchestration: orchestration_actor::OrchestrationHandle,
    wait_jobs: WaitJobRegistry,
    startup_warnings: Arc<Mutex<Vec<String>>>,
}

type WaitJobRegistry = Arc<Mutex<HashMap<String, RuntimeWaitJob>>>;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
enum RuntimeWaitKind {
    Stable,
    Sentinel,
    Prompt,
    CodingReady,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
enum RuntimeWaitStatus {
    Pending,
    Completed,
    Failed,
    Canceled,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct RuntimeWaitDetail {
    message: String,
    elapsed_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct RuntimeWaitSnapshot {
    wait_id: String,
    node: String,
    session: String,
    kind: RuntimeWaitKind,
    profile: Option<String>,
    status: RuntimeWaitStatus,
    created_at_ms: u64,
    updated_at_ms: u64,
    completed_at_ms: Option<u64>,
    timeout_seconds: f64,
    poll_seconds: f64,
    stability_seconds: f64,
    sentinel: Option<String>,
    prompt: Option<String>,
    result: Option<RuntimeWaitDetail>,
    error: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ListSessionsArgs {
    #[serde(default = "default_wait_node")]
    node: String,
    project_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectSessionListEntry {
    node: String,
    session: String,
    profile: String,
    workspace_path: String,
    bypass_permissions: bool,
    task_id: TaskId,
    role: String,
    kind: String,
    last_seen_ms: u64,
    runtime_state: String,
    windows: Option<u64>,
    attached: Option<u64>,
    created_at_seconds: Option<u64>,
}

struct RuntimeWaitJob {
    snapshot: RuntimeWaitSnapshot,
    handle: Option<JoinHandle<()>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WaitStartArgs {
    #[serde(default = "default_wait_node")]
    node: String,
    session: String,
    kind: RuntimeWaitKind,
    profile: Option<String>,
    sentinel: Option<String>,
    prompt: Option<String>,
    timeout_seconds: Option<f64>,
    poll_seconds: Option<f64>,
    stability_seconds: Option<f64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WaitStatusArgs {
    wait_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WaitCancelArgs {
    wait_id: String,
}

struct RuntimeWaitRunner {
    wait_id: String,
    wait_jobs: WaitJobRegistry,
    target: RuntimeWaitTarget,
    session: String,
    kind: RuntimeWaitKind,
    profile: Option<CliProfile>,
    sentinel: Option<String>,
    prompt: Option<String>,
    timeout_seconds: f64,
    poll_seconds: f64,
    stability_seconds: f64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CodingTaskSendArgs {
    #[serde(default = "default_wait_node")]
    node: String,
    #[serde(default = "default_coding_session")]
    session: String,
    profile: Option<String>,
    task_id_or_slug: String,
    prompt: String,
    template: Option<CodingTaskSendTemplate>,
    include_dependencies: Option<bool>,
    include_gates: Option<bool>,
    include_scope: Option<bool>,
    context_task_ids: Option<Vec<String>>,
    extra_context: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
enum CodingTaskSendTemplate {
    Task,
    Validate,
    Review,
    QualityGuard,
}

#[derive(Clone)]
enum RuntimeWaitTarget {
    Node {
        node_executor: ActorRef<NodeExecutionMessage>,
        node_id: String,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectCreateArgs {
    title: String,
    description: String,
    slug: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectStatusUpdateArgs {
    project_id: String,
    status: ProjectStatus,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlanCreateArgs {
    project_id: String,
    title: String,
    brief: String,
    slug: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlanUpdateArgs {
    plan_id: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    brief: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlanStatusUpdateArgs {
    plan_id: String,
    status: PlanStatus,
    outcome: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TaskCreateArgs {
    plan_id: String,
    title: String,
    objective: String,
    #[serde(default)]
    include_paths: Vec<String>,
    #[serde(default)]
    exclude_paths: Vec<String>,
    notes: Option<String>,
    #[serde(default)]
    gates: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TaskUpdateArgs {
    task_id: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    objective: Option<String>,
    #[serde(default)]
    include_paths: Option<Vec<String>>,
    #[serde(default)]
    exclude_paths: Option<Vec<String>>,
    #[serde(default)]
    notes: Option<Option<String>>,
    #[serde(default)]
    gates: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TaskEdgeAddArgs {
    from_task_id: String,
    to_task_id: String,
    kind: TaskEdgeKind,
    note: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TaskEdgeRemoveArgs {
    from_task_id: String,
    to_task_id: String,
    kind: TaskEdgeKind,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TaskSessionRecordArgs {
    node_id: String,
    session: String,
    profile: String,
    workspace_path: String,
    bypass_permissions: bool,
    task_id: String,
    role: String,
    kind: String,
    #[serde(default)]
    skills: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TaskStatusUpdateArgs {
    task_id: String,
    status: TaskStatus,
    outcome: Option<String>,
    blockers: Option<Vec<String>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct OrchestrationStatusArgs {
    project_id: Option<String>,
    plan_id: Option<String>,
    task_id: Option<String>,
    #[serde(default)]
    include_completed: bool,
}

struct NodeWaitOptions<'a> {
    mode: &'a str,
    sentinel: Option<&'a str>,
    prompt: Option<&'a str>,
    timeout: f64,
    poll: f64,
    stability: f64,
}

struct TaskAwareStart {
    session_name: String,
    workspace_path: String,
    task_id: TaskId,
    role: String,
    kind: String,
    skills: Vec<String>,
    previous_session: Option<TaskSession>,
}

impl TmuxMcpServer {
    fn new(
        profiles: ProfileRegistry,
        default_coder_profile: Option<String>,
        policy: ControllerPolicy,
        node_executor: ActorRef<NodeExecutionMessage>,
        registry: ActorRef<NodeRegistryMessage>,
        orchestration: orchestration_actor::OrchestrationHandle,
        wait_jobs: WaitJobRegistry,
        startup_warnings: Arc<Mutex<Vec<String>>>,
    ) -> Self {
        Self {
            profiles,
            default_coder_profile,
            policy,
            node_executor,
            registry,
            orchestration,
            wait_jobs,
            startup_warnings,
        }
    }

    fn text_result(text: impl Into<String>) -> CallToolResult {
        CallToolResult::success(vec![Content::text(text)])
    }

    fn error_result(text: impl Into<String>) -> CallToolResult {
        CallToolResult::error(vec![Content::text(text)])
    }

    fn json_result(value: impl Serialize) -> Result<CallToolResult, McpError> {
        let text = serde_json::to_string_pretty(&value).map_err(|error| {
            McpError::internal_error(format!("failed to serialize tool result: {}", error), None)
        })?;
        Ok(Self::text_result(text))
    }

    fn wait_job_snapshot(&self, wait_id: &str) -> Result<RuntimeWaitSnapshot, String> {
        let jobs = self
            .wait_jobs
            .lock()
            .map_err(|_| "wait job registry lock poisoned".to_string())?;
        jobs.get(wait_id)
            .map(|job| job.snapshot.clone())
            .ok_or_else(|| format!("wait job '{}' not found", wait_id))
    }

    async fn wait_start_tool(&self, args: Map<String, Value>) -> Result<CallToolResult, McpError> {
        let args: WaitStartArgs = parse_tool_args("wait_start", args)?;

        let timeout_seconds = self
            .policy
            .clamp_timeout(args.timeout_seconds.unwrap_or(match args.kind {
                RuntimeWaitKind::CodingReady => DEFAULT_CODING_READY_TIMEOUT_SECONDS as f64,
                _ => 30.0,
            }))
            .map_err(mcp_invalid_request)?;
        let poll_seconds = args.poll_seconds.unwrap_or(0.5);
        if !poll_seconds.is_finite() || poll_seconds <= 0.0 {
            return Err(McpError::invalid_request(
                "poll_seconds must be a positive finite number",
                None,
            ));
        }
        let stability_seconds = args.stability_seconds.unwrap_or(1.0);
        if !stability_seconds.is_finite() || stability_seconds < 0.0 {
            return Err(McpError::invalid_request(
                "stability_seconds must be a non-negative finite number",
                None,
            ));
        }

        let profile = match args.kind {
            RuntimeWaitKind::CodingReady => {
                let profile_name = args
                    .profile
                    .as_deref()
                    .filter(|value| !value.trim().is_empty())
                    .ok_or_else(|| {
                        McpError::invalid_request(
                            "profile is required for coding-ready waits",
                            None,
                        )
                    })?;
                Some(
                    self.resolve_profile(Some(profile_name))
                        .ok_or_else(|| McpError::invalid_request("unknown profile", None))?,
                )
            }
            _ => None,
        };
        if matches!(args.kind, RuntimeWaitKind::Sentinel)
            && args.sentinel.as_deref().map(str::is_empty).unwrap_or(true)
        {
            return Err(McpError::invalid_request(
                "sentinel is required for sentinel waits",
                None,
            ));
        }
        if matches!(args.kind, RuntimeWaitKind::Prompt)
            && args.prompt.as_deref().map(str::is_empty).unwrap_or(true)
        {
            return Err(McpError::invalid_request(
                "prompt is required for prompt waits",
                None,
            ));
        }

        let exists = match self.node_session_exists(&args.node, &args.session).await {
            Ok(exists) => exists,
            Err(error) => return Ok(Self::error_result(error)),
        };
        if !exists {
            return Ok(Self::error_result(format!(
                "Session '{}' does not exist",
                args.session
            )));
        }
        let target = match self.runtime_wait_target(&args.node) {
            Ok(target) => target,
            Err(error) => return Ok(Self::error_result(error)),
        };

        let wait_id = runtime_wait_id();
        let now = now_ms();
        let snapshot = RuntimeWaitSnapshot {
            wait_id: wait_id.clone(),
            node: args.node,
            session: args.session.clone(),
            kind: args.kind,
            profile: args.profile.clone(),
            status: RuntimeWaitStatus::Pending,
            created_at_ms: now,
            updated_at_ms: now,
            completed_at_ms: None,
            timeout_seconds,
            poll_seconds,
            stability_seconds,
            sentinel: args.sentinel.clone(),
            prompt: args.prompt.clone(),
            result: None,
            error: None,
        };
        {
            let mut jobs = self
                .wait_jobs
                .lock()
                .map_err(|_| McpError::internal_error("wait job registry lock poisoned", None))?;
            jobs.insert(
                wait_id.clone(),
                RuntimeWaitJob {
                    snapshot,
                    handle: None,
                },
            );
        }

        let runner = RuntimeWaitRunner {
            wait_id: wait_id.clone(),
            wait_jobs: self.wait_jobs.clone(),
            target,
            session: args.session,
            kind: args.kind,
            profile,
            sentinel: args.sentinel,
            prompt: args.prompt,
            timeout_seconds,
            poll_seconds,
            stability_seconds,
        };
        let handle = tokio::spawn(async move {
            run_runtime_wait_job(runner).await;
        });
        {
            let mut jobs = self
                .wait_jobs
                .lock()
                .map_err(|_| McpError::internal_error("wait job registry lock poisoned", None))?;
            if let Some(job) = jobs.get_mut(&wait_id) {
                job.handle = Some(handle);
            }
        }

        Self::json_result(
            self.wait_job_snapshot(&wait_id)
                .map_err(mcp_invalid_request)?,
        )
    }

    fn runtime_wait_target(&self, node: &str) -> Result<RuntimeWaitTarget, String> {
        Ok(RuntimeWaitTarget::Node {
            node_executor: self.node_executor.clone(),
            node_id: node.to_owned(),
        })
    }

    fn wait_status_tool(&self, args: Map<String, Value>) -> Result<CallToolResult, McpError> {
        let args: WaitStatusArgs = parse_tool_args("wait_status", args)?;
        Self::json_result(
            self.wait_job_snapshot(&args.wait_id)
                .map_err(mcp_invalid_request)?,
        )
    }

    fn wait_cancel_tool(&self, args: Map<String, Value>) -> Result<CallToolResult, McpError> {
        let args: WaitCancelArgs = parse_tool_args("wait_cancel", args)?;
        let mut jobs = self
            .wait_jobs
            .lock()
            .map_err(|_| McpError::internal_error("wait job registry lock poisoned", None))?;
        let job = jobs
            .get_mut(&args.wait_id)
            .ok_or_else(|| mcp_invalid_request(format!("wait job '{}' not found", args.wait_id)))?;
        if job.snapshot.status == RuntimeWaitStatus::Pending {
            if let Some(handle) = job.handle.take() {
                handle.abort();
            }
            let now = now_ms();
            job.snapshot.status = RuntimeWaitStatus::Canceled;
            job.snapshot.updated_at_ms = now;
            job.snapshot.completed_at_ms = Some(now);
            job.snapshot.result = Some(RuntimeWaitDetail {
                message: "wait canceled".into(),
                elapsed_ms: now.saturating_sub(job.snapshot.created_at_ms),
            });
            job.snapshot.error = None;
        }
        Self::json_result(job.snapshot.clone())
    }

    fn orchestration_tool_definitions(enable_admin_tools: bool) -> Vec<Tool> {
        let mut tools = Vec::new();
        if enable_admin_tools {
            tools.push(Tool::new(
                "project_create",
                "Create an orchestration project boundary and return the created Project object directly",
                Arc::new(tool_schema(
                    json!({
                        "title": { "type": "string" },
                        "description": { "type": "string" },
                        "slug": { "type": "string" }
                    }),
                    Some(vec!["title", "description"]),
                )),
            ));
        }
        tools.push(Tool::new(
            "project_list",
            "List orchestration projects",
            Arc::new(tool_schema(json!({}), None)),
        ));
        if enable_admin_tools {
            tools.push(Tool::new(
                "project_status_update",
                "Update orchestration project status",
                Arc::new(tool_schema(
                    json!({
                        "project_id": { "type": "string", "description": "Project UUID id or globally unique project slug" },
                        "status": { "type": "string", "enum": ["Active", "Archived"] }
                    }),
                    Some(vec!["project_id", "status"]),
                )),
            ));
        }
        tools.extend([
            Tool::new(
                "plan_create",
                "Create an orchestration plan document under a project and return the created Plan object directly",
                Arc::new(tool_schema(
                    json!({
                        "project_id": { "type": "string", "description": "Project UUID id or globally unique project slug" },
                        "title": { "type": "string" },
                        "brief": { "type": "string", "description": "Required Markdown plan brief with enough context and detail to derive tasks" },
                        "slug": { "type": "string" }
                    }),
                    Some(vec!["project_id", "title", "brief"]),
                )),
            ),
            Tool::new(
                "plan_list",
                "List orchestration plans, optionally filtered by project",
                Arc::new(tool_schema(
                    json!({
                        "project_id": { "type": "string", "description": "Optional project UUID id or globally unique project slug" }
                    }),
                    None,
                )),
            ),
            Tool::new(
                "plan_update",
                "Update mutable orchestration plan metadata without changing status, tasks, sessions, or id",
                Arc::new(tool_schema(
                    json!({
                        "plan_id": { "type": "string", "description": "Plan id or slug" },
                        "title": { "type": "string" },
                        "brief": { "type": "string", "description": "Markdown plan brief with enough context and detail to derive tasks" }
                    }),
                    Some(vec!["plan_id"]),
                )),
            ),
            Tool::new(
                "plan_status_update",
                "Update orchestration plan status and optional outcome",
                Arc::new(tool_schema(
                    json!({
                        "plan_id": { "type": "string", "description": "Plan id or slug" },
                        "status": { "type": "string", "enum": ["Backlog", "Planned", "Running", "WaitingForValidation", "Blocked", "Failed", "Passed", "Delivered", "Canceled"] },
                        "outcome": { "type": "string", "description": "Plan-level result after execution or validation" }
                    }),
                    Some(vec!["plan_id", "status"]),
                )),
            ),
            Tool::new(
                "task_create",
                "Create an orchestration task and return the created Task object directly",
                Arc::new(tool_schema(
                    json!({
                        "plan_id": { "type": "string", "description": "Plan id or slug" },
                        "title": { "type": "string" },
                        "objective": { "type": "string" },
                        "include_paths": { "type": "array", "items": { "type": "string" } },
                        "exclude_paths": { "type": "array", "items": { "type": "string" } },
                        "notes": { "type": "string" },
                        "gates": { "type": "array", "items": { "type": "string" } }
                    }),
                    Some(vec!["plan_id", "title", "objective"]),
                )),
            ),
            Tool::new(
                "task_update",
                "Update mutable orchestration task metadata without changing status, edges, sessions, id, or completion timestamp",
                Arc::new(tool_schema(
                    json!({
                        "task_id": { "type": "string" },
                        "title": { "type": "string" },
                        "objective": { "type": "string" },
                        "include_paths": { "type": "array", "items": { "type": "string" } },
                        "exclude_paths": { "type": "array", "items": { "type": "string" } },
                        "notes": { "type": ["string", "null"] },
                        "gates": { "type": "array", "items": { "type": "string" } }
                    }),
                    Some(vec!["task_id"]),
                )),
            ),
            Tool::new(
                "task_edge_add",
                "Add an orchestration task edge",
                Arc::new(tool_schema(
                    json!({
                        "from_task_id": { "type": "string" },
                        "to_task_id": { "type": "string" },
                        "kind": { "type": "string", "enum": ["ParentOf", "DependsOn", "Blocks", "Validates", "Audits", "Refines", "Supersedes", "Related"] },
                        "note": { "type": "string" }
                    }),
                    Some(vec!["from_task_id", "to_task_id", "kind"]),
                )),
            ),
            Tool::new(
                "task_edge_remove",
                "Remove an orchestration task edge",
                Arc::new(tool_schema(
                    json!({
                        "from_task_id": { "type": "string" },
                        "to_task_id": { "type": "string" },
                        "kind": { "type": "string", "enum": ["ParentOf", "DependsOn", "Blocks", "Validates", "Audits", "Refines", "Supersedes", "Related"] }
                    }),
                    Some(vec!["from_task_id", "to_task_id", "kind"]),
                )),
            ),
            Tool::new(
                "session_record",
                "Record durable orchestration metadata for a running session",
                Arc::new(tool_schema(
                    json!({
                        "node_id": { "type": "string" },
                        "session": { "type": "string" },
                        "profile": { "type": "string" },
                        "workspace_path": { "type": "string", "description": "Backend-owned workspace/start directory for the recorded session." },
                        "bypass_permissions": { "type": "boolean" },
                        "task_id": { "type": "string" },
                        "role": { "type": "string" },
                        "kind": { "type": "string" },
                        "skills": { "type": "array", "items": { "type": "string" } }
                    }),
                    Some(vec!["node_id", "session", "profile", "workspace_path", "bypass_permissions", "task_id", "role", "kind"]),
                )),
            ),
            Tool::new(
                "task_status_update",
                "Update orchestration task status and operator notes",
                Arc::new(tool_schema(
                    json!({
                        "task_id": { "type": "string" },
                        "status": { "type": "string", "enum": ["Backlog", "Planned", "Running", "WaitingForValidation", "Blocked", "Failed", "Passed", "Delivered", "Canceled"] },
                        "outcome": { "type": "string" },
                        "blockers": { "type": "array", "items": { "type": "string" } }
                    }),
                    Some(vec!["task_id", "status"]),
                )),
            ),
            Tool::new(
                "orchestration_status",
                "Return compact orchestration task/session summaries",
                Arc::new(tool_schema(
                    json!({
                        "task_id": { "type": "string" },
                        "project_id": { "type": "string", "description": "Project UUID id or globally unique project slug" },
                        "plan_id": { "type": "string", "description": "Plan id or slug" },
                        "include_completed": { "type": "boolean" }
                    }),
                    None,
                )),
            ),
            Tool::new(
                "orchestration_cleanup_zombies",
                "Dry-run or explicitly clean live local mmux-* sessions absent from durable orchestration storage",
                Arc::new(tool_schema(
                    json!({
                        "dry_run": { "type": "boolean", "description": "When true, only report candidates. Default: true." },
                        "older_than_seconds": { "type": "integer", "description": "Only include candidates at least this old." },
                        "node": { "type": "string", "description": "Execution node id. Only local is supported in v1; default: local." }
                    }),
                    None,
                )),
            ),
            Tool::new(
                "orchestration_prune_store",
                "Dry-run or explicitly prune stale durable task sessions and finished plans",
                Arc::new(tool_schema(
                    json!({
                        "dry_run": { "type": "boolean", "description": "When true, only report candidates. Default: true." },
                        "sessions_only": { "type": "boolean", "description": "Scope pruning to task sessions and skip finished plan pruning." },
                        "older_than_days": { "type": "integer", "description": "Only include stale task sessions last seen at least this many days ago." },
                        "node": { "type": "string", "description": "Execution node id. Only local is supported in v1; default: local." }
                    }),
                    None,
                )),
            ),
        ]);
        tools
    }

    fn call_orchestration_tool(
        &self,
        name: &str,
        args: Map<String, Value>,
    ) -> Option<Result<CallToolResult, McpError>> {
        let result = match name {
            "project_create" => self.project_create_tool(args),
            "project_list" => self.project_list_tool(args),
            "project_status_update" => self.project_status_update_tool(args),
            "plan_create" => self.plan_create_tool(args),
            "plan_list" => self.plan_list_tool(args),
            "plan_update" => self.plan_update_tool(args),
            "plan_status_update" => self.plan_status_update_tool(args),
            "task_create" => self.task_create_tool(args),
            "task_update" => self.task_update_tool(args),
            "task_edge_add" => self.task_edge_add_tool(args),
            "task_edge_remove" => self.task_edge_remove_tool(args),
            "task_status_update" => self.task_status_update_tool(args),
            "orchestration_status" => self.orchestration_status_tool(args),
            _ => return None,
        };
        Some(result)
    }

    fn project_create_tool(&self, args: Map<String, Value>) -> Result<CallToolResult, McpError> {
        self.policy.ensure_admin_tools_enabled("project_create")?;
        let args: ProjectCreateArgs = parse_tool_args("project_create", args)?;
        let project = self
            .orchestration
            .create_project(CreateProject {
                title: args.title,
                description: args.description,
                slug: args.slug,
            })
            .map_err(mcp_invalid_request)?;
        Self::json_result(project)
    }

    fn project_list_tool(&self, args: Map<String, Value>) -> Result<CallToolResult, McpError> {
        let _: Map<String, Value> = args;
        let status = self.orchestration.status().map_err(mcp_invalid_request)?;
        Self::json_result(status.projects)
    }

    fn project_status_update_tool(
        &self,
        args: Map<String, Value>,
    ) -> Result<CallToolResult, McpError> {
        self.policy
            .ensure_admin_tools_enabled("project_status_update")?;
        let args: ProjectStatusUpdateArgs = parse_tool_args("project_status_update", args)?;
        let state = self.orchestration.snapshot().map_err(mcp_invalid_request)?;
        let project_id =
            resolve_project_id_or_slug(&state, &args.project_id).map_err(mcp_invalid_request)?;
        let project = self
            .orchestration
            .update_project_status(project_id, args.status)
            .map_err(mcp_invalid_request)?;
        Self::json_result(project)
    }

    fn plan_create_tool(&self, args: Map<String, Value>) -> Result<CallToolResult, McpError> {
        let args: PlanCreateArgs = parse_tool_args("plan_create", args)?;
        let state = self.orchestration.snapshot().map_err(mcp_invalid_request)?;
        let project_id =
            resolve_project_id_or_slug(&state, &args.project_id).map_err(mcp_invalid_request)?;
        let plan = self
            .orchestration
            .create_plan(CreatePlan {
                project_id,
                title: args.title,
                brief: args.brief,
                slug: args.slug,
            })
            .map_err(mcp_invalid_request)?;
        Self::json_result(plan)
    }

    fn plan_list_tool(&self, args: Map<String, Value>) -> Result<CallToolResult, McpError> {
        let args: OrchestrationStatusArgs = parse_tool_args("plan_list", args)?;
        let status = self.orchestration.status().map_err(mcp_invalid_request)?;
        let status = filter_orchestration_status(status, args).map_err(mcp_invalid_request)?;
        Self::json_result(status.plans)
    }

    fn plan_update_tool(&self, args: Map<String, Value>) -> Result<CallToolResult, McpError> {
        let args: PlanUpdateArgs = parse_tool_args("plan_update", args)?;
        let state = self.orchestration.snapshot().map_err(mcp_invalid_request)?;
        let plan_id =
            resolve_plan_id_or_slug(&state, &args.plan_id).map_err(mcp_invalid_request)?;
        let plan = self
            .orchestration
            .update_plan(
                plan_id,
                UpdatePlan {
                    title: args.title,
                    brief: args.brief,
                },
            )
            .map_err(mcp_invalid_request)?;
        Self::json_result(plan)
    }

    fn plan_status_update_tool(
        &self,
        args: Map<String, Value>,
    ) -> Result<CallToolResult, McpError> {
        let args: PlanStatusUpdateArgs = parse_tool_args("plan_status_update", args)?;
        let state = self.orchestration.snapshot().map_err(mcp_invalid_request)?;
        let plan_id =
            resolve_plan_id_or_slug(&state, &args.plan_id).map_err(mcp_invalid_request)?;
        let plan = self
            .orchestration
            .update_plan_status(plan_id, args.status, args.outcome)
            .map_err(mcp_invalid_request)?;
        Self::json_result(plan)
    }

    fn task_create_tool(&self, args: Map<String, Value>) -> Result<CallToolResult, McpError> {
        let args: TaskCreateArgs = parse_tool_args("task_create", args)?;
        let state = self.orchestration.snapshot().map_err(mcp_invalid_request)?;
        let plan_id =
            resolve_plan_id_or_slug(&state, &args.plan_id).map_err(mcp_invalid_request)?;
        let task = self
            .orchestration
            .create_task(CreateTask {
                plan_id,
                title: args.title,
                objective: args.objective,
                scope: TaskScope {
                    include_paths: args.include_paths,
                    exclude_paths: args.exclude_paths,
                    notes: args.notes,
                },
                gates: args.gates,
                slug: None,
            })
            .map_err(mcp_invalid_request)?;
        Self::json_result(task)
    }

    fn task_update_tool(&self, args: Map<String, Value>) -> Result<CallToolResult, McpError> {
        let args: TaskUpdateArgs = parse_tool_args("task_update", args)?;
        let task = self
            .orchestration
            .update_task(
                TaskId(args.task_id),
                UpdateTask {
                    title: args.title,
                    objective: args.objective,
                    scope: UpdateTaskScope {
                        include_paths: args.include_paths,
                        exclude_paths: args.exclude_paths,
                        notes: args.notes,
                    },
                    gates: args.gates,
                },
            )
            .map_err(mcp_invalid_request)?;
        Self::json_result(task)
    }

    fn task_edge_add_tool(&self, args: Map<String, Value>) -> Result<CallToolResult, McpError> {
        let args: TaskEdgeAddArgs = parse_tool_args("task_edge_add", args)?;
        let edge = self
            .orchestration
            .add_task_edge(CreateTaskEdge {
                from: TaskId(args.from_task_id),
                to: TaskId(args.to_task_id),
                kind: args.kind,
                note: args.note,
            })
            .map_err(mcp_invalid_request)?;
        Self::json_result(edge)
    }

    fn task_edge_remove_tool(&self, args: Map<String, Value>) -> Result<CallToolResult, McpError> {
        let args: TaskEdgeRemoveArgs = parse_tool_args("task_edge_remove", args)?;
        self.orchestration
            .remove_task_edge(
                TaskId(args.from_task_id),
                TaskId(args.to_task_id),
                args.kind,
            )
            .map_err(mcp_invalid_request)?;
        Self::json_result(json!({ "success": true }))
    }

    async fn session_record_tool(
        &self,
        args: Map<String, Value>,
    ) -> Result<CallToolResult, McpError> {
        let args: TaskSessionRecordArgs = parse_tool_args("session_record", args)?;
        if args.workspace_path.trim().is_empty() {
            return Err(McpError::invalid_request(
                "session_record requires workspace_path",
                None,
            ));
        }
        if self.resolve_profile(Some(&args.profile)).is_none() {
            return Err(McpError::invalid_request("unknown profile", None));
        }
        if !self
            .node_session_exists(&args.node_id, &args.session)
            .await
            .map_err(mcp_invalid_request)?
        {
            return Err(McpError::invalid_request(
                format!(
                    "session '{}' does not exist on node '{}'",
                    args.session, args.node_id
                ),
                None,
            ));
        }
        let task_id = TaskId(args.task_id);
        let previous_session = self
            .orchestration
            .snapshot()
            .map_err(mcp_invalid_request)?
            .tasks
            .get(&task_id)
            .ok_or_else(|| mcp_invalid_request(format!("task '{}' not found", task_id.0)))?
            .session
            .clone();
        self.stop_replaced_task_session(previous_session.as_ref(), &args.node_id, &args.session)
            .await
            .map_err(mcp_invalid_request)?;
        let session = self
            .orchestration
            .record_session(
                task_id,
                TaskSession {
                    node_id: NodeId(args.node_id),
                    session: SessionId(args.session),
                    profile: args.profile,
                    workspace_path: args.workspace_path,
                    bypass_permissions: args.bypass_permissions,
                    role: args.role,
                    kind: args.kind,
                    skills: args.skills,
                    created_at_ms: 0,
                    updated_at_ms: 0,
                    last_seen_ms: 0,
                },
            )
            .map_err(mcp_invalid_request)?;
        Self::json_result(session)
    }

    fn task_status_update_tool(
        &self,
        args: Map<String, Value>,
    ) -> Result<CallToolResult, McpError> {
        let args: TaskStatusUpdateArgs = parse_tool_args("task_status_update", args)?;
        let task = self
            .orchestration
            .update_task_status_details(
                TaskId(args.task_id),
                args.status,
                args.outcome,
                args.blockers,
            )
            .map_err(mcp_invalid_request)?;
        Self::json_result(task)
    }

    fn orchestration_status_tool(
        &self,
        args: Map<String, Value>,
    ) -> Result<CallToolResult, McpError> {
        let args: OrchestrationStatusArgs = parse_tool_args("orchestration_status", args)?;
        let status = self.orchestration.status().map_err(mcp_invalid_request)?;
        let status = filter_orchestration_status(status, args).map_err(mcp_invalid_request)?;
        Self::json_result(status)
    }

    async fn orchestration_status_tool_async(
        &self,
        args: Map<String, Value>,
    ) -> Result<CallToolResult, McpError> {
        let args: OrchestrationStatusArgs = parse_tool_args("orchestration_status", args)?;
        let mut status = self.orchestration.status().map_err(mcp_invalid_request)?;
        self.decorate_status_with_runtime(&mut status).await;
        let status = filter_orchestration_status(status, args).map_err(mcp_invalid_request)?;
        Self::json_result(status)
    }

    async fn list_sessions_tool(
        &self,
        args: Map<String, Value>,
    ) -> Result<CallToolResult, McpError> {
        let args: ListSessionsArgs = parse_tool_args("list_sessions", args)?;
        let live_sessions = match self
            .node_tmux(
                &args.node,
                vec![
                    "list-sessions".into(),
                    "-F".into(),
                    SESSION_LIST_FORMAT.into(),
                ],
                Duration::from_secs(20),
            )
            .await
        {
            Ok(output) => parse_session_list(&args.node, &output),
            Err(error) if is_no_tmux_sessions_error(&error) => Vec::new(),
            Err(error) => return Ok(Self::error_result(error)),
        };
        let state = self.orchestration.snapshot().map_err(mcp_invalid_request)?;
        let project_id =
            resolve_project_id_or_slug(&state, &args.project_id).map_err(mcp_invalid_request)?;
        let entries =
            project_scoped_session_entries(&state, &project_id, &args.node, &live_sessions)
                .map_err(mcp_invalid_request)?;
        Self::json_result(entries)
    }

    async fn admin_list_node_sessions_tool(
        &self,
        args: Map<String, Value>,
    ) -> Result<CallToolResult, McpError> {
        let node = args.get("node").and_then(|v| v.as_str()).unwrap_or("local");
        match self
            .node_tmux(
                node,
                vec![
                    "list-sessions".into(),
                    "-F".into(),
                    SESSION_LIST_FORMAT.into(),
                ],
                Duration::from_secs(20),
            )
            .await
        {
            Ok(output) => Self::json_result(parse_session_list(node, &output)),
            Err(error) if is_no_tmux_sessions_error(&error) => {
                Self::json_result(Vec::<SessionListEntry>::new())
            }
            Err(error) => Ok(Self::error_result(error)),
        }
    }

    async fn orchestration_cleanup_zombies_tool(
        &self,
        args: Map<String, Value>,
    ) -> Result<CallToolResult, McpError> {
        let args: OrchestrationCleanupZombiesArgs =
            parse_tool_args("orchestration_cleanup_zombies", args)?;
        if args.node != "local" {
            return Err(McpError::invalid_request(
                "orchestration_cleanup_zombies supports only node='local' in v1",
                None,
            ));
        }

        let mut warnings = Vec::new();
        let live_sessions = match self.list_live_local_sessions().await {
            Ok(live_sessions) => live_sessions,
            Err(error) => {
                warnings.push(format!("local runtime unavailable: {error}"));
                Vec::new()
            }
        };
        let state = self.orchestration.snapshot().map_err(mcp_invalid_request)?;
        let candidates = cleanup_candidates_from_live_sessions(
            "local",
            &live_sessions,
            &durable_session_keys(&state),
            args.older_than_seconds,
            now_ms() / 1000,
        );
        let mut killed = Vec::new();
        if !args.dry_run {
            let durable_keys = durable_session_keys(&state);
            let (targets, target_warnings) = safe_cleanup_kill_targets(&candidates, &durable_keys);
            warnings.extend(target_warnings);
            for target in targets {
                match self
                    .node_tmux(
                        &args.node,
                        vec!["kill-session".into(), "-t".into(), target.clone()],
                        Duration::from_secs(20),
                    )
                    .await
                {
                    Ok(_) => killed.push(target),
                    Err(error) => {
                        warnings.push(format!("failed to kill candidate '{}': {}", target, error))
                    }
                }
            }
        }

        Self::json_result(OrchestrationCleanupZombiesResult {
            node: args.node,
            dry_run: args.dry_run,
            candidates,
            killed,
            warnings,
        })
    }

    async fn orchestration_prune_store_tool(
        &self,
        args: Map<String, Value>,
    ) -> Result<CallToolResult, McpError> {
        let args: OrchestrationPruneStoreArgs = parse_tool_args("orchestration_prune_store", args)?;
        if args.node != "local" {
            return Err(McpError::invalid_request(
                "orchestration_prune_store supports only node='local' in v1",
                None,
            ));
        }
        let live_sessions = self
            .list_live_local_sessions()
            .await
            .map_err(mcp_invalid_request)?;
        let live_session_names = live_sessions
            .into_iter()
            .map(|session| session.session)
            .collect::<HashSet<_>>();
        let report = self
            .orchestration
            .prune_stale_session_records(
                &live_session_names,
                args.dry_run,
                args.sessions_only,
                args.older_than_days,
            )
            .map_err(mcp_invalid_request)?;
        Self::json_result(report)
    }

    async fn decorate_status_with_runtime(&self, status: &mut OrchestrationStatus) {
        let startup_warnings = self
            .startup_warnings
            .lock()
            .map(|warnings| {
                warnings
                    .iter()
                    .filter(|warning| !is_zombie_cleanup_warning(warning))
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_else(|_| vec!["startup warning lock poisoned".into()]);
        status.warnings.extend(startup_warnings);

        match self.list_live_local_sessions().await {
            Ok(live_sessions) => {
                decorate_orchestration_status_with_local_runtime(
                    status,
                    &live_sessions,
                    "local",
                    None,
                    now_ms() / 1000,
                );
            }
            Err(error) => {
                status
                    .warnings
                    .push(format!("local runtime unavailable: {error}"));
                for session in &mut status.sessions {
                    if session.runtime_state.is_none() {
                        session.runtime_state = Some("unknown".into());
                    }
                }
                status.counts = summarize_orchestration_counts(status);
            }
        }
        status.warnings.sort();
        status.warnings.dedup();
    }

    async fn list_live_local_sessions(&self) -> Result<Vec<LocalSessionInfo>, String> {
        match self
            .node_tmux(
                "local",
                vec![
                    "list-sessions".into(),
                    "-F".into(),
                    SESSION_INFO_LIST_FORMAT.into(),
                ],
                Duration::from_secs(20),
            )
            .await
        {
            Ok(output) => Ok(parse_local_session_info_list(&output)),
            Err(error) if is_no_tmux_sessions_error(&error) => Ok(Vec::new()),
            Err(error) => Err(error),
        }
    }

    async fn reconcile_startup_local_sessions(&self) {
        let mut warnings = Vec::new();
        let live_sessions = match self.list_live_local_sessions().await {
            Ok(live_sessions) => live_sessions,
            Err(error) => {
                warnings.push(format!(
                    "startup reconciliation skipped local runtime listing: {error}"
                ));
                self.extend_startup_warnings(warnings);
                return;
            }
        };
        let state = match self.orchestration.snapshot() {
            Ok(state) => state,
            Err(error) => {
                warnings.push(format!(
                    "startup reconciliation skipped orchestration snapshot: {error}"
                ));
                self.extend_startup_warnings(warnings);
                return;
            }
        };
        for action in plan_local_startup_reconciliation(&state, &live_sessions, &self.profiles) {
            match action {
                LocalStartupReconciliationAction::Recreate { record } => {
                    let profile = match self.resolve_profile(Some(&record.profile)) {
                        Some(profile) => profile,
                        None => {
                            warnings.push(format!(
                                "stored active session '{}' is missing and profile '{}' is not loaded",
                                record.key(),
                                record.profile
                            ));
                            continue;
                        }
                    };
                    let command = match profile_launch_command(&profile, record.bypass_permissions)
                    {
                        Ok(command) => command,
                        Err(error) => {
                            warnings.push(format!(
                                "stored active session '{}' is missing and cannot be recreated: {}",
                                record.key(),
                                error
                            ));
                            continue;
                        }
                    };
                    match self
                        .create_coding_session_with_command(
                            "local",
                            &record.session.0,
                            &command,
                            Some(record.workspace_path.as_str()),
                            &profile,
                        )
                        .await
                    {
                        Ok(_) => {
                            warnings.push(format!(
                                "recreated stored active session '{}'; operator may need to provide fresh task context",
                                record.key()
                            ));
                        }
                        Err(error) => warnings.push(format!(
                            "stored active session '{}' is missing and recreation failed: {}",
                            record.key(),
                            error
                        )),
                    }
                }
                LocalStartupReconciliationAction::Missing { key, reason } => {
                    warnings.push(format!(
                        "stored active session '{}' is missing and cannot be recreated: {}",
                        key, reason
                    ));
                }
                LocalStartupReconciliationAction::Historical { .. } => {}
            }
        }
        self.extend_startup_warnings(warnings);
    }

    fn extend_startup_warnings(&self, warnings: Vec<String>) {
        if warnings.is_empty() {
            return;
        }
        if let Ok(mut startup_warnings) = self.startup_warnings.lock() {
            startup_warnings.extend(warnings);
            startup_warnings.sort();
            startup_warnings.dedup();
        }
    }

    fn task_aware_start_metadata(
        &self,
        args: &Map<String, Value>,
    ) -> Result<Option<TaskAwareStart>, McpError> {
        let has_task_metadata = ["task_id", "role", "kind", "skills"]
            .iter()
            .any(|field| args.contains_key(*field))
            || args
                .get("generate_session_name")
                .and_then(|value| value.as_bool())
                .unwrap_or(false);
        if !has_task_metadata {
            return Ok(None);
        }

        for field in ["profile", "node", "workspace_path", "bypass_permissions"] {
            if !args.contains_key(field) {
                return Err(McpError::invalid_request(
                    format!("task-aware start_coding_session requires explicit {field}"),
                    None,
                ));
            }
        }
        for field in ["profile", "node", "workspace_path"] {
            if args
                .get(field)
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .is_none()
            {
                return Err(McpError::invalid_request(
                    format!("{field} must be a non-empty string"),
                    None,
                ));
            }
        }
        if args
            .get("bypass_permissions")
            .and_then(|value| value.as_bool())
            .is_none()
        {
            return Err(McpError::invalid_request(
                "bypass_permissions must be a boolean",
                None,
            ));
        }

        let task_id = args
            .get("task_id")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| McpError::invalid_request("task_id is required", None))?
            .to_owned();
        let role = args
            .get("role")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| McpError::invalid_request("role is required", None))?
            .to_owned();
        let kind = args
            .get("kind")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| McpError::invalid_request("kind is required", None))?
            .to_owned();
        let skills = string_vec_arg(args, "skills")?;
        let workspace_path = args
            .get("workspace_path")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .ok_or_else(|| McpError::invalid_request("workspace_path is required", None))?;
        let generate_session_name = args
            .get("generate_session_name")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        if !generate_session_name && !args.contains_key("session") {
            return Err(McpError::invalid_request(
                "task-aware start_coding_session requires session or generate_session_name",
                None,
            ));
        }

        let state = self.orchestration.snapshot().map_err(mcp_invalid_request)?;
        let task_id = TaskId(task_id);
        let task = state
            .tasks
            .get(&task_id)
            .ok_or_else(|| mcp_invalid_request(format!("task '{}' not found", task_id.0)))?;

        let session_name = if generate_session_name {
            generated_orchestration_session_name(&task.slug, &kind, &short_session_suffix())
        } else {
            args.get("session")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| McpError::invalid_request("session is required", None))?
                .to_owned()
        };

        Ok(Some(TaskAwareStart {
            session_name,
            workspace_path,
            task_id,
            role,
            kind,
            skills,
            previous_session: task.session.clone(),
        }))
    }

    fn default_profile_name(&self) -> Option<&str> {
        self.default_coder_profile.as_deref()
    }

    fn resolve_profile(&self, name: Option<&str>) -> Option<CliProfile> {
        let profile_name = name
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .or_else(|| self.default_profile_name())?;
        mmux_node::get_profile(&self.profiles, profile_name)
    }

    async fn start_coding_session_tool(
        &self,
        args: Map<String, Value>,
    ) -> Result<CallToolResult, McpError> {
        if args.contains_key("timeout_seconds") {
            return Err(McpError::invalid_request(
                "start_coding_session no longer waits for readiness; use wait_start kind='coding-ready' with timeout_seconds",
                None,
            ));
        }
        if args.contains_key("cwd") {
            return Err(McpError::invalid_request(
                "start_coding_session uses workspace_path, not cwd",
                None,
            ));
        }
        let profile = self
            .resolve_profile(args.get("profile").and_then(|v| v.as_str()))
            .ok_or_else(|| McpError::invalid_request("unknown profile", None))?;
        let task_metadata = self.task_aware_start_metadata(&args)?;
        let node = args.get("node").and_then(|v| v.as_str()).unwrap_or("local");
        let session_name = task_metadata
            .as_ref()
            .map(|metadata| metadata.session_name.as_str())
            .or_else(|| args.get("session").and_then(|v| v.as_str()))
            .unwrap_or(profile.name.as_str())
            .to_owned();
        let workspace_path = args
            .get("workspace_path")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned);
        let bypass_permissions = args
            .get("bypass_permissions")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let cmd = profile_launch_command(&profile, bypass_permissions)
            .map_err(|error| McpError::invalid_request(error, None))?;

        let message = match self
            .create_coding_session_with_command(
                node,
                &session_name,
                cmd,
                workspace_path.as_deref(),
                &profile,
            )
            .await
        {
            Ok(message) => message,
            Err(e) => return Ok(Self::error_result(e)),
        };

        let session_record = if let Some(metadata) = task_metadata {
            self.stop_replaced_task_session(
                metadata.previous_session.as_ref(),
                node,
                &session_name,
            )
            .await
            .map_err(mcp_invalid_request)?;
            Some(
                self.orchestration
                    .record_session(
                        metadata.task_id,
                        TaskSession {
                            node_id: NodeId(node.to_owned()),
                            session: SessionId(session_name.clone()),
                            profile: profile.name.clone(),
                            workspace_path: metadata.workspace_path,
                            bypass_permissions,
                            role: metadata.role,
                            kind: metadata.kind,
                            skills: metadata.skills,
                            created_at_ms: 0,
                            updated_at_ms: 0,
                            last_seen_ms: 0,
                        },
                    )
                    .map_err(mcp_invalid_request)?,
            )
        } else {
            None
        };

        Self::json_result(json!({
            "message": message,
            "node": node,
            "session": session_name,
            "profile": profile.name,
            "workspace_path": workspace_path,
            "bypass_permissions": bypass_permissions,
            "readiness": {
                "status": "not_waited",
                "next_tool": "wait_start",
                "kind": "coding-ready",
                "profile": profile.name,
            },
            "session_record": session_record,
        }))
    }

    async fn create_session_with_command(
        &self,
        node: &str,
        session: &str,
        cmd: &str,
        workspace_path: Option<&str>,
    ) -> Result<String, String> {
        if self.node_session_exists(node, session).await? {
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
        if let Some(workspace_path) = workspace_path {
            tmux_args.push("-c".into());
            tmux_args.push(workspace_path.into());
        }
        tmux_args.push(cmd.into());
        self.node_tmux(node, tmux_args, Duration::from_secs(30))
            .await?;
        tokio::time::sleep(Duration::from_millis(300)).await;
        Ok(format!(
            "Created session '{}' with command '{}' on node '{}'",
            session, cmd, node
        ))
    }

    async fn create_coding_session_with_command(
        &self,
        node: &str,
        session: &str,
        cmd: &str,
        workspace_path: Option<&str>,
        profile: &CliProfile,
    ) -> Result<String, String> {
        match profile_launch_strategy(profile)? {
            "direct" => {
                self.create_session_with_command(node, session, cmd, workspace_path)
                    .await
            }
            "shell_send" => {
                let exists = self.node_session_exists(node, session).await?;
                if exists {
                    return Ok(format!("Session '{}' already exists", session));
                }

                self.create_session_with_command(node, session, "bash", workspace_path)
                    .await?;

                tokio::time::sleep(Duration::from_millis(1000)).await;

                self.node_tmux(
                    node,
                    vec![
                        "send-keys".into(),
                        "-l".into(),
                        "-t".into(),
                        session.into(),
                        cmd.into(),
                    ],
                    Duration::from_secs(20),
                )
                .await?;
                tokio::time::sleep(coding_prompt_submit_delay(cmd)).await;
                self.node_tmux(
                    node,
                    vec![
                        "send-keys".into(),
                        "-t".into(),
                        session.into(),
                        "Enter".into(),
                    ],
                    Duration::from_secs(20),
                )
                .await?;

                Ok(format!(
                    "Created session '{}' with shell-send command '{}'",
                    session, cmd
                ))
            }
            _ => unreachable!("profile_launch_strategy validates supported values"),
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

    async fn node_command(
        &self,
        node_id: &str,
        kind: NodeCommandKind,
        timeout: Duration,
    ) -> Result<NodeCommandResult, String> {
        node_execution_actor_call(&self.node_executor, |reply| NodeExecutionMessage::Command {
            node_id: node_id.to_owned(),
            kind,
            timeout,
            reply,
        })
        .await
    }

    async fn node_tmux(
        &self,
        node_id: &str,
        args: Vec<String>,
        timeout: Duration,
    ) -> Result<String, String> {
        match self
            .node_command(node_id, NodeCommandKind::Tmux { args }, timeout)
            .await?
        {
            NodeCommandResult::TmuxOutput(output) => Ok(output),
            NodeCommandResult::Error { message } => Err(message),
            other => Err(format!("unexpected tmux command result: {:?}", other)),
        }
    }

    async fn node_session_exists(&self, node_id: &str, session: &str) -> Result<bool, String> {
        match self
            .node_command(
                node_id,
                NodeCommandKind::Tmux {
                    args: vec!["has-session".into(), "-t".into(), session.into()],
                },
                Duration::from_secs(10),
            )
            .await
        {
            Ok(NodeCommandResult::TmuxOutput(_)) => Ok(true),
            Ok(NodeCommandResult::Error { message }) if is_tmux_missing_session_error(&message) => {
                Ok(false)
            }
            Ok(NodeCommandResult::Error { message }) => Err(format!(
                "failed to check session '{}' on node '{}': {}",
                session, node_id, message
            )),
            Ok(other) => Err(format!(
                "unexpected session existence result from node '{}': {:?}",
                node_id, other
            )),
            Err(error) => Err(format!(
                "node '{}' is unreachable while checking session '{}': {}",
                node_id, session, error
            )),
        }
    }

    async fn stop_replaced_task_session(
        &self,
        previous: Option<&TaskSession>,
        new_node: &str,
        new_session: &str,
    ) -> Result<bool, String> {
        let Some(previous) = previous else {
            return Ok(false);
        };
        if previous.node_id.0 == new_node && previous.session.0 == new_session {
            return Ok(false);
        }
        match self
            .node_session_exists(&previous.node_id.0, &previous.session.0)
            .await
        {
            Ok(false) => Ok(false),
            Ok(true) => match self
                .node_tmux(
                    &previous.node_id.0,
                    vec![
                        "kill-session".into(),
                        "-t".into(),
                        previous.session.0.clone(),
                    ],
                    Duration::from_secs(20),
                )
                .await
            {
                Ok(_) => Ok(true),
                Err(error) if is_tmux_missing_session_error(&error) => Ok(false),
                Err(error) => Err(format!(
                    "failed to stop previous task session '{}:{}': {}",
                    previous.node_id.0, previous.session.0, error
                )),
            },
            Err(error) => Err(format!(
                "failed to inspect previous task session '{}:{}': {}",
                previous.node_id.0, previous.session.0, error
            )),
        }
    }

    async fn node_session_capture(
        &self,
        node_id: &str,
        session: &str,
        lines: Option<usize>,
        scrollback: bool,
    ) -> Result<String, String> {
        self.node_tmux(
            node_id,
            tmux_capture_output_args(session, lines, scrollback),
            Duration::from_secs(20),
        )
        .await
    }

    async fn node_session_first_pane(
        &self,
        node_id: &str,
        session: &str,
    ) -> Result<String, String> {
        let panes = self
            .node_tmux(
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

    async fn send_coding_prompt(
        &self,
        node: &str,
        session: &str,
        profile_name: Option<&str>,
        prompt: &str,
    ) -> Result<CallToolResult, McpError> {
        validate_coding_prompt(prompt)?;
        let profile = self
            .resolve_profile(profile_name)
            .ok_or_else(|| McpError::invalid_request("unknown profile", None))?;
        let pane = match self.node_session_first_pane(node, session).await {
            Ok(pane) => pane,
            Err(error) => return Ok(Self::error_result(error)),
        };
        let buf = self
            .node_tmux(
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
        if let Some(key) = startup_dismiss_key(&buf, &profile) {
            let _ = self
                .node_tmux(
                    node,
                    node_send_key_args(&pane, &key),
                    Duration::from_secs(20),
                )
                .await;
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
        let buf = self
            .node_tmux(
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
        if mmux_node::profiles::has_blocking_confirmation(&buf, &profile) {
            return Ok(Self::error_result(format!(
                "session '{}' on node '{}' is showing a blocking {} confirmation; use coding_action or manual intervention before sending a prompt",
                session, node, profile.name
            )));
        }
        let text_mode = match profile_text_mode(&profile) {
            Ok(text_mode) => text_mode,
            Err(error) => return Ok(Self::error_result(error)),
        };
        match text_mode {
            "paste-buffer" => {
                let buffer = tmux_buffer_name("mmux-coding-prompt", &pane);
                if let Err(error) = self
                    .node_tmux(
                        node,
                        tmux_set_buffer_args(&buffer, prompt),
                        Duration::from_secs(20),
                    )
                    .await
                {
                    return Ok(Self::error_result(error));
                }
                if let Err(error) = self
                    .node_tmux(
                        node,
                        tmux_paste_buffer_args(&pane, &buffer),
                        Duration::from_secs(20),
                    )
                    .await
                {
                    return Ok(Self::error_result(error));
                }
            }
            "literal-keys" => {
                if let Err(error) = self
                    .node_tmux(
                        node,
                        tmux_literal_text_args(&pane, prompt),
                        Duration::from_secs(20),
                    )
                    .await
                {
                    return Ok(Self::error_result(error));
                }
            }
            _ => unreachable!("profile_text_mode validates supported values"),
        }
        tokio::time::sleep(coding_prompt_submit_delay(prompt)).await;
        if profile.submit_after_text && !profile.submit_keys.trim().is_empty() {
            if let Err(error) = self
                .node_tmux(
                    node,
                    tmux_submit_keys_args(&pane, &profile.submit_keys),
                    Duration::from_secs(20),
                )
                .await
            {
                return Ok(Self::error_result(error));
            }
        }
        Ok(Self::text_result(format!(
            "Sent to {} on node {} (profile: {}, text_mode: {}, submit_after_text: {}): {}",
            session, node, profile.name, text_mode, profile.submit_after_text, prompt
        )))
    }

    async fn node_wait_for(
        &self,
        node_id: &str,
        session: &str,
        options: NodeWaitOptions<'_>,
    ) -> Result<String, String> {
        let NodeWaitOptions {
            mode,
            sentinel,
            prompt,
            timeout,
            poll,
            stability,
        } = options;
        let deadline = Instant::now() + Duration::from_secs_f64(timeout);
        let poll_dur = Duration::from_secs_f64(poll);
        match mode {
            "sentinel" => {
                let sentinel = sentinel.ok_or("sentinel required for sentinel mode")?;
                while Instant::now() < deadline {
                    let output = self
                        .node_session_capture(node_id, session, Some(200), false)
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
                        .node_session_capture(node_id, session, Some(200), false)
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
                        .node_session_capture(node_id, session, Some(200), false)
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

fn default_wait_node() -> String {
    "local".into()
}

fn default_coding_session() -> String {
    "kimi_codex".into()
}

fn runtime_wait_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!("wait-{nanos:x}")
}

async fn node_execution_actor_call<T>(
    node_executor: &ActorRef<NodeExecutionMessage>,
    build: impl FnOnce(RpcReplyPort<Result<T, String>>) -> NodeExecutionMessage,
) -> Result<T, String>
where
    T: Send + 'static,
{
    match node_executor
        .call(build, Some(DEFAULT_NODE_EXECUTION_ACTOR_CALL_TIMEOUT))
        .await
        .map_err(|error| format!("node execution actor call failed: {}", error))?
    {
        CallResult::Success(result) => result,
        CallResult::Timeout => Err(format!(
            "node execution actor call timed out after {}",
            format_duration_for_error(DEFAULT_NODE_EXECUTION_ACTOR_CALL_TIMEOUT)
        )),
        CallResult::SenderError => Err("node execution actor reply channel closed".into()),
    }
}

fn format_duration_for_error(duration: Duration) -> String {
    if duration.as_secs() == 0 {
        return format!("{}ms", duration.as_millis());
    }
    if duration.subsec_millis() == 0 {
        return format!("{}s", duration.as_secs());
    }
    format!("{:.3}s", duration.as_secs_f64())
}

async fn run_runtime_wait_job(runner: RuntimeWaitRunner) {
    let wait_id = runner.wait_id.clone();
    let wait_jobs = runner.wait_jobs.clone();
    let result = runtime_wait_loop(&runner).await;
    let mut jobs = match wait_jobs.lock() {
        Ok(jobs) => jobs,
        Err(_) => return,
    };
    let Some(job) = jobs.get_mut(&wait_id) else {
        return;
    };
    if job.snapshot.status != RuntimeWaitStatus::Pending {
        return;
    }
    let now = now_ms();
    job.handle = None;
    job.snapshot.updated_at_ms = now;
    job.snapshot.completed_at_ms = Some(now);
    match result {
        Ok(detail) => {
            job.snapshot.status = RuntimeWaitStatus::Completed;
            job.snapshot.result = Some(detail);
            job.snapshot.error = None;
        }
        Err(error) => {
            job.snapshot.status = RuntimeWaitStatus::Failed;
            job.snapshot.result = None;
            job.snapshot.error = Some(error);
        }
    }
}

async fn runtime_wait_loop(runner: &RuntimeWaitRunner) -> Result<RuntimeWaitDetail, String> {
    let started = Instant::now();
    let deadline = started + Duration::from_secs_f64(runner.timeout_seconds);
    let poll_dur = Duration::from_secs_f64(runner.poll_seconds);
    match runner.kind {
        RuntimeWaitKind::Sentinel => {
            let sentinel = runner
                .sentinel
                .as_deref()
                .ok_or_else(|| "sentinel is required for sentinel waits".to_string())?;
            while Instant::now() <= deadline {
                let output = runner.target.capture(&runner.session).await?;
                if output.contains(sentinel) {
                    return Ok(RuntimeWaitDetail {
                        message: format!("sentinel found in session '{}'", runner.session),
                        elapsed_ms: started.elapsed().as_millis().try_into().unwrap_or(u64::MAX),
                    });
                }
                tokio::time::sleep(poll_dur).await;
            }
            Err(format!(
                "timeout after {}s waiting for sentinel in session '{}'",
                runner.timeout_seconds, runner.session
            ))
        }
        RuntimeWaitKind::Prompt => {
            let prompt = runner
                .prompt
                .as_deref()
                .ok_or_else(|| "prompt is required for prompt waits".to_string())?;
            while Instant::now() <= deadline {
                let output = runner.target.capture(&runner.session).await?;
                if output.contains(prompt) {
                    return Ok(RuntimeWaitDetail {
                        message: format!("prompt found in session '{}'", runner.session),
                        elapsed_ms: started.elapsed().as_millis().try_into().unwrap_or(u64::MAX),
                    });
                }
                tokio::time::sleep(poll_dur).await;
            }
            Err(format!(
                "timeout after {}s waiting for prompt in session '{}'",
                runner.timeout_seconds, runner.session
            ))
        }
        RuntimeWaitKind::Stable => {
            let stable_needed = (runner.stability_seconds / runner.poll_seconds)
                .ceil()
                .max(1.0) as usize;
            let mut last_output = String::new();
            let mut stable_count = 0usize;
            while Instant::now() <= deadline {
                let output = runner.target.capture(&runner.session).await?;
                if output == last_output {
                    stable_count += 1;
                    if stable_count >= stable_needed {
                        return Ok(RuntimeWaitDetail {
                            message: format!(
                                "output stable for {}s in session '{}'",
                                runner.stability_seconds, runner.session
                            ),
                            elapsed_ms: started
                                .elapsed()
                                .as_millis()
                                .try_into()
                                .unwrap_or(u64::MAX),
                        });
                    }
                } else {
                    stable_count = 0;
                    last_output = output;
                }
                tokio::time::sleep(poll_dur).await;
            }
            Err(format!(
                "timeout after {}s waiting for stable output in session '{}'",
                runner.timeout_seconds, runner.session
            ))
        }
        RuntimeWaitKind::CodingReady => {
            let profile = runner
                .profile
                .as_ref()
                .ok_or_else(|| "profile is required for coding-ready waits".to_string())?;
            let required_stability = Duration::from_secs_f64(runner.stability_seconds);
            let mut idle_since: Option<Instant> = None;
            while Instant::now() <= deadline {
                let output = runner.target.capture(&runner.session).await?;
                if let Some(key) = startup_dismiss_key(&output, profile) {
                    let _ = runner.target.send_key(&runner.session, &key).await;
                    idle_since = None;
                    tokio::time::sleep(Duration::from_millis(300)).await;
                    continue;
                }
                if profile_turn_idle(&output, profile) {
                    let now = Instant::now();
                    let since = *idle_since.get_or_insert(now);
                    if now.duration_since(since) >= required_stability {
                        return Ok(RuntimeWaitDetail {
                            message: format!(
                                "{} is ready (profile: {})",
                                runner.session, profile.name
                            ),
                            elapsed_ms: started
                                .elapsed()
                                .as_millis()
                                .try_into()
                                .unwrap_or(u64::MAX),
                        });
                    }
                } else {
                    idle_since = None;
                }
                if required_stability.is_zero() && profile_turn_idle(&output, profile) {
                    return Ok(RuntimeWaitDetail {
                        message: format!("{} is ready (profile: {})", runner.session, profile.name),
                        elapsed_ms: started.elapsed().as_millis().try_into().unwrap_or(u64::MAX),
                    });
                }
                tokio::time::sleep(poll_dur).await;
            }
            Err(format!(
                "timeout after {}s waiting for {} to be coding-ready",
                runner.timeout_seconds, runner.session
            ))
        }
    }
}

impl RuntimeWaitTarget {
    async fn capture(&self, session: &str) -> Result<String, String> {
        self.tmux(
            tmux_capture_output_args(session, Some(200), false),
            Duration::from_secs(20),
        )
        .await
    }

    async fn send_key(&self, session: &str, key: &str) -> Result<(), String> {
        self.tmux(node_send_key_args(session, key), Duration::from_secs(20))
            .await
            .map(|_| ())
    }

    async fn tmux(&self, args: Vec<String>, timeout: Duration) -> Result<String, String> {
        let RuntimeWaitTarget::Node {
            node_executor,
            node_id,
        } = self;
        match node_execution_actor_call(node_executor, |reply| NodeExecutionMessage::Command {
            node_id: node_id.clone(),
            kind: NodeCommandKind::Tmux { args },
            timeout,
            reply,
        })
        .await?
        {
            NodeCommandResult::TmuxOutput(output) => Ok(output),
            NodeCommandResult::Error { message } => Err(message),
            other => Err(format!("unexpected tmux command result: {:?}", other)),
        }
    }
}

fn parse_tool_args<T: DeserializeOwned>(
    tool_name: &str,
    args: Map<String, Value>,
) -> Result<T, McpError> {
    serde_json::from_value(Value::Object(args)).map_err(|error| {
        McpError::invalid_request(format!("invalid {tool_name} arguments: {error}"), None)
    })
}

fn mcp_invalid_request(error: impl ToString) -> McpError {
    McpError::invalid_request(error.to_string(), None)
}

fn validate_prompt_text_value(field: &str, prompt: &str) -> Result<(), String> {
    let trimmed = prompt.trim();
    if trimmed.is_empty() {
        return Err(format!("{field} must not be empty"));
    }
    if matches!(trimmed, "null" | "undefined") {
        return Err(format!(
            "{field} must not be the placeholder string '{trimmed}'"
        ));
    }
    Ok(())
}

fn validate_coding_prompt(prompt: &str) -> Result<(), McpError> {
    validate_prompt_text_value("coding_send prompt", prompt).map_err(mcp_invalid_request)
}

fn build_coding_task_prompt(
    state: &OrchestrationState,
    args: &CodingTaskSendArgs,
) -> Result<String, String> {
    validate_prompt_text_value("task_id_or_slug", &args.task_id_or_slug)?;
    validate_prompt_text_value("coding_task_send prompt", &args.prompt)?;
    if let Some(extra_context) = args.extra_context.as_deref() {
        validate_prompt_text_value("extra_context", extra_context)?;
    }
    let context_tasks = resolve_context_task_cards(state, args.context_task_ids.as_deref())?;

    let task = resolve_task_by_id_or_slug(state, &args.task_id_or_slug)?;
    let plan = state.plans.get(&task.plan_id);
    let project = plan.and_then(|plan| state.projects.get(&plan.project_id));
    let project_text = project
        .map(|project| format!("{} / {}", project.id.0, project.slug))
        .unwrap_or_else(|| "<missing>".into());
    let plan_text = plan
        .map(|plan| format!("{} / {}", plan.id.0, plan.slug))
        .unwrap_or_else(|| format!("{} / <missing>", task.plan_id.0));

    let template = args.template.unwrap_or(CodingTaskSendTemplate::Task);
    let mut rendered = coding_task_send_template_text(template).to_owned();
    for (placeholder, value) in [
        ("{{task_id}}", task.id.0.as_str()),
        ("{{task_slug}}", task.slug.as_str()),
        ("{{task_title}}", task.title.as_str()),
        ("{{task_status}}", task_status_name(task.status)),
        ("{{project}}", project_text.as_str()),
        ("{{plan}}", plan_text.as_str()),
        ("{{objective}}", task.objective.as_str()),
    ] {
        rendered = rendered.replace(placeholder, value);
    }

    rendered = rendered
        .replace(
            "{{scope_section}}",
            &optional_section(
                args.include_scope.unwrap_or(true),
                build_scope_section(&task.scope),
            ),
        )
        .replace(
            "{{plan_brief_section}}",
            &build_plan_brief_section(plan.map(|plan| plan.brief.as_str())),
        )
        .replace(
            "{{gates_section}}",
            &optional_section(
                args.include_gates.unwrap_or(true),
                build_gates_section(&task.gates),
            ),
        )
        .replace(
            "{{dependencies_section}}",
            &optional_section(
                args.include_dependencies.unwrap_or(true),
                build_dependencies_section(state, task),
            ),
        )
        .replace(
            "{{blockers_section}}",
            &build_blockers_section(&task.blockers),
        )
        .replace(
            "{{task_card_context_section}}",
            &build_task_card_context_section(state, &context_tasks),
        )
        .replace(
            "{{extra_context_section}}",
            &args
                .extra_context
                .as_deref()
                .map(build_extra_context_section)
                .unwrap_or_default(),
        )
        .replace("{{instruction}}", args.prompt.trim());

    Ok(rendered)
}

fn resolve_context_task_cards<'a>(
    state: &'a OrchestrationState,
    selectors: Option<&'a [String]>,
) -> Result<Vec<&'a Task>, String> {
    let Some(selectors) = selectors else {
        return Ok(Vec::new());
    };
    let mut seen = HashSet::new();
    let mut tasks = Vec::new();
    for (index, selector) in selectors.iter().enumerate() {
        validate_prompt_text_value(&format!("context_task_ids[{index}]"), selector)?;
        let task = resolve_task_by_id_or_slug(state, selector)?;
        if !seen.insert(task.id.clone()) {
            return Err(format!(
                "context_task_ids contains duplicate task '{}'",
                task.id.0
            ));
        }
        tasks.push(task);
    }
    Ok(tasks)
}

fn coding_task_send_template_text(template: CodingTaskSendTemplate) -> &'static str {
    match template {
        CodingTaskSendTemplate::Task => CODING_TASK_SEND_PROMPT,
        CodingTaskSendTemplate::Validate => CODING_VALIDATE_SEND_PROMPT,
        CodingTaskSendTemplate::Review => CODING_REVIEW_SEND_PROMPT,
        CodingTaskSendTemplate::QualityGuard => CODING_QUALITY_GUARD_SEND_PROMPT,
    }
}

fn optional_section(include: bool, section: String) -> String {
    if include {
        section
    } else {
        String::new()
    }
}

fn resolve_task_by_id_or_slug<'a>(
    state: &'a OrchestrationState,
    task_id_or_slug: &str,
) -> Result<&'a Task, String> {
    let selector = task_id_or_slug.trim();
    let task_id = TaskId(selector.to_owned());
    if let Some(task) = state.tasks.get(&task_id) {
        return Ok(task);
    }

    let mut matches = state
        .tasks
        .values()
        .filter(|task| task.slug == selector)
        .collect::<Vec<_>>();
    matches.sort_by(|left, right| left.id.0.cmp(&right.id.0));
    match matches.as_slice() {
        [task] => Ok(task),
        [] => Err(format!("task '{}' not found", selector)),
        tasks => {
            let matches = tasks
                .iter()
                .map(|task| format!("{} in plan {}", task.id.0, task.plan_id.0))
                .collect::<Vec<_>>()
                .join(", ");
            Err(format!(
                "task slug '{}' is ambiguous; matches: {}",
                selector, matches
            ))
        }
    }
}

fn build_scope_section(scope: &TaskScope) -> String {
    format!(
        "Scope:\nInclude paths:\n{}Exclude paths:\n{}Notes:\n{}\n\n",
        format_string_list(&scope.include_paths),
        format_string_list(&scope.exclude_paths),
        scope.notes.as_deref().unwrap_or("- none")
    )
}

fn build_plan_brief_section(brief: Option<&str>) -> String {
    format!(
        "Plan Brief:\n{}\n\n",
        brief
            .map(str::trim)
            .filter(|brief| !brief.is_empty())
            .unwrap_or("- missing")
    )
}

fn build_gates_section(gates: &[String]) -> String {
    format!("Gates:\n{}\n", format_string_list(gates))
}

fn build_dependencies_section(state: &OrchestrationState, task: &Task) -> String {
    let parents = state
        .task_edges
        .iter()
        .filter(|edge| edge.kind == TaskEdgeKind::ParentOf && edge.to == task.id)
        .collect::<Vec<_>>();
    let dependencies = state
        .task_edges
        .iter()
        .filter(|edge| edge.kind == TaskEdgeKind::DependsOn && edge.from == task.id)
        .collect::<Vec<_>>();
    let blocking_tasks = state
        .task_edges
        .iter()
        .filter(|edge| edge.kind == TaskEdgeKind::Blocks && edge.to == task.id)
        .collect::<Vec<_>>();

    format!(
        "Dependencies:\nParent:\n{}Depends on:\n{}Blocked by task edges:\n{}\n",
        format_task_edge_list(state, &parents, |edge| &edge.from),
        format_task_edge_list(state, &dependencies, |edge| &edge.to),
        format_task_edge_list(state, &blocking_tasks, |edge| &edge.from),
    )
}

fn build_blockers_section(blockers: &[String]) -> String {
    format!("Blockers:\n{}\n", format_string_list(blockers))
}

fn build_extra_context_section(extra_context: &str) -> String {
    format!("Extra Context:\n{}\n\n", extra_context.trim())
}

fn build_task_card_context_section(state: &OrchestrationState, tasks: &[&Task]) -> String {
    if tasks.is_empty() {
        return String::new();
    }

    let mut section = String::from(
        "Operator Task Card Bundle:\n\
         Purpose: read-only multi-task evidence supplied by the operator. Do not call mmux from the worker session to reconstruct this context.\n\
         Field checklist for each card: id, plan_id, slug, title, objective, status, outcome, gates, scope, blockers, edges, session.\n\
         Validation rule: every gate must be addressed by the outcome, command evidence, an explicit caveat, or a named waiver.\n\n",
    );
    for task in tasks {
        section.push_str(&format!("Task Card: {}\n", task.id.0));
        section.push_str(&format!("Plan: {}\n", task.plan_id.0));
        section.push_str(&format!("Slug: {}\n", task.slug));
        section.push_str(&format!("Title: {}\n", task.title));
        section.push_str(&format!("Status: {}\n", task_status_name(task.status)));
        section.push_str(&format!("Objective:\n{}\n", task.objective));
        section.push_str(&build_scope_section(&task.scope));
        section.push_str(&build_gates_section(&task.gates));
        section.push_str(&format!(
            "Outcome:\n{}\n",
            task.outcome.as_deref().unwrap_or("- none")
        ));
        section.push_str(&build_blockers_section(&task.blockers));
        section.push_str("Incoming edges:\n");
        section.push_str(&format_task_edges(
            state,
            &state
                .task_edges
                .iter()
                .filter(|edge| edge.to == task.id)
                .collect::<Vec<_>>(),
        ));
        section.push_str("Outgoing edges:\n");
        section.push_str(&format_task_edges(
            state,
            &state
                .task_edges
                .iter()
                .filter(|edge| edge.from == task.id)
                .collect::<Vec<_>>(),
        ));
        section.push_str("Session:\n");
        section.push_str(&format_task_session(task.session.as_ref()));
        section.push('\n');
    }
    section
}

fn format_task_edges(state: &OrchestrationState, edges: &[&TaskEdge]) -> String {
    if edges.is_empty() {
        return "- none\n".into();
    }
    let mut labels = edges
        .iter()
        .map(|edge| {
            format!(
                "- {} -> {} kind={:?} note={}\n",
                task_label(state, &edge.from),
                task_label(state, &edge.to),
                edge.kind,
                edge.note.as_deref().unwrap_or("<none>")
            )
        })
        .collect::<Vec<_>>();
    labels.sort();
    labels.concat()
}

fn format_task_session(session: Option<&TaskSession>) -> String {
    let Some(record) = session else {
        return "- none\n".into();
    };
    format!(
        "- node={} session={} profile={} role={} kind={} skills={} workspace={} bypass_permissions={}\n",
        record.node_id.0,
        record.session.0,
        record.profile,
        record.role,
        record.kind,
        format_inline_list(&record.skills),
        record.workspace_path,
        record.bypass_permissions
    )
}

fn format_inline_list(values: &[String]) -> String {
    if values.is_empty() {
        "<none>".into()
    } else {
        values.join(",")
    }
}

fn format_string_list(values: &[String]) -> String {
    if values.is_empty() {
        return "- none\n".into();
    }
    values
        .iter()
        .map(|value| format!("- {}\n", value))
        .collect::<String>()
}

fn format_task_edge_list<'a>(
    state: &OrchestrationState,
    edges: &[&'a TaskEdge],
    task_id: impl Fn(&'a TaskEdge) -> &'a TaskId,
) -> String {
    if edges.is_empty() {
        return "- none\n".into();
    }
    let mut labels = edges
        .iter()
        .map(|edge| task_label(state, task_id(edge)))
        .collect::<Vec<_>>();
    labels.sort();
    labels
        .into_iter()
        .map(|label| format!("- {}\n", label))
        .collect()
}

fn task_label(state: &OrchestrationState, task_id: &TaskId) -> String {
    state.tasks.get(task_id).map_or_else(
        || format!("{} / <missing>", task_id.0),
        |task| {
            format!(
                "{} / {} / {} [{}]",
                task.id.0,
                task.slug,
                task.title,
                task_status_name(task.status)
            )
        },
    )
}

fn task_status_name(status: TaskStatus) -> &'static str {
    match status {
        TaskStatus::Backlog => "Backlog",
        TaskStatus::Planned => "Planned",
        TaskStatus::Running => "Running",
        TaskStatus::WaitingForValidation => "WaitingForValidation",
        TaskStatus::Blocked => "Blocked",
        TaskStatus::Failed => "Failed",
        TaskStatus::Passed => "Passed",
        TaskStatus::Delivered => "Delivered",
        TaskStatus::Canceled => "Canceled",
    }
}

fn filter_orchestration_status(
    mut status: OrchestrationStatus,
    args: OrchestrationStatusArgs,
) -> Result<OrchestrationStatus, String> {
    let project_filter = args
        .project_id
        .as_deref()
        .map(|selector| resolve_project_id_or_slug_from_status(&status, selector))
        .transpose()?;
    let plan_filter = args
        .plan_id
        .as_deref()
        .map(|selector| resolve_plan_id_or_slug_from_status(&status, selector))
        .transpose()?;
    let task_filter = args.task_id.map(TaskId);
    if let Some(project_id) = project_filter.as_ref() {
        if !status
            .projects
            .iter()
            .any(|project| &project.id == project_id)
        {
            return Err(format!("project '{}' not found", project_id.0));
        }
    }
    if let Some(plan_id) = plan_filter.as_ref() {
        if !status.plans.iter().any(|plan| &plan.id == plan_id) {
            return Err(format!("plan '{}' not found", plan_id.0));
        }
    }
    if let Some(task_id) = task_filter.as_ref() {
        if !status.tasks.iter().any(|task| &task.id == task_id) {
            return Err(format!("task '{}' not found", task_id.0));
        }
    }

    status.plans.retain(|plan| {
        let project_matches = project_filter
            .as_ref()
            .map(|project_id| &plan.project_id == project_id)
            .unwrap_or(true);
        let plan_matches = plan_filter
            .as_ref()
            .map(|plan_id| &plan.id == plan_id)
            .unwrap_or(true);
        let completion_matches = args.include_completed || !plan.status.is_finished();
        project_matches && plan_matches && completion_matches
    });
    let visible_plan_ids = status
        .plans
        .iter()
        .map(|plan| plan.id.clone())
        .collect::<std::collections::HashSet<_>>();

    status.tasks.retain(|task| {
        let plan_visible = visible_plan_ids.contains(&task.plan_id);
        let plan_matches = plan_filter
            .as_ref()
            .map(|plan_id| &task.plan_id == plan_id)
            .unwrap_or(true);
        let task_matches = task_filter
            .as_ref()
            .map(|task_id| &task.id == task_id)
            .unwrap_or(true);
        let completion_matches = args.include_completed || !task.status.is_finished();
        plan_visible && plan_matches && task_matches && completion_matches
    });

    let visible_task_ids = status
        .tasks
        .iter()
        .map(|task| task.id.clone())
        .collect::<std::collections::HashSet<_>>();
    status.task_edges.retain(|edge| {
        visible_task_ids.contains(&edge.from) || visible_task_ids.contains(&edge.to)
    });
    status.sessions.retain(|session| {
        if task_filter.is_some() || plan_filter.is_some() || project_filter.is_some() {
            return visible_task_ids.contains(&session.task_id);
        }
        true
    });
    let visible_project_ids = status
        .plans
        .iter()
        .map(|plan| plan.project_id.clone())
        .collect::<std::collections::HashSet<_>>();
    status.projects.retain(|project| {
        if let Some(project_id) = project_filter.as_ref() {
            return &project.id == project_id;
        }
        if task_filter.is_some() {
            return visible_project_ids.contains(&project.id);
        }
        true
    });
    status.counts = summarize_orchestration_counts(&status);
    Ok(status)
}

fn resolve_project_id_or_slug_from_status(
    status: &OrchestrationStatus,
    project_id_or_slug: &str,
) -> Result<ProjectId, String> {
    let selector = project_id_or_slug.trim();
    if selector.is_empty() {
        return Err("project_id must not be empty".into());
    }
    status
        .projects
        .iter()
        .find(|project| project.id.0 == selector || project.slug == selector)
        .map(|project| project.id.clone())
        .ok_or_else(|| format!("project '{selector}' not found"))
}

fn resolve_plan_id_or_slug_from_status(
    status: &OrchestrationStatus,
    plan_id_or_slug: &str,
) -> Result<PlanId, String> {
    let selector = plan_id_or_slug.trim();
    if selector.is_empty() {
        return Err("plan_id must not be empty".into());
    }
    if status.plans.iter().any(|plan| plan.id.0 == selector) {
        return Ok(PlanId(selector.to_owned()));
    }
    let mut matches = status
        .plans
        .iter()
        .filter(|plan| plan.slug == selector)
        .collect::<Vec<_>>();
    matches.sort_by(|left, right| left.id.0.cmp(&right.id.0));
    match matches.as_slice() {
        [plan] => Ok(plan.id.clone()),
        [] => Err(format!("plan '{selector}' not found")),
        plans => {
            let matches = plans
                .iter()
                .map(|plan| format!("{} in project {}", plan.id.0, plan.project_id.0))
                .collect::<Vec<_>>()
                .join(", ");
            Err(format!(
                "plan slug '{}' is ambiguous; matches: {}",
                selector, matches
            ))
        }
    }
}

fn summarize_orchestration_counts(status: &OrchestrationStatus) -> OrchestrationCounts {
    let mut counts = OrchestrationCounts {
        total_projects: status.projects.len(),
        total_plans: status.plans.len(),
        total_tasks: status.tasks.len(),
        durable_session_records: status.sessions.len(),
        cleanup_candidates: status.cleanup_candidates.len(),
        ..OrchestrationCounts::default()
    };
    for project in &status.projects {
        match project.status {
            ProjectStatus::Active => counts.active_projects += 1,
            ProjectStatus::Archived => counts.archived_projects += 1,
        }
    }
    for plan in &status.plans {
        if !plan.status.is_finished() {
            counts.active_plans += 1;
        }
        if plan.status == PlanStatus::Blocked {
            counts.blocked_plans += 1;
        }
        match plan.status {
            PlanStatus::WaitingForValidation => counts.waiting_for_validation_plans += 1,
            PlanStatus::Passed => counts.passed_plans += 1,
            PlanStatus::Delivered => counts.delivered_plans += 1,
            PlanStatus::Failed => counts.failed_plans += 1,
            PlanStatus::Canceled => counts.canceled_plans += 1,
            _ => {}
        }
    }
    for task in &status.tasks {
        if !task.status.is_finished() {
            counts.active_tasks += 1;
        }
        if task.status == TaskStatus::Blocked || !task.blocked_by.is_empty() {
            counts.blocked_tasks += 1;
        }
        match task.status {
            TaskStatus::WaitingForValidation => counts.waiting_for_validation_tasks += 1,
            TaskStatus::Passed => counts.passed_tasks += 1,
            TaskStatus::Delivered => counts.delivered_tasks += 1,
            TaskStatus::Failed => counts.failed_tasks += 1,
            TaskStatus::Canceled => counts.canceled_tasks += 1,
            _ => {}
        }
    }
    counts
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
        let mut tools = vec![
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
                    "List durable sessions attached to tasks in a project",
                    Arc::new(tool_schema(
                        json!({
                            "node": { "type": "string", "description": "Execution node id (default: local)" },
                            "project_id": { "type": "string", "description": "Project UUID id or globally unique project slug whose recorded task sessions should be listed" }
                        }),
                        Some(vec!["project_id"]),
                    )),
                ),
                Tool::new(
                    "admin_list_node_sessions",
                    "Admin/debug: list raw live tmux sessions on a node, including unrecorded sessions",
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
                    "List enabled built-in coder profiles",
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
                        "node": { "type": "string", "description": "Execution node id (default: local)" },
                        "session": { "type": "string" },
                        "text": { "type": "string", "description": "Text to send" },
                        "enter": { "type": "boolean", "description": "Send Enter after text (default: true)" }
                    }), Some(vec!["text"]))),
                ),
                Tool::new(
                    "send_key",
                    "Send a special key (C-c, C-d, Escape, Enter, etc.)",
                    Arc::new(tool_schema(json!({
                        "node": { "type": "string", "description": "Execution node id (default: local)" },
                        "session": { "type": "string" },
                        "key": { "type": "string", "description": "Key sequence (e.g. C-c, Escape, Enter)" }
                    }), Some(vec!["key"]))),
                ),
                Tool::new(
                    "capture_output",
                    "Capture pane output from a session",
                    Arc::new(tool_schema(json!({
                        "node": { "type": "string", "description": "Execution node id (default: local)" },
                        "session": { "type": "string" },
                        "lines": { "type": "integer", "description": "Number of lines to capture" },
                        "scrollback": { "type": "boolean", "description": "Capture full scrollback" }
                    }), None)),
                ),
                Tool::new(
                    "wait_start",
                    "Start a cancellable runtime wait job",
                    Arc::new(tool_schema(json!({
                        "node": { "type": "string", "description": "Execution node id (default: local)" },
                        "session": { "type": "string" },
                        "kind": { "type": "string", "enum": ["stable", "sentinel", "prompt", "coding-ready"], "description": "Wait kind" },
                        "profile": { "type": "string", "description": "CLI profile name; required for coding-ready" },
                        "sentinel": { "type": "string", "description": "Text to wait for (sentinel kind)" },
                        "prompt": { "type": "string", "description": "Prompt marker to wait for (prompt kind)" },
                        "timeout_seconds": { "type": "number", "description": "Max seconds to wait (default: 30)" },
                        "poll_seconds": { "type": "number", "description": "Poll interval (default: 0.5)" },
                        "stability_seconds": { "type": "number", "description": "Seconds of stability required (default: 1.0)" }
                    }), None)),
                ),
                Tool::new(
                    "wait_status",
                    "Read the current status of a runtime wait job",
                    Arc::new(tool_schema(json!({
                        "wait_id": { "type": "string" }
                    }), Some(vec!["wait_id"]))),
                ),
                Tool::new(
                    "wait_cancel",
                    "Cancel a pending runtime wait job",
                    Arc::new(tool_schema(json!({
                        "wait_id": { "type": "string" }
                    }), Some(vec!["wait_id"]))),
                ),
                Tool::new(
                    "interact",
                    "Send input and wait for output in one call",
                    Arc::new(tool_schema(json!({
                        "node": { "type": "string", "description": "Execution node id (default: local)" },
                        "session": { "type": "string" },
                        "text": { "type": "string" },
                        "timeout_seconds": { "type": "number", "description": "Max seconds to wait (default: 30)" }
                    }), Some(vec!["text"]))),
                ),
                Tool::new(
                    "exec",
                    "Execute a shell command in a session and return the output. Creates the session if it does not exist.",
                    Arc::new(tool_schema(json!({
                        "node": { "type": "string", "description": "Execution node id (default: local)" },
                        "session": { "type": "string", "description": "Session name (default: mmux_shell)" },
                        "command": { "type": "string", "description": "Shell command to execute" },
                        "workspace_path": { "type": "string", "description": "Backend-owned workspace/start directory, only used when creating the session" },
                        "timeout_seconds": { "type": "number", "description": "Max seconds to wait for output (default: 30)" },
                        "lines": { "type": "integer", "description": "Lines of output to capture (default: 40)" }
                    }), Some(vec!["command"]))),
                ),
                Tool::new(
                    "start_coding_session",
                    "Create or adopt a coding CLI session from a profile-defined command. Does not wait for readiness; use wait_start kind=coding-ready.",
                    Arc::new(tool_schema(json!({
                        "profile": { "type": "string", "description": "CLI profile name (default: controller default coder profile)" },
                        "session": { "type": "string", "description": "Session name (default: profile name)" },
                        "node": { "type": "string", "description": "Execution node id (default: local)" },
                        "workspace_path": { "type": "string", "description": "Backend-owned workspace/start directory for the selected node/backend. Used as the tmux start directory when creating the session." },
                        "bypass_permissions": { "type": "boolean", "description": "Use the profile's explicit permission_bypass_cmd for this session. This may disable the coder CLI's approval prompts or sandboxing. Default: false." },
                        "task_id": { "type": "string", "description": "Task ID to record this coder session against. Enables task-aware recording." },
                        "role": { "type": "string", "description": "Task session role to persist when task_id is provided." },
                        "kind": { "type": "string", "description": "Task participant kind to persist and use in generated orchestration session names." },
                        "skills": { "type": "array", "items": { "type": "string" }, "description": "Task session skills to persist when task_id is provided." },
                        "generate_session_name": { "type": "boolean", "description": "Generate an orchestration-owned session name mmux-{task_slug}-{kind}-{short_suffix}." }
                    }), None)),
                ),
                // ── Session introspection ──
                Tool::new(
                    "session_info",
                    "Get detailed info about a tmux session: panes, windows, dimensions, running commands",
                    Arc::new(tool_schema(json!({
                        "node": { "type": "string", "description": "Execution node id (default: local)" },
                        "session": { "type": "string", "description": "Session name" }
                    }), None)),
                ),
                Tool::new(
                    "list_panes",
                    "List all panes in a tmux session with dimensions and running commands",
                    Arc::new(tool_schema(json!({
                        "node": { "type": "string", "description": "Execution node id (default: local)" },
                        "session": { "type": "string", "description": "Session name" }
                    }), None)),
                ),
                Tool::new(
                    "check_state",
                    "Quick non-blocking check: promptable, busy, and turn-idle state. Returns JSON.",
                    Arc::new(tool_schema(json!({
                        "node": { "type": "string", "description": "Execution node id (default: local)" },
                        "session": { "type": "string" },
                        "profile": { "type": "string", "description": "CLI profile name (default: controller default coder profile)" }
                    }), None)),
                ),
                Tool::new(
                    "resize_pane",
                    "Resize the main pane in a tmux session. Useful for TUI apps.",
                    Arc::new(tool_schema(json!({
                        "node": { "type": "string", "description": "Execution node id (default: local)" },
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
                        "node": { "type": "string", "description": "Execution node id (default: local)" },
                        "path": { "type": "string", "description": "Absolute or relative file path" },
                        "offset": { "type": "integer", "description": "Optional byte offset" },
                        "limit": { "type": "integer", "description": "Optional max bytes (default 4 MiB)" }
                    }), Some(vec!["path"]))),
                ),
                Tool::new(
                    "save_file",
                    "Save a file to disk. Accepts content + encoding (utf-8 or base64). Creates parent dirs.",
                    Arc::new(tool_schema(json!({
                        "node": { "type": "string", "description": "Execution node id (default: local)" },
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
                        "node": { "type": "string", "description": "Execution node id (default: local)" },
                        "session": { "type": "string" },
                        "prompt": { "type": "string" },
                        "profile": { "type": "string", "description": "CLI profile name (default: controller default coder profile)" }
                    }), Some(vec!["prompt"]))),
                ),
                Tool::new(
                    "coding_task_send",
                    "Send an initial task-scoped prompt to a coding CLI. Builds deterministic task context from orchestration state, appends the provided instruction, then sends it with coding_send behavior.",
                    Arc::new(tool_schema(json!({
                        "node": { "type": "string", "description": "Execution node id (default: local)" },
                        "session": { "type": "string" },
                        "profile": { "type": "string", "description": "CLI profile name (default: controller default coder profile)" },
                        "task_id_or_slug": { "type": "string", "description": "Task id or unique task slug" },
                        "prompt": { "type": "string", "description": "Instruction appended below generated task context" },
                        "template": { "type": "string", "enum": ["task", "validate", "review", "quality-guard"], "description": "Task prompt template to render (default: task)" },
                        "include_dependencies": { "type": "boolean", "description": "Include parent/dependency/blocking task context (default: true)" },
                        "include_gates": { "type": "boolean", "description": "Include task gates (default: true)" },
                        "include_scope": { "type": "boolean", "description": "Include task scope paths and notes (default: true)" },
                        "context_task_ids": { "type": "array", "items": { "type": "string" }, "description": "Optional task ids or unique slugs to render as an operator-supplied task-card bundle for multi-task validation/review" },
                        "extra_context": { "type": "string", "description": "Optional extra operator context appended before Instruction" }
                    }), Some(vec!["task_id_or_slug", "prompt"]))),
                ),
                Tool::new(
                    "coding_read",
                    "Read recent coding CLI output. Returns compact profile-aware output by default; pass raw=true for full pane text.",
                    Arc::new(tool_schema(json!({
                        "node": { "type": "string", "description": "Execution node id (default: local)" },
                        "session": { "type": "string" },
                        "profile": { "type": "string", "description": "CLI profile name (default: controller default coder profile)" },
                        "lines": { "type": "integer", "description": "Lines to capture before compaction (default: 40)" },
                        "raw": { "type": "boolean", "description": "Return raw pane text instead of compact profile-aware output (default: false)" }
                    }), None)),
                ),
                Tool::new(
                    "coding_action",
                    "Send a profile-aware action to a coding CLI (approve, reject, cancel, escape, dismiss)",
                    Arc::new(tool_schema(json!({
                        "node": { "type": "string", "description": "Execution node id (default: local)" },
                        "session": { "type": "string" },
                        "action": { "type": "string", "enum": ["approve", "reject", "cancel", "escape", "dismiss"], "description": "Action to perform" },
                        "profile": { "type": "string", "description": "CLI profile name (default: controller default coder profile)" }
                    }), Some(vec!["action"]))),
                ),
            ];
        tools.extend(Self::orchestration_tool_definitions(
            self.policy.enable_admin_tools,
        ));
        Ok(ListToolsResult::with_all_items(tools))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let args = request.arguments.unwrap_or_default();
        match request.name.as_ref() {
            "orchestration_status" => return self.orchestration_status_tool_async(args).await,
            "orchestration_cleanup_zombies" => {
                return self.orchestration_cleanup_zombies_tool(args).await
            }
            "orchestration_prune_store" => return self.orchestration_prune_store_tool(args).await,
            "session_record" => return self.session_record_tool(args).await,
            "wait_start" => return self.wait_start_tool(args).await,
            "wait_status" => return self.wait_status_tool(args),
            "wait_cancel" => return self.wait_cancel_tool(args),
            _ => {}
        }
        if let Some(result) = self.call_orchestration_tool(request.name.as_ref(), args.clone()) {
            return result;
        }
        let session = args
            .get("session")
            .and_then(|v| v.as_str())
            .unwrap_or("kimi_codex");
        let node = args.get("node").and_then(|v| v.as_str()).unwrap_or("local");

        let result = match request.name.as_ref() {
            // ── Universal session management ──
            "kill_session" => {
                match self.node_session_exists(node, session).await {
                    Ok(true) => {}
                    Ok(false) => {
                        return Ok(Self::text_result(format!(
                            "Session '{}' not found on node '{}'",
                            session, node
                        )));
                    }
                    Err(error) => return Ok(Self::error_result(error)),
                }
                match self
                    .node_tmux(
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
                }
            }
            "list_sessions" => {
                self.list_sessions_tool(args).await
            }
            "admin_list_node_sessions" => {
                self.admin_list_node_sessions_tool(args).await
            }
            "list_nodes" => match self
                .registry_call(|reply| NodeRegistryMessage::ListNodes { reply }, None)
                .await
            {
                Ok(msg) => Ok(Self::text_result(msg)),
                Err(e) => Ok(Self::error_result(e)),
            },
            "list_coder_profiles" => {
                let mut profiles: Vec<_> = self.profiles.values().cloned().collect();
                profiles.sort_by(|a, b| a.name.cmp(&b.name));
                let json = serde_json::to_string_pretty(
                    &profiles
                        .into_iter()
                        .map(|profile| {
                            json!({
                                "name": profile.name,
                                "cmd": profile.cmd,
                                "permission_bypass_cmd": profile.permission_bypass_cmd,
                                "launch_strategy": profile.launch_strategy,
                                "text_mode": profile.text_mode,
                                "submit_keys": profile.submit_keys,
                                "submit_after_text": profile.submit_after_text,
                                "prompt_indicator": profile.prompt_indicator,
                                "busy_indicators": profile.busy_indicators,
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
                let text = args
                    .get("text")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| McpError::invalid_request("missing text", None))?;
                let enter = args.get("enter").and_then(|v| v.as_bool()).unwrap_or(true);
                if let Err(error) = self
                    .node_tmux(
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
                if enter {
                    tokio::time::sleep(coding_prompt_submit_delay(text)).await;
                    if let Err(error) = self
                        .node_tmux(
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
                Ok(Self::text_result(format!(
                    "Sent to {} on node {}: {}",
                    session, node, text
                )))
            }
            "send_key" => {
                let key = args
                    .get("key")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| McpError::invalid_request("missing key", None))?;
                match self
                    .node_tmux(
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
                match self.node_session_capture(node, session, lines, scrollback).await {
                    Ok(output) => Ok(Self::text_result(self.policy.limit_capture_output(output))),
                    Err(e) => Ok(Self::error_result(e)),
                }
            }
            "interact" => {
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
                if let Err(error) = self
                    .node_tmux(
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
                tokio::time::sleep(coding_prompt_submit_delay(text)).await;
                if let Err(error) = self
                    .node_tmux(
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
                match self
                    .node_wait_for(
                        node,
                        session,
                        NodeWaitOptions {
                            mode: "stable",
                            sentinel: None,
                            prompt: None,
                            timeout,
                            poll: 0.5,
                            stability: 1.0,
                        },
                    )
                    .await
                {
                    Ok(msg) => Ok(Self::text_result(msg)),
                    Err(e) => Ok(Self::error_result(e)),
                }
            }
            "exec" => {
                let command = args
                    .get("command")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| McpError::invalid_request("missing command", None))?;
                let session = args
                    .get("session")
                    .and_then(|v| v.as_str())
                    .unwrap_or("mmux_shell");
                let workspace_path = args
                    .get("workspace_path")
                    .and_then(|v| v.as_str())
                    .map(str::to_owned);
                let timeout = self
                    .policy
                    .clamp_timeout(
                        args.get("timeout_seconds")
                            .and_then(|v| v.as_f64())
                            .unwrap_or(30.0),
                    )
                    .map_err(|e| McpError::invalid_request(e, None))?;
                let lines = args.get("lines").and_then(|v| v.as_u64()).unwrap_or(40) as usize;
                let exists = match self.node_session_exists(node, session).await {
                    Ok(exists) => exists,
                    Err(error) => return Ok(Self::error_result(error)),
                };
                if !exists {
                    if let Err(error) = self
                        .create_session_with_command(
                            node,
                            session,
                            "bash",
                            workspace_path.as_deref(),
                        )
                        .await
                    {
                        return Ok(Self::error_result(error));
                    }
                }
                let sentinel = format!(
                    "__MMUX_{}__",
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_nanos()
                );
                for text in [format!("echo '{}'", sentinel), command.to_owned()] {
                    let delay = coding_prompt_submit_delay(&text);
                    if let Err(error) = self
                        .node_tmux(
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
                    tokio::time::sleep(delay).await;
                    if let Err(error) = self
                        .node_tmux(
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
                    .node_wait_for(
                        node,
                        session,
                        NodeWaitOptions {
                            mode: "stable",
                            sentinel: None,
                            prompt: None,
                            timeout,
                            poll: 0.5,
                            stability: 1.0,
                        },
                    )
                    .await
                {
                    return Ok(Self::error_result(error));
                }
                match self.node_session_capture(node, session, None, true).await {
                    Ok(output) => {
                        let all_lines: Vec<&str> = output.lines().collect();
                        let sentinel_idx = all_lines
                            .iter()
                            .enumerate()
                            .filter_map(|(i, line)| (line.trim() == sentinel).then_some(i))
                            .next_back();
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
                }
            }
            // ── File operations ──
            "read_file" => {
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
                let resolved_text = path.to_owned();
                match self
                    .node_command(
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
                                "node returned invalid base64: {}",
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
                }
            }
            "save_file" => {
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
                let resolved_text = path.to_owned();
                let bytes = match encoding {
                    "base64" => match BASE64.decode(content.as_bytes()) {
                        Ok(bytes) => bytes,
                        Err(e) => {
                            return Ok(Self::error_result(format!("base64 decode error: {}", e)))
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
                match self
                    .node_command(
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
                            mime_type: Some(mmux_node::detect_mime_type(Path::new(path), &bytes)),
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
                }
            }
            // ── Coding CLI adapters ──
            "coding_send" => {
                let prompt = args
                    .get("prompt")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| McpError::invalid_request("missing prompt", None))?;
                self.send_coding_prompt(
                    node,
                    session,
                    args.get("profile").and_then(|v| v.as_str()),
                    prompt,
                )
                .await
            }
            "coding_task_send" => {
                let args: CodingTaskSendArgs = parse_tool_args("coding_task_send", args)?;
                let prompt = build_coding_task_prompt(
                    &self.orchestration.snapshot().map_err(mcp_invalid_request)?,
                    &args,
                )
                .map_err(mcp_invalid_request)?;
                self.send_coding_prompt(&args.node, &args.session, args.profile.as_deref(), &prompt)
                    .await
            }
            "start_coding_session" => self.start_coding_session_tool(args).await,
            "coding_read" => {
                let lines = args.get("lines").and_then(|v| v.as_u64()).unwrap_or(40) as usize;
                let raw = args.get("raw").and_then(|v| v.as_bool()).unwrap_or(false);
                let profile = self
                    .resolve_profile(args.get("profile").and_then(|v| v.as_str()))
                    .ok_or_else(|| McpError::invalid_request("unknown profile", None))?;
                match self
                    .node_session_capture(node, session, Some(lines), false)
                    .await
                {
                    Ok(output) => {
                        let output = if raw {
                            output
                        } else {
                            compact_coding_output(&output, &profile)
                        };
                        Ok(Self::text_result(self.policy.limit_capture_output(output)))
                    }
                    Err(e) => Ok(Self::error_result(e)),
                }
            }
            "coding_action" => {
                let action = args
                    .get("action")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| McpError::invalid_request("missing action", None))?;
                let profile = self
                    .resolve_profile(args.get("profile").and_then(|v| v.as_str()))
                    .ok_or_else(|| McpError::invalid_request("unknown profile", None))?;
                let pane = match self.node_session_first_pane(node, session).await {
                    Ok(pane) => pane,
                    Err(error) => return Ok(Self::error_result(error)),
                };
                let keys = match action {
                    "approve" => profile.approve_keys,
                    "reject" => profile.reject_keys,
                    "cancel" => profile.cancel_keys,
                    "escape" | "dismiss" => profile.escape_keys,
                    other => return Ok(Self::error_result(format!("Unknown action: {}", other))),
                };
                match self
                    .node_tmux(
                        node,
                        node_send_key_args(&pane, &keys),
                        Duration::from_secs(20),
                    )
                    .await
                {
                    Ok(_) => Ok(Self::text_result(format!(
                        "Sent action '{}' to {} on node {}",
                        action, session, node
                    ))),
                    Err(e) => Ok(Self::error_result(e)),
                }
            }
            "session_info" => {
                let panes = self
                    .node_tmux(
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
                    .node_tmux(
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
                match (panes, windows) {
                    (Ok(panes), Ok(windows)) => Ok(Self::text_result(format!(
                        "Node: {}\nSession: {}\nPanes:\n{}\nWindows:\n{}",
                        node, session, panes, windows
                    ))),
                    (Err(e), _) | (_, Err(e)) => Ok(Self::error_result(e)),
                }
            }
            "list_panes" => {
                match self
                    .node_tmux(
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
                }
            }
            "check_state" => {
                let profile = self
                    .resolve_profile(args.get("profile").and_then(|v| v.as_str()))
                    .ok_or_else(|| McpError::invalid_request("unknown profile", None))?;
                let buf = self
                    .node_session_capture(node, session, None, false)
                    .await
                    .unwrap_or_default();
                let has_prompt = profile_has_prompt(&buf, &profile);
                let busy = profile_is_busy(&buf, &profile);
                let promptable = has_prompt;
                let turn_idle = has_prompt && !busy;
                Ok(Self::text_result(
                    json!({
                        "node": node,
                        "session": session,
                        "has_prompt": has_prompt,
                        "promptable": promptable,
                        "busy": busy,
                        "turn_idle": turn_idle,
                        "profile": profile.name,
                    })
                    .to_string(),
                ))
            }
            "resize_pane" => {
                let width = args.get("width").and_then(|v| v.as_u64()).map(|n| n as u32);
                let height = args
                    .get("height")
                    .and_then(|v| v.as_u64())
                    .map(|n| n as u32);
                if let Some(width) = width {
                    let pane = match self.node_session_first_pane(node, session).await {
                        Ok(pane) => pane,
                        Err(error) => return Ok(Self::error_result(error)),
                    };
                    if let Err(error) = self
                        .node_tmux(
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
                    let pane = match self.node_session_first_pane(node, session).await {
                        Ok(pane) => pane,
                        Err(error) => return Ok(Self::error_result(error)),
                    };
                    if let Err(error) = self
                        .node_tmux(
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
                Ok(Self::text_result(format!(
                    "Resized pane {} on node {}",
                    session, node
                )))
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
        let resources: Vec<Resource> = self
            .profiles
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
            if let Some(profile) = self.profiles.get(name) {
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
                "info" => {
                    let panes = self
                        .node_tmux(
                            "local",
                            vec![
                                "list-panes".into(),
                                "-t".into(),
                                session_name.into(),
                                "-F".into(),
                                "pane_id=#{pane_id} index=#{pane_index} width=#{pane_width} height=#{pane_height} command=#{pane_current_command} title=#{pane_title}".into(),
                            ],
                            Duration::from_secs(20),
                        )
                        .await;
                    let windows = self
                        .node_tmux(
                            "local",
                            vec![
                                "list-windows".into(),
                                "-t".into(),
                                session_name.into(),
                                "-F".into(),
                                "window_id=#{window_id} index=#{window_index} name=#{window_name} active=#{window_active}".into(),
                            ],
                            Duration::from_secs(20),
                        )
                        .await;
                    match (panes, windows) {
                        (Ok(panes), Ok(windows)) => {
                            let text = format!(
                                "Node: local\nSession: {}\nPanes:\n{}\nWindows:\n{}",
                                session_name, panes, windows
                            );
                            Ok(ReadResourceResult::new(vec![
                                ResourceContents::TextResourceContents {
                                    uri: uri.clone(),
                                    mime_type: Some("text/plain".into()),
                                    text,
                                    meta: None,
                                },
                            ]))
                        }
                        (Err(e), _) | (_, Err(e)) => Err(McpError::invalid_request(e, None)),
                    }
                }
                "scrollback" => match self
                    .node_session_capture("local", session_name, None, true)
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
                    .node_session_capture("local", session_name, Some(200), false)
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
                    PromptArgument::new("profile").with_description(
                        "CLI profile to use (e.g. codex, opencode, kimi, claude)",
                    ),
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
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .or_else(|| self.default_profile_name())
                    .unwrap_or("codex");
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
                                    "You are driving a coding CLI via mmux.\n\nProfile: {}\nSession: {}\n\nWorkflow:\n1. Start the session with start_coding_session using the profile-defined command\n2. For initial task delegation, use coding_task_send with task_id_or_slug, template, and a concrete instruction; for follow-up or non-task prompts, use coding_send\n3. For validation/review spanning multiple tasks, pass context_task_ids so mmux renders operator-supplied task cards; do not ask the worker to call mmux for missing prior task results\n4. Start a coding-ready wait with wait_start kind=coding-ready and this profile\n5. Poll wait_status until completed, failed, or canceled\n6. Use coding_read to capture the output\n7. Use coding_action (approve/reject/cancel/escape) to interact\n\ncoding_task_send templates:\n- task: initial implementation/delegation\n- validate: task gates and objective validation; for task sets require field_coverage_table over supplied context_task_ids\n- review: correctness, regression, risk, missing-test, and scope-drift review\n- quality-guard: maintainability, architecture fit, naming, boundaries, lifecycle, API shape, and operator/project quality preferences\n\nTips:\n- check_state is a quick non-blocking way to inspect has_prompt, promptable, busy, and turn_idle\n- promptable means the CLI can accept text; turn_idle means foreground work has settled\n- resize_pane can help if the TUI layout is broken\n- capture_output with scrollback:true gets full history\n- Use wait_start with sentinel or prompt kind to detect specific output strings",
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
                                    "Debug tmux session '{}'. Follow this checklist:\n\n1. session_info — check if session exists, see panes/dimensions/commands\n2. capture_output with scrollback:true — see full history\n3. check_state with appropriate profile — inspect has_prompt, promptable, busy, and turn_idle\n4. If stuck: send_key C-c (cancel), or send_key Escape\n5. If TUI is garbled: resize_pane to a reasonable size (e.g. 120x40)\n6. If the CLI crashed: kill_session then start_coding_session again\n\nCommon issues:\n- 'Session does not exist' → start_coding_session first\n- Output truncated → use scrollback:true or increase lines\n- Prompt not detected → verify profile.prompt_indicator matches the CLI\n- promptable=true means text can be sent; use turn_idle=true or a completed coding-ready wait when you need foreground work to have settled",
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

async fn wire_auth_middleware(
    mut request: Request<Body>,
    next: Next,
    policy: Arc<NodeWireAuthPolicy>,
    token: Option<Arc<String>>,
) -> Response {
    let auth_header = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let token_valid = token.as_ref().is_some_and(|expected| {
        let expected_auth = format!("Bearer {}", expected);
        constant_time_eq(auth_header.as_bytes(), expected_auth.as_bytes())
    });
    let mtls_identity = request
        .extensions()
        .get::<NodeWireIdentity>()
        .cloned()
        .or_else(|| {
            request
                .extensions()
                .get::<ConnectInfo<LocalConnectInfo>>()
                .and_then(|info| info.0.node_wire_identity.clone())
        });
    match policy.authenticate(token_valid, mtls_identity) {
        Ok(context) => {
            request.extensions_mut().insert(context);
            next.run(request).await
        }
        Err(message) => Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .body(Body::from(message))
            .unwrap(),
    }
}

struct NodeRegistryConnectService {
    registry: ActorRef<NodeRegistryMessage>,
}

#[derive(Clone, Debug)]
struct LocalConnectInfo {
    node_wire_identity: Option<NodeWireIdentity>,
}

struct NativeMtlsListener {
    listener: tokio::net::TcpListener,
    acceptor: TlsAcceptor,
}

struct NativeMtlsStream {
    stream: TlsStream<TcpStream>,
    node_wire_identity: Option<NodeWireIdentity>,
}

impl NativeMtlsListener {
    fn new(listener: tokio::net::TcpListener, tls_config: Arc<ServerConfig>) -> Self {
        Self {
            listener,
            acceptor: TlsAcceptor::from(tls_config),
        }
    }
}

impl axum::serve::Listener for NativeMtlsListener {
    type Io = NativeMtlsStream;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            let (stream, remote_addr) = match self.listener.accept().await {
                Ok(value) => value,
                Err(error) => {
                    eprintln!("TLS listener accept failed: {}", error);
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    continue;
                }
            };
            match tokio::time::timeout(Duration::from_secs(10), self.acceptor.accept(stream)).await
            {
                Ok(Ok(tls_stream)) => {
                    let node_wire_identity = tls_stream
                        .get_ref()
                        .1
                        .peer_certificates()
                        .and_then(|certs| certs.first())
                        .and_then(|cert| node_wire_identity_from_cert(cert.as_ref()));
                    return (
                        NativeMtlsStream {
                            stream: tls_stream,
                            node_wire_identity,
                        },
                        remote_addr,
                    );
                }
                Ok(Err(error)) => {
                    eprintln!("TLS handshake failed from {}: {}", remote_addr, error);
                }
                Err(_) => {
                    eprintln!("TLS handshake timed out from {}", remote_addr);
                }
            }
        }
    }

    fn local_addr(&self) -> std::io::Result<Self::Addr> {
        self.listener.local_addr()
    }
}

impl AsyncRead for NativeMtlsStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_read(cx, buf)
    }
}

impl AsyncWrite for NativeMtlsStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.stream).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}

impl Connected<IncomingStream<'_, NativeMtlsListener>> for LocalConnectInfo {
    fn connect_info(stream: IncomingStream<'_, NativeMtlsListener>) -> Self {
        Self {
            node_wire_identity: stream.io().node_wire_identity.clone(),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ResolvedNodeWirePolicy {
    policy: NodeWireAuthPolicy,
    token: Option<String>,
    native_mtls: Option<NativeMtlsConfig>,
}

#[derive(Clone, Debug)]
pub(crate) struct NativeMtlsConfig {
    server_cert: PathBuf,
    server_key: PathBuf,
    client_ca: PathBuf,
}

fn invalid_argument(error: impl ToString) -> ConnectError {
    ConnectError::invalid_argument(error.to_string())
}

fn internal_error(error: impl ToString) -> ConnectError {
    ConnectError::internal(error.to_string())
}

fn build_native_mtls_server_config(config: &NativeMtlsConfig) -> Result<Arc<ServerConfig>, String> {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let server_cert = load_cert_chain(&config.server_cert, "TLS server certificate")?;
    let server_key = load_private_key(&config.server_key, "TLS server private key")?;
    let mut client_roots = RootCertStore::empty();
    for cert in load_cert_chain(&config.client_ca, "node client CA certificate")? {
        client_roots.add(cert).map_err(|error| {
            format!(
                "failed to add node client CA '{}': {}",
                config.client_ca.display(),
                error
            )
        })?;
    }
    let client_verifier = WebPkiClientVerifier::builder(Arc::new(client_roots))
        .allow_unauthenticated()
        .build()
        .map_err(|error| {
            format!(
                "failed to build node client certificate verifier: {}",
                error
            )
        })?;
    let tls_config = ServerConfig::builder()
        .with_client_cert_verifier(client_verifier)
        .with_single_cert(server_cert, server_key)
        .map_err(|error| format!("failed to build TLS server config: {}", error))?;
    Ok(Arc::new(tls_config))
}

fn load_cert_chain(path: &Path, description: &str) -> Result<Vec<CertificateDer<'static>>, String> {
    let file = std::fs::File::open(path).map_err(|error| {
        format!(
            "failed to open {} '{}': {}",
            description,
            path.display(),
            error
        )
    })?;
    let certs = rustls_pemfile::certs(&mut BufReader::new(file))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            format!(
                "failed to parse {} '{}': {}",
                description,
                path.display(),
                error
            )
        })?;
    if certs.is_empty() {
        return Err(format!(
            "{} '{}' contains no certificates",
            description,
            path.display()
        ));
    }
    Ok(certs)
}

fn load_private_key(path: &Path, description: &str) -> Result<PrivateKeyDer<'static>, String> {
    let file = std::fs::File::open(path).map_err(|error| {
        format!(
            "failed to open {} '{}': {}",
            description,
            path.display(),
            error
        )
    })?;
    rustls_pemfile::private_key(&mut BufReader::new(file))
        .map_err(|error| {
            format!(
                "failed to parse {} '{}': {}",
                description,
                path.display(),
                error
            )
        })?
        .ok_or_else(|| {
            format!(
                "{} '{}' contains no private key",
                description,
                path.display()
            )
        })
}

fn node_wire_identity_from_cert(cert_der: &[u8]) -> Option<NodeWireIdentity> {
    let (_, cert) = x509_parser::certificate::X509Certificate::from_der(cert_der).ok()?;
    if let Ok(Some(san)) = cert.subject_alternative_name() {
        for name in &san.value.general_names {
            if let GeneralName::URI(uri) = name {
                if let Some(node_id) = node_id_from_uri_san(uri) {
                    return Some(NodeWireIdentity::mtls(node_id));
                }
            }
        }
    }
    None
}

fn node_id_from_uri_san(uri: &str) -> Option<String> {
    uri.strip_prefix("mmux:node:")
        .or_else(|| uri.strip_prefix("spiffe://mmux/node/"))
        .map(str::trim)
        .filter(|node_id| !node_id.is_empty())
        .map(ToOwned::to_owned)
}

fn require_wire_node_identity(
    ctx: &ConnectRequestContext,
    node_id: &str,
) -> Result<(), ConnectError> {
    match ctx.extensions.get::<NodeWireAuthContext>() {
        Some(auth) => auth
            .require_node_id(node_id)
            .map_err(ConnectError::permission_denied),
        None => Err(ConnectError::unauthenticated(
            "node wire request was not authenticated",
        )),
    }
}

#[allow(refining_impl_trait)]
impl MmuxNodeRegistryService for NodeRegistryConnectService {
    async fn register_node(
        &self,
        ctx: ConnectRequestContext,
        request: OwnedRegisterNodeRequestView,
    ) -> ServiceResult<wire_proto::RegisterNodeResponse> {
        let request = register_node_request_from_proto(request.to_owned_message())
            .map_err(invalid_argument)?;
        require_wire_node_identity(&ctx, &request.descriptor.node_id)?;
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
        ctx: ConnectRequestContext,
        request: OwnedPullCommandsRequestView,
    ) -> ServiceResult<wire_proto::PullCommandsResponse> {
        let request = pull_commands_request_from_proto(request.to_owned_message());
        require_wire_node_identity(&ctx, &request.node_id)?;
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
        ctx: ConnectRequestContext,
        request: OwnedSubmitCommandResultRequestView,
    ) -> ServiceResult<wire_proto::SubmitCommandResultResponse> {
        let request = submit_command_result_request_from_proto(request.to_owned_message())
            .map_err(invalid_argument)?;
        require_wire_node_identity(&ctx, &request.node_id)?;
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
        ctx: ConnectRequestContext,
        request: OwnedHeartbeatRequestView,
    ) -> ServiceResult<wire_proto::HeartbeatResponse> {
        let request =
            heartbeat_request_from_proto(request.to_owned_message()).map_err(invalid_argument)?;
        require_wire_node_identity(&ctx, &request.node_id)?;
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

pub(crate) async fn run_mcp_http_server(
    bind: SocketAddr,
    profiles: ProfileRegistry,
    default_coder_profile: String,
    policy: ControllerPolicy,
    mcp_token: Option<String>,
    wire_auth: ResolvedNodeWirePolicy,
    embedded_node: Option<EmbeddedNodeConfig>,
    orchestration: orchestration_actor::OrchestrationHandle,
) -> Result<(), String> {
    let embedded_node_label = embedded_node.as_ref().map(|config| config.display_name());
    let embedded_local_node_enabled = matches!(
        embedded_node.as_ref(),
        Some(EmbeddedNodeConfig::Local { .. })
    );
    let embedded_backend = if let Some(config) = embedded_node {
        Some(match config {
            EmbeddedNodeConfig::Local {
                store_path,
                tmux_config,
            } => {
                mmux_node::EmbeddedNodeBackend::local(store_path.as_deref(), tmux_config.as_deref())
                    .await?
            }
            EmbeddedNodeConfig::Microsandbox { sandbox_name } => {
                mmux_node::EmbeddedNodeBackend::microsandbox(&sandbox_name).await?
            }
        })
    } else {
        None
    };
    let (registry, _registry_handle) =
        Actor::spawn(None, NodeRegistryActor, embedded_node_label.clone())
            .await
            .map_err(|error| format!("failed to start node registry actor: {}", error))?;
    let (node_executor, _node_executor_handle) = Actor::spawn(
        None,
        NodeExecutionActor,
        NodeExecutionWorkerState {
            embedded: embedded_backend,
            registry: registry.clone(),
        },
    )
    .await
    .map_err(|error| format!("failed to start node execution actor: {}", error))?;
    let startup_warnings = Arc::new(Mutex::new(Vec::new()));
    let wait_jobs = Arc::new(Mutex::new(HashMap::new()));
    if embedded_local_node_enabled {
        TmuxMcpServer::new(
            profiles.clone(),
            Some(default_coder_profile.clone()),
            policy.clone(),
            node_executor.clone(),
            registry.clone(),
            orchestration.clone(),
            wait_jobs.clone(),
            startup_warnings.clone(),
        )
        .reconcile_startup_local_sessions()
        .await;
    }
    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .map_err(|e| format!("failed to bind: {}", e))?;

    let service_policy = policy.clone();
    let request_body_limit = policy.max_request_bytes;
    let service_registry = registry.clone();
    let service_orchestration = orchestration.clone();
    let service_wait_jobs = wait_jobs.clone();
    let service: StreamableHttpService<TmuxMcpServer, LocalSessionManager> =
        StreamableHttpService::new(
            move || {
                Ok(TmuxMcpServer::new(
                    profiles.clone(),
                    Some(default_coder_profile.clone()),
                    service_policy.clone(),
                    node_executor.clone(),
                    service_registry.clone(),
                    service_orchestration.clone(),
                    service_wait_jobs.clone(),
                    startup_warnings.clone(),
                ))
            },
            LocalSessionManager::default().into(),
            StreamableHttpServerConfig::default()
                .with_allowed_origins(loopback_allowed_origins())
                .with_stateful_mode(false)
                .with_json_response(true)
                .disable_allowed_hosts(),
        );

    let native_mtls = wire_auth.native_mtls.clone();
    let mcp_token_arc = mcp_token.map(Arc::new);
    let wire_auth_mode = wire_auth.policy.mode;
    let wire_token_arc = wire_auth.token.map(Arc::new);
    let wire_auth_policy = Arc::new(wire_auth.policy);
    let has_mcp_token = mcp_token_arc.is_some();
    let has_wire_token = wire_token_arc.is_some();
    let api_token_arc = mcp_token_arc.clone();
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
            let p = wire_auth_policy.clone();
            wire_auth_middleware(req, next, p, t)
        }))
        .layer(middleware::from_fn(security_middleware));

    let health_router = axum::Router::new().route("/health", axum::routing::get(|| async { "OK" }));

    let router = health_router.merge(api_router).merge(wire_router);

    let scheme = if native_mtls.is_some() {
        "https"
    } else {
        "http"
    };
    println!(
        "mmux MCP HTTP server listening on {}://{}/mcp",
        scheme, bind
    );
    if let Some(label) = embedded_node_label {
        println!("  Embedded node enabled: {} backend as node 'local'", label);
    }
    if has_mcp_token {
        println!("  MCP bearer token authentication enabled");
    } else {
        println!(
            "  Warning: no MCP bearer token set. Use --mcp-token to prevent unauthorized MCP access."
        );
    }
    println!("  Node wire auth mode: {}", wire_auth_mode.as_str());
    if wire_auth_mode.allows_token() && has_wire_token {
        println!("  Node wire bearer token authentication enabled");
    }
    if wire_auth_mode.allows_mtls() {
        println!("  Node wire mTLS authentication enabled with native TLS termination.");
    }
    if wire_auth_mode == NodeWireAuthMode::Unauthenticated {
        println!(
            "  Warning: node wire RPC is unauthenticated. Use only for development or trusted private tunnels."
        );
    } else if wire_auth_mode == NodeWireAuthMode::Token && !has_wire_token {
        println!(
            "  Node wire RPC requires --wire-token; unauthenticated node requests will be rejected."
        );
    }

    if let Some(native_mtls) = native_mtls {
        let tls_config = build_native_mtls_server_config(&native_mtls)?;
        let tls_listener = NativeMtlsListener::new(listener, tls_config);
        return axum::serve(
            tls_listener,
            router.into_make_service_with_connect_info::<LocalConnectInfo>(),
        )
        .await
        .map_err(|e| format!("server error: {}", e));
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

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
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

fn resolve_token_value(
    token_flag: &str,
    token: Option<&String>,
    token_file: Option<&String>,
    token_env: &str,
) -> Result<Option<String>, String> {
    if let Some(token) = token {
        if token.is_empty() {
            return Err(format!("{} must not be empty", token_flag));
        }
        return Ok(Some(token.clone()));
    }

    if let Some(path) = token_file {
        let token_path = Path::new(path);
        let real_token_path = std::fs::canonicalize(token_path)
            .map_err(|e| format!("failed to canonicalize token file '{}': {}", path, e))?;
        warn_if_secret_file_permissions_are_loose(&real_token_path);
        let token = std::fs::read_to_string(&real_token_path)
            .map_err(|e| format!("failed to read token file '{}': {}", path, e))?
            .trim()
            .to_owned();
        if token.is_empty() {
            return Err(format!("token file '{}' is empty", path));
        }
        return Ok(Some(token));
    }

    match std::env::var(token_env) {
        Ok(token) if !token.is_empty() => Ok(Some(token)),
        Ok(_) => Err(format!("{} is set but empty", token_env)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(e) => Err(format!("failed to read {}: {}", token_env, e)),
    }
}

fn explicit_token_source_configured(token: Option<&String>, token_file: Option<&String>) -> bool {
    token.is_some() || token_file.is_some()
}

fn resolve_mcp_token_value(cli: &Cli) -> Result<Option<String>, String> {
    if cli.allow_remote_without_mcp_token {
        if explicit_token_source_configured(cli.mcp_token.as_ref(), cli.mcp_token_file.as_ref()) {
            return Err(
                "--allow-remote-without-mcp-token is mutually exclusive with --mcp-token or --mcp-token-file"
                    .into(),
            );
        }
        return Ok(None);
    }

    resolve_token_value(
        "--mcp-token",
        cli.mcp_token.as_ref(),
        cli.mcp_token_file.as_ref(),
        &cli.mcp_token_env,
    )
}

fn canonicalize_required_path(flag: &str, path: Option<&String>) -> Result<PathBuf, String> {
    let path = path.ok_or_else(|| format!("{} is required", flag))?;
    std::fs::canonicalize(path)
        .map_err(|error| format!("failed to canonicalize {} '{}': {}", flag, path, error))
}

#[cfg(unix)]
fn warn_if_secret_file_permissions_are_loose(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(meta) = std::fs::metadata(path) {
        let mode = meta.permissions().mode();
        if mode & 0o077 != 0 {
            eprintln!(
                "Warning: secret file '{}' is readable or writable by group/other; prefer mode 0400 or 0440.",
                path.display()
            );
        }
    }
}

#[cfg(not(unix))]
fn warn_if_secret_file_permissions_are_loose(_path: &Path) {}

fn validate_remote_mcp_bind_auth(
    bind: SocketAddr,
    mcp_token: Option<&String>,
    allow_remote_without_mcp_token: bool,
) -> Result<(), String> {
    if is_loopback_bind(bind) || mcp_token.is_some() {
        return Ok(());
    }
    if allow_remote_without_mcp_token {
        eprintln!(
            "Warning: mmux MCP is bound to {} without authentication. Only use this behind localhost-only port forwarding or another trusted network boundary.",
            bind
        );
        return Ok(());
    }
    Err(format!(
        "refusing to bind unauthenticated MCP to {}; set --mcp-token, --mcp-token-file, or MMUX_MCP_TOKEN, or deliberately use --allow-remote-without-mcp-token behind a trusted network boundary",
        bind
    ))
}

fn resolve_node_wire_policy(
    cli: &Cli,
    allow_reject_all_without_wire_auth: bool,
) -> Result<ResolvedNodeWirePolicy, String> {
    let explicit_wire_token =
        explicit_token_source_configured(cli.wire_token.as_ref(), cli.wire_token_file.as_ref());

    if cli.wire_mtls {
        if explicit_wire_token {
            return Err(
                "--wire-mtls is mutually exclusive with --wire-token or --wire-token-file".into(),
            );
        }
        if cli.allow_unauthenticated_node_wire {
            return Err(
                "--wire-mtls is mutually exclusive with --allow-unauthenticated-node-wire".into(),
            );
        }
        let server_cert = canonicalize_required_path("--tls-cert", cli.tls_cert.as_ref())?;
        let server_key = canonicalize_required_path("--tls-key", cli.tls_key.as_ref())?;
        let client_ca =
            canonicalize_required_path("--wire-client-ca", cli.wire_client_ca.as_ref())?;
        warn_if_secret_file_permissions_are_loose(&server_key);
        return Ok(ResolvedNodeWirePolicy {
            policy: NodeWireAuthPolicy {
                mode: NodeWireAuthMode::Mtls,
            },
            token: None,
            native_mtls: Some(NativeMtlsConfig {
                server_cert,
                server_key,
                client_ca,
            }),
        });
    }

    if cli.tls_cert.is_some() || cli.tls_key.is_some() || cli.wire_client_ca.is_some() {
        return Err("--tls-cert, --tls-key, and --wire-client-ca require --wire-mtls".into());
    }

    if cli.allow_unauthenticated_node_wire {
        if explicit_wire_token {
            return Err(
                "--allow-unauthenticated-node-wire is mutually exclusive with --wire-token or --wire-token-file"
                    .into(),
            );
        }
        return Ok(ResolvedNodeWirePolicy {
            policy: NodeWireAuthPolicy {
                mode: NodeWireAuthMode::Unauthenticated,
            },
            token: None,
            native_mtls: None,
        });
    }

    let token = resolve_token_value(
        "--wire-token",
        cli.wire_token.as_ref(),
        cli.wire_token_file.as_ref(),
        &cli.wire_token_env,
    )?;

    match token {
        Some(token) => Ok(ResolvedNodeWirePolicy {
            policy: NodeWireAuthPolicy {
                mode: NodeWireAuthMode::Token,
            },
            token: Some(token),
            native_mtls: None,
        }),
        None if allow_reject_all_without_wire_auth => Ok(ResolvedNodeWirePolicy {
            policy: NodeWireAuthPolicy {
                mode: NodeWireAuthMode::Token,
            },
            token: None,
            native_mtls: None,
        }),
        None => Err(
            "node wire RPC requires --wire-token, --wire-token-file, MMUX_WIRE_TOKEN, --wire-mtls, or explicit --allow-unauthenticated-node-wire"
                .into(),
        ),
    }
}

fn resolve_embedded_node_config(cli: &Cli) -> Result<Option<EmbeddedNodeConfig>, String> {
    match (cli.enable_local_node, cli.enable_microsandbox_node) {
        (true, true) => {
            Err("--enable-local-node is mutually exclusive with --enable-microsandbox-node".into())
        }
        (true, false) => {
            if cli.sandbox_name.is_some() {
                return Err("--sandbox-name requires --enable-microsandbox-node".into());
            }
            Ok(Some(EmbeddedNodeConfig::Local {
                store_path: cli.store_path.clone(),
                tmux_config: cli.tmux_config.clone(),
            }))
        }
        (false, true) => {
            if cli.tmux_config.is_some() {
                return Err("--tmux-config requires --enable-local-node".into());
            }
            let sandbox_name = cli.sandbox_name.clone().ok_or_else(|| {
                "--sandbox-name is required with --enable-microsandbox-node".to_owned()
            })?;
            Ok(Some(EmbeddedNodeConfig::Microsandbox { sandbox_name }))
        }
        (false, false) => {
            if cli.sandbox_name.is_some() {
                return Err("--sandbox-name requires --enable-microsandbox-node".into());
            }
            if cli.tmux_config.is_some() {
                return Err("--tmux-config requires --enable-local-node".into());
            }
            Ok(None)
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Entry point
// ═══════════════════════════════════════════════════════════════════════════════

pub fn main_entry() {
    main_entry_from(std::env::args_os());
}

pub fn print_help() -> std::io::Result<()> {
    let mut command = Cli::command();
    command.print_help()?;
    println!();
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalProjectEntry {
    pub id: String,
    pub slug: String,
    pub title: String,
    pub status: String,
    pub active_task_count: usize,
    pub task_count: usize,
}

fn local_project_entry(
    state: &OrchestrationState,
    project: &mmux_controller_core::orchestration::Project,
) -> LocalProjectEntry {
    let task_count = state
        .tasks
        .values()
        .filter(|task| task_project_id(state, task).as_ref() == Some(&project.id))
        .count();
    let active_task_count = state
        .tasks
        .values()
        .filter(|task| {
            task_project_id(state, task).as_ref() == Some(&project.id) && !task.status.is_finished()
        })
        .count();
    LocalProjectEntry {
        id: project.id.0.clone(),
        slug: project.slug.clone(),
        title: project.title.clone(),
        status: format!("{:?}", project.status),
        active_task_count,
        task_count,
    }
}

pub fn local_create_project(
    store_path: Option<&Path>,
    title: String,
    description: String,
    slug: Option<String>,
) -> Result<LocalProjectEntry, String> {
    let store_path = mmux_node::resolve_store_path(store_path)?;
    let store = store::OrchestrationStore::open(store_path)?;
    let mut state = store.load()?.unwrap_or_default();
    let now_ms = now_ms();
    let project = state.create_project(
        CreateProject {
            title,
            description,
            slug,
        },
        now_ms,
    )?;
    store.save(&state, now_ms)?;
    Ok(local_project_entry(&state, &project))
}

pub fn local_projects(store_path: Option<&Path>) -> Result<Vec<LocalProjectEntry>, String> {
    let store_path = mmux_node::resolve_store_path(store_path)?;
    let store = store::OrchestrationStore::open(store_path)?;
    let Some(state) = store.load()? else {
        return Ok(Vec::new());
    };
    let mut projects = state
        .projects
        .values()
        .map(|project| local_project_entry(&state, project))
        .collect::<Vec<_>>();
    projects.sort_by(|left, right| {
        left.id
            .cmp(&right.id)
            .then_with(|| left.slug.cmp(&right.slug))
            .then_with(|| left.title.cmp(&right.title))
    });
    Ok(projects)
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct LocalPruneStoreReport {
    pub dry_run: bool,
    pub sessions_only: bool,
    pub candidates: Vec<LocalPruneSessionCandidate>,
    pub pruned_session_count: usize,
    pub pruned_plan_count: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct LocalPruneSessionCandidate {
    pub key: String,
    pub session: String,
    pub task_id: String,
    pub last_seen_ms: u64,
    pub reason: String,
}

pub(crate) fn prune_finished_plans(
    state: &mut OrchestrationState,
    sessions_only: bool,
    cutoff_ms: Option<u64>,
) -> usize {
    if sessions_only {
        return 0;
    }
    let plan_ids = state
        .plans
        .iter()
        .filter(|(plan_id, plan)| {
            if !plan.status.is_finished() {
                return false;
            }
            let plan_finished_at_ms = plan.completed_at_ms.unwrap_or(plan.updated_at_ms);
            if cutoff_ms.is_some_and(|cutoff_ms| plan_finished_at_ms > cutoff_ms) {
                return false;
            }
            state
                .tasks
                .values()
                .filter(|task| &task.plan_id == *plan_id)
                .all(|task| task.status.is_finished())
        })
        .map(|(plan_id, _)| plan_id.clone())
        .collect::<Vec<_>>();
    let pruned_count = plan_ids.len();
    let plan_ids = plan_ids.into_iter().collect::<HashSet<_>>();
    let task_ids = state
        .tasks
        .iter()
        .filter(|(_, task)| plan_ids.contains(&task.plan_id))
        .map(|(task_id, _)| task_id.clone())
        .collect::<HashSet<_>>();
    for plan_id in &plan_ids {
        state.plans.remove(&plan_id);
    }
    for task_id in &task_ids {
        state.tasks.remove(task_id);
    }
    state
        .task_edges
        .retain(|edge| !task_ids.contains(&edge.from) && !task_ids.contains(&edge.to));
    pruned_count
}

pub fn local_prune_store(
    store_path: Option<&Path>,
    live_local_sessions: &HashSet<String>,
    dry_run: bool,
    sessions_only: bool,
    older_than_days: Option<u64>,
) -> Result<LocalPruneStoreReport, String> {
    let store_path = mmux_node::resolve_store_path(store_path)?;
    let store = store::OrchestrationStore::open(store_path)?;
    let Some(mut state) = store.load()? else {
        return Ok(LocalPruneStoreReport {
            dry_run,
            sessions_only,
            candidates: Vec::new(),
            pruned_session_count: 0,
            pruned_plan_count: 0,
        });
    };
    let now_ms = now_ms();
    let cutoff_ms = older_than_days
        .map(|days| {
            days.checked_mul(86_400_000)
                .and_then(|duration_ms| now_ms.checked_sub(duration_ms))
                .ok_or_else(|| format!("--older-than-days value {days} is too large"))
        })
        .transpose()?;
    let mut candidates = state
        .tasks
        .values()
        .filter_map(|task| {
            let session = task.session.as_ref()?;
            if session.node_id.0 != "local" {
                return None;
            }
            if live_local_sessions.contains(&session.session.0) {
                return None;
            }
            if cutoff_ms.is_some_and(|cutoff_ms| session.last_seen_ms > cutoff_ms) {
                return None;
            }
            if !task.status.is_finished() {
                return None;
            }
            Some(LocalPruneSessionCandidate {
                key: session.key(),
                session: session.session.0.clone(),
                task_id: task.id.0.clone(),
                last_seen_ms: session.last_seen_ms,
                reason: "missing local tmux session attached only to finished tasks".into(),
            })
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        left.session
            .cmp(&right.session)
            .then_with(|| left.key.cmp(&right.key))
    });
    let pruned_session_count = candidates.len();
    let pruned_plan_count = if dry_run {
        let mut preview = state.clone();
        for candidate in &candidates {
            if let Some(task) = preview.tasks.get_mut(&TaskId(candidate.task_id.clone())) {
                task.session = None;
            }
        }
        prune_finished_plans(&mut preview, sessions_only, cutoff_ms)
    } else {
        for candidate in &candidates {
            if let Some(task) = state.tasks.get_mut(&TaskId(candidate.task_id.clone())) {
                task.session = None;
            }
        }
        let pruned_plan_count = prune_finished_plans(&mut state, sessions_only, cutoff_ms);
        store.save(&state, now_ms)?;
        pruned_plan_count
    };
    Ok(LocalPruneStoreReport {
        dry_run,
        sessions_only,
        candidates,
        pruned_session_count,
        pruned_plan_count,
    })
}

pub fn local_project_session_names(
    store_path: Option<&Path>,
    project: &str,
) -> Result<HashSet<String>, String> {
    let project = project.trim();
    if project.is_empty() {
        return Err("--project must not be empty".into());
    }

    let store_path = mmux_node::resolve_store_path(store_path)?;
    let store = store::OrchestrationStore::open(store_path)?;
    let state = store
        .load()?
        .ok_or_else(|| "no orchestration state found in mmux store".to_owned())?;
    let project_id = state
        .projects
        .values()
        .find(|candidate| candidate.id.0 == project || candidate.slug == project)
        .map(|candidate| candidate.id.clone())
        .ok_or_else(|| format!("project '{project}' not found"))?;
    let sessions = state
        .tasks
        .values()
        .filter(|task| task_project_id(&state, task).as_ref() == Some(&project_id))
        .filter_map(|task| task.session.as_ref())
        .filter(|session| session.node_id.0 == "local")
        .map(|session| session.session.0.clone())
        .collect();
    Ok(sessions)
}

pub fn main_entry_from<I, T>(args: I)
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let cli = Cli::parse_from(args);
    let policy = ControllerPolicy::new(&cli).unwrap_or_else(|e| {
        eprintln!("Controller policy error: {}", e);
        std::process::exit(1);
    });
    let mcp_token = resolve_mcp_token_value(&cli).unwrap_or_else(|e| {
        eprintln!("MCP token error: {}", e);
        std::process::exit(1);
    });
    let embedded_node = resolve_embedded_node_config(&cli).unwrap_or_else(|e| {
        eprintln!("Embedded node config error: {}", e);
        std::process::exit(1);
    });
    let wire_auth = resolve_node_wire_policy(&cli, embedded_node.is_some()).unwrap_or_else(|e| {
        eprintln!("Node wire policy error: {}", e);
        std::process::exit(1);
    });

    let coder_profiles = resolve_coder_profiles(&cli).unwrap_or_else(|e| {
        eprintln!("Coder profile config error: {}", e);
        std::process::exit(1);
    });

    // MCP HTTP server mode
    let bind: SocketAddr = format!("{}:{}", cli.host, cli.port)
        .parse()
        .unwrap_or_else(|_| "127.0.0.1:3000".parse().unwrap());
    validate_remote_mcp_bind_auth(bind, mcp_token.as_ref(), cli.allow_remote_without_mcp_token)
        .unwrap_or_else(|e| {
            eprintln!("Controller policy error: {}", e);
            std::process::exit(1);
        });
    let orchestration = orchestration_actor::OrchestrationHandle::open(cli.store_path.as_deref())
        .unwrap_or_else(|e| {
            eprintln!("Orchestration store error: {}", e);
            std::process::exit(1);
        });

    let rt = tokio::runtime::Runtime::new().expect("failed to create tokio runtime");
    let local_runtime = runtime::LocalRuntime::new(runtime::LocalRuntimeConfig {
        bind,
        profiles: coder_profiles.profiles,
        default_coder_profile: coder_profiles.default_profile,
        policy,
        mcp_token,
        wire_auth,
        embedded_node,
        orchestration,
    });
    if let Err(e) = rt.block_on(runtime::ControllerRuntime::run(local_runtime)) {
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
    use mmux_controller_core::orchestration::{
        CreateProject, CreateTask, OrchestrationState, Plan, Project, ProjectSummary, Task,
        TaskEdge, TaskSummary,
    };
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{}-{unique}", std::process::id()))
    }

    fn test_cli() -> Cli {
        Cli {
            host: "127.0.0.1".into(),
            port: 3000,
            mcp_token: None,
            mcp_token_file: None,
            mcp_token_env: "MMUX_MCP_TOKEN".into(),
            wire_token: None,
            wire_mtls: false,
            tls_cert: None,
            tls_key: None,
            wire_client_ca: None,
            wire_token_file: None,
            wire_token_env: "MMUX_WIRE_TOKEN".into(),
            store_path: None,
            tmux_config: None,
            allow_remote_without_mcp_token: false,
            allow_unauthenticated_node_wire: false,
            enable_admin_tools: true,
            max_read_bytes: 4 * 1024 * 1024,
            max_write_bytes: 4 * 1024 * 1024,
            max_timeout_seconds: 120.0,
            max_request_bytes: 2 * 1024 * 1024,
            max_capture_bytes: 2 * 1024 * 1024,
            enabled_coder_profiles: None,
            default_coder_profile: None,
            enable_local_node: false,
            enable_microsandbox_node: false,
            sandbox_name: None,
        }
    }

    async fn test_orchestration_server(store_path: &Path) -> TmuxMcpServer {
        let (registry, _registry_handle) = Actor::spawn(None, NodeRegistryActor, None)
            .await
            .expect("registry actor");
        let (node_executor, _node_executor_handle) = Actor::spawn(
            None,
            NodeExecutionActor,
            NodeExecutionWorkerState {
                embedded: None,
                registry: registry.clone(),
            },
        )
        .await
        .expect("node execution actor");
        TmuxMcpServer::new(
            Arc::new(HashMap::new()),
            None,
            ControllerPolicy::new(&test_cli()).unwrap(),
            node_executor,
            registry,
            orchestration_actor::OrchestrationHandle::open(Some(store_path)).unwrap(),
            Arc::new(Mutex::new(HashMap::new())),
            Arc::new(Mutex::new(Vec::new())),
        )
    }

    async fn test_orchestration_server_with_profiles(
        store_path: &Path,
        profiles: ProfileRegistry,
    ) -> TmuxMcpServer {
        let (registry, _registry_handle) = Actor::spawn(None, NodeRegistryActor, None)
            .await
            .expect("registry actor");
        let (node_executor, _node_executor_handle) = Actor::spawn(
            None,
            NodeExecutionActor,
            NodeExecutionWorkerState {
                embedded: None,
                registry: registry.clone(),
            },
        )
        .await
        .expect("node execution actor");
        let default_coder_profile = first_enabled_builtin_profile(&profiles);
        TmuxMcpServer::new(
            profiles,
            default_coder_profile,
            ControllerPolicy::new(&test_cli()).unwrap(),
            node_executor,
            registry,
            orchestration_actor::OrchestrationHandle::open(Some(store_path)).unwrap(),
            Arc::new(Mutex::new(HashMap::new())),
            Arc::new(Mutex::new(Vec::new())),
        )
    }

    async fn test_coding_server(
        store_path: &Path,
        local_store_path: &Path,
        profiles: ProfileRegistry,
    ) -> TmuxMcpServer {
        let backend = mmux_node::EmbeddedNodeBackend::local(Some(local_store_path), None)
            .await
            .expect("local backend");
        let (registry, _registry_handle) =
            Actor::spawn(None, NodeRegistryActor, Some("Local tmux node".into()))
                .await
                .expect("registry actor");
        let (node_executor, _node_executor_handle) = Actor::spawn(
            None,
            NodeExecutionActor,
            NodeExecutionWorkerState {
                embedded: Some(backend),
                registry: registry.clone(),
            },
        )
        .await
        .expect("node execution actor");
        let default_coder_profile = first_enabled_builtin_profile(&profiles);
        TmuxMcpServer::new(
            profiles,
            default_coder_profile,
            ControllerPolicy::new(&test_cli()).unwrap(),
            node_executor,
            registry,
            orchestration_actor::OrchestrationHandle::open(Some(store_path)).unwrap(),
            Arc::new(Mutex::new(HashMap::new())),
            Arc::new(Mutex::new(Vec::new())),
        )
    }

    fn profile_registry(profile: CliProfile) -> ProfileRegistry {
        Arc::new(HashMap::from([(profile.name.clone(), profile)]))
    }

    async fn test_create_session(server: &TmuxMcpServer, session: &str, command: &str) {
        server
            .create_session_with_command("local", session, command, None)
            .await
            .unwrap();
    }

    async fn test_kill_session(server: &TmuxMcpServer, session: &str) {
        let _ = server
            .node_tmux(
                "local",
                vec!["kill-session".into(), "-t".into(), session.into()],
                Duration::from_secs(20),
            )
            .await;
    }

    async fn test_session_exists(server: &TmuxMcpServer, session: &str) -> bool {
        server
            .node_session_exists("local", session)
            .await
            .unwrap_or(false)
    }

    fn ready_profile() -> CliProfile {
        CliProfile {
            name: "codex".into(),
            cmd: Some("sh -c 'printf READY; sleep 30'".into()),
            permission_bypass_cmd: Some("sh -c 'printf READY; sleep 30'".into()),
            prompt_indicator: "READY".into(),
            busy_indicators: Vec::new(),
            ..CliProfile::default()
        }
    }

    fn object_args(value: Value) -> Map<String, Value> {
        value.as_object().expect("object args").clone()
    }

    fn call_orchestration(
        server: &TmuxMcpServer,
        name: &str,
        args: Value,
    ) -> Result<CallToolResult, McpError> {
        server
            .call_orchestration_tool(name, object_args(args))
            .expect("known orchestration tool")
    }

    async fn call_session_record(
        server: &TmuxMcpServer,
        args: Value,
    ) -> Result<CallToolResult, McpError> {
        server.session_record_tool(object_args(args)).await
    }

    async fn create_test_project(server: &TmuxMcpServer, title: &str) -> Project {
        let result = call_orchestration(
            server,
            "project_create",
            json!({
                "title": title,
                "description": format!("Project for {title}")
            }),
        )
        .unwrap();
        result_json(&result)
    }

    async fn create_test_task_in_project(
        server: &TmuxMcpServer,
        project: &Project,
        title: &str,
    ) -> Task {
        let plan = create_test_plan_in_project(server, project, &format!("{title} Plan")).await;
        create_test_task_in_plan(server, &plan, title).await
    }

    async fn create_test_plan_in_project(
        server: &TmuxMcpServer,
        project: &Project,
        title: &str,
    ) -> Plan {
        let result = call_orchestration(
            server,
            "plan_create",
            json!({
                "project_id": project.id.0,
                "title": title,
                "brief": format!("Detailed plan brief for {title}")
            }),
        )
        .unwrap();
        result_json(&result)
    }

    async fn create_test_task_in_plan(server: &TmuxMcpServer, plan: &Plan, title: &str) -> Task {
        let result = call_orchestration(
            server,
            "task_create",
            json!({
                "plan_id": plan.id.0,
                "title": title,
                "objective": format!("Objective for {title}")
            }),
        )
        .unwrap();
        result_json(&result)
    }

    fn result_text(result: &CallToolResult) -> &str {
        result.content[0].as_text().unwrap().text.as_str()
    }

    fn result_json<T: DeserializeOwned>(result: &CallToolResult) -> T {
        serde_json::from_str(result_text(result)).unwrap()
    }

    async fn ensure_test_project(server: &TmuxMcpServer) -> Project {
        let list = call_orchestration(server, "project_list", json!({})).unwrap();
        let projects: Vec<ProjectSummary> = result_json(&list);
        if let Some(project) = projects.into_iter().next() {
            return Project {
                id: project.id,
                slug: project.slug,
                title: project.title,
                description: project.description,
                status: project.status,
                created_at_ms: 0,
                updated_at_ms: project.updated_at_ms,
            };
        }
        let result = call_orchestration(
            server,
            "project_create",
            json!({
                "title": "Test Project",
                "description": "Project for MCP tests"
            }),
        )
        .unwrap();
        result_json(&result)
    }

    async fn create_test_task(server: &TmuxMcpServer, title: &str) -> Task {
        let project = ensure_test_project(server).await;
        create_test_task_in_project(server, &project, title).await
    }

    fn coding_task_send_args(
        task_id_or_slug: impl Into<String>,
        prompt: impl Into<String>,
    ) -> CodingTaskSendArgs {
        CodingTaskSendArgs {
            node: "local".into(),
            session: "worker".into(),
            profile: Some("codex".into()),
            task_id_or_slug: task_id_or_slug.into(),
            prompt: prompt.into(),
            template: None,
            include_dependencies: None,
            include_gates: None,
            include_scope: None,
            context_task_ids: None,
            extra_context: None,
        }
    }

    async fn wait_for_runtime_status(
        server: &TmuxMcpServer,
        wait_id: &str,
        status: RuntimeWaitStatus,
    ) -> RuntimeWaitSnapshot {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let result = server
                .wait_status_tool(object_args(json!({ "wait_id": wait_id })))
                .unwrap();
            let snapshot: RuntimeWaitSnapshot = result_json(&result);
            if snapshot.status == status || Instant::now() >= deadline {
                return snapshot;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    async fn register_test_node(server: &TmuxMcpServer, node_id: &str) {
        let descriptor = NodeDescriptor {
            node_id: node_id.to_owned(),
            display_name: format!("test node {node_id}"),
        };
        server
            .registry_call(
                |reply| NodeRegistryMessage::Register { descriptor, reply },
                None,
            )
            .await
            .unwrap();
    }

    async fn pull_next_node_command(server: &TmuxMcpServer, node_id: &str) -> NodeCommand {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            let node_id = node_id.to_owned();
            let mut commands = server
                .registry_call(|reply| NodeRegistryMessage::Pull { node_id, reply }, None)
                .await
                .unwrap();
            if !commands.is_empty() {
                return commands.remove(0);
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("timed out waiting for node command");
    }

    async fn submit_node_result(
        server: &TmuxMcpServer,
        node_id: &str,
        command_id: String,
        result: NodeCommandResult,
    ) {
        let node_id = node_id.to_owned();
        server
            .registry_call(
                |reply| NodeRegistryMessage::SubmitResult {
                    node_id,
                    command_id,
                    result,
                    reply,
                },
                None,
            )
            .await
            .unwrap();
    }

    fn string_args(args: &[&str]) -> Vec<String> {
        args.iter().map(|arg| (*arg).to_owned()).collect()
    }

    #[test]
    fn test_resolve_embedded_node_config_validates_flags() {
        let mut cli = test_cli();
        assert!(resolve_embedded_node_config(&cli).unwrap().is_none());

        cli.enable_local_node = true;
        assert!(matches!(
            resolve_embedded_node_config(&cli).unwrap(),
            Some(EmbeddedNodeConfig::Local {
                store_path: None,
                tmux_config: None,
            })
        ));

        cli.enable_microsandbox_node = true;
        assert!(resolve_embedded_node_config(&cli)
            .unwrap_err()
            .contains("mutually exclusive"));

        cli.enable_local_node = false;
        assert!(resolve_embedded_node_config(&cli)
            .unwrap_err()
            .contains("--sandbox-name is required"));

        cli.sandbox_name = Some("mmux-node".into());
        assert!(matches!(
            resolve_embedded_node_config(&cli).unwrap(),
            Some(EmbeddedNodeConfig::Microsandbox { sandbox_name }) if sandbox_name == "mmux-node"
        ));

        cli.enable_microsandbox_node = false;
        assert!(resolve_embedded_node_config(&cli)
            .unwrap_err()
            .contains("--sandbox-name requires"));
    }

    #[test]
    fn test_resolve_embedded_node_config_validates_tmux_config_scope() {
        let mut cli = test_cli();
        cli.tmux_config = Some(PathBuf::from("tmux.local.conf"));
        assert!(resolve_embedded_node_config(&cli)
            .unwrap_err()
            .contains("--tmux-config requires --enable-local-node"));

        cli.enable_microsandbox_node = true;
        cli.sandbox_name = Some("mmux-node".into());
        assert!(resolve_embedded_node_config(&cli)
            .unwrap_err()
            .contains("--tmux-config requires --enable-local-node"));

        cli.enable_microsandbox_node = false;
        cli.sandbox_name = None;
        cli.enable_local_node = true;
        assert!(matches!(
            resolve_embedded_node_config(&cli).unwrap(),
            Some(EmbeddedNodeConfig::Local {
                store_path: None,
                tmux_config: Some(path),
            }) if path == PathBuf::from("tmux.local.conf")
        ));
    }

    #[test]
    fn test_orchestration_tools_are_listed_by_helper() {
        let admin_tools = TmuxMcpServer::orchestration_tool_definitions(true);
        let admin_names = admin_tools
            .iter()
            .map(|tool| tool.name.as_ref())
            .collect::<Vec<_>>();

        for expected in [
            "project_create",
            "project_list",
            "project_status_update",
            "plan_create",
            "plan_list",
            "plan_update",
            "plan_status_update",
            "task_create",
            "task_update",
            "task_edge_add",
            "task_edge_remove",
            "session_record",
            "task_status_update",
            "orchestration_status",
            "orchestration_cleanup_zombies",
        ] {
            assert!(admin_names.contains(&expected), "missing {expected}");
        }

        let non_admin_tools = TmuxMcpServer::orchestration_tool_definitions(false);
        let non_admin_names = non_admin_tools
            .iter()
            .map(|tool| tool.name.as_ref())
            .collect::<Vec<_>>();
        assert!(non_admin_names.contains(&"project_list"));
        assert!(!non_admin_names.contains(&"project_create"));
        assert!(!non_admin_names.contains(&"project_status_update"));
    }

    #[tokio::test]
    async fn test_project_mutation_tools_require_admin_flag_but_project_list_does_not() {
        let dir = unique_temp_dir("mmux-mcp-admin-tools");
        let mut cli = test_cli();
        cli.enable_admin_tools = false;
        let (registry, _registry_handle) = Actor::spawn(None, NodeRegistryActor, None)
            .await
            .expect("registry actor");
        let (node_executor, _node_executor_handle) = Actor::spawn(
            None,
            NodeExecutionActor,
            NodeExecutionWorkerState {
                embedded: None,
                registry: registry.clone(),
            },
        )
        .await
        .expect("node execution actor");
        let server = TmuxMcpServer::new(
            Arc::new(HashMap::new()),
            None,
            ControllerPolicy::new(&cli).unwrap(),
            node_executor,
            registry,
            orchestration_actor::OrchestrationHandle::open(Some(&dir)).unwrap(),
            Arc::new(Mutex::new(HashMap::new())),
            Arc::new(Mutex::new(Vec::new())),
        );

        let create_error = call_orchestration(
            &server,
            "project_create",
            json!({ "title": "Admin Project" }),
        )
        .unwrap_err();
        assert!(
            create_error.message.contains("--enable-admin-tools"),
            "{create_error}"
        );

        let status_error = call_orchestration(
            &server,
            "project_status_update",
            json!({ "project_id": "missing", "status": "Archived" }),
        )
        .unwrap_err();
        assert!(
            status_error.message.contains("--enable-admin-tools"),
            "{status_error}"
        );

        let projects: Vec<ProjectSummary> =
            result_json(&call_orchestration(&server, "project_list", json!({})).unwrap());
        assert!(projects.is_empty());
        let _ = fs::remove_dir_all(dir);
    }

    fn local_info(session: &str, created_at_seconds: u64) -> LocalSessionInfo {
        LocalSessionInfo {
            session: session.into(),
            created_at_seconds: Some(created_at_seconds),
        }
    }

    fn recorded_session(session: &str) -> TaskSession {
        TaskSession {
            node_id: NodeId("local".into()),
            session: SessionId(session.into()),
            profile: "codex".into(),
            workspace_path: "/workspace".into(),
            bypass_permissions: false,
            role: "implementation-worker".into(),
            kind: "codex".into(),
            skills: vec!["rust".into()],
            created_at_ms: 0,
            updated_at_ms: 0,
            last_seen_ms: 0,
        }
    }

    fn create_state_plan(state: &mut OrchestrationState, project: &Project) -> Plan {
        state
            .create_plan(
                CreatePlan {
                    project_id: project.id.clone(),
                    title: "Plan".into(),
                    brief: "Detailed plan brief for test task derivation.".into(),
                    slug: Some("plan".into()),
                },
                95,
            )
            .unwrap()
    }

    fn state_with_task(status: TaskStatus) -> (OrchestrationState, TaskId) {
        let mut state = OrchestrationState::new();
        let project = state
            .create_project(
                CreateProject {
                    title: "Project".into(),
                    description: "Test project".into(),
                    slug: None,
                },
                90,
            )
            .unwrap();
        let plan = state
            .create_plan(
                CreatePlan {
                    project_id: project.id,
                    title: "Plan".into(),
                    brief: "Detailed plan brief for test task.".into(),
                    slug: None,
                },
                95,
            )
            .unwrap();
        let task = state
            .create_task(
                CreateTask {
                    plan_id: plan.id,
                    title: "Task".into(),
                    objective: "Objective".into(),
                    scope: TaskScope::default(),
                    gates: Vec::new(),
                    slug: None,
                },
                100,
            )
            .unwrap();
        if status != TaskStatus::Backlog {
            state.update_task_status(&task.id, status, 200).unwrap();
        }
        (state, task.id)
    }

    #[test]
    fn test_zombie_candidate_detection_filters_by_prefix_and_age() {
        let live = vec![
            local_info("mmux-old", 10),
            local_info("mmux-new", 95),
            local_info("user-session", 1),
        ];
        let durable = HashSet::new();

        let candidates =
            cleanup_candidates_from_live_sessions("local", &live, &durable, Some(60), 100);

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].session, "mmux-old");
        assert_eq!(candidates[0].node_id, "local");
        assert_eq!(candidates[0].created_at_ms, Some(10_000));
    }

    #[test]
    fn test_cleanup_dry_run_noop_has_no_kill_targets() {
        let live = vec![local_info("mmux-zombie", 10)];
        let durable = HashSet::new();
        let candidates = cleanup_candidates_from_live_sessions("local", &live, &durable, None, 100);
        let dry_run = true;
        let killed = if dry_run {
            Vec::new()
        } else {
            safe_cleanup_kill_targets(&candidates, &durable).0
        };

        assert_eq!(candidates.len(), 1);
        assert!(killed.is_empty());
    }

    #[test]
    fn test_explicit_cleanup_kill_targets_are_candidates_only() {
        let candidates = vec![
            SessionCleanupCandidate {
                node_id: "local".into(),
                session: "mmux-zombie".into(),
                reason: "missing durable record".into(),
                created_at_ms: None,
            },
            SessionCleanupCandidate {
                node_id: "local".into(),
                session: "not-mmux".into(),
                reason: "bad input".into(),
                created_at_ms: None,
            },
        ];
        let durable = HashSet::new();

        let (targets, warnings) = safe_cleanup_kill_targets(&candidates, &durable);

        assert_eq!(targets, vec!["mmux-zombie"]);
        assert!(warnings.iter().any(|warning| warning.contains("non-mmux")));
    }

    #[test]
    fn test_durable_session_is_protected_from_cleanup() {
        let live = vec![
            local_info("mmux-recorded", 10),
            local_info("mmux-zombie", 10),
        ];
        let durable = HashSet::from([runtime_session_key("local", "mmux-recorded")]);

        let candidates = cleanup_candidates_from_live_sessions("local", &live, &durable, None, 100);
        let (targets, _) = safe_cleanup_kill_targets(&candidates, &durable);

        assert_eq!(
            candidates
                .iter()
                .map(|candidate| candidate.session.as_str())
                .collect::<Vec<_>>(),
            vec!["mmux-zombie"]
        );
        assert_eq!(targets, vec!["mmux-zombie"]);
    }

    #[test]
    fn test_status_decoration_sets_runtime_state_and_cleanup_candidates() {
        let mut state = OrchestrationState::new();
        let project = state
            .create_project(
                CreateProject {
                    title: "Project".into(),
                    description: "Test project".into(),
                    slug: None,
                },
                90,
            )
            .unwrap();
        let plan = state
            .create_plan(
                CreatePlan {
                    project_id: project.id,
                    title: "Plan".into(),
                    brief: "Detailed plan brief for runtime status.".into(),
                    slug: None,
                },
                95,
            )
            .unwrap();
        let live_task = state
            .create_task(
                CreateTask {
                    plan_id: plan.id.clone(),
                    title: "Runtime Live".into(),
                    objective: "Live status".into(),
                    scope: TaskScope::default(),
                    gates: Vec::new(),
                    slug: None,
                },
                100,
            )
            .unwrap();
        let missing_task = state
            .create_task(
                CreateTask {
                    plan_id: plan.id,
                    title: "Runtime Missing".into(),
                    objective: "Missing status".into(),
                    scope: TaskScope::default(),
                    gates: Vec::new(),
                    slug: None,
                },
                101,
            )
            .unwrap();
        state
            .record_session(&live_task.id, recorded_session("mmux-live"), 110)
            .unwrap();
        state
            .record_session(&missing_task.id, recorded_session("mmux-missing"), 120)
            .unwrap();
        let mut status = state.orchestration_status(200);
        let live = vec![local_info("mmux-live", 10), local_info("mmux-zombie", 10)];

        decorate_orchestration_status_with_local_runtime(&mut status, &live, "local", None, 100);

        let states = status
            .sessions
            .iter()
            .map(|session| (session.session.as_str(), session.runtime_state.as_deref()))
            .collect::<HashMap<_, _>>();
        assert_eq!(states.get("mmux-live").copied().flatten(), Some("live"));
        assert_eq!(
            states.get("mmux-missing").copied().flatten(),
            Some("missing")
        );
        assert_eq!(status.cleanup_candidates.len(), 1);
        assert_eq!(status.cleanup_candidates[0].session, "mmux-zombie");
        assert_eq!(status.cleanup_candidates[0].created_at_ms, Some(10_000));
    }

    #[test]
    fn test_missing_active_stored_session_is_planned_for_recreation() {
        let (mut state, task_id) = state_with_task(TaskStatus::Running);
        state
            .record_session(&task_id, recorded_session("mmux-active"), 200)
            .unwrap();

        let actions =
            plan_local_startup_reconciliation(&state, &[], &profile_registry(ready_profile()));

        assert!(matches!(
            actions.as_slice(),
            [LocalStartupReconciliationAction::Recreate { record }]
                if record.session.0 == "mmux-active"
        ));
    }

    #[test]
    fn test_missing_active_stored_session_reports_when_not_recreatable() {
        let (mut state, task_id) = state_with_task(TaskStatus::Running);
        let mut record = recorded_session("mmux-active");
        record.profile = "missing-profile".into();
        state.record_session(&task_id, record, 200).unwrap();

        let actions =
            plan_local_startup_reconciliation(&state, &[], &profile_registry(ready_profile()));

        assert!(matches!(
            actions.as_slice(),
            [LocalStartupReconciliationAction::Missing { reason, .. }]
                if reason.contains("profile 'missing-profile' is not loaded")
        ));
    }

    #[test]
    fn test_finished_missing_stored_session_remains_historical() {
        let (mut state, task_id) = state_with_task(TaskStatus::Delivered);
        state
            .record_session(&task_id, recorded_session("mmux-finished"), 200)
            .unwrap();

        let actions =
            plan_local_startup_reconciliation(&state, &[], &profile_registry(ready_profile()));

        assert!(matches!(
            actions.as_slice(),
            [LocalStartupReconciliationAction::Historical { key }]
                if key == "local:mmux-finished"
        ));
    }

    #[test]
    fn test_local_prune_store_prunes_finished_plans_and_contained_tasks() {
        let dir = unique_temp_dir("mmux-local-prune-plans");
        let store = store::OrchestrationStore::open(dir.clone()).unwrap();
        let mut state = OrchestrationState::new();
        let project = state
            .create_project(
                CreateProject {
                    title: "Project".into(),
                    description: "Test project".into(),
                    slug: None,
                },
                90,
            )
            .unwrap();
        let plan = create_state_plan(&mut state, &project);
        let parent = state
            .create_task(
                CreateTask {
                    plan_id: plan.id.clone(),
                    title: "Parent".into(),
                    objective: "Parent objective".into(),
                    scope: TaskScope::default(),
                    gates: Vec::new(),
                    slug: None,
                },
                100,
            )
            .unwrap();
        let child = state
            .create_task(
                CreateTask {
                    plan_id: plan.id.clone(),
                    title: "Child".into(),
                    objective: "Child objective".into(),
                    scope: TaskScope::default(),
                    gates: Vec::new(),
                    slug: None,
                },
                101,
            )
            .unwrap();
        state
            .add_task_edge(
                CreateTaskEdge {
                    from: parent.id.clone(),
                    to: child.id.clone(),
                    kind: TaskEdgeKind::Related,
                    note: None,
                },
                102,
            )
            .unwrap();
        state
            .record_session(&child.id, recorded_session("mmux-finished"), 103)
            .unwrap();
        state
            .update_task_status(&parent.id, TaskStatus::Delivered, 104)
            .unwrap();
        state
            .update_task_status(&child.id, TaskStatus::Delivered, 105)
            .unwrap();
        state
            .update_plan_status(&plan.id, PlanStatus::Delivered, Some("done".into()), 106)
            .unwrap();
        store.save(&state, 107).unwrap();

        let live = HashSet::new();
        let dry_run = local_prune_store(Some(&dir), &live, true, false, None).unwrap();
        assert_eq!(dry_run.pruned_session_count, 1);
        assert_eq!(dry_run.pruned_plan_count, 1);
        assert_eq!(store.load().unwrap().unwrap().plans.len(), 1);

        let pruned = local_prune_store(Some(&dir), &live, false, false, None).unwrap();
        assert_eq!(pruned.pruned_session_count, 1);
        assert_eq!(pruned.pruned_plan_count, 1);
        let loaded = store.load().unwrap().unwrap();
        assert!(loaded.projects.contains_key(&project.id));
        assert!(loaded.plans.is_empty());
        assert!(loaded.tasks.is_empty());
        assert!(loaded.task_edges.is_empty());
        assert!(loaded.tasks.is_empty());
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn test_task_create_json_parsing_and_persistence() {
        let dir = unique_temp_dir("mmux-mcp-task-create");
        let server = test_orchestration_server(&dir).await;
        let project = ensure_test_project(&server).await;
        let plan = create_test_plan_in_project(&server, &project, "Tooling Plan").await;

        let result = call_orchestration(
            &server,
            "task_create",
            json!({
                "plan_id": plan.id.0,
                "title": "Implement Tooling",
                "objective": "Expose orchestration tools",
                "include_paths": ["crates/mmux-controller/src/lib.rs"],
                "exclude_paths": ["target"],
                "notes": "controller only",
                "gates": ["cargo test -p mmux-controller"]
            }),
        )
        .unwrap();
        let task: Task = result_json(&result);
        let reloaded = orchestration_actor::OrchestrationHandle::open(Some(&dir)).unwrap();
        let state = reloaded.snapshot().unwrap();

        assert_eq!(task.title, "Implement Tooling");
        assert_eq!(
            task.scope.include_paths,
            vec!["crates/mmux-controller/src/lib.rs"]
        );
        assert!(state.tasks.contains_key(&task.id));
        let projects: Vec<ProjectSummary> =
            result_json(&call_orchestration(&server, "project_list", json!({})).unwrap());
        let project_summary = projects
            .iter()
            .find(|summary| summary.id == project.id)
            .expect("project summary");
        assert_eq!(
            project_summary.task_status_counts.len(),
            TaskStatus::ALL.len()
        );
        assert_eq!(project_summary.task_status_counts[&TaskStatus::Backlog], 1);
        assert_eq!(project_summary.task_status_counts[&TaskStatus::Planned], 0);
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn test_orchestration_task_update_json_parsing_persistence_and_boundaries() {
        let dir = unique_temp_dir("mmux-mcp-task-update");
        let server =
            test_orchestration_server_with_profiles(&dir, profile_registry(ready_profile())).await;
        let project = ensure_test_project(&server).await;
        let plan = create_test_plan_in_project(&server, &project, "Update Plan").await;
        let task = create_test_task_in_plan(&server, &plan, "Original Update").await;
        let related = create_test_task_in_plan(&server, &plan, "Related Update").await;

        call_orchestration(
            &server,
            "task_edge_add",
            json!({
                "from_task_id": task.id.0,
                "to_task_id": related.id.0,
                "kind": "Related",
                "note": "must stay"
            }),
        )
        .unwrap();
        server
            .orchestration
            .record_session(
                task.id.clone(),
                TaskSession {
                    node_id: NodeId("node-a".into()),
                    session: SessionId("worker-a".into()),
                    profile: "codex".into(),
                    workspace_path: "/workspace/project".into(),
                    bypass_permissions: false,
                    role: "implementation-worker".into(),
                    kind: "codex".into(),
                    skills: vec!["rust".into()],
                    created_at_ms: 0,
                    updated_at_ms: 0,
                    last_seen_ms: 0,
                },
            )
            .unwrap();
        let failed: Task = result_json(
            &call_orchestration(
                &server,
                "task_status_update",
                json!({
                    "task_id": task.id.0,
                    "status": "Failed",
                    "outcome": "status must stay failed"
                }),
            )
            .unwrap(),
        );
        let before = server.orchestration.snapshot().unwrap();

        let result = call_orchestration(
            &server,
            "task_update",
            json!({
                "task_id": task.id.0,
                "title": "Updated Update",
                "objective": "Patch mutable task metadata",
                "include_paths": ["crates/mmux-controller/src/lib.rs"],
                "exclude_paths": [],
                "notes": "updated notes",
                "gates": ["review", "tests"]
            }),
        )
        .unwrap();
        let updated: Task = result_json(&result);

        assert_eq!(updated.id, failed.id);
        assert_eq!(updated.slug, failed.slug);
        assert_eq!(updated.title, "Updated Update");
        assert_eq!(updated.objective, "Patch mutable task metadata");
        assert_eq!(
            updated.scope.include_paths,
            vec!["crates/mmux-controller/src/lib.rs"]
        );
        assert!(updated.scope.exclude_paths.is_empty());
        assert_eq!(updated.scope.notes.as_deref(), Some("updated notes"));
        assert_eq!(updated.gates, vec!["review", "tests"]);
        assert_eq!(updated.status, TaskStatus::Failed);
        assert_eq!(updated.completed_at_ms, failed.completed_at_ms);
        assert!(updated.completed_at_ms.is_some());

        let after = server.orchestration.snapshot().unwrap();
        assert_eq!(after.task_edges, before.task_edges);
        assert_eq!(
            after.tasks.get(&task.id).unwrap().session,
            before.tasks.get(&task.id).unwrap().session
        );

        let status: OrchestrationStatus = result_json(
            &call_orchestration(
                &server,
                "orchestration_status",
                json!({ "task_id": task.id.0, "include_completed": true }),
            )
            .unwrap(),
        );
        assert_eq!(status.tasks[0].open_gate_count, 2);

        let reloaded = orchestration_actor::OrchestrationHandle::open(Some(&dir)).unwrap();
        let reloaded_task = reloaded
            .snapshot()
            .unwrap()
            .tasks
            .get(&updated.id)
            .unwrap()
            .clone();
        assert_eq!(reloaded_task.scope, updated.scope);
        assert_eq!(reloaded_task.session, updated.session);
        assert_eq!(reloaded_task.gates, updated.gates);
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn test_orchestration_task_update_rejects_unknown_fields() {
        let dir = unique_temp_dir("mmux-mcp-task-update-reject");
        let server = test_orchestration_server(&dir).await;
        let task = create_test_task(&server, "Reject Update").await;

        for field in ["id", "status", "completed_at_ms"] {
            let mut args = json!({
                "task_id": task.id.0
            });
            args.as_object_mut()
                .unwrap()
                .insert(field.into(), json!("forbidden"));
            let error = call_orchestration(&server, "task_update", args).unwrap_err();
            assert!(error.message.contains("unknown field"));
            assert!(
                error.message.contains(field),
                "expected error for {field}, got {error}"
            );
        }

        let error = call_orchestration(
            &server,
            "task_update",
            json!({
                "task_id": task.id.0,
                "agents": []
            }),
        )
        .unwrap_err();
        assert!(error.message.contains("unknown field"));
        assert!(error.message.contains("agents"));
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn test_task_create_rejects_agents_field() {
        let dir = unique_temp_dir("mmux-mcp-agent-fields");
        let server = test_orchestration_server(&dir).await;
        let project = ensure_test_project(&server).await;
        let plan = create_test_plan_in_project(&server, &project, "Agent Field Rejection").await;

        let error = call_orchestration(
            &server,
            "task_create",
            json!({
                "plan_id": plan.id.0,
                "title": "Bad Agent",
                "objective": "Reject placement",
                "agents": []
            }),
        )
        .unwrap_err();
        assert!(error.message.contains("unknown field"));
        assert!(error.message.contains("agents"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_generated_orchestration_session_name_sanitizes_and_keeps_prefix() {
        let name = generated_orchestration_session_name(
            "Task 12: Orchestration/Core Model!!!",
            "Model Owner++",
            "A1_B2!",
        );

        assert_eq!(
            name,
            "mmux-task-12-orchestration-core-model-model-owner-a1-b2"
        );
        assert!(name.starts_with("mmux-"));
        assert!(name
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-'));
    }

    #[tokio::test]
    async fn test_session_record_tool_records_existing_task_session() {
        let dir = unique_temp_dir("mmux-mcp-session-record");
        let local_dir = unique_temp_dir("mmux-mcp-session-record-local");
        let server = test_coding_server(&dir, &local_dir, profile_registry(ready_profile())).await;
        let task = create_test_task(&server, "Recordable").await;
        test_create_session(&server, "worker-a", "sleep 30").await;

        let result = call_session_record(
            &server,
            json!({
                "node_id": "local",
                "session": "worker-a",
                "profile": "codex",
                "workspace_path": "/workspace/project",
                "bypass_permissions": true,
                "task_id": task.id.0,
                "role": "implementation-worker",
                "kind": "model-owner",
                "skills": ["rust"]
            }),
        )
        .await
        .unwrap();
        let record: TaskSession = result_json(&result);

        assert_eq!(record.node_id.0, "local");
        assert_eq!(record.session.0, "worker-a");
        assert_eq!(record.profile, "codex");
        assert!(record.bypass_permissions);
        assert_eq!(
            server
                .orchestration
                .snapshot()
                .unwrap()
                .tasks
                .get(&task.id)
                .unwrap()
                .session
                .as_ref()
                .unwrap()
                .session
                .0,
            "worker-a"
        );
        test_kill_session(&server, "worker-a").await;
        let _ = fs::remove_dir_all(local_dir);
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn test_session_record_replaces_task_session_and_stops_old_session() {
        let dir = unique_temp_dir("mmux-mcp-session-record-replace");
        let local_dir = unique_temp_dir("mmux-mcp-session-record-replace-local");
        let server = test_coding_server(&dir, &local_dir, profile_registry(ready_profile())).await;
        let task = create_test_task(&server, "Replace Record").await;
        test_create_session(&server, "worker-old", "sleep 30").await;
        test_create_session(&server, "worker-new", "sleep 30").await;

        call_session_record(
            &server,
            json!({
                "node_id": "local",
                "session": "worker-old",
                "profile": "codex",
                "workspace_path": "/workspace/project",
                "bypass_permissions": false,
                "task_id": task.id.0,
                "role": "implementation-worker",
                "kind": "coder",
                "skills": ["rust"]
            }),
        )
        .await
        .unwrap();

        let result = call_session_record(
            &server,
            json!({
                "node_id": "local",
                "session": "worker-new",
                "profile": "codex",
                "workspace_path": "/workspace/project",
                "bypass_permissions": false,
                "task_id": task.id.0,
                "role": "implementation-worker",
                "kind": "coder",
                "skills": ["rust"]
            }),
        )
        .await
        .unwrap();
        let record: TaskSession = result_json(&result);

        assert_eq!(record.session.0, "worker-new");
        assert!(!test_session_exists(&server, "worker-old").await);
        assert!(test_session_exists(&server, "worker-new").await);
        assert_eq!(
            server
                .orchestration
                .snapshot()
                .unwrap()
                .tasks
                .get(&task.id)
                .and_then(|task| task.session.as_ref())
                .unwrap()
                .session
                .0,
            "worker-new"
        );
        test_kill_session(&server, "worker-new").await;
        let _ = fs::remove_dir_all(local_dir);
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn test_list_sessions_requires_project_id() {
        let dir = unique_temp_dir("mmux-mcp-list-sessions-project-required");
        let local_dir = unique_temp_dir("mmux-mcp-list-sessions-project-required-local");
        let server = test_coding_server(&dir, &local_dir, profile_registry(ready_profile())).await;

        let error = server
            .list_sessions_tool(object_args(json!({})))
            .await
            .unwrap_err();
        assert!(error.message.contains("project_id"), "{error}");

        let _ = fs::remove_dir_all(local_dir);
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn test_project_slug_is_accepted_where_project_id_is_required() {
        let dir = unique_temp_dir("mmux-mcp-project-slug-selector");
        let server = test_orchestration_server(&dir).await;
        let project: Project = result_json(
            &call_orchestration(
                &server,
                "project_create",
                json!({
                    "title": "Slug Selectable",
                    "description": "Project for slug selector tests",
                    "slug": "slug-selectable"
                }),
            )
            .unwrap(),
        );

        let plan: Plan = result_json(
            &call_orchestration(
                &server,
                "plan_create",
                json!({
                    "project_id": "slug-selectable",
                    "title": "Plan via slug",
                    "brief": "Detailed plan brief selected by project slug."
                }),
            )
            .unwrap(),
        );
        assert_eq!(plan.project_id, project.id);

        let task: Task = result_json(
            &call_orchestration(
                &server,
                "task_create",
                json!({
                    "plan_id": plan.id.0,
                    "title": "Task via slug",
                    "objective": "Use the plan as selector"
                }),
            )
            .unwrap(),
        );
        assert_eq!(task.plan_id, plan.id);

        let archived: Project = result_json(
            &call_orchestration(
                &server,
                "project_status_update",
                json!({
                    "project_id": "slug-selectable",
                    "status": "Archived"
                }),
            )
            .unwrap(),
        );
        assert_eq!(archived.id, project.id);
        assert_eq!(archived.status, ProjectStatus::Archived);

        let status: OrchestrationStatus = result_json(
            &call_orchestration(
                &server,
                "orchestration_status",
                json!({
                    "project_id": "slug-selectable",
                    "include_completed": true
                }),
            )
            .unwrap(),
        );
        assert_eq!(status.projects.len(), 1);
        assert_eq!(status.projects[0].id, project.id);
        assert_eq!(status.tasks.len(), 1);
        assert_eq!(status.tasks[0].id, task.id);

        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn test_list_sessions_filters_by_project_and_admin_lists_raw_sessions() {
        let dir = unique_temp_dir("mmux-mcp-list-sessions-project-filter");
        let local_dir = unique_temp_dir("mmux-mcp-list-sessions-project-filter-local");
        let server = test_coding_server(&dir, &local_dir, profile_registry(ready_profile())).await;
        let first_project = create_test_project(&server, "First Project").await;
        let second_project = create_test_project(&server, "Second Project").await;
        let first_task = create_test_task_in_project(&server, &first_project, "First Task").await;
        let second_task =
            create_test_task_in_project(&server, &second_project, "Second Task").await;

        for session in [
            "project-first-worker",
            "project-second-worker",
            "raw-unrecorded-worker",
        ] {
            test_create_session(&server, session, "sleep 30").await;
        }

        server
            .orchestration
            .record_session(
                first_task.id.clone(),
                recorded_session("project-first-worker"),
            )
            .unwrap();
        server
            .orchestration
            .record_session(
                second_task.id.clone(),
                recorded_session("project-second-worker"),
            )
            .unwrap();

        let first_result = server
            .list_sessions_tool(object_args(json!({
                "project_id": first_project.id.0
            })))
            .await
            .unwrap();
        let first_sessions: Vec<ProjectSessionListEntry> = result_json(&first_result);
        let first_names = first_sessions
            .iter()
            .map(|session| session.session.as_str())
            .collect::<Vec<_>>();
        assert_eq!(first_names, vec!["project-first-worker"]);
        assert_eq!(first_sessions[0].task_id, first_task.id);
        assert!(first_sessions
            .iter()
            .all(|session| session.runtime_state == "running"));

        let first_slug_result = server
            .list_sessions_tool(object_args(json!({
                "project_id": first_project.slug
            })))
            .await
            .unwrap();
        let first_slug_sessions: Vec<ProjectSessionListEntry> = result_json(&first_slug_result);
        assert_eq!(
            first_slug_sessions
                .iter()
                .map(|session| session.session.as_str())
                .collect::<Vec<_>>(),
            first_names
        );

        let second_result = server
            .list_sessions_tool(object_args(json!({
                "project_id": second_project.id.0
            })))
            .await
            .unwrap();
        let second_sessions: Vec<ProjectSessionListEntry> = result_json(&second_result);
        let second_names = second_sessions
            .iter()
            .map(|session| session.session.as_str())
            .collect::<Vec<_>>();
        assert_eq!(second_names, vec!["project-second-worker"]);
        assert_eq!(second_sessions[0].task_id, second_task.id);

        let admin_result = server
            .admin_list_node_sessions_tool(object_args(json!({})))
            .await
            .unwrap();
        let raw_sessions: Vec<SessionListEntry> = result_json(&admin_result);
        let raw_names = raw_sessions
            .iter()
            .map(|session| session.session.as_str())
            .collect::<std::collections::HashSet<_>>();
        assert!(raw_names.contains("raw-unrecorded-worker"));
        assert!(raw_names.contains("project-first-worker"));
        assert!(raw_names.contains("project-second-worker"));

        for session in [
            "project-first-worker",
            "project-second-worker",
            "raw-unrecorded-worker",
        ] {
            test_kill_session(&server, session).await;
        }
        let _ = fs::remove_dir_all(local_dir);
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn test_session_record_rejects_missing_local_task_session() {
        let dir = unique_temp_dir("mmux-mcp-session-record-local-missing");
        let local_dir = unique_temp_dir("mmux-mcp-session-record-local-missing-node");
        let server = test_coding_server(&dir, &local_dir, profile_registry(ready_profile())).await;
        let task = create_test_task(&server, "Missing Local").await;

        let error = call_session_record(
            &server,
            json!({
                "node_id": "local",
                "session": "missing-worker",
                "profile": "codex",
                "workspace_path": "/workspace/project",
                "bypass_permissions": false,
                "task_id": task.id.0,
                "role": "implementation-worker",
                "kind": "model-owner"
            }),
        )
        .await
        .unwrap_err();

        assert!(
            error
                .message
                .contains("session 'missing-worker' does not exist on node 'local'"),
            "{error}"
        );
        assert!(server
            .orchestration
            .snapshot()
            .unwrap()
            .tasks
            .get(&task.id)
            .unwrap()
            .session
            .is_none());
        let _ = fs::remove_dir_all(local_dir);
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn test_orchestration_wait_jobs_support_lifecycle_statuses_and_validation() {
        let dir = unique_temp_dir("mmux-mcp-wait-lifecycle");
        let local_dir = unique_temp_dir("mmux-mcp-wait-lifecycle-local");
        let server = test_coding_server(&dir, &local_dir, profile_registry(ready_profile())).await;
        test_create_session(&server, "wait-lifecycle", "sh -c 'printf READY; sleep 30'").await;

        for args in [
            json!({
                "node": "local",
                "session": "wait-lifecycle",
                "kind": "stable",
                "timeout_seconds": 2,
                "poll_seconds": 0.05,
                "stability_seconds": 0.05
            }),
            json!({
                "node": "local",
                "session": "wait-lifecycle",
                "kind": "sentinel",
                "sentinel": "READY",
                "timeout_seconds": 2,
                "poll_seconds": 0.05
            }),
            json!({
                "node": "local",
                "session": "wait-lifecycle",
                "kind": "prompt",
                "prompt": "READY",
                "timeout_seconds": 2,
                "poll_seconds": 0.05
            }),
            json!({
                "node": "local",
                "session": "wait-lifecycle",
                "kind": "coding-ready",
                "profile": "codex",
                "timeout_seconds": 2,
                "poll_seconds": 0.05
            }),
        ] {
            let result = server.wait_start_tool(object_args(args)).await.unwrap();
            let snapshot: RuntimeWaitSnapshot = result_json(&result);
            assert!(snapshot.wait_id.starts_with("wait-"));
            assert!(
                matches!(
                    snapshot.status,
                    RuntimeWaitStatus::Pending | RuntimeWaitStatus::Completed
                ),
                "{:?}",
                snapshot.status
            );
        }

        let complete_start = server
            .wait_start_tool(object_args(json!({
                "node": "local",
                "session": "wait-lifecycle",
                "kind": "sentinel",
                "sentinel": "READY",
                "timeout_seconds": 2,
                "poll_seconds": 0.05
            })))
            .await
            .unwrap();
        let complete: RuntimeWaitSnapshot = result_json(&complete_start);
        let complete =
            wait_for_runtime_status(&server, &complete.wait_id, RuntimeWaitStatus::Completed).await;
        assert_eq!(complete.status, RuntimeWaitStatus::Completed);
        assert!(complete.result.unwrap().message.contains("sentinel found"));

        let failed_start = server
            .wait_start_tool(object_args(json!({
                "node": "local",
                "session": "wait-lifecycle",
                "kind": "sentinel",
                "sentinel": "NEVER_MATCHES",
                "timeout_seconds": 0.1,
                "poll_seconds": 0.05
            })))
            .await
            .unwrap();
        let failed: RuntimeWaitSnapshot = result_json(&failed_start);
        let failed =
            wait_for_runtime_status(&server, &failed.wait_id, RuntimeWaitStatus::Failed).await;
        assert_eq!(failed.status, RuntimeWaitStatus::Failed);
        assert!(failed.error.unwrap().contains("timeout"));

        let missing_profile = server
            .wait_start_tool(object_args(json!({
                "node": "local",
                "session": "wait-lifecycle",
                "kind": "coding-ready",
                "timeout_seconds": 1
            })))
            .await
            .unwrap_err();
        assert!(
            missing_profile.message.contains("profile is required"),
            "{missing_profile}"
        );

        test_kill_session(&server, "wait-lifecycle").await;
        let _ = fs::remove_dir_all(local_dir);
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn test_orchestration_wait_jobs_support_distributed_sentinel_completion() {
        let dir = unique_temp_dir("mmux-mcp-wait-distributed-sentinel");
        let server =
            test_orchestration_server_with_profiles(&dir, profile_registry(ready_profile())).await;
        let node_id = "node-a";
        register_test_node(&server, node_id).await;

        let start = server.wait_start_tool(object_args(json!({
            "node": node_id,
            "session": "remote-wait",
            "kind": "sentinel",
            "sentinel": "READY",
            "timeout_seconds": 2,
            "poll_seconds": 0.05
        })));
        let node = async {
            let NodeCommand { command_id, kind } = pull_next_node_command(&server, node_id).await;
            let NodeCommandKind::Tmux { args } = kind else {
                panic!("expected tmux command");
            };
            assert_eq!(args, string_args(&["has-session", "-t", "remote-wait"]));
            submit_node_result(
                &server,
                node_id,
                command_id,
                NodeCommandResult::TmuxOutput(String::new()),
            )
            .await;

            let NodeCommand { command_id, kind } = pull_next_node_command(&server, node_id).await;
            let NodeCommandKind::Tmux { args } = kind else {
                panic!("expected tmux command");
            };
            assert_eq!(
                args,
                string_args(&["capture-pane", "-t", "remote-wait", "-p", "-S", "-200"])
            );
            submit_node_result(
                &server,
                node_id,
                command_id,
                NodeCommandResult::TmuxOutput("still booting".into()),
            )
            .await;

            let NodeCommand { command_id, kind } = pull_next_node_command(&server, node_id).await;
            let NodeCommandKind::Tmux { args } = kind else {
                panic!("expected tmux command");
            };
            assert_eq!(
                args,
                string_args(&["capture-pane", "-t", "remote-wait", "-p", "-S", "-200"])
            );
            submit_node_result(
                &server,
                node_id,
                command_id,
                NodeCommandResult::TmuxOutput("READY".into()),
            )
            .await;
        };

        let (result, _) = tokio::join!(start, node);
        let snapshot: RuntimeWaitSnapshot = result_json(&result.unwrap());
        assert_eq!(snapshot.node, node_id);
        assert!(
            matches!(
                snapshot.status,
                RuntimeWaitStatus::Pending | RuntimeWaitStatus::Completed
            ),
            "{:?}",
            snapshot.status
        );

        let complete =
            wait_for_runtime_status(&server, &snapshot.wait_id, RuntimeWaitStatus::Completed).await;
        assert_eq!(complete.status, RuntimeWaitStatus::Completed);
        assert!(complete.result.unwrap().message.contains("sentinel found"));

        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn test_orchestration_wait_start_rejects_missing_distributed_session_before_job_creation()
    {
        let dir = unique_temp_dir("mmux-mcp-wait-distributed-missing");
        let server =
            test_orchestration_server_with_profiles(&dir, profile_registry(ready_profile())).await;
        let node_id = "node-a";
        register_test_node(&server, node_id).await;

        let start = server.wait_start_tool(object_args(json!({
            "node": node_id,
            "session": "missing-remote",
            "kind": "sentinel",
            "sentinel": "READY",
            "timeout_seconds": 2,
            "poll_seconds": 0.05
        })));
        let node = async {
            let NodeCommand { command_id, kind } = pull_next_node_command(&server, node_id).await;
            let NodeCommandKind::Tmux { args } = kind else {
                panic!("expected tmux command");
            };
            assert_eq!(args, string_args(&["has-session", "-t", "missing-remote"]));
            submit_node_result(
                &server,
                node_id,
                command_id,
                NodeCommandResult::Error {
                    message: "tmux error: can't find session: missing-remote".into(),
                },
            )
            .await;
        };

        let (result, _) = tokio::join!(start, node);
        let result = result.unwrap();
        assert_eq!(result.is_error, Some(true));
        assert!(result_text(&result).contains("Session 'missing-remote' does not exist"));
        assert!(server.wait_jobs.lock().unwrap().is_empty());

        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn test_orchestration_wait_jobs_support_distributed_coding_ready_startup_dismiss() {
        let dir = unique_temp_dir("mmux-mcp-wait-distributed-dismiss");
        let server =
            test_orchestration_server_with_profiles(&dir, mmux_node::default_profiles()).await;
        let node_id = "node-a";
        register_test_node(&server, node_id).await;

        let start = server.wait_start_tool(object_args(json!({
            "node": node_id,
            "session": "remote-coder",
            "kind": "coding-ready",
            "profile": "codex",
            "timeout_seconds": 2,
            "poll_seconds": 0.05,
            "stability_seconds": 0.0
        })));
        let node = async {
            let NodeCommand { command_id, kind } = pull_next_node_command(&server, node_id).await;
            let NodeCommandKind::Tmux { args } = kind else {
                panic!("expected tmux command");
            };
            assert_eq!(args, string_args(&["has-session", "-t", "remote-coder"]));
            submit_node_result(
                &server,
                node_id,
                command_id,
                NodeCommandResult::TmuxOutput(String::new()),
            )
            .await;

            let NodeCommand { command_id, kind } = pull_next_node_command(&server, node_id).await;
            let NodeCommandKind::Tmux { args } = kind else {
                panic!("expected tmux command");
            };
            assert_eq!(
                args,
                string_args(&["capture-pane", "-t", "remote-coder", "-p", "-S", "-200"])
            );
            submit_node_result(
                &server,
                node_id,
                command_id,
                NodeCommandResult::TmuxOutput(
                    "✨ Update available!\n› 1. Update now\n  2. Skip".into(),
                ),
            )
            .await;

            let NodeCommand { command_id, kind } = pull_next_node_command(&server, node_id).await;
            let NodeCommandKind::Tmux { args } = kind else {
                panic!("expected tmux command");
            };
            assert_eq!(
                args,
                string_args(&["send-keys", "-t", "remote-coder", "Down", "Enter"])
            );
            submit_node_result(
                &server,
                node_id,
                command_id,
                NodeCommandResult::TmuxOutput(String::new()),
            )
            .await;

            let NodeCommand { command_id, kind } = pull_next_node_command(&server, node_id).await;
            let NodeCommandKind::Tmux { args } = kind else {
                panic!("expected tmux command");
            };
            assert_eq!(
                args,
                string_args(&["capture-pane", "-t", "remote-coder", "-p", "-S", "-200"])
            );
            submit_node_result(
                &server,
                node_id,
                command_id,
                NodeCommandResult::TmuxOutput("› Improve documentation in @filename".into()),
            )
            .await;
        };

        let (result, _) = tokio::join!(start, node);
        let snapshot: RuntimeWaitSnapshot = result_json(&result.unwrap());
        let complete =
            wait_for_runtime_status(&server, &snapshot.wait_id, RuntimeWaitStatus::Completed).await;
        assert_eq!(complete.status, RuntimeWaitStatus::Completed);
        assert!(complete
            .result
            .unwrap()
            .message
            .contains("remote-coder is ready"));

        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn test_orchestration_coding_ready_requires_stable_idle_state() {
        let dir = unique_temp_dir("mmux-mcp-wait-coding-ready-stable");
        let mut profile = ready_profile();
        profile.busy_indicators = vec!["• Working".into()];
        let server = test_orchestration_server_with_profiles(&dir, profile_registry(profile)).await;
        let node_id = "node-a";
        register_test_node(&server, node_id).await;

        let start = server.wait_start_tool(object_args(json!({
            "node": node_id,
            "session": "remote-coder",
            "kind": "coding-ready",
            "profile": "codex",
            "timeout_seconds": 2,
            "poll_seconds": 0.05,
            "stability_seconds": 0.1
        })));
        let node = async {
            let NodeCommand { command_id, kind } = pull_next_node_command(&server, node_id).await;
            let NodeCommandKind::Tmux { args } = kind else {
                panic!("expected tmux command");
            };
            assert_eq!(args, string_args(&["has-session", "-t", "remote-coder"]));
            submit_node_result(
                &server,
                node_id,
                command_id,
                NodeCommandResult::TmuxOutput(String::new()),
            )
            .await;

            for output in [
                "READY",
                "• Working (3s • esc to interrupt)\n\nREADY",
                "READY",
                "READY",
                "READY",
            ] {
                let NodeCommand { command_id, kind } =
                    pull_next_node_command(&server, node_id).await;
                let NodeCommandKind::Tmux { args } = kind else {
                    panic!("expected tmux command");
                };
                assert_eq!(
                    args,
                    string_args(&["capture-pane", "-t", "remote-coder", "-p", "-S", "-200"])
                );
                submit_node_result(
                    &server,
                    node_id,
                    command_id,
                    NodeCommandResult::TmuxOutput(output.into()),
                )
                .await;
            }
        };

        let (result, _) = tokio::join!(start, node);
        let snapshot: RuntimeWaitSnapshot = result_json(&result.unwrap());
        let complete =
            wait_for_runtime_status(&server, &snapshot.wait_id, RuntimeWaitStatus::Completed).await;
        assert_eq!(complete.status, RuntimeWaitStatus::Completed);
        assert!(complete
            .result
            .unwrap()
            .message
            .contains("remote-coder is ready"));

        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn test_orchestration_wait_cancel_keeps_quick_inspection_responsive() {
        let dir = unique_temp_dir("mmux-mcp-wait-responsive");
        let local_dir = unique_temp_dir("mmux-mcp-wait-responsive-local");
        let server = test_coding_server(&dir, &local_dir, profile_registry(ready_profile())).await;
        test_create_session(&server, "wait-responsive", "sh -c 'printf READY; sleep 30'").await;

        let start = server
            .wait_start_tool(object_args(json!({
                "node": "local",
                "session": "wait-responsive",
                "kind": "sentinel",
                "sentinel": "NEVER_MATCHES",
                "timeout_seconds": 10,
                "poll_seconds": 0.1
            })))
            .await
            .unwrap();
        let snapshot: RuntimeWaitSnapshot = result_json(&start);
        assert_eq!(snapshot.status, RuntimeWaitStatus::Pending);

        let capture = tokio::time::timeout(
            Duration::from_secs(2),
            server.node_session_capture("local", "wait-responsive", Some(40), false),
        )
        .await
        .expect("capture_output should not be blocked by pending wait")
        .unwrap();
        assert!(capture.contains("READY"));

        let state = tokio::time::timeout(Duration::from_secs(2), async {
            let buf = server
                .node_session_capture("local", "wait-responsive", None, false)
                .await
                .unwrap_or_default();
            let profile = ready_profile();
            Ok::<String, String>(
                json!({
                    "node": "local",
                    "session": "wait-responsive",
                    "has_prompt": buf.contains(&profile.prompt_indicator),
                    "busy": profile_is_busy(&buf, &profile),
                    "profile": profile.name,
                })
                .to_string(),
            )
        })
        .await
        .expect("check_state should not be blocked by pending wait")
        .unwrap();
        assert!(state.contains("\"has_prompt\":true"));

        let pending = server
            .wait_status_tool(object_args(json!({ "wait_id": snapshot.wait_id })))
            .unwrap();
        let pending: RuntimeWaitSnapshot = result_json(&pending);
        assert_eq!(pending.status, RuntimeWaitStatus::Pending);

        let canceled = server
            .wait_cancel_tool(object_args(json!({ "wait_id": pending.wait_id })))
            .unwrap();
        let canceled: RuntimeWaitSnapshot = result_json(&canceled);
        assert_eq!(canceled.status, RuntimeWaitStatus::Canceled);

        assert!(test_session_exists(&server, "wait-responsive").await);

        test_kill_session(&server, "wait-responsive").await;
        let _ = fs::remove_dir_all(local_dir);
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn test_session_record_rejects_unknown_remote_node() {
        let dir = unique_temp_dir("mmux-mcp-session-record-remote-node");
        let server =
            test_orchestration_server_with_profiles(&dir, profile_registry(ready_profile())).await;
        let task = create_test_task(&server, "Missing Remote Node").await;

        let error = call_session_record(
            &server,
            json!({
                "node_id": "missing-node",
                "session": "worker-a",
                "profile": "codex",
                "workspace_path": "/workspace/project",
                "bypass_permissions": false,
                "task_id": task.id.0,
                "role": "implementation-worker",
                "kind": "model-owner"
            }),
        )
        .await
        .unwrap_err();

        assert!(error.message.contains("missing-node"), "{error}");
        assert!(error.message.contains("unreachable"), "{error}");
        assert!(server
            .orchestration
            .snapshot()
            .unwrap()
            .tasks
            .get(&task.id)
            .unwrap()
            .session
            .is_none());
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn test_session_record_rejects_missing_remote_task_session() {
        let dir = unique_temp_dir("mmux-mcp-session-record-remote-missing");
        let server =
            test_orchestration_server_with_profiles(&dir, profile_registry(ready_profile())).await;
        let node_id = "node-a";
        register_test_node(&server, node_id).await;
        let task = create_test_task(&server, "Missing Remote Session").await;

        let record = call_session_record(
            &server,
            json!({
                "node_id": node_id,
                "session": "worker-a",
                "profile": "codex",
                "workspace_path": "/workspace/project",
                "bypass_permissions": false,
                "task_id": task.id.0,
                "role": "implementation-worker",
                "kind": "model-owner"
            }),
        );
        let node = async {
            let NodeCommand { command_id, kind } = pull_next_node_command(&server, node_id).await;
            let NodeCommandKind::Tmux { args } = kind else {
                panic!("expected tmux command");
            };
            assert_eq!(args, string_args(&["has-session", "-t", "worker-a"]));
            submit_node_result(
                &server,
                node_id,
                command_id,
                NodeCommandResult::Error {
                    message: "tmux error: can't find session: worker-a".into(),
                },
            )
            .await;
        };

        let (result, _) = tokio::join!(record, node);
        let error = result.unwrap_err();

        assert!(
            error
                .message
                .contains("session 'worker-a' does not exist on node 'node-a'"),
            "{error}"
        );
        assert!(server
            .orchestration
            .snapshot()
            .unwrap()
            .tasks
            .get(&task.id)
            .unwrap()
            .session
            .is_none());
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn test_session_record_rejects_unknown_profile_for_task_participant() {
        let dir = unique_temp_dir("mmux-mcp-session-record-profile");
        let server = test_orchestration_server(&dir).await;
        let task = create_test_task(&server, "Unknown Profile").await;

        let error = call_session_record(
            &server,
            json!({
                "node_id": "node-a",
                "session": "worker-a",
                "profile": "missing",
                "workspace_path": "/workspace/project",
                "bypass_permissions": false,
                "task_id": task.id.0,
                "role": "implementation-worker",
                "kind": "model-owner"
            }),
        )
        .await
        .unwrap_err();

        assert!(error.message.contains("unknown profile"));
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn test_task_aware_start_records_actual_runtime_choices() {
        let dir = unique_temp_dir("mmux-mcp-start-record");
        let local_dir = unique_temp_dir("mmux-mcp-start-record-local");
        let workspace_root = unique_temp_dir("mmux-mcp-start-record-workspace");
        let workspace_real = workspace_root.join("actual");
        fs::create_dir_all(&workspace_real).unwrap();
        let workspace_path = workspace_real
            .join("..")
            .join("actual")
            .to_string_lossy()
            .into_owned();
        let server = test_coding_server(&dir, &local_dir, profile_registry(ready_profile())).await;
        let task = create_test_task(&server, "Orchestration Core Model").await;

        let result = server
            .start_coding_session_tool(object_args(json!({
                "profile": "codex",
                "node": "local",
                "workspace_path": workspace_path.clone(),
                "bypass_permissions": true,
                "task_id": task.id.0,
                "role": "implementation-worker",
                "kind": "Model Owner",
                "skills": ["rust", "mcp"],
                "generate_session_name": true
            })))
            .await
            .unwrap();
        let payload: Value = result_json(&result);
        let record: TaskSession =
            serde_json::from_value(payload["session_record"].clone()).unwrap();

        assert_eq!(payload["readiness"]["status"], "not_waited");
        assert_eq!(payload["readiness"]["next_tool"], "wait_start");
        assert_eq!(payload["session"], record.session.0);
        assert!(record
            .session
            .0
            .starts_with(&format!("mmux-{}-model-owner-", task.slug)));
        assert_eq!(record.node_id.0, "local");
        assert_eq!(record.profile, "codex");
        assert_eq!(record.workspace_path, workspace_path);
        assert!(record.bypass_permissions);
        assert_eq!(record.role, "implementation-worker");
        assert_eq!(record.kind, "Model Owner");
        assert_eq!(record.skills, vec!["rust", "mcp"]);

        test_kill_session(&server, &record.session.0).await;
        let _ = fs::remove_dir_all(workspace_root);
        let _ = fs::remove_dir_all(local_dir);
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn test_task_aware_start_replaces_task_session_and_stops_old_session() {
        let dir = unique_temp_dir("mmux-mcp-start-replace");
        let local_dir = unique_temp_dir("mmux-mcp-start-replace-local");
        let workspace_root = unique_temp_dir("mmux-mcp-start-replace-workspace");
        fs::create_dir_all(&workspace_root).unwrap();
        let workspace_path = workspace_root.to_string_lossy().into_owned();
        let server = test_coding_server(&dir, &local_dir, profile_registry(ready_profile())).await;
        let task = create_test_task(&server, "Replace Start").await;
        test_create_session(&server, "start-old", "sleep 30").await;
        call_session_record(
            &server,
            json!({
                "node_id": "local",
                "session": "start-old",
                "profile": "codex",
                "workspace_path": workspace_path.clone(),
                "bypass_permissions": false,
                "task_id": task.id.0,
                "role": "implementation-worker",
                "kind": "coder",
                "skills": ["rust"]
            }),
        )
        .await
        .unwrap();

        let result = server
            .start_coding_session_tool(object_args(json!({
                "profile": "codex",
                "node": "local",
                "session": "start-new",
                "workspace_path": workspace_path.clone(),
                "bypass_permissions": false,
                "task_id": task.id.0,
                "role": "implementation-worker",
                "kind": "coder",
                "skills": ["rust"]
            })))
            .await
            .unwrap();
        let payload: Value = result_json(&result);
        let record: TaskSession =
            serde_json::from_value(payload["session_record"].clone()).unwrap();

        assert_eq!(record.session.0, "start-new");
        assert!(!test_session_exists(&server, "start-old").await);
        assert!(test_session_exists(&server, "start-new").await);
        test_kill_session(&server, "start-new").await;
        let _ = fs::remove_dir_all(workspace_root);
        let _ = fs::remove_dir_all(local_dir);
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn test_task_aware_remote_start_passes_backend_owned_workspace_path_exactly() {
        let dir = unique_temp_dir("mmux-mcp-remote-start-workspace");
        let server =
            test_orchestration_server_with_profiles(&dir, profile_registry(ready_profile())).await;
        let node_id = "sandbox-a";
        register_test_node(&server, node_id).await;
        let task = create_test_task(&server, "Remote Workspace").await;
        let backend_workspace = format!(
            "/__mmux_backend_only__/{}",
            unique_temp_dir("remote-workspace")
                .file_name()
                .unwrap()
                .to_string_lossy()
        );
        assert!(!Path::new(&backend_workspace).exists());

        let start = server.start_coding_session_tool(object_args(json!({
            "profile": "codex",
            "node": node_id,
            "session": "remote-worker",
            "workspace_path": backend_workspace.clone(),
            "bypass_permissions": false,
            "task_id": task.id.0,
            "role": "implementation-worker",
            "kind": "codex",
            "skills": ["rust"]
        })));
        let node = async {
            let NodeCommand { command_id, kind } = pull_next_node_command(&server, node_id).await;
            let NodeCommandKind::Tmux { args } = kind else {
                panic!("expected tmux command");
            };
            assert_eq!(args, string_args(&["has-session", "-t", "remote-worker"]));
            submit_node_result(
                &server,
                node_id,
                command_id,
                NodeCommandResult::Error {
                    message: "missing".into(),
                },
            )
            .await;

            let NodeCommand { command_id, kind } = pull_next_node_command(&server, node_id).await;
            let NodeCommandKind::Tmux { args } = kind else {
                panic!("expected tmux command");
            };
            assert_eq!(
                args,
                vec![
                    "new-session".to_owned(),
                    "-d".to_owned(),
                    "-s".to_owned(),
                    "remote-worker".to_owned(),
                    "-c".to_owned(),
                    backend_workspace.clone(),
                    "sh -c 'printf READY; sleep 30'".to_owned()
                ]
            );
            submit_node_result(
                &server,
                node_id,
                command_id,
                NodeCommandResult::TmuxOutput(String::new()),
            )
            .await;
        };

        let (result, _) = tokio::join!(start, node);
        let payload: Value = result_json(&result.unwrap());
        let record: TaskSession =
            serde_json::from_value(payload["session_record"].clone()).unwrap();
        assert_eq!(payload["readiness"]["status"], "not_waited");
        assert_eq!(record.node_id.0, node_id);
        assert_eq!(record.session.0, "remote-worker");
        assert_eq!(record.workspace_path, backend_workspace);

        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn test_orchestration_cleanup_zombies_tool_dry_run_and_explicit_cleanup() {
        let dir = unique_temp_dir("mmux-mcp-cleanup-zombies");
        let local_dir = unique_temp_dir("mmux-mcp-cleanup-zombies-local");
        let server = test_coding_server(&dir, &local_dir, profile_registry(ready_profile())).await;
        let task = create_test_task(&server, "Cleanup").await;

        for session in [
            "mmux-zombie-cleanup",
            "mmux-recorded-cleanup",
            "user-cleanup",
        ] {
            test_create_session(&server, session, "sleep 30").await;
        }
        server
            .orchestration
            .record_session(task.id.clone(), recorded_session("mmux-recorded-cleanup"))
            .unwrap();

        let dry_run: OrchestrationCleanupZombiesResult = result_json(
            &server
                .orchestration_cleanup_zombies_tool(object_args(json!({})))
                .await
                .unwrap(),
        );

        assert!(dry_run.dry_run);
        assert_eq!(
            dry_run
                .candidates
                .iter()
                .map(|candidate| candidate.session.as_str())
                .collect::<Vec<_>>(),
            vec!["mmux-zombie-cleanup"]
        );
        server
            .startup_warnings
            .lock()
            .unwrap()
            .push("live local session 'mmux-zombie-cleanup' is a zombie cleanup candidate".into());
        let status_before_cleanup: OrchestrationStatus = result_json(
            &server
                .orchestration_status_tool_async(object_args(json!({})))
                .await
                .unwrap(),
        );
        assert_eq!(status_before_cleanup.cleanup_candidates.len(), 1);
        assert!(status_before_cleanup.warnings.iter().any(|warning| warning
            == "live local session 'mmux-zombie-cleanup' is a zombie cleanup candidate"));
        for session in [
            "mmux-zombie-cleanup",
            "mmux-recorded-cleanup",
            "user-cleanup",
        ] {
            assert!(
                test_session_exists(&server, session).await,
                "{session} should survive dry run"
            );
        }

        let cleanup: OrchestrationCleanupZombiesResult = result_json(
            &server
                .orchestration_cleanup_zombies_tool(object_args(json!({
                    "dry_run": false
                })))
                .await
                .unwrap(),
        );

        assert_eq!(cleanup.killed, vec!["mmux-zombie-cleanup"]);
        assert!(!test_session_exists(&server, "mmux-zombie-cleanup").await);
        let status_after_cleanup: OrchestrationStatus = result_json(
            &server
                .orchestration_status_tool_async(object_args(json!({})))
                .await
                .unwrap(),
        );
        assert!(status_after_cleanup.cleanup_candidates.is_empty());
        assert!(!status_after_cleanup
            .warnings
            .iter()
            .any(|warning| warning.contains("mmux-zombie-cleanup")));
        for session in ["mmux-recorded-cleanup", "user-cleanup"] {
            assert!(
                test_session_exists(&server, session).await,
                "{session} should not be killed"
            );
            test_kill_session(&server, session).await;
        }
        let _ = fs::remove_dir_all(local_dir);
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn test_startup_reconciliation_recreates_missing_active_recorded_session() {
        let dir = unique_temp_dir("mmux-mcp-reconcile");
        let local_dir = unique_temp_dir("mmux-mcp-reconcile-local");
        let workspace_root = unique_temp_dir("mmux-mcp-reconcile-workspace");
        let workspace_real = workspace_root.join("actual");
        fs::create_dir_all(&workspace_real).unwrap();
        let workspace_path = workspace_real
            .join("..")
            .join("actual")
            .to_string_lossy()
            .into_owned();
        let server = test_coding_server(&dir, &local_dir, profile_registry(ready_profile())).await;
        let task = create_test_task(&server, "Reconcile").await;
        call_orchestration(
            &server,
            "task_status_update",
            json!({
                "task_id": task.id.0,
                "status": "Running"
            }),
        )
        .unwrap();
        let mut record = recorded_session("mmux-reconcile-active");
        record.workspace_path = workspace_path.clone();
        server
            .orchestration
            .record_session(task.id.clone(), record)
            .unwrap();

        server.reconcile_startup_local_sessions().await;

        assert!(test_session_exists(&server, "mmux-reconcile-active").await);
        assert!(server
            .startup_warnings
            .lock()
            .unwrap()
            .iter()
            .any(|warning| warning.contains("recreated stored active session")));
        assert_eq!(
            server
                .orchestration
                .snapshot()
                .unwrap()
                .tasks
                .get(&task.id)
                .unwrap()
                .session
                .as_ref()
                .unwrap()
                .workspace_path
                .as_str(),
            workspace_path.as_str()
        );

        test_kill_session(&server, "mmux-reconcile-active").await;
        let _ = fs::remove_dir_all(workspace_root);
        let _ = fs::remove_dir_all(local_dir);
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn test_task_aware_start_requires_workspace_path() {
        let dir = unique_temp_dir("mmux-mcp-requires-workspace");
        let server =
            test_orchestration_server_with_profiles(&dir, profile_registry(ready_profile())).await;
        let project = ensure_test_project(&server).await;
        let task = create_test_task_in_project(&server, &project, "Workspace Required").await;

        let error = server
            .start_coding_session_tool(object_args(json!({
                "profile": "codex",
                "node": "local",
                "bypass_permissions": false,
                "task_id": task.id.0,
                "role": "implementation-worker",
                "kind": "model-owner",
                "generate_session_name": true
            })))
            .await
            .unwrap_err();

        assert!(error.message.contains("explicit workspace_path"), "{error}");
        assert!(server
            .orchestration
            .snapshot()
            .unwrap()
            .tasks
            .get(&task.id)
            .unwrap()
            .session
            .is_none());
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn test_start_coding_session_without_task_fields_returns_nonblocking_metadata() {
        let dir = unique_temp_dir("mmux-mcp-start");
        let local_dir = unique_temp_dir("mmux-mcp-start-local");
        let workspace = unique_temp_dir("mmux-mcp-start-workspace");
        fs::create_dir_all(&workspace).unwrap();
        let server = test_coding_server(&dir, &local_dir, profile_registry(ready_profile())).await;

        let result = server
            .start_coding_session_tool(object_args(json!({
                "profile": "codex",
                "session": "plain-start",
                "node": "local",
                "workspace_path": workspace.to_string_lossy()
            })))
            .await
            .unwrap();

        let payload: Value = result_json(&result);
        assert_eq!(payload["session"], "plain-start");
        assert_eq!(payload["profile"], "codex");
        assert_eq!(payload["readiness"]["status"], "not_waited");
        assert_eq!(payload["readiness"]["next_tool"], "wait_start");
        assert!(payload["session_record"].is_null());
        assert!(server
            .orchestration
            .snapshot()
            .unwrap()
            .tasks
            .values()
            .all(|task| task.session.is_none()));
        test_kill_session(&server, "plain-start").await;
        let _ = fs::remove_dir_all(workspace);
        let _ = fs::remove_dir_all(local_dir);
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn test_start_coding_session_rejects_readiness_timeout_argument() {
        let dir = unique_temp_dir("mmux-mcp-start-timeout-reject");
        let local_dir = unique_temp_dir("mmux-mcp-start-timeout-reject-local");
        let workspace = unique_temp_dir("mmux-mcp-start-timeout-reject-workspace");
        fs::create_dir_all(&workspace).unwrap();
        let server = test_coding_server(&dir, &local_dir, profile_registry(ready_profile())).await;

        let error = server
            .start_coding_session_tool(object_args(json!({
                "profile": "codex",
                "session": "timeout-start",
                "node": "local",
                "workspace_path": workspace.to_string_lossy(),
                "timeout_seconds": 2
            })))
            .await
            .unwrap_err();

        assert!(error.message.contains("use wait_start"), "{error}");
        assert!(!test_session_exists(&server, "timeout-start").await);
        let _ = fs::remove_dir_all(workspace);
        let _ = fs::remove_dir_all(local_dir);
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn test_session_record_persists_round_trip() {
        let dir = unique_temp_dir("mmux-mcp-session-record-roundtrip");
        let local_dir = unique_temp_dir("mmux-mcp-session-record-roundtrip-local");
        let server = test_coding_server(&dir, &local_dir, profile_registry(ready_profile())).await;
        let task = create_test_task(&server, "Round Trip").await;
        let backend_workspace = format!(
            "/__mmux_backend_only__/{}",
            unique_temp_dir("session-record-workspace")
                .file_name()
                .unwrap()
                .to_string_lossy()
        );
        assert!(!Path::new(&backend_workspace).exists());
        test_create_session(&server, "roundtrip-worker", "sleep 30").await;

        call_session_record(
            &server,
            json!({
                "node_id": "local",
                "session": "roundtrip-worker",
                "profile": "codex",
                "workspace_path": backend_workspace.clone(),
                "bypass_permissions": false,
                "task_id": task.id.0,
                "role": "implementation-worker",
                "kind": "validator",
                "skills": ["rust"]
            }),
        )
        .await
        .unwrap();

        let reloaded = orchestration_actor::OrchestrationHandle::open(Some(&dir)).unwrap();
        let state = reloaded.snapshot().unwrap();
        let record = state
            .tasks
            .get(&task.id)
            .and_then(|task| task.session.as_ref())
            .expect("persisted task session");
        assert_eq!(record.profile, "codex");
        assert_eq!(record.kind, "validator");
        assert_eq!(record.workspace_path, backend_workspace);
        test_kill_session(&server, "roundtrip-worker").await;
        let _ = fs::remove_dir_all(local_dir);
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn test_task_edge_add_rejects_cyclic_dependency() {
        let dir = unique_temp_dir("mmux-mcp-edge-cycle");
        let server = test_orchestration_server(&dir).await;
        let project = ensure_test_project(&server).await;
        let plan = create_test_plan_in_project(&server, &project, "Edge Cycle Plan").await;
        let a = create_test_task_in_plan(&server, &plan, "A").await;
        let b = create_test_task_in_plan(&server, &plan, "B").await;

        let edge: TaskEdge = result_json(
            &call_orchestration(
                &server,
                "task_edge_add",
                json!({
                    "from_task_id": a.id.0,
                    "to_task_id": b.id.0,
                    "kind": "DependsOn"
                }),
            )
            .unwrap(),
        );
        assert_eq!(edge.kind, TaskEdgeKind::DependsOn);

        let error = call_orchestration(
            &server,
            "task_edge_add",
            json!({
                "from_task_id": b.id.0,
                "to_task_id": a.id.0,
                "kind": "DependsOn"
            }),
        )
        .unwrap_err();
        assert!(error.message.contains("cycle"), "{error}");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_coding_task_prompt_builds_context_by_id() {
        let mut state = OrchestrationState::new();
        let project = state
            .create_project(
                CreateProject {
                    title: "MMUX".into(),
                    description: "Test project".into(),
                    slug: None,
                },
                100,
            )
            .unwrap();
        let plan = create_state_plan(&mut state, &project);
        let dependency = state
            .create_task(
                CreateTask {
                    plan_id: plan.id.clone(),
                    title: "Dependency".into(),
                    objective: "Finish dependency".into(),
                    scope: TaskScope::default(),
                    gates: Vec::new(),
                    slug: None,
                },
                110,
            )
            .unwrap();
        let task = state
            .create_task(
                CreateTask {
                    plan_id: plan.id.clone(),
                    title: "Prompt Context".into(),
                    objective: "Build deterministic context".into(),
                    scope: TaskScope {
                        include_paths: vec!["crates/mmux-controller/src/lib.rs".into()],
                        exclude_paths: vec!["target".into()],
                        notes: Some("Controller-only change.".into()),
                    },
                    gates: vec!["Context includes task identity".into()],
                    slug: None,
                },
                120,
            )
            .unwrap();
        state
            .add_task_edge(
                CreateTaskEdge {
                    from: task.id.clone(),
                    to: dependency.id.clone(),
                    kind: TaskEdgeKind::DependsOn,
                    note: None,
                },
                130,
            )
            .unwrap();

        let mut args = coding_task_send_args(task.id.0.clone(), "Implement now.");
        args.extra_context = Some("Use focused tests.".into());
        let prompt = build_coding_task_prompt(&state, &args).unwrap();

        assert!(prompt.contains("Task: task-2"));
        assert!(prompt.contains("Slug: prompt-context"));
        assert!(prompt.contains(&format!("Project: {} / mmux", project.id.0)));
        assert!(prompt.contains(&format!("Plan: {} / plan", plan.id.0)));
        assert!(prompt.contains("Plan Brief:\nDetailed plan brief for test task derivation."));
        assert!(prompt.contains("Objective:\nBuild deterministic context"));
        assert!(prompt.contains("Include paths:\n- crates/mmux-controller/src/lib.rs"));
        assert!(prompt.contains("Gates:\n- Context includes task identity"));
        assert!(prompt.contains("Depends on:\n- task-1 / dependency / Dependency [Backlog]"));
        assert!(prompt.contains("Extra Context:\nUse focused tests."));
        assert!(prompt.contains("Instruction:\nImplement now."));
    }

    #[test]
    fn test_coding_task_prompt_resolves_slug_and_honors_include_flags() {
        let mut state = OrchestrationState::new();
        let project = state
            .create_project(
                CreateProject {
                    title: "MMUX".into(),
                    description: "Test project".into(),
                    slug: None,
                },
                100,
            )
            .unwrap();
        let plan = create_state_plan(&mut state, &project);
        let task = state
            .create_task(
                CreateTask {
                    plan_id: plan.id,
                    title: "Prompt Context".into(),
                    objective: "Build deterministic context".into(),
                    scope: TaskScope {
                        include_paths: vec!["README.md".into()],
                        exclude_paths: Vec::new(),
                        notes: None,
                    },
                    gates: vec!["Review docs".into()],
                    slug: None,
                },
                120,
            )
            .unwrap();

        let mut args = coding_task_send_args(task.slug, "Implement now.");
        args.include_dependencies = Some(false);
        args.include_gates = Some(false);
        args.include_scope = Some(false);
        let prompt = build_coding_task_prompt(&state, &args).unwrap();

        assert!(prompt.contains("Task: task-1"));
        assert!(!prompt.contains("Scope:"));
        assert!(!prompt.contains("Gates:"));
        assert!(!prompt.contains("Dependencies:"));
    }

    #[test]
    fn test_coding_task_prompt_supports_validate_review_and_quality_guard_templates() {
        let mut state = OrchestrationState::new();
        let project = state
            .create_project(
                CreateProject {
                    title: "MMUX".into(),
                    description: "Test project".into(),
                    slug: None,
                },
                100,
            )
            .unwrap();
        let plan = create_state_plan(&mut state, &project);
        let task = state
            .create_task(
                CreateTask {
                    plan_id: plan.id,
                    title: "Prompt Context".into(),
                    objective: "Build deterministic context".into(),
                    scope: TaskScope::default(),
                    gates: vec!["Review docs".into()],
                    slug: None,
                },
                120,
            )
            .unwrap();

        let mut validate_args = coding_task_send_args(task.id.0.clone(), "Check every gate.");
        validate_args.template = Some(CodingTaskSendTemplate::Validate);
        let validate_prompt = build_coding_task_prompt(&state, &validate_args).unwrap();
        assert!(validate_prompt.starts_with("Validation Context"));
        assert!(validate_prompt.contains("Validation Rules:"));
        assert!(validate_prompt.contains("Validation Instruction:\nCheck every gate."));

        let mut review_args = coding_task_send_args(task.id.0.clone(), "Review for regressions.");
        review_args.template = Some(CodingTaskSendTemplate::Review);
        let review_prompt = build_coding_task_prompt(&state, &review_args).unwrap();
        assert!(review_prompt.starts_with("Review Context"));
        assert!(review_prompt.contains("Review Rules:"));
        assert!(review_prompt.contains("Review Instruction:\nReview for regressions."));

        let mut quality_guard_args =
            coding_task_send_args(task.id.0, "Check for hidden runtime assumptions.");
        quality_guard_args.template = Some(CodingTaskSendTemplate::QualityGuard);
        let quality_guard_prompt = build_coding_task_prompt(&state, &quality_guard_args).unwrap();
        assert!(quality_guard_prompt.starts_with("Quality Guard Context"));
        assert!(quality_guard_prompt.contains("Built-In Quality Heuristics:"));
        assert!(quality_guard_prompt.contains("operator_supplied_guard_point_results"));
        assert!(quality_guard_prompt
            .contains("Quality Guard Instruction:\nCheck for hidden runtime assumptions."));
    }

    #[test]
    fn test_coding_task_prompt_renders_context_task_cards_for_validation() {
        let mut state = OrchestrationState::new();
        let project = state
            .create_project(
                CreateProject {
                    title: "MMUX".into(),
                    description: "Test project".into(),
                    slug: None,
                },
                100,
            )
            .unwrap();
        let plan = create_state_plan(&mut state, &project);
        let prerequisite = state
            .create_task(
                CreateTask {
                    plan_id: plan.id.clone(),
                    title: "Prerequisite Evidence".into(),
                    objective: "Produce evidence for the validator.".into(),
                    scope: TaskScope {
                        include_paths: vec!["src/lib.rs".into()],
                        exclude_paths: vec!["target".into()],
                        notes: Some("Operator-card source.".into()),
                    },
                    gates: vec!["Unit tests pass".into(), "Outcome names evidence".into()],
                    slug: None,
                },
                110,
            )
            .unwrap();
        state
            .record_session(
                &prerequisite.id,
                recorded_session("mmux-prerequisite-worker"),
                120,
            )
            .unwrap();
        state
            .update_task_status(&prerequisite.id, TaskStatus::Passed, 130)
            .unwrap();
        state.tasks.get_mut(&prerequisite.id).unwrap().outcome =
            Some("Evidence: cargo test -p mmux-controller passed.".into());
        let validator = state
            .create_task(
                CreateTask {
                    plan_id: plan.id,
                    title: "Validate Evidence".into(),
                    objective: "Validate the prerequisite task card.".into(),
                    scope: TaskScope::default(),
                    gates: vec!["Prerequisite card was checked".into()],
                    slug: None,
                },
                140,
            )
            .unwrap();
        state
            .add_task_edge(
                CreateTaskEdge {
                    from: validator.id.clone(),
                    to: prerequisite.id.clone(),
                    kind: TaskEdgeKind::DependsOn,
                    note: Some("Validator consumes prerequisite evidence.".into()),
                },
                150,
            )
            .unwrap();

        let mut args = coding_task_send_args(validator.id.0, "Check the supplied cards.");
        args.template = Some(CodingTaskSendTemplate::Validate);
        args.context_task_ids = Some(vec![prerequisite.id.0.clone()]);
        let prompt = build_coding_task_prompt(&state, &args).unwrap();

        assert!(prompt.contains("Operator Task Card Bundle:"));
        assert!(prompt.contains("Field checklist for each card:"));
        assert!(prompt.contains("Task Card: task-1"));
        assert!(prompt.contains("Status: Passed"));
        assert!(prompt.contains("Objective:\nProduce evidence for the validator."));
        assert!(prompt.contains("Include paths:\n- src/lib.rs"));
        assert!(prompt.contains("Gates:\n- Unit tests pass\n- Outcome names evidence"));
        assert!(prompt.contains("Outcome:\nEvidence: cargo test -p mmux-controller passed."));
        assert!(prompt.contains("Outgoing edges:\n- none"));
        assert!(prompt.contains("Session:\n- node=local session=mmux-prerequisite-worker"));
        assert!(prompt.contains("field_coverage_table"));
        assert!(prompt.contains("do not infer prior task results from local files alone"));
    }

    #[test]
    fn test_coding_task_prompt_rejects_duplicate_context_task_cards() {
        let mut state = OrchestrationState::new();
        let project = state
            .create_project(
                CreateProject {
                    title: "MMUX".into(),
                    description: "Test project".into(),
                    slug: None,
                },
                100,
            )
            .unwrap();
        let plan = create_state_plan(&mut state, &project);
        let task = state
            .create_task(
                CreateTask {
                    plan_id: plan.id,
                    title: "Task".into(),
                    objective: "Do work.".into(),
                    scope: TaskScope::default(),
                    gates: Vec::new(),
                    slug: None,
                },
                110,
            )
            .unwrap();

        let mut args = coding_task_send_args(task.id.0.clone(), "Validate.");
        args.context_task_ids = Some(vec![task.id.0.clone(), task.slug]);
        let error = build_coding_task_prompt(&state, &args).unwrap_err();

        assert!(error.contains("duplicate task 'task-1'"), "{error}");
    }

    #[test]
    fn test_coding_task_send_rejects_unknown_template() {
        let error = parse_tool_args::<CodingTaskSendArgs>(
            "coding_task_send",
            object_args(json!({
                "task_id_or_slug": "task-1",
                "prompt": "Implement now.",
                "template": "unknown"
            })),
        )
        .unwrap_err();

        assert!(error.message.contains("unknown"), "{error}");
    }

    #[test]
    fn test_coding_task_send_omitted_profile_uses_server_default_path() {
        let args = parse_tool_args::<CodingTaskSendArgs>(
            "coding_task_send",
            object_args(json!({
                "session": "worker",
                "task_id_or_slug": "task-1",
                "prompt": "Implement now."
            })),
        )
        .unwrap();

        assert_eq!(args.profile, None);
    }

    #[test]
    fn test_coding_task_prompt_rejects_ambiguous_slug_and_bad_context() {
        let mut state = OrchestrationState::new();
        let first_project = state
            .create_project(
                CreateProject {
                    title: "First".into(),
                    description: "Test project".into(),
                    slug: None,
                },
                100,
            )
            .unwrap();
        let second_project = state
            .create_project(
                CreateProject {
                    title: "Second".into(),
                    description: "Test project".into(),
                    slug: None,
                },
                101,
            )
            .unwrap();
        for project in [&first_project, &second_project] {
            let plan = create_state_plan(&mut state, project);
            state
                .create_task(
                    CreateTask {
                        plan_id: plan.id,
                        title: "Same Slug".into(),
                        objective: "Create ambiguity".into(),
                        scope: TaskScope::default(),
                        gates: Vec::new(),
                        slug: None,
                    },
                    110,
                )
                .unwrap();
        }

        let error = build_coding_task_prompt(
            &state,
            &coding_task_send_args("same-slug", "Implement now."),
        )
        .unwrap_err();
        assert!(error.contains("ambiguous"), "{error}");

        let mut args = coding_task_send_args("task-1", "Implement now.");
        args.extra_context = Some("null".into());
        let error = build_coding_task_prompt(&state, &args).unwrap_err();
        assert!(error.contains("extra_context"), "{error}");
    }

    #[tokio::test]
    async fn test_orchestration_status_filters_tasks() {
        let dir = unique_temp_dir("mmux-mcp-status-filter");
        let server = test_orchestration_server(&dir).await;
        let active = create_test_task(&server, "Active").await;
        let completed = create_test_task(&server, "Completed").await;
        call_orchestration(
            &server,
            "task_status_update",
            json!({
                "task_id": completed.id.0,
                "status": "Delivered"
            }),
        )
        .unwrap();

        let status: OrchestrationStatus =
            result_json(&call_orchestration(&server, "orchestration_status", json!({})).unwrap());
        assert_eq!(status.tasks.len(), 1);
        assert_eq!(status.tasks[0].id, active.id);

        let status: OrchestrationStatus = result_json(
            &call_orchestration(
                &server,
                "orchestration_status",
                json!({ "include_completed": true }),
            )
            .unwrap(),
        );
        assert_eq!(status.tasks.len(), 2);

        let status: OrchestrationStatus = result_json(
            &call_orchestration(
                &server,
                "orchestration_status",
                json!({ "task_id": active.id.0 }),
            )
            .unwrap(),
        );
        assert_eq!(
            status.tasks,
            vec![TaskSummary {
                id: active.id,
                plan_id: active.plan_id,
                slug: active.slug,
                title: active.title,
                status: TaskStatus::Backlog,
                outcome: None,
                session: None,
                parent: None,
                child_count: 0,
                dependency_count: 0,
                blocked_by: Vec::new(),
                open_gate_count: 0,
                failed_gate_count: 0,
                blocker_count: 0,
                blockers: Vec::new(),
                updated_at_ms: active.updated_at_ms,
            }]
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn test_missing_task_selectors_return_mcp_errors() {
        let dir = unique_temp_dir("mmux-mcp-missing-task");
        let server =
            test_orchestration_server_with_profiles(&dir, profile_registry(ready_profile())).await;

        let error = call_orchestration(
            &server,
            "task_status_update",
            json!({
                "task_id": "task-missing",
                "status": "Running"
            }),
        )
        .unwrap_err();
        assert!(error.message.contains("task 'task-missing' not found"));

        let error = call_orchestration(
            &server,
            "orchestration_status",
            json!({ "task_id": "task-missing" }),
        )
        .unwrap_err();
        assert!(error.message.contains("task 'task-missing' not found"));
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn test_orchestration_status_update_requires_outcome_for_gated_passed_or_delivered() {
        let dir = unique_temp_dir("mmux-mcp-status-gates");
        let server = test_orchestration_server(&dir).await;
        let project = ensure_test_project(&server).await;
        let plan = create_test_plan_in_project(&server, &project, "Gated Plan").await;
        let result = call_orchestration(
            &server,
            "task_create",
            json!({
                "plan_id": plan.id.0,
                "title": "Gated",
                "objective": "Needs evidence",
                "gates": ["tests pass"]
            }),
        )
        .unwrap();
        let task: Task = result_json(&result);

        let running: Task = result_json(
            &call_orchestration(
                &server,
                "task_status_update",
                json!({
                    "task_id": task.id.0,
                    "status": "Running",
                    "outcome": "worker started"
                }),
            )
            .unwrap(),
        );
        assert_eq!(running.status, TaskStatus::Running);
        assert_eq!(running.outcome.as_deref(), Some("worker started"));

        let error = call_orchestration(
            &server,
            "task_status_update",
            json!({
                "task_id": task.id.0,
                "status": "Passed"
            }),
        )
        .unwrap_err();
        assert!(error.message.contains("outcome is required"));

        let result = call_orchestration(
            &server,
            "task_status_update",
            json!({
                "task_id": task.id.0,
                "status": "Passed",
                "outcome": "cargo test -p mmux-controller passed"
            }),
        )
        .unwrap();
        let task: Task = result_json(&result);
        assert_eq!(task.status, TaskStatus::Passed);
        assert_eq!(
            task.outcome.as_deref(),
            Some("cargo test -p mmux-controller passed")
        );
        let status: OrchestrationStatus = result_json(
            &call_orchestration(
                &server,
                "orchestration_status",
                json!({ "task_id": task.id.0 }),
            )
            .unwrap(),
        );
        assert_eq!(
            status.tasks[0].outcome.as_deref(),
            Some("cargo test -p mmux-controller passed")
        );
        assert!(status.tasks[0].blockers.is_empty());

        let error = call_orchestration(
            &server,
            "task_status_update",
            json!({
                "task_id": task.id.0,
                "status": "Delivered",
                "outcome": "   "
            }),
        )
        .unwrap_err();
        assert!(error.message.contains("outcome must not be empty"));
        let state = server.orchestration.snapshot().unwrap();
        let persisted = state.tasks.get(&task.id).unwrap();
        assert_eq!(persisted.status, TaskStatus::Passed);
        assert_eq!(
            persisted.outcome.as_deref(),
            Some("cargo test -p mmux-controller passed")
        );

        let result = call_orchestration(
            &server,
            "task_status_update",
            json!({
                "task_id": task.id.0,
                "status": "Delivered",
                "outcome": "review evidence accepted"
            }),
        )
        .unwrap();
        let delivered: Task = result_json(&result);
        assert_eq!(delivered.status, TaskStatus::Delivered);
        assert_eq!(
            delivered.outcome.as_deref(),
            Some("review evidence accepted")
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_profile_launch_command_requires_explicit_permission_bypass_cmd() {
        let profiles = mmux_node::default_profiles();
        let opencode = mmux_node::get_profile(&profiles, "opencode").unwrap();
        assert_eq!(
            profile_launch_command(&opencode, false).unwrap(),
            "opencode"
        );
        assert!(profile_launch_command(&opencode, true)
            .unwrap_err()
            .contains("does not define permission_bypass_cmd"));

        let codex = mmux_node::get_profile(&profiles, "codex").unwrap();
        assert_eq!(
            profile_launch_command(&codex, true).unwrap(),
            "codex --dangerously-bypass-approvals-and-sandbox"
        );
    }

    #[test]
    fn test_profile_launch_strategy_defaults_and_validates() {
        let mut profile = CliProfile {
            name: "codex".into(),
            ..CliProfile::default()
        };

        assert_eq!(profile_launch_strategy(&profile).unwrap(), "direct");

        profile.launch_strategy = Some("shell_send".into());
        assert_eq!(profile_launch_strategy(&profile).unwrap(), "shell_send");

        profile.launch_strategy = Some("unknown".into());
        assert!(profile_launch_strategy(&profile)
            .unwrap_err()
            .contains("unsupported launch_strategy"));
    }

    #[test]
    fn test_enabled_coder_profiles_filters_default_registry() {
        let mut cli = test_cli();
        cli.enabled_coder_profiles = Some("codex, claude".into());

        let resolved = resolve_coder_profiles(&cli).unwrap();
        let profiles = resolved.profiles;

        assert_eq!(profiles.len(), 2);
        assert!(profiles.contains_key("codex"));
        assert!(profiles.contains_key("claude"));
        assert!(!profiles.contains_key("opencode"));
        assert!(!profiles.contains_key("kimi"));
        assert_eq!(resolved.default_profile, "codex");
    }

    #[test]
    fn test_enabled_coder_profiles_rejects_bad_values() {
        let mut cli = test_cli();
        cli.enabled_coder_profiles = Some("codex,,claude".into());
        assert!(resolve_coder_profiles(&cli)
            .unwrap_err()
            .contains("empty profile name"));

        cli.enabled_coder_profiles = Some("codex,missing".into());
        assert!(resolve_coder_profiles(&cli)
            .unwrap_err()
            .contains("unknown coder profile 'missing'"));
    }

    #[test]
    fn test_default_coder_profile_flag_must_be_enabled() {
        let mut cli = test_cli();
        cli.enabled_coder_profiles = Some("codex,claude".into());
        cli.default_coder_profile = Some("claude".into());

        let resolved = resolve_coder_profiles(&cli).unwrap();
        assert_eq!(resolved.default_profile, "claude");

        cli.default_coder_profile = Some("opencode".into());
        assert!(resolve_coder_profiles(&cli)
            .unwrap_err()
            .contains("is not enabled"));
    }

    #[test]
    fn test_profile_default_uses_first_enabled_builtin_without_opencode_special_case() {
        let cli = test_cli();

        let resolved = resolve_coder_profiles(&cli).unwrap();

        assert_eq!(resolved.default_profile, "codex");
    }

    #[tokio::test]
    async fn test_profile_default_uses_first_enabled_when_opencode_disabled() {
        let dir = unique_temp_dir("mmux-profile-allowlist");
        let codex = mmux_node::get_profile(&mmux_node::default_profiles(), "codex").unwrap();
        let server = test_orchestration_server_with_profiles(&dir, profile_registry(codex)).await;

        assert_eq!(server.default_profile_name(), Some("codex"));
        assert_eq!(server.resolve_profile(None).unwrap().name, "codex");
        assert!(server.resolve_profile(Some("opencode")).is_none());
    }

    #[test]
    fn test_profile_text_mode_defaults_and_validates() {
        let mut profile = CliProfile {
            name: "codex".into(),
            ..CliProfile::default()
        };

        assert_eq!(profile_text_mode(&profile).unwrap(), "paste-buffer");

        profile.text_mode = "literal-keys".into();
        assert_eq!(profile_text_mode(&profile).unwrap(), "literal-keys");

        profile.text_mode = "unknown".into();
        assert!(profile_text_mode(&profile)
            .unwrap_err()
            .contains("unsupported text_mode"));
    }

    #[test]
    fn test_controller_policy_clamp_timeout_rejects_invalid_and_clamps() {
        let mut cli = test_cli();
        cli.max_timeout_seconds = 12.5;
        let policy = ControllerPolicy::new(&cli).unwrap();

        assert!(policy.clamp_timeout(0.0).is_err());
        assert!(policy.clamp_timeout(f64::NAN).is_err());
        assert_eq!(policy.clamp_timeout(5.0).unwrap(), 5.0);
        assert_eq!(policy.clamp_timeout(99.0).unwrap(), 12.5);
    }

    #[test]
    fn test_limit_capture_output_truncates_on_utf8_boundary() {
        let mut cli = test_cli();
        cli.max_capture_bytes = 5;
        let policy = ControllerPolicy::new(&cli).unwrap();

        let result = policy.limit_capture_output("prefixétail".into());

        assert!(result.starts_with("[mmux truncated capture to last 5 bytes]\n"));
        assert!(result.ends_with("tail"));
        assert!(std::str::from_utf8(result.as_bytes()).is_ok());
    }

    #[test]
    fn test_validate_remote_mcp_bind_auth_rejects_remote_without_auth_unless_allowed() {
        let bind: SocketAddr = "203.0.113.10:3000".parse().unwrap();

        assert!(validate_remote_mcp_bind_auth(bind, None, false).is_err());
        assert!(validate_remote_mcp_bind_auth(bind, None, true).is_ok());
        assert!(validate_remote_mcp_bind_auth(bind, Some(&"secret".to_owned()), false).is_ok());
    }

    #[test]
    fn test_resolve_node_wire_policy_uses_one_canonical_mode() {
        let mut cli = test_cli();
        cli.wire_token_env = format!("MMUX_TEST_WIRE_TOKEN_{}", std::process::id());
        std::env::remove_var(&cli.wire_token_env);

        assert!(resolve_node_wire_policy(&cli, false)
            .unwrap_err()
            .contains("node wire RPC requires"));

        let embedded_auth = resolve_node_wire_policy(&cli, true).unwrap();
        assert_eq!(embedded_auth.policy.mode, NodeWireAuthMode::Token);
        assert!(embedded_auth.token.is_none());

        cli.wire_token = Some("secret".into());
        let auth = resolve_node_wire_policy(&cli, false).unwrap();
        assert_eq!(auth.policy.mode, NodeWireAuthMode::Token);
        assert_eq!(auth.token.as_deref(), Some("secret"));

        cli.wire_mtls = true;
        assert!(resolve_node_wire_policy(&cli, false)
            .unwrap_err()
            .contains("mutually exclusive"));

        cli.wire_token = None;
        assert!(resolve_node_wire_policy(&cli, false)
            .unwrap_err()
            .contains("--tls-cert is required"));

        let cert_dir = unique_temp_dir("mmux-mtls-policy");
        fs::create_dir_all(&cert_dir).unwrap();
        let cert = cert_dir.join("controller.pem");
        let key = cert_dir.join("controller-key.pem");
        let ca = cert_dir.join("node-ca.pem");
        fs::write(&cert, "not parsed in policy resolver").unwrap();
        fs::write(&key, "not parsed in policy resolver").unwrap();
        fs::write(&ca, "not parsed in policy resolver").unwrap();
        cli.tls_cert = Some(cert.to_string_lossy().into_owned());
        cli.tls_key = Some(key.to_string_lossy().into_owned());
        cli.wire_client_ca = Some(ca.to_string_lossy().into_owned());
        let auth = resolve_node_wire_policy(&cli, false).unwrap();
        assert_eq!(auth.policy.mode, NodeWireAuthMode::Mtls);
        assert!(auth.token.is_none());
        assert!(auth.native_mtls.is_some());

        cli.wire_mtls = false;
        cli.tls_cert = None;
        cli.tls_key = None;
        cli.wire_client_ca = None;
        cli.allow_unauthenticated_node_wire = true;
        let auth = resolve_node_wire_policy(&cli, false).unwrap();
        assert_eq!(auth.policy.mode, NodeWireAuthMode::Unauthenticated);
        assert!(auth.token.is_none());
        let _ = fs::remove_dir_all(cert_dir);
    }

    #[test]
    fn test_allow_unauthenticated_flags_ignore_default_token_env() {
        let mut cli = test_cli();
        cli.mcp_token_env = format!("MMUX_TEST_MCP_TOKEN_{}", std::process::id());
        cli.wire_token_env = format!("MMUX_TEST_WIRE_TOKEN_{}", std::process::id());
        std::env::set_var(&cli.mcp_token_env, "mcp-secret");
        std::env::set_var(&cli.wire_token_env, "wire-secret");

        cli.allow_remote_without_mcp_token = true;
        assert_eq!(resolve_mcp_token_value(&cli).unwrap(), None);

        cli.allow_remote_without_mcp_token = false;
        assert_eq!(
            resolve_mcp_token_value(&cli).unwrap().as_deref(),
            Some("mcp-secret")
        );

        cli.allow_unauthenticated_node_wire = true;
        let auth = resolve_node_wire_policy(&cli, false).unwrap();
        assert_eq!(auth.policy.mode, NodeWireAuthMode::Unauthenticated);
        assert!(auth.token.is_none());

        std::env::remove_var(&cli.mcp_token_env);
        std::env::remove_var(&cli.wire_token_env);
    }

    #[test]
    fn test_unauthenticated_flags_reject_explicit_token_sources() {
        let mut cli = test_cli();

        cli.allow_remote_without_mcp_token = true;
        cli.mcp_token = Some("secret".into());
        assert!(resolve_mcp_token_value(&cli)
            .unwrap_err()
            .contains("mutually exclusive"));

        cli.mcp_token = None;
        cli.allow_remote_without_mcp_token = false;
        cli.allow_unauthenticated_node_wire = true;
        cli.wire_token = Some("secret".into());
        assert!(resolve_node_wire_policy(&cli, false)
            .unwrap_err()
            .contains("mutually exclusive"));
    }

    #[test]
    fn test_resolve_token_value_reads_configured_env_and_rejects_empty() {
        let mut cli = test_cli();
        cli.mcp_token_env = format!("MMUX_TEST_TOKEN_{}", std::process::id());
        std::env::set_var(&cli.mcp_token_env, "abc123");
        assert_eq!(
            resolve_token_value(
                "--mcp-token",
                cli.mcp_token.as_ref(),
                cli.mcp_token_file.as_ref(),
                &cli.mcp_token_env,
            )
            .unwrap(),
            Some("abc123".into())
        );

        std::env::set_var(&cli.mcp_token_env, "");
        let error = resolve_token_value(
            "--mcp-token",
            cli.mcp_token.as_ref(),
            cli.mcp_token_file.as_ref(),
            &cli.mcp_token_env,
        )
        .unwrap_err();
        assert!(error.contains("is set but empty"));
        std::env::remove_var(&cli.mcp_token_env);
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
    fn test_session_list_format_exposes_basic_runtime_fields() {
        assert!(SESSION_LIST_FORMAT.contains("#{session_name}"));
        assert!(SESSION_LIST_FORMAT.contains("#{session_windows}"));
        assert!(SESSION_LIST_FORMAT.contains("#{session_attached}"));
        assert!(SESSION_LIST_FORMAT.contains("#{session_created}"));
    }

    #[test]
    fn test_parse_session_list_returns_structured_entries() {
        let sessions = parse_session_list("local", "codex|1|0|1780500000\nempty|2|1|1780500010\n");
        assert_eq!(
            sessions,
            vec![
                SessionListEntry {
                    node: "local".into(),
                    session: "codex".into(),
                    windows: Some(1),
                    attached: Some(0),
                    created_at_seconds: Some(1780500000),
                },
                SessionListEntry {
                    node: "local".into(),
                    session: "empty".into(),
                    windows: Some(2),
                    attached: Some(1),
                    created_at_seconds: Some(1780500010),
                },
            ]
        );
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

    #[test]
    fn test_compact_coding_output_strips_codex_startup_chrome() {
        let profile = CliProfile {
            name: "codex".into(),
            prompt_indicator: "›".into(),
            ..CliProfile::default()
        };
        let output = r#"
╭───────────────────────────────────────╮
│ >_ OpenAI Codex (v0.137.0)            │
│ model:     loading   /model to change │
│ directory: /tmp/mmux-coder-smoke      │
╰───────────────────────────────────────╯

› Find and fix a bug in @filename
› Improve documentation in @filename
› Implement {feature}
› Summarize recent commits
  gpt-5.5 default fast · /tmp/mmux-coder-smoke
  Tip: Use /skills to list available skills or ask Codex to use one.
⚠ The cormiloDev MCP server is not logged in. Run `codex mcp login cormiloDev`.

› Reply exactly: MMUX_SMOKE_CODEX

⚠ The cormiloDev MCP server is not logged in. Run `codex mcp login cormiloDev`.
• Working (1m 14s • esc to interrupt)
"#;

        assert_eq!(
            compact_coding_output(output, &profile),
            "⚠ The cormiloDev MCP server is not logged in. Run `codex mcp login cormiloDev`.\n› Reply exactly: MMUX_SMOKE_CODEX"
        );
    }

    #[test]
    fn test_compact_coding_output_strips_kimi_update_prompt() {
        let profile = CliProfile {
            name: "kimi".into(),
            prompt_indicator: ">".into(),
            ..CliProfile::default()
        };
        let output = r#"
Kimi Code Update Available
Kimi Code has a newer release ready.
View changelog: https://moonshotai.github.io/kimi-code/en/release-notes/changelo
g.html
tmux extended-keys-format is xterm. Kimi Code works best with csi-u. Add
`set -g extended-keys-format csi-u` to ~/.tmux.conf and restart tmux.
Current  0.6.0
Target   0.10.0
Source   native
Command  curl -fsSL https://code.kimi.com/kimi-code/install.sh | bash
↑↓ choose · Enter confirm · Esc continue
 ❯ Install update now (0.10.0)
   Continue with current version
> Reply exactly: MMUX_SMOKE_KIMI
MMUX_SMOKE_KIMI
Kimi-k2.6 thinking  /tmp/mmux-coder-smoke                 shift+enter: newline
context: 5.7% (14.8k/262.1k)
"#;

        assert_eq!(
            compact_coding_output(output, &profile),
            "> Reply exactly: MMUX_SMOKE_KIMI\nMMUX_SMOKE_KIMI"
        );
    }

    #[test]
    fn test_compact_coding_output_strips_claude_dashboard() {
        let profile = CliProfile {
            name: "claude".into(),
            prompt_indicator: "❯".into(),
            ..CliProfile::default()
        };
        let output = r#"
╭─── Claude Code v2.1.165 ─────────────────────────────────────────────────────╮
│              Welcome back Flatmaptech!             │ started                 │
│  Opus 4.8 · Claude Pro · ilija@example.com's Organization                   │
▝▜█████▛▘  Opus 4.8 · Claude Pro
▘▘ ▝▝    /tmp/mmux-coder-smoke
Feature of the week: /loop — run a prompt or slash command on a recurring interval
interval
╰──────────────────────────────────────────────────────────────────────────────╯
────────────────────────────────────────────────────────────────────────────────
❯ Try "edit <filepath> to..."
? for shortcuts · ← for agents

❯ Reply exactly: MMUX_SMOKE_CLAUDE
● MMUX_SMOKE_CLAUDE
✻ Brewed for 1s
"#;

        assert_eq!(
            compact_coding_output(output, &profile),
            "❯ Reply exactly: MMUX_SMOKE_CLAUDE\n● MMUX_SMOKE_CLAUDE"
        );
    }

    #[test]
    fn test_compact_coding_output_strips_opencode_chrome() {
        let profile = CliProfile {
            name: "opencode".into(),
            prompt_indicator: "ctrl+p commands".into(),
            ..CliProfile::default()
        };
        let output = r#"
┃  Reply exactly: MMUX_SMOKE_OPENCODE
▣  Build · GLM-4.7
┃  Build · GLM-4.7 Z.AI
╹▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀
■■■■⬝⬝Insufficient balance or no resource package. Please recharge. [retrying
attempt #4]
"#;

        assert_eq!(
            compact_coding_output(output, &profile),
            "┃  Reply exactly: MMUX_SMOKE_OPENCODE\n■■■■⬝⬝Insufficient balance or no resource package. Please recharge. [retrying\nattempt #4]"
        );
    }

    #[test]
    fn test_profile_is_busy_scans_active_trailing_region_only() {
        let profile = CliProfile {
            name: "kimi".into(),
            prompt_indicator: ">".into(),
            busy_indicators: vec!["thinking".into()],
            ..CliProfile::default()
        };
        let stale_history = (0..40)
            .map(|i| {
                if i == 0 {
                    "old thinking text".to_owned()
                } else {
                    format!("old line {i}")
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        let idle_output = format!("{stale_history}\n╭────╮\n│ >  │\n╰────╯");

        assert!(!profile_is_busy(&idle_output, &profile));
    }

    #[test]
    fn test_profile_is_busy_detects_live_trailing_status() {
        let profile = CliProfile {
            name: "kimi".into(),
            prompt_indicator: ">".into(),
            busy_indicators: vec!["ctrl+c: cancel".into(), "ctrl-s to steer".into()],
            ..CliProfile::default()
        };
        let output = "old transcript\n╭────╮\n│ >  │\n╰────╯\nKimi-k2.6 thinking  ctrl+c: cancel";

        assert!(profile_is_busy(output, &profile));
    }

    #[test]
    fn test_profile_is_busy_uses_profile_specific_markers_for_common_clis() {
        for (name, marker) in [
            ("codex", "• Working"),
            ("opencode", "Processing"),
            ("kimi", "ctrl-s to steer"),
            ("claude", "Thinking"),
        ] {
            let profile = CliProfile {
                name: name.into(),
                prompt_indicator: ">".into(),
                busy_indicators: vec![marker.into()],
                ..CliProfile::default()
            };
            let output = format!("old output\n╭────╮\n│ >  │\n╰────╯\nstatus: {marker}");

            assert!(
                profile_is_busy(&output, &profile),
                "{name} marker not detected"
            );
        }
    }

    #[test]
    fn test_claude_bypass_confirmation_is_not_promptable() {
        let profile = CliProfile {
            name: "claude".into(),
            prompt_indicator: "❯".into(),
            busy_indicators: vec!["Thinking".into()],
            ..CliProfile::default()
        };
        let output = r#"
╭──────────────────────────────────────────────────────────────────────────────╮
│ Bypass Permissions                                                           │
│ This mode can modify files and run commands without asking first.            │
╰──────────────────────────────────────────────────────────────────────────────╯
❯ Yes, I accept
  No, go back
"#;

        assert!(profile_is_busy(output, &profile));
        assert!(!profile_has_prompt(output, &profile));
        assert!(!profile_turn_idle(output, &profile));
    }

    #[test]
    fn test_startup_dismiss_triggers_count_as_busy_and_return_key() {
        let profile = mmux_node::get_profile(&mmux_node::default_profiles(), "codex").unwrap();
        let output = "✨ Update available!\n› 1. Update now\n  2. Skip\nPress enter to continue";

        assert!(profile_is_busy(output, &profile));
        assert_eq!(
            startup_dismiss_key(output, &profile),
            Some("Down Enter".into())
        );
        assert_eq!(
            node_send_key_args("%1", "Down Enter"),
            vec!["send-keys", "-t", "%1", "Down", "Enter"]
        );
    }

    #[test]
    fn test_codex_trust_prompt_is_not_startup_dismissed() {
        let profile = mmux_node::get_profile(&mmux_node::default_profiles(), "codex").unwrap();
        let output = "\
> You are in /tmp

  Do you trust the contents of this directory? Working with untrusted contents
  comes with higher risk of prompt injection. Trusting the directory allows
  project-local config, hooks, and exec policies to load.

› 1. Yes, continue
  2. No, quit

  Press enter to continue";

        assert!(!profile_is_busy(output, &profile));
        assert_eq!(startup_dismiss_key(output, &profile), None);
    }

    #[test]
    fn test_stale_startup_dismiss_trigger_outside_active_region_is_ignored() {
        let profile = mmux_node::get_profile(&mmux_node::default_profiles(), "codex").unwrap();
        let old_banner =
            "✨ Update available!\n› 1. Update now\n  2. Skip\nPress enter to continue";
        let filler = (0..30)
            .map(|index| format!("old line {index}"))
            .collect::<Vec<_>>()
            .join("\n");
        let ready_prompt = "› Write tests for @filename\n\n  gpt-5.5 xhigh · /mnt/Radni/reqvire";
        let output = format!("{old_banner}\n{filler}\n{ready_prompt}");

        assert!(!profile_is_busy(&output, &profile));
        assert_eq!(startup_dismiss_key(&output, &profile), None);
    }

    #[test]
    fn test_stale_startup_dismiss_trigger_before_current_prompt_is_ignored() {
        let profile = mmux_node::get_profile(&mmux_node::default_profiles(), "codex").unwrap();
        let output = "\
  ✨ Update available! 0.135.0 -> 0.137.0
› 1. Update now
  2. Skip
  Press enter to continue

⚠ The cormiloDev MCP server is not logged in.
• Starting MCP servers (2/3): graphenedb_dev

› Improve documentation in @filename

  gpt-5.5 xhigh · /mnt/Radni/mmux";

        assert!(!profile_is_busy(output, &profile));
        assert_eq!(startup_dismiss_key(output, &profile), None);
    }

    #[test]
    fn test_busy_indicator_before_current_prompt_still_counts() {
        let profile = mmux_node::get_profile(&mmux_node::default_profiles(), "codex").unwrap();
        let output = "\
⚠ The cormiloDev MCP server is not logged in.
• Working (4m 12s • esc to interrupt)

› Implement {feature}

  gpt-5.5 xhigh · /mnt/Radni/reqvire";

        assert!(profile_is_busy(output, &profile));
        assert_eq!(startup_dismiss_key(output, &profile), None);
    }

    #[test]
    fn test_coding_prompt_submit_delay_scales_for_multiline_prompts() {
        assert_eq!(
            coding_prompt_submit_delay("ping"),
            Duration::from_millis(201)
        );
        assert!(coding_prompt_submit_delay("line 1\nline 2") > Duration::from_millis(200));
        assert_eq!(
            coding_prompt_submit_delay(&"x".repeat(20_000)),
            Duration::from_millis(2_000)
        );
    }

    #[test]
    fn test_coding_prompt_paste_uses_bracketed_paste_buffer() {
        assert_eq!(
            tmux_set_buffer_args("mmux-test", "-line 1\nline 2"),
            vec!["set-buffer", "-b", "mmux-test", "--", "-line 1\nline 2"]
        );
        assert_eq!(
            tmux_paste_buffer_args("%7", "mmux-test"),
            vec!["paste-buffer", "-d", "-p", "-b", "mmux-test", "-t", "%7"]
        );
        assert_eq!(
            tmux_submit_args("%7"),
            vec!["send-keys", "-t", "%7", "Enter"]
        );
    }

    #[test]
    fn test_coding_prompt_literal_keys_uses_real_submit_keys() {
        assert_eq!(
            tmux_literal_text_args("%7", "Fix `a` and \"b\"\nthen report"),
            vec![
                "send-keys",
                "-t",
                "%7",
                "-l",
                "--",
                "Fix `a` and \"b\"\nthen report"
            ]
        );
        assert_eq!(
            tmux_submit_keys_args("%7", "Enter"),
            vec!["send-keys", "-t", "%7", "Enter"]
        );
        assert_eq!(
            tmux_submit_keys_args("%7", "C-x Enter"),
            vec!["send-keys", "-t", "%7", "C-x", "Enter"]
        );
    }

    #[test]
    fn test_coding_send_rejects_empty_or_placeholder_prompts() {
        for prompt in ["", "   ", "null", " undefined "] {
            let error = validate_coding_prompt(prompt).unwrap_err();
            assert!(
                error.message.contains("prompt"),
                "expected prompt validation error for {prompt:?}, got {error}"
            );
        }
        validate_coding_prompt("Explain how null is represented in JSON.").unwrap();
    }
}
