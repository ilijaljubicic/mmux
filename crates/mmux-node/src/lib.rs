use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use clap::{Parser, ValueEnum};
use connectrpc::{
    client::{ClientConfig, HttpClient},
    rustls,
};
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
use std::io::{BufReader, Cursor, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Output, Stdio};
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const DEFAULT_NODE_PROFILE_CONFIG_NAME: &str = "mmux.toml";
pub const DEFAULT_STORE_DIR_NAME: &str = ".mmux";
pub const LOCAL_TMUX_SOCKET_NAME: &str = "tmux-local.sock";
const LOCAL_TMUX_QUICK_TIMEOUT: Duration = Duration::from_secs(5);
const LOCAL_TMUX_CONTROL_TIMEOUT: Duration = Duration::from_secs(10);
const MICROSANDBOX_HEALTH_TIMEOUT: Duration = Duration::from_secs(20);
const MICROSANDBOX_TMUX_TIMEOUT: Duration = Duration::from_secs(20);
const MICROSANDBOX_FILE_TIMEOUT: Duration = Duration::from_secs(60);
const BACKEND_PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(25);

pub type ProfileRegistry = Arc<RwLock<HashMap<String, CliProfile>>>;

#[derive(Clone, Debug)]
struct NodeClientIdentity {
    cert_pem: Arc<Vec<u8>>,
    key_pem: Arc<Vec<u8>>,
}

impl NodeClientIdentity {
    fn from_cli(cli: &NodeCli) -> Result<Option<Self>, String> {
        match (&cli.client_cert, &cli.client_key) {
            (Some(cert_path), Some(key_path)) => {
                let cert_path = canonicalize_required_path("--client-cert", cert_path)?;
                let key_path = canonicalize_required_path("--client-key", key_path)?;
                Ok(Some(Self {
                    cert_pem: Arc::new(read_pem_file("--client-cert", &cert_path)?),
                    key_pem: Arc::new(read_pem_file("--client-key", &key_path)?),
                }))
            }
            (None, None) => Ok(None),
            (Some(_), None) => Err("--client-cert requires --client-key".into()),
            (None, Some(_)) => Err("--client-key requires --client-cert".into()),
        }
    }
}

#[derive(Clone, Debug)]
struct ResolvedNodeWirePolicy {
    token: Option<String>,
    client_identity: Option<NodeClientIdentity>,
    controller_ca: Option<PathBuf>,
}

impl ResolvedNodeWirePolicy {
    fn from_cli(cli: &NodeCli) -> Result<Self, String> {
        let token = cli
            .wire_token
            .clone()
            .or_else(|| std::env::var("MMUX_WIRE_TOKEN").ok());
        let client_identity = NodeClientIdentity::from_cli(cli)?;
        let controller_ca = cli
            .controller_ca
            .as_deref()
            .map(|path| canonicalize_required_path("--controller-ca", path))
            .transpose()?;
        if token.is_some() && client_identity.is_some() {
            return Err("--wire-token/MMUX_WIRE_TOKEN is mutually exclusive with --client-cert/--client-key".into());
        }
        Ok(Self {
            token,
            client_identity,
            controller_ca,
        })
    }
}

#[derive(Parser, Debug)]
#[command(name = "mmux node")]
#[command(about = "Run an mmux execution node that owns local tmux/filesystem access")]
pub struct NodeCli {
    #[arg(long, value_enum, default_value_t = NodeBackendKind::Local)]
    pub backend: NodeBackendKind,
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
    pub wire_token: Option<String>,
    #[arg(
        long,
        help = "PEM CA certificate(s) used to verify the HTTPS controller."
    )]
    pub controller_ca: Option<PathBuf>,
    #[arg(long, help = "PEM certificate chain to present for node wire mTLS.")]
    pub client_cert: Option<PathBuf>,
    #[arg(long, help = "PEM private key to present for node wire mTLS.")]
    pub client_key: Option<PathBuf>,
    #[arg(
        long,
        default_value_t = 500,
        help = "Milliseconds between command polls"
    )]
    pub poll_interval_ms: u64,
    #[arg(long, help = "Path to node profile TOML file")]
    pub node_config: Option<String>,
    #[arg(
        long,
        help = "Directory for local node runtime state. The local tmux socket is <store-path>/tmux-local.sock."
    )]
    pub store_path: Option<PathBuf>,
    #[arg(
        long,
        help = "Path to tmux.conf used by the local backend tmux server. Only valid with --backend local."
    )]
    pub tmux_config: Option<PathBuf>,
    #[arg(
        long,
        help = "Existing running Microsandbox sandbox name used with --backend microsandbox"
    )]
    pub sandbox_name: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum NodeBackendKind {
    Local,
    Microsandbox,
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
    if let Err(error) = validate_node_backend_flags(&cli) {
        eprintln!("Node backend config error: {}", error);
        std::process::exit(1);
    }
    if let Some(path) = cli.node_config.as_deref() {
        if let Err(error) = load_profiles_from_config(path) {
            eprintln!("Node profile config error: {}", error);
            std::process::exit(1);
        }
    }

    println!("mmux node '{}'", cli.node_id);
    if let Some(url) = cli.controller_url.as_deref() {
        println!("  Controller URL: {}", url);
        let wire_auth = match ResolvedNodeWirePolicy::from_cli(&cli) {
            Ok(auth) => auth,
            Err(error) => {
                eprintln!("Node wire auth error: {}", error);
                std::process::exit(1);
            }
        };
        let runtime = tokio::runtime::Runtime::new().expect("failed to create tokio runtime");
        if let Err(error) = runtime.block_on(async {
            let backend = NodeExecutionBackend::from_cli(&cli).await?;
            run_registered_node(
                &cli,
                url,
                wire_auth.token.as_deref(),
                wire_auth.client_identity,
                wire_auth.controller_ca,
                backend,
            )
            .await
        }) {
            eprintln!("Node error: {}", error);
            std::process::exit(1);
        }
    } else {
        println!("  No controller URL provided; node is idle.");
    }
}

fn validate_node_backend_flags(cli: &NodeCli) -> Result<(), String> {
    if cli.backend != NodeBackendKind::Local && cli.tmux_config.is_some() {
        return Err("--tmux-config is only valid with --backend local".to_owned());
    }
    Ok(())
}

async fn run_registered_node(
    cli: &NodeCli,
    controller_url: &str,
    token: Option<&str>,
    client_identity: Option<NodeClientIdentity>,
    controller_ca: Option<PathBuf>,
    mut backend: NodeExecutionBackend,
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
        let client = match connect_client(
            controller_url,
            token,
            client_identity.as_ref(),
            controller_ca.as_deref(),
        ) {
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
            if !response.commands.is_empty() {
                log_node(&format!(
                    "mmux node: received {} command(s)",
                    response.commands.len()
                ));
            }
            for command in response.commands {
                let command_id = command.command_id.clone();
                log_node(&format!(
                    "mmux node: executing command {} {:?}",
                    command_id, command.kind
                ));
                let result = backend.execute(command).await;
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

#[derive(Clone)]
enum NodeExecutionBackend {
    Local(LocalNode),
    Microsandbox(MicrosandboxNodeBackend),
}

impl NodeExecutionBackend {
    async fn from_cli(cli: &NodeCli) -> Result<Self, String> {
        match cli.backend {
            NodeBackendKind::Local => Ok(Self::Local(LocalNode::new(
                cli.store_path.as_deref(),
                cli.tmux_config.as_deref(),
            )?)),
            NodeBackendKind::Microsandbox => {
                validate_node_backend_flags(cli)?;
                let sandbox_name = cli.sandbox_name.clone().ok_or_else(|| {
                    "--sandbox-name is required with --backend microsandbox".to_owned()
                })?;
                ensure_existing_microsandbox(&sandbox_name).await?;
                Ok(Self::Microsandbox(MicrosandboxNodeBackend { sandbox_name }))
            }
        }
    }

    async fn execute(&mut self, command: NodeCommand) -> NodeCommandResult {
        match self {
            Self::Local(local) => execute_local_node_command(local, command),
            Self::Microsandbox(backend) => backend.execute(command).await,
        }
    }
}

#[derive(Clone)]
pub struct EmbeddedNodeBackend {
    backend: NodeExecutionBackend,
}

impl EmbeddedNodeBackend {
    pub async fn local(
        store_path: Option<&Path>,
        tmux_config: Option<&Path>,
    ) -> Result<Self, String> {
        let local = LocalNode::new(store_path, tmux_config)?;
        ensure_local_tmux_backend_available(&local)?;
        Ok(Self {
            backend: NodeExecutionBackend::Local(local),
        })
    }

    pub async fn microsandbox(sandbox_name: &str) -> Result<Self, String> {
        ensure_existing_microsandbox(sandbox_name).await?;
        Ok(Self {
            backend: NodeExecutionBackend::Microsandbox(MicrosandboxNodeBackend {
                sandbox_name: sandbox_name.to_owned(),
            }),
        })
    }

    pub async fn execute(&mut self, kind: NodeCommandKind) -> NodeCommandResult {
        self.backend
            .execute(NodeCommand {
                command_id: "embedded".into(),
                kind,
            })
            .await
    }
}

fn ensure_local_tmux_backend_available(local: &LocalNode) -> Result<(), String> {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let session = format!("mmux-health-{}-{suffix}", std::process::id());
    local
        .tmux(&["new-session", "-d", "-s", &session, "sh -c 'sleep 30'"])
        .map_err(|error| format!("failed to start local tmux backend: {error}"))?;
    let _ = local.tmux(&["kill-session", "-t", &session]);
    Ok(())
}

#[derive(Clone)]
struct MicrosandboxNodeBackend {
    sandbox_name: String,
}

async fn ensure_existing_microsandbox(sandbox_name: &str) -> Result<(), String> {
    run_microsandbox_shell(
        sandbox_name,
        "true",
        MICROSANDBOX_HEALTH_TIMEOUT,
        "msb health check",
    )
    .await
    .map(|_| ())
    .map_err(|error| {
        format!(
            "failed to use existing running Microsandbox '{}' via 'msb exec': {}",
            sandbox_name, error
        )
    })
}

impl MicrosandboxNodeBackend {
    async fn execute(&self, command: NodeCommand) -> NodeCommandResult {
        match command.kind {
            NodeCommandKind::Tmux { args } => {
                match self
                    .run_command("tmux", &args, MICROSANDBOX_TMUX_TIMEOUT, "msb tmux")
                    .await
                {
                    Ok(output) => NodeCommandResult::TmuxOutput(output),
                    Err(message) => NodeCommandResult::Error { message },
                }
            }
            NodeCommandKind::ReadFile {
                path,
                offset,
                limit,
            } => match self.read_file(&path, offset, limit).await {
                Ok(content_base64) => NodeCommandResult::FileContent { content_base64 },
                Err(message) => NodeCommandResult::Error { message },
            },
            NodeCommandKind::WriteFile {
                path,
                content_base64,
                append,
            } => match self.write_file(&path, &content_base64, append).await {
                Ok(bytes_written) => NodeCommandResult::WriteComplete { bytes_written },
                Err(message) => NodeCommandResult::Error { message },
            },
            NodeCommandKind::Shutdown => std::process::exit(0),
        }
    }

    async fn read_file(
        &self,
        path: &str,
        offset: Option<u64>,
        limit: usize,
    ) -> Result<String, String> {
        let skip = offset.unwrap_or(0);
        let command = format!(
            "dd if={} bs=1 skip={} count={} status=none | base64 -w0",
            shell_quote(path),
            skip,
            limit
        );
        self.run_shell(&command, MICROSANDBOX_FILE_TIMEOUT, "msb file read")
            .await
    }

    async fn write_file(
        &self,
        path: &str,
        content_base64: &str,
        append: bool,
    ) -> Result<usize, String> {
        let parent = Path::new(path)
            .parent()
            .and_then(Path::to_str)
            .filter(|parent| !parent.is_empty())
            .unwrap_or(".");
        let operator = if append { ">>" } else { ">" };
        let command = format!(
            "mkdir -p {} && printf %s {} | base64 -d {} {}",
            shell_quote(parent),
            shell_quote(content_base64),
            operator,
            shell_quote(path)
        );
        self.run_shell(&command, MICROSANDBOX_FILE_TIMEOUT, "msb file write")
            .await?;
        BASE64
            .decode(content_base64.as_bytes())
            .map(|bytes| bytes.len())
            .map_err(|error| format!("base64 decode error: {}", error))
    }

    async fn run_shell(
        &self,
        command: &str,
        timeout: Duration,
        description: &'static str,
    ) -> Result<String, String> {
        run_microsandbox_shell(&self.sandbox_name, command, timeout, description).await
    }

    async fn run_command(
        &self,
        program: &str,
        args: &[String],
        timeout: Duration,
        description: &'static str,
    ) -> Result<String, String> {
        run_microsandbox_command(&self.sandbox_name, program, args, timeout, description).await
    }
}

fn execute_local_node_command(local: &LocalNode, command: NodeCommand) -> NodeCommandResult {
    match command.kind {
        NodeCommandKind::Tmux { args } => {
            let refs: Vec<&str> = args.iter().map(String::as_str).collect();
            match local.tmux(&refs) {
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

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

async fn run_microsandbox_command(
    sandbox_name: &str,
    program: &str,
    args: &[String],
    timeout: Duration,
    description: &'static str,
) -> Result<String, String> {
    let sandbox_name = sandbox_name.to_owned();
    let program = program.to_owned();
    let args = args.to_vec();
    let output = tokio::task::spawn_blocking(move || {
        let mut command_runner = Command::new("msb");
        command_runner
            .arg("exec")
            .arg("-q")
            .arg(&sandbox_name)
            .arg("--")
            .arg(&program)
            .args(&args);
        run_output_command_with_timeout(
            command_runner,
            &format!("{} for {}", description, sandbox_name),
            timeout,
        )
    })
    .await
    .map_err(|error| format!("msb exec task failed: {}", error))?
    .map_err(|error| format!("msb failed to execute: {}", error))?;

    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    if output.status.success() {
        return Ok(stdout);
    }
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    Err(format!(
        "msb exec exited with code {}\nstdout:\n{}\nstderr:\n{}",
        output.status.code().unwrap_or(-1),
        stdout,
        stderr
    ))
}

async fn run_microsandbox_shell(
    sandbox_name: &str,
    command: &str,
    timeout: Duration,
    description: &'static str,
) -> Result<String, String> {
    let sandbox_name = sandbox_name.to_owned();
    let command = command.to_owned();
    let output = tokio::task::spawn_blocking(move || {
        let mut command_runner = Command::new("msb");
        command_runner
            .arg("exec")
            .arg("-q")
            .arg(&sandbox_name)
            .arg("--")
            .arg("bash")
            .arg("-lc")
            .arg(&command);
        run_output_command_with_timeout(
            command_runner,
            &format!("{} for {}", description, sandbox_name),
            timeout,
        )
    })
    .await
    .map_err(|error| format!("msb exec task failed: {}", error))?
    .map_err(|error| format!("msb failed to execute: {}", error))?;

    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    if output.status.success() {
        return Ok(stdout);
    }
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    Err(format!(
        "msb exec exited with code {}\nstdout:\n{}\nstderr:\n{}",
        output.status.code().unwrap_or(-1),
        stdout,
        stderr
    ))
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
    client_identity: Option<&NodeClientIdentity>,
    controller_ca: Option<&Path>,
) -> Result<MmuxNodeRegistryServiceClient<HttpClient>, String> {
    let uri: http::Uri = controller_url
        .parse()
        .map_err(|error| format!("invalid controller URL '{}': {}", controller_url, error))?;
    let transport = controller_transport(&uri, client_identity, controller_ca)?;
    let mut config = ClientConfig::new(uri);
    if let Some(token) = token {
        let value = HeaderValue::from_str(&format!("Bearer {}", token))
            .map_err(|error| format!("invalid controller token header: {}", error))?;
        config.default_headers.insert(AUTHORIZATION, value);
    }
    Ok(MmuxNodeRegistryServiceClient::new(transport, config))
}

fn controller_transport(
    uri: &http::Uri,
    client_identity: Option<&NodeClientIdentity>,
    controller_ca: Option<&Path>,
) -> Result<HttpClient, String> {
    match uri.scheme_str() {
        Some("http") => {
            if client_identity.is_some() {
                return Err("--client-cert/--client-key require an https:// controller URL".into());
            }
            if controller_ca.is_some() {
                return Err("--controller-ca requires an https:// controller URL".into());
            }
            Ok(HttpClient::plaintext())
        }
        Some("https") => Ok(HttpClient::with_tls(default_tls_config(
            client_identity,
            controller_ca,
        )?)),
        Some(other) => Err(format!("unsupported controller URL scheme '{}'", other)),
        None => Err("controller URL must include http:// or https:// scheme".into()),
    }
}

fn canonicalize_required_path(flag: &str, path: &Path) -> Result<PathBuf, String> {
    std::fs::canonicalize(path).map_err(|error| {
        format!(
            "failed to canonicalize {} '{}': {}",
            flag,
            path.display(),
            error
        )
    })
}

fn read_pem_file(flag: &str, path: &Path) -> Result<Vec<u8>, String> {
    std::fs::read(path)
        .map_err(|error| format!("failed to read {} '{}': {}", flag, path.display(), error))
}

fn default_tls_config(
    client_identity: Option<&NodeClientIdentity>,
    controller_ca: Option<&Path>,
) -> Result<Arc<rustls::ClientConfig>, String> {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    if let Some(controller_ca) = controller_ca {
        for cert in load_cert_chain(controller_ca, "controller CA certificate")? {
            roots.add(cert).map_err(|error| {
                format!(
                    "failed to add controller CA '{}': {}",
                    controller_ca.display(),
                    error
                )
            })?;
        }
    }
    let builder = rustls::ClientConfig::builder().with_root_certificates(roots);
    let config = if let Some(identity) = client_identity {
        let cert_chain =
            load_cert_chain_from_bytes(identity.cert_pem.as_slice(), "client certificate")?;
        let private_key =
            load_private_key_from_bytes(identity.key_pem.as_slice(), "client private key")?;
        builder
            .with_client_auth_cert(cert_chain, private_key)
            .map_err(|error| format!("invalid client TLS identity: {}", error))?
    } else {
        builder.with_no_client_auth()
    };
    Ok(Arc::new(config))
}

fn load_cert_chain_from_bytes(
    bytes: &[u8],
    description: &str,
) -> Result<Vec<rustls::pki_types::CertificateDer<'static>>, String> {
    let certs = rustls_pemfile::certs(&mut Cursor::new(bytes))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("failed to parse {}: {}", description, error))?;
    if certs.is_empty() {
        return Err(format!("{} contains no certificates", description));
    }
    Ok(certs)
}

fn load_cert_chain(
    path: &Path,
    description: &str,
) -> Result<Vec<rustls::pki_types::CertificateDer<'static>>, String> {
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

fn load_private_key_from_bytes(
    bytes: &[u8],
    description: &str,
) -> Result<rustls::pki_types::PrivateKeyDer<'static>, String> {
    rustls_pemfile::private_key(&mut Cursor::new(bytes))
        .map_err(|error| format!("failed to parse {}: {}", description, error))?
        .ok_or_else(|| format!("{} contains no private key", description))
}

#[derive(Clone, Debug)]
pub struct LocalNode {
    tmux_socket: PathBuf,
    tmux_config: Option<PathBuf>,
}

impl LocalNode {
    pub fn new(store_path: Option<&Path>, tmux_config: Option<&Path>) -> Result<Self, String> {
        let store_path = resolve_store_path(store_path)?;
        ensure_store_dir(&store_path)?;
        let tmux_config = tmux_config
            .map(|path| canonicalize_required_path("--tmux-config", path))
            .transpose()?;
        Ok(Self {
            tmux_socket: local_tmux_socket_path(&store_path),
            tmux_config,
        })
    }

    pub fn socket_path(&self) -> &Path {
        &self.tmux_socket
    }

    pub fn tmux_config_path(&self) -> Option<&Path> {
        self.tmux_config.as_deref()
    }

    pub fn tmux(&self, args: &[&str]) -> Result<String, String> {
        tmux_with_socket(Some(&self.tmux_socket), self.tmux_config.as_deref(), args)
    }

    pub fn session_exists(&self, session: &str) -> bool {
        session_exists_with_socket(
            Some(&self.tmux_socket),
            self.tmux_config.as_deref(),
            session,
        )
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

pub fn default_store_path() -> Result<PathBuf, String> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .ok_or_else(|| "HOME is not set; pass --store-path explicitly".to_owned())?;
    Ok(home.join(DEFAULT_STORE_DIR_NAME))
}

pub fn resolve_store_path(path: Option<&Path>) -> Result<PathBuf, String> {
    match path {
        Some(path) => expand_tilde_path(path),
        None => default_store_path(),
    }
}

fn expand_tilde_path(path: &Path) -> Result<PathBuf, String> {
    let text = path.to_string_lossy();
    if text == "~" {
        return std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or_else(|| "HOME is not set; cannot expand '~'".to_owned());
    }
    if let Some(rest) = text.strip_prefix("~/") {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or_else(|| "HOME is not set; cannot expand '~/'".to_owned())?;
        return Ok(home.join(rest));
    }
    Ok(path.to_path_buf())
}

pub fn ensure_store_dir(path: &Path) -> Result<(), String> {
    std::fs::create_dir_all(path).map_err(|error| {
        format!(
            "failed to create store path '{}': {}",
            path.display(),
            error
        )
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).map_err(
            |error| {
                format!(
                    "failed to set store path permissions '{}': {}",
                    path.display(),
                    error
                )
            },
        )?;
    }
    Ok(())
}

pub fn local_tmux_socket_path(store_path: &Path) -> PathBuf {
    store_path.join(LOCAL_TMUX_SOCKET_NAME)
}

pub fn tmux_with_socket(
    socket: Option<&Path>,
    config: Option<&Path>,
    args: &[&str],
) -> Result<String, String> {
    let command = build_tmux_command(socket, config, args);
    if is_tmux_control_command(args) {
        let status = run_status_command_with_timeout(
            command,
            "tmux control command",
            local_tmux_timeout_for_args(args),
        )
        .map_err(|error| format!("tmux failed to execute: {}", error))?;
        if !status.success() {
            return Err(format!(
                "tmux error: control command exited with code {}",
                status.code().unwrap_or(-1)
            ));
        }
        return Ok(String::new());
    }

    let out =
        run_output_command_with_timeout(command, "tmux command", local_tmux_timeout_for_args(args))
            .map_err(|error| format!("tmux failed to execute: {}", error))?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if !out.status.success() {
        return Err(format!("tmux error: {}{}", stdout, stderr));
    }
    Ok(stdout)
}

pub fn tmux(args: &[&str]) -> Result<String, String> {
    tmux_with_socket(None, None, args)
}

pub fn session_exists_with_socket(
    socket: Option<&Path>,
    config: Option<&Path>,
    session: &str,
) -> bool {
    tmux_with_socket(socket, config, &["has-session", "-t", session]).is_ok()
}

pub fn session_exists(session: &str) -> bool {
    session_exists_with_socket(None, None, session)
}

fn build_tmux_command(socket: Option<&Path>, config: Option<&Path>, args: &[&str]) -> Command {
    let mut command = Command::new("tmux");
    if let Some(socket) = socket {
        command.arg("-S").arg(socket);
    }
    if let Some(config) = config {
        command.arg("-f").arg(config);
    }
    command.args(args);
    command
}

fn is_tmux_control_command(args: &[&str]) -> bool {
    matches!(
        args.first().copied(),
        Some(
            "start-server"
                | "new-session"
                | "kill-session"
                | "send-keys"
                | "set-buffer"
                | "paste-buffer"
                | "resize-pane"
                | "set-option"
                | "rename-session"
                | "detach-client"
        )
    )
}

fn local_tmux_timeout_for_args(args: &[&str]) -> Duration {
    if is_tmux_control_command(args) {
        LOCAL_TMUX_CONTROL_TIMEOUT
    } else {
        LOCAL_TMUX_QUICK_TIMEOUT
    }
}

fn run_output_command_with_timeout(
    mut command: Command,
    description: &str,
    timeout: Duration,
) -> Result<Output, String> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|error| format!("spawn failed: {}", error))?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_status)) => {
                return child
                    .wait_with_output()
                    .map_err(|error| format!("wait failed: {}", error));
            }
            Ok(None) if started.elapsed() >= timeout => {
                let pid = child.id();
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "{} timed out after {}s (pid {})",
                    description,
                    timeout.as_secs(),
                    pid
                ));
            }
            Ok(None) => thread::sleep(BACKEND_PROCESS_POLL_INTERVAL),
            Err(error) => return Err(format!("wait failed: {}", error)),
        }
    }
}

fn run_status_command_with_timeout(
    mut command: Command,
    description: &str,
    timeout: Duration,
) -> Result<ExitStatus, String> {
    command.stdout(Stdio::null()).stderr(Stdio::null());
    let mut child = command
        .spawn()
        .map_err(|error| format!("spawn failed: {}", error))?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) if started.elapsed() >= timeout => {
                let pid = child.id();
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "{} timed out after {}s (pid {})",
                    description,
                    timeout.as_secs(),
                    pid
                ));
            }
            Ok(None) => thread::sleep(BACKEND_PROCESS_POLL_INTERVAL),
            Err(error) => return Err(format!("wait failed: {}", error)),
        }
    }
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
    validate_profile(&profile)?;
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
            launch_strategy: Some("shell_send".into()),
            text_mode: "paste-buffer".into(),
            submit_keys: "Enter".into(),
            submit_after_text: true,
            prompt_indicator: "ctrl+p commands".into(),
            busy_indicators: vec![
                "Thinking".into(),
                "Working".into(),
                "Running".into(),
                "Processing".into(),
                "Generating".into(),
            ],
            startup_dismiss: None,
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
            launch_strategy: None,
            text_mode: "paste-buffer".into(),
            submit_keys: "Enter".into(),
            submit_after_text: true,
            prompt_indicator: ">".into(),
            busy_indicators: vec![
                "Working".into(),
                "Running".into(),
                "ctrl+c: cancel".into(),
                "ctrl-s to steer".into(),
                "to edit".into(),
            ],
            startup_dismiss: Some(StartupDismiss {
                policy: "custom-keys".into(),
                key: Some("Escape".into()),
                triggers: vec!["Kimi Code Update Available".into()],
            }),
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
            launch_strategy: None,
            text_mode: "paste-buffer".into(),
            submit_keys: "Enter".into(),
            submit_after_text: true,
            prompt_indicator: "›".into(),
            busy_indicators: vec!["• Working".into()],
            startup_dismiss: Some(StartupDismiss {
                policy: "skip-update".into(),
                key: None,
                triggers: vec!["Update now".into()],
            }),
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
            launch_strategy: None,
            text_mode: "literal-keys".into(),
            submit_keys: "Enter".into(),
            submit_after_text: true,
            prompt_indicator: "❯".into(),
            busy_indicators: vec!["Thinking".into(), "Working".into(), "Running".into()],
            startup_dismiss: Some(StartupDismiss {
                policy: "custom-keys".into(),
                key: Some("Escape".into()),
                triggers: vec![
                    "Update available".into(),
                    "Claude Code Update Available".into(),
                    "A new version of Claude Code is available".into(),
                ],
            }),
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
    validate_profile(&profile)?;
    Ok(profile)
}

fn validate_profile(profile: &CliProfile) -> Result<(), String> {
    if let Some(dismiss) = profile.startup_dismiss.as_ref() {
        match dismiss.policy.as_str() {
            "skip-update" | "update-now" => {
                if dismiss.key.is_some() {
                    return Err(format!(
                        "profile '{}' startup_dismiss policy '{}' does not accept key; use policy 'custom-keys' for literal key sequences",
                        profile.name, dismiss.policy
                    ));
                }
            }
            "custom-keys" => {
                if dismiss
                    .key
                    .as_deref()
                    .map(str::trim)
                    .unwrap_or_default()
                    .is_empty()
                {
                    return Err(format!(
                        "profile '{}' startup_dismiss policy 'custom-keys' requires key",
                        profile.name
                    ));
                }
            }
            other => {
                return Err(format!(
                    "profile '{}' uses unsupported startup_dismiss policy '{}'",
                    profile.name, other
                ));
            }
        }
    }
    Ok(())
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

    fn unique_temp_dir(name: &str) -> std::path::PathBuf {
        unique_temp_file(name)
    }

    #[test]
    fn local_node_uses_store_path_socket() {
        let store = unique_temp_dir("mmux-node-store");
        let local = LocalNode::new(Some(&store), None).unwrap();

        assert_eq!(local.socket_path(), store.join(LOCAL_TMUX_SOCKET_NAME));
        assert!(store.exists());

        let _ = std::fs::remove_dir_all(store);
    }

    #[test]
    fn local_node_accepts_tmux_config_path() {
        let store = unique_temp_dir("mmux-node-store");
        let config = unique_temp_file("mmux-node-tmux-conf");
        std::fs::write(&config, "set -g mouse on\n").unwrap();
        let canonical_config = std::fs::canonicalize(&config).unwrap();
        let local = LocalNode::new(Some(&store), Some(&config)).unwrap();

        assert_eq!(local.tmux_config_path(), Some(canonical_config.as_path()));

        let _ = std::fs::remove_dir_all(store);
        let _ = std::fs::remove_file(config);
    }

    #[test]
    fn tmux_config_is_local_backend_only() {
        let cli = NodeCli {
            backend: NodeBackendKind::Microsandbox,
            node_id: "msb-1".into(),
            controller_url: None,
            node_name: None,
            wire_token: None,
            controller_ca: None,
            client_cert: None,
            client_key: None,
            poll_interval_ms: 500,
            node_config: None,
            store_path: None,
            tmux_config: Some(PathBuf::from("tmux.local.conf")),
            sandbox_name: Some("mmux-node".into()),
        };

        assert!(validate_node_backend_flags(&cli)
            .unwrap_err()
            .contains("--tmux-config is only valid with --backend local"));
    }

    #[test]
    fn tmux_server_control_commands_do_not_capture_output() {
        assert!(is_tmux_control_command(&["start-server"]));
        assert!(is_tmux_control_command(&["new-session", "-d"]));
        assert!(is_tmux_control_command(&["send-keys", "-t", "s", "Enter"]));
        assert!(!is_tmux_control_command(&["list-sessions"]));
        assert!(!is_tmux_control_command(&["capture-pane", "-p"]));
        assert!(!is_tmux_control_command(&["has-session", "-t", "s"]));
    }

    #[test]
    fn local_tmux_timeout_classification_matches_command_kind() {
        assert_eq!(
            local_tmux_timeout_for_args(&["new-session", "-d"]),
            LOCAL_TMUX_CONTROL_TIMEOUT
        );
        assert_eq!(
            local_tmux_timeout_for_args(&["start-server"]),
            LOCAL_TMUX_CONTROL_TIMEOUT
        );
        assert_eq!(
            local_tmux_timeout_for_args(&["list-sessions"]),
            LOCAL_TMUX_QUICK_TIMEOUT
        );
        assert_eq!(
            local_tmux_timeout_for_args(&["capture-pane", "-p"]),
            LOCAL_TMUX_QUICK_TIMEOUT
        );
    }

    #[test]
    fn local_node_private_socket_new_session_returns() {
        let store = unique_temp_dir("mmux-node-store");
        let local = LocalNode::new(Some(&store), None).unwrap();
        let session = format!("mmux-test-{}", std::process::id());

        local
            .tmux(&["new-session", "-d", "-s", &session, "sh -c 'sleep 30'"])
            .unwrap();
        assert!(local.session_exists(&session));

        let _ = local.tmux(&["kill-session", "-t", &session]);
        let _ = std::fs::remove_dir_all(store);
    }

    #[test]
    fn resolve_store_path_expands_tilde() {
        let home = std::env::var_os("HOME").expect("HOME set for test");
        let resolved = resolve_store_path(Some(Path::new("~/mmux-test-store"))).unwrap();

        assert_eq!(resolved, PathBuf::from(home).join("mmux-test-store"));
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
    fn default_profiles_include_tuned_coder_profiles() {
        let profiles = default_profiles();

        let opencode = get_profile(&profiles, "opencode").expect("opencode profile");
        assert_eq!(opencode.cmd.as_deref(), Some("opencode"));
        assert_eq!(opencode.launch_strategy.as_deref(), Some("shell_send"));
        assert_eq!(opencode.prompt_indicator, "ctrl+p commands");
        assert!(opencode
            .busy_indicators
            .iter()
            .any(|marker| marker == "Working"));

        let kimi = get_profile(&profiles, "kimi").expect("kimi profile");
        assert_eq!(kimi.cmd.as_deref(), Some("kimi"));
        assert!(kimi
            .busy_indicators
            .iter()
            .any(|marker| marker == "ctrl-s to steer"));

        let codex = get_profile(&profiles, "codex").expect("codex profile");
        assert_eq!(codex.prompt_indicator, "›");
        assert_eq!(codex.text_mode, "paste-buffer");
        assert_eq!(codex.submit_keys, "Enter");
        assert!(codex.submit_after_text);
        assert_eq!(
            codex.permission_bypass_cmd.as_deref(),
            Some("codex --dangerously-bypass-approvals-and-sandbox")
        );
        let codex_dismiss = codex.startup_dismiss.expect("codex startup dismiss");
        assert_eq!(codex_dismiss.policy, "skip-update");
        assert_eq!(codex_dismiss.key, None);
        assert!(codex_dismiss
            .triggers
            .iter()
            .any(|trigger| trigger == "Update now"));

        let claude = get_profile(&profiles, "claude").expect("claude profile");
        assert_eq!(claude.cmd.as_deref(), Some("claude"));
        assert_eq!(
            claude.permission_bypass_cmd.as_deref(),
            Some("claude --dangerously-skip-permissions")
        );
        assert_eq!(claude.prompt_indicator, "❯");
        assert_eq!(claude.text_mode, "literal-keys");
        assert_eq!(claude.submit_keys, "Enter");
        assert!(claude.submit_after_text);
        assert!(claude
            .busy_indicators
            .iter()
            .any(|marker| marker == "Thinking"));
    }

    #[test]
    fn controller_transport_supports_http_and_https() {
        let http_transport =
            controller_transport(&"http://localhost:3000".parse().unwrap(), None, None)
                .expect("http transport");
        assert!(format!("{http_transport:?}").contains("plaintext"));

        let https_transport =
            controller_transport(&"https://mmux.example.com".parse().unwrap(), None, None)
                .expect("https transport");
        assert!(format!("{https_transport:?}").contains("tls"));
    }

    #[test]
    fn controller_transport_rejects_unsupported_schemes() {
        let error = controller_transport(&"ftp://mmux.example.com".parse().unwrap(), None, None)
            .unwrap_err();

        assert!(error.contains("unsupported controller URL scheme"));
    }

    #[test]
    fn controller_transport_rejects_client_identity_without_https() {
        let identity = NodeClientIdentity {
            cert_pem: Arc::new(Vec::new()),
            key_pem: Arc::new(Vec::new()),
        };

        let error = controller_transport(
            &"http://localhost:3000".parse().unwrap(),
            Some(&identity),
            None,
        )
        .unwrap_err();

        assert!(error.contains("require an https:// controller URL"));
    }

    #[test]
    fn controller_transport_rejects_controller_ca_without_https() {
        let ca = Path::new("controller-ca.pem");

        let error = controller_transport(&"http://localhost:3000".parse().unwrap(), None, Some(ca))
            .unwrap_err();

        assert!(error.contains("--controller-ca requires an https:// controller URL"));
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
