use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use clap::Parser;
use connectrpc::client::{ClientConfig, HttpClient};
use http::header::{HeaderValue, AUTHORIZATION};
use mmux_shared::{CliProfile, ReadFileResult, SaveFileResult, StartupDismiss};
use mmux_wire::connect::mmux::wire::v1::MmuxNodeRegistryServiceClient;
use mmux_wire::{
    heartbeat_request_to_proto, pull_commands_request_to_proto, pull_commands_response_from_proto,
    register_node_request_to_proto, submit_command_result_request_to_proto, HeartbeatRequest,
    NodeCommand, NodeCommandKind, NodeCommandResult, NodeDescriptor, NodeStatus,
    PullCommandsRequest, RegisterNodeRequest, SubmitCommandResultRequest,
};
use std::collections::HashMap;
use std::ffi::OsString;
use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::process::Command;
use std::sync::{Arc, RwLock};
use std::time::Duration;

pub const DEFAULT_NODE_PROFILE_CONFIG_NAME: &str = "mmux.toml";

pub type ProfileRegistry = Arc<RwLock<HashMap<String, CliProfile>>>;

#[derive(Parser, Debug)]
#[command(name = "mmux node")]
#[command(about = "Run an mmux execution node that owns local tmux/filesystem access")]
pub struct NodeCli {
    #[arg(
        long,
        default_value = "local",
        help = "Node identifier advertised to the controller"
    )]
    pub node_id: String,
    #[arg(long, help = "Controller URL this node should register with")]
    pub controller_url: Option<String>,
    #[arg(long, help = "Human-readable node name advertised to the controller")]
    pub node_name: Option<String>,
    #[arg(long, help = "Bearer token for controller wire endpoints")]
    pub controller_token: Option<String>,
    #[arg(
        long,
        default_value_t = 500,
        help = "Milliseconds between command polls"
    )]
    pub poll_interval_ms: u64,
    #[arg(long, help = "Path to node profile TOML file")]
    pub node_config: Option<String>,
}

pub fn main_entry() {
    main_entry_from(std::env::args_os());
}

pub fn main_entry_from<I, T>(args: I)
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let cli = NodeCli::parse_from(args);
    if let Some(path) = cli.node_config.as_deref() {
        if let Err(error) = load_profiles_from_config(path) {
            eprintln!("Node profile config error: {}", error);
            std::process::exit(1);
        }
    }

    println!("mmux node '{}'", cli.node_id);
    if let Some(url) = cli.controller_url.as_deref() {
        println!("  Controller URL: {}", url);
        let token = cli
            .controller_token
            .clone()
            .or_else(|| std::env::var("MMUX_CONTROLLER_TOKEN").ok());
        let runtime = tokio::runtime::Runtime::new().expect("failed to create tokio runtime");
        if let Err(error) = runtime.block_on(run_registered_node(&cli, url, token.as_deref())) {
            eprintln!("Node error: {}", error);
            std::process::exit(1);
        }
    } else {
        println!("  No controller URL provided; node is idle.");
    }
}

async fn run_registered_node(
    cli: &NodeCli,
    controller_url: &str,
    token: Option<&str>,
) -> Result<(), String> {
    let descriptor = NodeDescriptor {
        node_id: cli.node_id.clone(),
        display_name: cli
            .node_name
            .clone()
            .unwrap_or_else(|| format!("mmux node {}", cli.node_id)),
    };
    let poll_interval = Duration::from_millis(cli.poll_interval_ms);
    let rpc_timeout = Duration::from_secs(5);
    let mut reconnect_delay = Duration::from_secs(1);
    loop {
        let client = match connect_client(controller_url, token) {
            Ok(client) => client,
            Err(error) => {
                eprintln!("controller connect failed: {error}");
                tokio::time::sleep(reconnect_delay).await;
                reconnect_delay = next_reconnect_delay(reconnect_delay);
                continue;
            }
        };

        let response = match tokio::time::timeout(
            rpc_timeout,
            client.register_node(register_node_request_to_proto(RegisterNodeRequest {
                descriptor: descriptor.clone(),
            })),
        )
        .await
        {
            Ok(Ok(response)) => response.into_owned(),
            Ok(Err(error)) => {
                eprintln!("register node RPC failed: {error}");
                tokio::time::sleep(reconnect_delay).await;
                reconnect_delay = next_reconnect_delay(reconnect_delay);
                continue;
            }
            Err(_) => {
                eprintln!(
                    "register node RPC timed out after {:?}; reconnecting",
                    rpc_timeout
                );
                tokio::time::sleep(reconnect_delay).await;
                reconnect_delay = next_reconnect_delay(reconnect_delay);
                continue;
            }
        };

        if !response.accepted {
            eprintln!("controller rejected registration: {}", response.message);
            tokio::time::sleep(reconnect_delay).await;
            reconnect_delay = next_reconnect_delay(reconnect_delay);
            continue;
        }
        println!("  {}", response.message);
        reconnect_delay = Duration::from_secs(1);

        loop {
            log_node(&format!("mmux node: heartbeat {}", cli.node_id));
            match tokio::time::timeout(
                rpc_timeout,
                client.heartbeat(heartbeat_request_to_proto(HeartbeatRequest {
                    node_id: cli.node_id.clone(),
                    status: NodeStatus::Ready,
                })),
            )
            .await
            {
                Ok(Ok(_)) => {}
                Ok(Err(error)) => {
                    eprintln!("heartbeat RPC failed: {error}");
                    break;
                }
                Err(_) => {
                    eprintln!(
                        "heartbeat RPC timed out after {:?}; reconnecting",
                        rpc_timeout
                    );
                    break;
                }
            }

            log_node(&format!("mmux node: pull commands {}", cli.node_id));
            let response = match tokio::time::timeout(
                rpc_timeout,
                client.pull_commands(pull_commands_request_to_proto(PullCommandsRequest {
                    node_id: cli.node_id.clone(),
                })),
            )
            .await
            {
                Ok(Ok(response)) => response.into_owned(),
                Ok(Err(error)) => {
                    eprintln!("pull commands RPC failed: {error}");
                    break;
                }
                Err(_) => {
                    eprintln!(
                        "pull commands RPC timed out after {:?}; reconnecting",
                        rpc_timeout
                    );
                    break;
                }
            };
            let response = match pull_commands_response_from_proto(response) {
                Ok(response) => response,
                Err(error) => {
                    eprintln!("pull commands decode failed: {error}");
                    break;
                }
            };
            log_node(&format!(
                "mmux node: received {} command(s)",
                response.commands.len()
            ));
            for command in response.commands {
                let command_id = command.command_id.clone();
                log_node(&format!(
                    "mmux node: executing command {} {:?}",
                    command_id, command.kind
                ));
                let result = execute_node_command(command);
                log_node(&format!("mmux node: finished command {}", command_id));
                match tokio::time::timeout(
                    rpc_timeout,
                    client.submit_command_result(submit_command_result_request_to_proto(
                        SubmitCommandResultRequest {
                            node_id: cli.node_id.clone(),
                            command_id,
                            result,
                        },
                    )),
                )
                .await
                {
                    Ok(Ok(_)) => log_node("mmux node: submitted command result"),
                    Ok(Err(error)) => {
                        eprintln!("submit command result RPC failed: {error}");
                        break;
                    }
                    Err(_) => {
                        eprintln!(
                            "submit command result RPC timed out after {:?}; reconnecting",
                            rpc_timeout
                        );
                        break;
                    }
                }
            }
            tokio::time::sleep(poll_interval).await;
        }

        eprintln!(
            "mmux node: controller connection lost, retrying in {:?}",
            reconnect_delay
        );
        tokio::time::sleep(reconnect_delay).await;
        reconnect_delay = next_reconnect_delay(reconnect_delay);
    }
}

fn log_node(message: &str) {
    eprintln!("{}", message);
    let _ = std::io::stderr().flush();
}

fn next_reconnect_delay(current: Duration) -> Duration {
    let next = current.saturating_mul(2);
    next.min(Duration::from_secs(30))
}

fn execute_node_command(command: NodeCommand) -> NodeCommandResult {
    match command.kind {
        NodeCommandKind::Tmux { args } => {
            let refs: Vec<&str> = args.iter().map(String::as_str).collect();
            match tmux(&refs) {
                Ok(output) => NodeCommandResult::TmuxOutput(output),
                Err(message) => NodeCommandResult::Error { message },
            }
        }
        NodeCommandKind::ReadFile {
            path,
            offset,
            limit,
        } => match read_file_bytes(&path, offset, limit) {
            Ok(bytes) => NodeCommandResult::FileContent {
                content_base64: BASE64.encode(bytes),
            },
            Err(message) => NodeCommandResult::Error { message },
        },
        NodeCommandKind::WriteFile {
            path,
            content_base64,
            append,
        } => match BASE64.decode(content_base64.as_bytes()) {
            Ok(bytes) => match write_file_bytes(&path, &bytes, append) {
                Ok(bytes_written) => NodeCommandResult::WriteComplete { bytes_written },
                Err(message) => NodeCommandResult::Error { message },
            },
            Err(error) => NodeCommandResult::Error {
                message: format!("base64 decode error: {}", error),
            },
        },
        NodeCommandKind::Shutdown => std::process::exit(0),
    }
}

fn read_file_bytes(path: &str, offset: Option<u64>, limit: usize) -> Result<Vec<u8>, String> {
    let mut file = std::fs::File::open(path).map_err(|error| format!("read error: {}", error))?;
    if let Some(offset) = offset {
        file.seek(SeekFrom::Start(offset))
            .map_err(|error| format!("seek error: {}", error))?;
    }
    let mut bytes = Vec::new();
    file.take(limit as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("read error: {}", error))?;
    Ok(bytes)
}

fn write_file_bytes(path: &str, bytes: &[u8], append: bool) -> Result<usize, String> {
    if let Some(parent) = Path::new(path).parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            format!("failed to create parent '{}': {}", parent.display(), error)
        })?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .append(append)
        .truncate(!append)
        .open(path)
        .map_err(|error| format!("write error: {}", error))?;
    file.write_all(bytes)
        .map_err(|error| format!("write error: {}", error))?;
    Ok(bytes.len())
}

fn connect_client(
    controller_url: &str,
    token: Option<&str>,
) -> Result<MmuxNodeRegistryServiceClient<HttpClient>, String> {
    let uri = controller_url
        .parse()
        .map_err(|error| format!("invalid controller URL '{}': {}", controller_url, error))?;
    let mut config = ClientConfig::new(uri);
    if let Some(token) = token {
        let value = HeaderValue::from_str(&format!("Bearer {}", token))
            .map_err(|error| format!("invalid controller token header: {}", error))?;
        config.default_headers.insert(AUTHORIZATION, value);
    }
    Ok(MmuxNodeRegistryServiceClient::new(
        HttpClient::plaintext(),
        config,
    ))
}

#[derive(Clone, Debug, Default)]
pub struct LocalNode;

impl LocalNode {
    pub fn tmux(&self, args: &[&str]) -> Result<String, String> {
        tmux(args)
    }

    pub fn session_exists(&self, session: &str) -> bool {
        session_exists(session)
    }

    pub fn read_file(
        &self,
        path: &str,
        offset: Option<u64>,
        limit: Option<usize>,
    ) -> Result<ReadFileResult, String> {
        read_file_impl(path, offset, limit)
    }

    pub fn save_file(
        &self,
        path: &str,
        content: &str,
        encoding: &str,
        append: bool,
        max_bytes: Option<usize>,
    ) -> Result<SaveFileResult, String> {
        save_file_impl(path, content, encoding, append, max_bytes)
    }
}

pub fn tmux(args: &[&str]) -> Result<String, String> {
    let out = Command::new("tmux")
        .args(args)
        .output()
        .map_err(|e| format!("tmux failed to execute: {}", e))?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if !out.status.success() {
        return Err(format!("tmux error: {}{}", stdout, stderr));
    }
    Ok(stdout)
}

pub fn session_exists(session: &str) -> bool {
    Command::new("tmux")
        .args(["has-session", "-t", session])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub fn default_profile_config_in_cwd() -> Option<String> {
    let path = std::path::Path::new(DEFAULT_NODE_PROFILE_CONFIG_NAME);
    if path.exists() {
        Some(path.to_string_lossy().into_owned())
    } else {
        None
    }
}

pub fn load_profile_from_toml(text: &str) -> Result<CliProfile, String> {
    let profile: CliProfile =
        toml::from_str(text).map_err(|e| format!("toml parse error: {}", e))?;
    Ok(profile)
}

pub fn get_profile(registry: &ProfileRegistry, name: &str) -> Option<CliProfile> {
    registry.read().unwrap().get(name).cloned()
}

pub fn load_profiles_from_config(path: &str) -> Result<ProfileRegistry, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("failed to read node profile config '{}': {}", path, e))?;
    let config: toml::Table = toml::from_str(&text)
        .map_err(|e| format!("failed to parse node profile config '{}': {}", path, e))?;

    let mut registry = default_profile_map();

    if let Some(profiles) = config.get("coder_profile").and_then(|v| v.as_table()) {
        for (name, value) in profiles {
            let profile = load_profile_overlay(name, value, registry.get(name).cloned())?;
            registry.insert(name.clone(), profile);
        }
    }

    Ok(Arc::new(RwLock::new(registry)))
}

fn default_profile_map() -> HashMap<String, CliProfile> {
    let mut registry = HashMap::new();

    registry.insert(
        "opencode".into(),
        CliProfile {
            name: "opencode".into(),
            cmd: Some("opencode".into()),
            permission_bypass_cmd: None,
            prompt_indicator: ">".into(),
            busy_indicators: vec!["Processing".into(), "Generating".into()],
            startup_dismiss: None,
            launch: None,
            approve_keys: "y Enter".into(),
            reject_keys: "n Enter".into(),
            cancel_keys: "C-c".into(),
            escape_keys: "Escape".into(),
        },
    );

    registry.insert(
        "kimi".into(),
        CliProfile {
            name: "kimi".into(),
            cmd: Some("kimi".into()),
            permission_bypass_cmd: Some("kimi --yolo".into()),
            prompt_indicator: ">".into(),
            busy_indicators: vec![
                "Working".into(),
                "Running".into(),
                "ctrl+c: cancel".into(),
                "ctrl-s to steer".into(),
                "to edit".into(),
            ],
            startup_dismiss: Some(StartupDismiss {
                key: "Escape".into(),
                triggers: vec!["Kimi Code Update Available".into()],
            }),
            launch: None,
            approve_keys: "y Enter".into(),
            reject_keys: "n Enter".into(),
            cancel_keys: "C-c".into(),
            escape_keys: "Escape".into(),
        },
    );

    registry.insert(
        "codex".into(),
        CliProfile {
            name: "codex".into(),
            cmd: Some("codex".into()),
            permission_bypass_cmd: Some("codex --dangerously-bypass-approvals-and-sandbox".into()),
            prompt_indicator: "›".into(),
            busy_indicators: vec!["• Working".into(), "Starting MCP servers".into()],
            startup_dismiss: Some(StartupDismiss {
                key: "Down Enter".into(),
                triggers: vec!["Update available!".into(), "Press enter to continue".into()],
            }),
            launch: None,
            approve_keys: "y Enter".into(),
            reject_keys: "n Enter".into(),
            cancel_keys: "C-c".into(),
            escape_keys: "Escape".into(),
        },
    );

    registry.insert(
        "claude".into(),
        CliProfile {
            name: "claude".into(),
            cmd: Some("claude".into()),
            permission_bypass_cmd: Some("claude --dangerously-skip-permissions".into()),
            prompt_indicator: "❯".into(),
            busy_indicators: vec!["Thinking".into(), "Working".into(), "Running".into()],
            startup_dismiss: Some(StartupDismiss {
                key: "Escape".into(),
                triggers: vec![
                    "Update available".into(),
                    "Claude Code Update Available".into(),
                    "A new version of Claude Code is available".into(),
                ],
            }),
            launch: None,
            approve_keys: "y Enter".into(),
            reject_keys: "n Enter".into(),
            cancel_keys: "C-c".into(),
            escape_keys: "Escape".into(),
        },
    );

    registry.insert("generic".into(), CliProfile::default());

    registry
}

pub fn default_profiles() -> ProfileRegistry {
    Arc::new(RwLock::new(default_profile_map()))
}

fn load_profile_overlay(
    name: &str,
    value: &toml::Value,
    base: Option<CliProfile>,
) -> Result<CliProfile, String> {
    let mut merged = match base {
        Some(profile) => toml::Value::try_from(profile)
            .map_err(|error| format!("serialize built-in profile '{}': {}", name, error))?,
        None => toml::Value::Table(toml::Table::new()),
    };
    merge_toml_value(&mut merged, value.clone());
    let mut profile: CliProfile = merged
        .try_into()
        .map_err(|error| format!("profile '{}': {}", name, error))?;
    if profile.name.is_empty() {
        profile.name = name.to_owned();
    }
    Ok(profile)
}

fn merge_toml_value(base: &mut toml::Value, overlay: toml::Value) {
    match (base, overlay) {
        (toml::Value::Table(base), toml::Value::Table(overlay)) => {
            for (key, value) in overlay {
                match base.get_mut(&key) {
                    Some(existing) => merge_toml_value(existing, value),
                    None => {
                        base.insert(key, value);
                    }
                }
            }
        }
        (base, overlay) => *base = overlay,
    }
}

pub fn detect_compression(bytes: &[u8]) -> Option<String> {
    if bytes.len() < 2 {
        return None;
    }
    let candidate = match &bytes[..2] {
        [0x1f, 0x8b] => Some("gzip".into()),
        [0x42, 0x5a] if bytes.get(2) == Some(&0x68) => Some("bzip2".into()),
        [0x50, 0x4b] => Some("zip".into()),
        _ => None,
    };
    if candidate.is_some() {
        return candidate;
    }
    if bytes.len() >= 4 && bytes[..4] == [0x28, 0xb5, 0x2f, 0xfd] {
        return Some("zstd".into());
    }
    if bytes.len() >= 6 && bytes[..6] == [0xfd, 0x37, 0x7a, 0x58, 0x5a, 0x00] {
        return Some("xz".into());
    }
    None
}

pub fn detect_mime_type(path: &Path, bytes: &[u8]) -> String {
    if bytes.len() >= 8 {
        match &bytes[..8] {
            [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a] => return "image/png".into(),
            [0xff, 0xd8, 0xff, ..] => return "image/jpeg".into(),
            [0x25, 0x50, 0x44, 0x46, ..] => return "application/pdf".into(),
            [0x47, 0x49, 0x46, 0x38, ..] => return "image/gif".into(),
            _ => {}
        }
    }
    if bytes.len() >= 4 {
        match &bytes[..4] {
            [0x52, 0x49, 0x46, 0x46] if bytes.get(8..12) == Some(b"WEBP") => {
                return "image/webp".into()
            }
            _ => {}
        }
    }
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    match ext.as_str() {
        "txt" | "md" | "markdown" => "text/plain",
        "rs" => "text/x-rustsrc",
        "js" => "text/javascript",
        "ts" => "text/typescript",
        "json" => "application/json",
        "yaml" | "yml" => "application/yaml",
        "toml" => "application/toml",
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "xml" => "application/xml",
        "py" => "text/x-python",
        "sh" => "text/x-shellscript",
        "c" | "h" => "text/x-c",
        "cpp" | "hpp" | "cc" => "text/x-c++",
        "go" => "text/x-go",
        "java" => "text/x-java",
        "rb" => "text/x-ruby",
        "php" => "text/x-php",
        "sql" => "text/x-sql",
        "csv" => "text/csv",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        "zip" => "application/zip",
        "tar" => "application/x-tar",
        "gz" => "application/gzip",
        "bz2" => "application/x-bzip2",
        "xz" => "application/x-xz",
        "zst" | "zstd" => "application/zstd",
        "wasm" => "application/wasm",
        "ico" => "image/x-icon",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mp3" => "audio/mpeg",
        "ogg" => "audio/ogg",
        "wav" => "audio/wav",
        _ => "application/octet-stream",
    }
    .into()
}

pub fn read_file_impl(
    path: &str,
    offset: Option<u64>,
    limit: Option<usize>,
) -> Result<ReadFileResult, String> {
    use std::fs;
    let meta = fs::metadata(path).map_err(|e| format!("metadata error: {}", e))?;
    let total_size = meta.len();

    let mut file = fs::File::open(path).map_err(|e| format!("open error: {}", e))?;
    if let Some(off) = offset {
        use std::io::Seek;
        file.seek(std::io::SeekFrom::Start(off))
            .map_err(|e| format!("seek error: {}", e))?;
    }

    let cap = limit.unwrap_or(4 * 1024 * 1024);
    let mut buf = Vec::with_capacity(cap.min(total_size as usize));
    use std::io::Read;
    let mut chunk = file.take(cap as u64);
    chunk
        .read_to_end(&mut buf)
        .map_err(|e| format!("read error: {}", e))?;
    let read_len = buf.len();

    let compression = detect_compression(&buf);
    let mime = detect_mime_type(Path::new(path), &buf);

    let (content, encoding) = match std::str::from_utf8(&buf) {
        Ok(s) => (s.to_owned(), "utf-8".into()),
        Err(_) => (BASE64.encode(&buf), "base64".into()),
    };

    Ok(ReadFileResult {
        path: path.into(),
        content,
        encoding,
        mime_type: mime,
        size_bytes: total_size,
        read_bytes: read_len,
        compression,
    })
}

pub fn save_file_impl(
    path: &str,
    content: &str,
    encoding: &str,
    append: bool,
    max_bytes: Option<usize>,
) -> Result<SaveFileResult, String> {
    use std::fs;
    use std::io::Write;

    let bytes: Vec<u8> = match encoding {
        "base64" => BASE64
            .decode(content)
            .map_err(|e| format!("base64 decode error: {}", e))?,
        "utf-8" => content.as_bytes().to_vec(),
        other => return Err(format!("unsupported encoding: {}", other)),
    };

    let written = bytes.len();
    if let Some(max) = max_bytes {
        if written > max {
            return Err(format!(
                "write too large: {} bytes exceeds limit of {}",
                written, max
            ));
        }
    }
    if append {
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|e| format!("open error: {}", e))?;
        file.write_all(&bytes)
            .map_err(|e| format!("write error: {}", e))?;
    } else {
        fs::write(path, &bytes).map_err(|e| format!("write error: {}", e))?;
    }

    let mime = detect_mime_type(Path::new(path), &bytes);
    Ok(SaveFileResult {
        path: path.into(),
        bytes_written: written,
        mime_type: Some(mime),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{engine::general_purpose::STANDARD as BASE64, Engine};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "{}-{}-{}",
            name,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ))
    }

    #[test]
    fn load_profiles_from_config_overlays_built_in_profiles() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "mmux-coder-profile-test-{}.toml",
            std::process::id()
        ));
        std::fs::write(
            &path,
            r#"
[coder_profile.codex]
prompt_indicator = "codex ready"
"#,
        )
        .unwrap();

        let profiles = load_profiles_from_config(path.to_str().unwrap()).unwrap();
        let profile = get_profile(&profiles, "codex").expect("profile loaded");
        assert_eq!(profile.name, "codex");
        assert_eq!(profile.cmd.as_deref(), Some("codex"));
        assert_eq!(profile.prompt_indicator, "codex ready");
        assert!(profile
            .busy_indicators
            .iter()
            .any(|marker| marker == "• Working"));

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn root_mmux_example_config_parses() {
        let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let config_path = manifest_dir.join("../..").join("mmux.toml.example");

        let profiles = load_profiles_from_config(config_path.to_str().unwrap()).unwrap();

        assert!(get_profile(&profiles, "codex").is_some());
        assert!(get_profile(&profiles, "kimi").is_some());
    }

    #[test]
    fn default_profiles_include_tuned_codex_and_claude() {
        let profiles = default_profiles();

        let kimi = get_profile(&profiles, "kimi").expect("kimi profile");
        assert_eq!(kimi.cmd.as_deref(), Some("kimi"));
        assert!(kimi
            .busy_indicators
            .iter()
            .any(|marker| marker == "ctrl-s to steer"));

        let codex = get_profile(&profiles, "codex").expect("codex profile");
        assert_eq!(codex.prompt_indicator, "›");
        assert_eq!(
            codex.permission_bypass_cmd.as_deref(),
            Some("codex --dangerously-bypass-approvals-and-sandbox")
        );
        let codex_dismiss = codex.startup_dismiss.expect("codex startup dismiss");
        assert_eq!(codex_dismiss.key, "Down Enter");
        assert!(codex_dismiss
            .triggers
            .iter()
            .any(|trigger| trigger == "Update available!"));

        let claude = get_profile(&profiles, "claude").expect("claude profile");
        assert_eq!(claude.cmd.as_deref(), Some("claude"));
        assert_eq!(
            claude.permission_bypass_cmd.as_deref(),
            Some("claude --dangerously-skip-permissions")
        );
        assert_eq!(claude.prompt_indicator, "❯");
        assert!(claude
            .busy_indicators
            .iter()
            .any(|marker| marker == "Thinking"));
    }

    #[test]
    fn save_file_impl_rejects_writes_over_max_bytes() {
        let path = unique_temp_file("mmux-node-max-write.txt");

        let error = save_file_impl(path.to_str().unwrap(), "too large", "utf-8", false, Some(3))
            .unwrap_err();

        assert!(error.contains("write too large"));
        assert!(!path.exists());
    }

    #[test]
    fn save_file_impl_appends_when_requested() {
        let path = unique_temp_file("mmux-node-append.txt");

        save_file_impl(path.to_str().unwrap(), "first", "utf-8", false, None).unwrap();
        save_file_impl(path.to_str().unwrap(), "second", "utf-8", true, None).unwrap();

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "firstsecond");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn save_file_impl_enforces_max_bytes_after_base64_decode() {
        let path = unique_temp_file("mmux-node-base64-max.bin");
        let content = BASE64.encode([0, 1, 2, 3]);

        let error =
            save_file_impl(path.to_str().unwrap(), &content, "base64", false, Some(3)).unwrap_err();

        assert!(error.contains("write too large"));
        assert!(!path.exists());
    }
}
