use clap::{Parser, Subcommand};
use http::Uri;
use microsandbox::sandbox::{DiskImageFormat, HostPermissions, MountBuilder, StatVirtualization};
use microsandbox::snapshot::ExportOpts;
use microsandbox::{Sandbox, Snapshot, Volume};
use microsandbox_network::policy::{
    Action, Destination, DestinationGroup, NetworkPolicyBuilder, PortRange, Protocol, Rule,
};
use mmux_shared::CliProfile;
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::path::{Path, PathBuf};

pub const DEFAULT_IMAGE: &str = "debian:bookworm-slim";

#[derive(Debug, Clone, Parser)]
#[command(name = "mmux-microsandbox-node")]
#[command(about = "Launch and manage an mmux node inside Microsandbox")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Clone, Subcommand)]
pub enum Command {
    #[command(about = "Prepare a sandbox image without starting mmux node")]
    Prepare(PrepareArgs),
    #[command(about = "Launch mmux node in a prepared sandbox or image")]
    Launch(LaunchArgs),
    #[command(about = "Stop a sandbox and create a Microsandbox snapshot")]
    Snapshot(SnapshotArgs),
    #[command(about = "Stop, snapshot, and export a sandbox bundle")]
    SnapshotExport(SnapshotExportArgs),
    #[command(about = "Import an exported Microsandbox snapshot bundle")]
    SnapshotImport(SnapshotImportArgs),
    #[command(about = "Show sandbox status")]
    Status(SharedArgs),
    #[command(about = "Resume an existing sandbox")]
    Resume(SharedArgs),
    #[command(about = "Stop a sandbox")]
    Stop(SharedArgs),
    #[command(about = "Show sandbox and guest mmux logs")]
    Logs(SharedArgs),
}

#[derive(Debug, Clone, Parser)]
pub struct SharedArgs {
    #[arg(long, default_value = "mmux-node")]
    pub name: String,
    #[arg(long)]
    pub image: Option<String>,
}

#[derive(Debug, Clone, Parser)]
pub struct LaunchArgs {
    #[command(flatten)]
    pub shared: SharedArgs,

    #[arg(long, default_value_os_t = default_node_config())]
    pub node_config: PathBuf,
    #[arg(long)]
    pub snapshot: Option<String>,
    #[arg(long)]
    pub controller_url: String,
    #[arg(long, default_value = "msb-mmux-1")]
    pub node_id: String,
    #[arg(long, default_value = "Microsandbox mmux node")]
    pub node_name: String,
}

#[derive(Debug, Clone, Parser)]
pub struct PrepareArgs {
    #[command(flatten)]
    pub shared: SharedArgs,

    #[arg(long, default_value_os_t = default_node_config())]
    pub node_config: PathBuf,

    #[arg(long)]
    pub mmux_binary: Option<PathBuf>,
}

#[derive(Debug, Clone, Parser)]
pub struct SnapshotArgs {
    #[command(flatten)]
    pub shared: SharedArgs,

    #[arg(long, default_value = "mmux-node-seed")]
    pub snapshot_name: String,
}

#[derive(Debug, Clone, Parser)]
pub struct SnapshotExportArgs {
    #[command(flatten)]
    pub shared: SharedArgs,

    #[arg(long, default_value = "mmux-node-seed")]
    pub snapshot_name: String,
    #[arg(long, default_value = ".artifacts/mmux-node-seed.tar.zst")]
    pub bundle: PathBuf,
}

#[derive(Debug, Clone, Parser)]
pub struct SnapshotImportArgs {
    #[arg(long)]
    pub bundle: PathBuf,
    #[arg(long)]
    pub dest: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LaunchReport {
    pub name: String,
    pub image: String,
    pub node_id: String,
    pub controller_url: String,
    pub snapshot: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrepareReport {
    pub name: String,
    pub image: String,
    pub node_config: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotReport {
    pub name: String,
    pub source_sandbox: String,
    pub path: String,
    pub digest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotExportReport {
    pub name: String,
    pub source_sandbox: String,
    pub path: String,
    pub digest: String,
    pub bundle: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotImportReport {
    pub bundle: String,
    pub path: String,
    pub digest: String,
    pub name: Option<String>,
}

pub async fn launch(args: LaunchArgs) -> Result<LaunchReport, Box<dyn Error + Send + Sync>> {
    let microsandbox_config = load_microsandbox_config(&args.node_config)?;
    let image = args
        .shared
        .image
        .clone()
        .unwrap_or_else(|| microsandbox_config.sandbox.runtime.image.clone());
    let script_assets = if args.snapshot.is_some() {
        Vec::new()
    } else {
        load_host_scripts(
            &microsandbox_config.microsandbox,
            &microsandbox_config.base_dir,
        )?
    };
    if args.snapshot.is_none()
        && image == DEFAULT_IMAGE
        && microsandbox_config
            .microsandbox
            .assets
            .mmux_version
            .is_none()
        && script_assets.is_empty()
    {
        return Err(
            "runtime config does not prepare a stock image; use bundle-launch with a prepared snapshot, use microsandbox-setup.toml for preparation, or set a prepared image in the config".into(),
        );
    }
    create_sandbox(SandboxLaunchSpec {
        shared: &args.shared,
        image: &image,
        node_config: &args.node_config,
        node_launch: Some(NodeLaunchSpec {
            controller_url: &args.controller_url,
            node_id: &args.node_id,
            node_name: &args.node_name,
        }),
        setup_network: false,
        snapshot: args.snapshot.as_deref(),
        runtime: &microsandbox_config.sandbox.runtime,
        sandbox_config: &microsandbox_config.sandbox,
        microsandbox_config: &microsandbox_config.microsandbox,
        coder_profiles: &microsandbox_config.coder_profiles,
        config_base_dir: &microsandbox_config.base_dir,
        script_assets: &script_assets,
        mmux_binary: None,
    })
    .await?;

    Ok(LaunchReport {
        name: args.shared.name,
        image,
        node_id: args.node_id,
        controller_url: args.controller_url,
        snapshot: args.snapshot,
    })
}

pub async fn prepare(args: PrepareArgs) -> Result<PrepareReport, Box<dyn Error + Send + Sync>> {
    let microsandbox_config = load_microsandbox_config(&args.node_config)?;
    let runtime = microsandbox_config
        .microsandbox
        .runtime
        .as_ref()
        .unwrap_or(&microsandbox_config.sandbox.runtime);
    let image = args
        .shared
        .image
        .clone()
        .unwrap_or_else(|| runtime.image.clone());
    let script_assets = load_host_scripts(
        &microsandbox_config.microsandbox,
        &microsandbox_config.base_dir,
    )?;
    if image == DEFAULT_IMAGE
        && microsandbox_config
            .microsandbox
            .assets
            .mmux_version
            .is_none()
        && args.mmux_binary.is_none()
        && script_assets.is_empty()
    {
        return Err(
            "prepare config does not install or copy anything into a stock image; add setup scripts, mmux_version, --mmux-binary, or set a prepared image in the config".into(),
        );
    }

    create_sandbox(SandboxLaunchSpec {
        shared: &args.shared,
        image: &image,
        node_config: &args.node_config,
        node_launch: None,
        setup_network: true,
        snapshot: None,
        runtime,
        sandbox_config: &microsandbox_config.sandbox,
        microsandbox_config: &microsandbox_config.microsandbox,
        coder_profiles: &microsandbox_config.coder_profiles,
        config_base_dir: &microsandbox_config.base_dir,
        script_assets: &script_assets,
        mmux_binary: args.mmux_binary.as_deref(),
    })
    .await?;

    Ok(PrepareReport {
        name: args.shared.name,
        image,
        node_config: args.node_config.display().to_string(),
    })
}

pub async fn snapshot(args: SnapshotArgs) -> Result<SnapshotReport, Box<dyn Error + Send + Sync>> {
    let sandbox = Sandbox::get(&args.shared.name).await?;
    sandbox.stop().await?;
    let snap = sandbox.snapshot(&args.snapshot_name).await?;
    Ok(SnapshotReport {
        name: args.snapshot_name,
        source_sandbox: args.shared.name,
        path: snap.path().display().to_string(),
        digest: snap.digest().to_string(),
    })
}

pub async fn snapshot_export(
    args: SnapshotExportArgs,
) -> Result<SnapshotExportReport, Box<dyn Error + Send + Sync>> {
    let sandbox = Sandbox::get(&args.shared.name).await?;
    sandbox.stop().await?;
    let snap = sandbox.snapshot(&args.snapshot_name).await?;
    let snapshot_path = snap.path().to_path_buf();
    let snapshot_digest = snap.digest().to_string();
    let bundle = args.bundle;
    if let Some(parent) = bundle.parent() {
        std::fs::create_dir_all(parent)?;
    }
    Snapshot::export(
        snapshot_path.to_string_lossy().as_ref(),
        &bundle,
        ExportOpts::default(),
    )
    .await?;
    Ok(SnapshotExportReport {
        name: args.snapshot_name,
        source_sandbox: args.shared.name,
        path: snapshot_path.display().to_string(),
        digest: snapshot_digest,
        bundle: bundle.display().to_string(),
    })
}

pub async fn snapshot_import(
    args: SnapshotImportArgs,
) -> Result<SnapshotImportReport, Box<dyn Error + Send + Sync>> {
    let handle = Snapshot::import(&args.bundle, args.dest.as_deref()).await?;
    let snapshot = handle.open().await?;
    Ok(SnapshotImportReport {
        bundle: args.bundle.display().to_string(),
        path: handle.path().display().to_string(),
        digest: snapshot.digest().to_string(),
        name: handle.name().map(|name| name.to_string()),
    })
}

pub async fn status(args: SharedArgs) -> Result<serde_json::Value, Box<dyn Error + Send + Sync>> {
    let sandbox = Sandbox::get(&args.name).await?;
    let metrics = sandbox.metrics().await?;
    Ok(serde_json::json!({
        "name": args.name,
        "running": metrics.cpu_percent >= 0.0,
        "cpu": metrics.cpu_percent,
        "memory": metrics.memory_bytes,
        "disk_read": metrics.disk_read_bytes,
        "disk_write": metrics.disk_write_bytes,
    }))
}

pub async fn stop(args: SharedArgs) -> Result<(), Box<dyn Error + Send + Sync>> {
    let sandbox = Sandbox::get(&args.name).await?;
    sandbox.stop().await?;
    Ok(())
}

pub async fn resume(args: SharedArgs) -> Result<(), Box<dyn Error + Send + Sync>> {
    let _sandbox = Sandbox::start_detached(&args.name).await?;
    Ok(())
}

pub async fn logs(args: SharedArgs) -> Result<String, Box<dyn Error + Send + Sync>> {
    let handle = Sandbox::get(&args.name).await?;
    let options = microsandbox::logs::LogOptions {
        sources: vec![
            microsandbox::logs::LogSource::Stdout,
            microsandbox::logs::LogSource::Stderr,
            microsandbox::logs::LogSource::Output,
            microsandbox::logs::LogSource::System,
        ],
        ..Default::default()
    };
    let entries = handle
        .logs(&options)
        .await
        .map_err(|error| format!("log read failed: {}", error))?;
    let mut out = String::new();
    out.push_str(&format!("status: {:?}\n", handle.status()));
    for entry in entries {
        let source = match entry.source {
            microsandbox::logs::LogSource::Stdout => "stdout",
            microsandbox::logs::LogSource::Stderr => "stderr",
            microsandbox::logs::LogSource::Output => "output",
            microsandbox::logs::LogSource::System => "system",
        };
        let data = String::from_utf8_lossy(&entry.data);
        out.push_str(&format!("[{source}] {}\n", data.trim_end()));
    }
    if let Ok(sandbox) = handle.connect().await {
        if let Ok(output) = sandbox
            .shell(
                "cat /mmux/mmux-node.log 2>/dev/null || cat /tmp/mmux-node.log 2>/dev/null || true",
            )
            .await
        {
            out.push_str("== guest log ==\n");
            out.push_str(&output.stdout().unwrap_or_default());
            if !out.ends_with('\n') {
                out.push('\n');
            }
        }
        if let Ok(output) = sandbox
            .shell("ps -ef 2>/dev/null | grep '[m]mux\\|[t]mux' || true")
            .await
        {
            out.push_str("== processes ==\n");
            out.push_str(&output.stdout().unwrap_or_default());
            if !out.ends_with('\n') {
                out.push('\n');
            }
        }
    }
    Ok(out)
}

struct SandboxLaunchSpec<'a> {
    shared: &'a SharedArgs,
    image: &'a str,
    node_config: &'a Path,
    node_launch: Option<NodeLaunchSpec<'a>>,
    setup_network: bool,
    snapshot: Option<&'a str>,
    runtime: &'a SandboxRuntimeConfig,
    sandbox_config: &'a SandboxConfig,
    microsandbox_config: &'a MicrosandboxConfig,
    coder_profiles: &'a [CliProfile],
    config_base_dir: &'a Path,
    script_assets: &'a [MicrosandboxScriptAsset],
    mmux_binary: Option<&'a Path>,
}

#[derive(Debug, Clone, Copy)]
struct NodeLaunchSpec<'a> {
    controller_url: &'a str,
    node_id: &'a str,
    node_name: &'a str,
}

async fn create_sandbox(spec: SandboxLaunchSpec<'_>) -> Result<(), Box<dyn Error + Send + Sync>> {
    let SandboxLaunchSpec {
        shared,
        image,
        node_config,
        node_launch,
        setup_network,
        snapshot,
        runtime,
        sandbox_config,
        microsandbox_config,
        coder_profiles,
        config_base_dir,
        script_assets,
        mmux_binary,
    } = spec;
    ensure_configured_volumes(&sandbox_config.volumes).await?;
    let policy = if setup_network && !sandbox_config.has_network_policy {
        build_setup_network_policy()?
    } else {
        build_network_policy(
            &sandbox_config.network,
            node_launch.map(|launch| launch.controller_url),
        )?
    };

    let mut builder = Sandbox::builder(&shared.name)
        .memory(runtime.memory_mib)
        .cpus(runtime.cpus)
        .replace();
    if let Some(node_launch) = node_launch {
        builder = builder
            .env("MMUX_CONTROLLER_URL", node_launch.controller_url)
            .env("MMUX_NODE_ID", node_launch.node_id)
            .env("MMUX_NODE_NAME", node_launch.node_name)
            .env("MMUX_NODE_CONFIG", "/etc/mmux/mmux-node.toml")
            .env("MMUX_POLL_INTERVAL_MS", "500");
    }
    if let Some(snapshot_ref) = snapshot {
        builder = builder.from_snapshot(snapshot_ref.to_string());
    } else {
        builder = builder.image(image.to_owned());
    }
    builder = builder.network(|network| {
        network
            .policy(policy)
            .trust_host_cas(sandbox_config.network.trust_host_cas)
    });
    for profile in coder_profiles {
        if let Some(launch) = profile.launch.as_ref() {
            for (key, value) in &launch.env {
                builder = builder.env(key, value);
            }
            for secret in &launch.secrets {
                let value = resolve_host_secret_value(&secret.value_from)?;
                builder = builder.env(&secret.env, value);
            }
        }
    }
    for secret in &sandbox_config.secrets {
        let value = resolve_host_secret_value(&secret.value_from)?;
        builder = builder.env(&secret.env, value.clone());
        builder = builder.secret(|s| {
            let mut s = s
                .env(&secret.env)
                .value(value)
                .allow_host(&secret.allowed_host);
            if let Some(placeholder) = secret.placeholder.as_deref() {
                s = s.placeholder(placeholder);
            }
            s = s.inject_headers(secret.inject_headers);
            s = s.inject_basic_auth(secret.inject_basic_auth);
            s = s.inject_query(secret.inject_query);
            s = s.inject_body(secret.inject_body);
            s.require_tls_identity(secret.require_tls_identity)
        });
    }
    for mount in &sandbox_config.mounts {
        builder = builder.volume(mount_guest(mount), |v| {
            apply_mount(v, mount, config_base_dir)
        });
    }

    if let Some(mmux_version) = microsandbox_config.assets.mmux_version.as_ref() {
        builder = builder.env("MMUX_VERSION", mmux_version);
    }
    if mmux_binary.is_some() {
        builder = builder.env("MMUX_SKIP_RELEASE_INSTALL", "1");
    }

    if snapshot.is_none() {
        for script in script_assets {
            builder = builder.script(&script.name, &script.content);
        }
    }

    if snapshot.is_none() {
        let mmux_assets_dir = microsandbox_config
            .assets
            .assets_dir
            .as_ref()
            .map(|path| resolve_host_path(config_base_dir, path));
        let tmux_conf = microsandbox_config
            .assets
            .tmux_conf
            .as_ref()
            .map(|path| resolve_host_path(config_base_dir, path))
            .or_else(|| {
                mmux_assets_dir
                    .as_ref()
                    .map(|assets_dir| assets_dir.join("tmux.conf"))
                    .filter(|path| path.is_file())
            });
        builder = builder.patch(|p: microsandbox::sandbox::PatchBuilder| {
            let p = p.mkdir("/etc/mmux", Some(0o755)).copy_file(
                node_config,
                "/etc/mmux/mmux-node.toml",
                None,
                true,
            );
            let p = p
                .mkdir("/mmux", Some(0o755))
                .mkdir("/usr/local/bin", Some(0o755))
                .mkdir("/workspace", Some(0o755));
            let p = if let Some(mmux_binary) = mmux_binary {
                p.copy_file(
                    resolve_host_path(config_base_dir, mmux_binary),
                    "/usr/local/bin/mmux",
                    Some(0o755),
                    true,
                )
            } else {
                p
            };
            let p = if let Some(tmux_conf) = tmux_conf.as_ref() {
                p.copy_file(tmux_conf, "/mmux/tmux.conf", None, true).text(
                    "/etc/tmux.conf",
                    "source-file /mmux/tmux.conf
",
                    Some(0o644),
                    false,
                )
            } else {
                p
            };
            let p = if let Some(mmux_assets_dir) = mmux_assets_dir.as_ref() {
                p.copy_dir(mmux_assets_dir, "/mmux/mmux_sources/assets", true)
            } else {
                p
            };
            apply_config_patches(p, &microsandbox_config.patches, config_base_dir)
        });
    }

    let sandbox = builder.create_detached().await?;

    if snapshot.is_some() {
        install_node_config(&sandbox, node_config).await?;
    }
    if snapshot.is_none() {
        run_scripts(&sandbox, script_assets).await?;
    }
    if node_launch.is_some() {
        launch_mmux_node(&sandbox).await?;
    }
    sandbox.detach().await;

    Ok(())
}

async fn install_node_config(
    sandbox: &Sandbox,
    node_config: &Path,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let content = std::fs::read_to_string(node_config)?;
    let delimiter = "__MMUX_NODE_CONFIG_EOF__";
    if content.contains(delimiter) {
        return Err(format!(
            "node config contains reserved heredoc delimiter '{}'",
            delimiter
        )
        .into());
    }
    let command = format!(
        "set -eu\nmkdir -p /etc/mmux\ncat > /etc/mmux/mmux-node.toml <<'{}'\n{}\n{}\nchmod 0644 /etc/mmux/mmux-node.toml\n",
        delimiter, content, delimiter
    );
    run_shell_checked(sandbox, "install runtime node config", &command).await
}

async fn launch_mmux_node(sandbox: &Sandbox) -> Result<(), Box<dyn Error + Send + Sync>> {
    let command = r#"set -eu
mkdir -p /mmux
nohup /usr/local/bin/mmux node \
  --controller-url "$MMUX_CONTROLLER_URL" \
  --node-id "$MMUX_NODE_ID" \
  --node-name "$MMUX_NODE_NAME" \
  --node-config "$MMUX_NODE_CONFIG" \
  --poll-interval-ms "$MMUX_POLL_INTERVAL_MS" \
  >/mmux/mmux-node.log 2>&1 &
"#;

    run_shell_checked(sandbox, "launch mmux node", command).await?;

    let ready_check = r#"for _ in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15; do
  if ps -ef 2>/dev/null | grep '[m]mux node' >/dev/null 2>&1; then
    exit 0
  fi
  sleep 1
done
if [ -f /mmux/mmux-node.log ]; then
  cat /mmux/mmux-node.log >&2
fi
exit 1
"#;
    run_shell_checked(sandbox, "check mmux node readiness", ready_check).await?;

    Ok(())
}

async fn run_scripts(
    sandbox: &Sandbox,
    scripts: &[MicrosandboxScriptAsset],
) -> Result<(), Box<dyn Error + Send + Sync>> {
    for script in scripts {
        run_shell_checked(sandbox, &script.name, &script.name).await?;
    }
    Ok(())
}

async fn run_shell_checked(
    sandbox: &Sandbox,
    label: &str,
    command: &str,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let output = sandbox
        .shell(command)
        .await
        .map_err(|error| format!("{} failed to run: {}", label, error))?;
    let status = output.status();
    if status.success {
        return Ok(());
    }
    let stdout = output
        .stdout()
        .unwrap_or_else(|_| String::from_utf8_lossy(output.stdout_bytes()).into_owned());
    let stderr = output
        .stderr()
        .unwrap_or_else(|_| String::from_utf8_lossy(output.stderr_bytes()).into_owned());
    Err(format!(
        "{} exited with code {}\nstdout:\n{}\nstderr:\n{}",
        label, status.code, stdout, stderr
    )
    .into())
}

fn default_node_config() -> PathBuf {
    PathBuf::from("mmux-node.toml")
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct SandboxConfig {
    #[serde(default)]
    runtime: SandboxRuntimeConfig,
    #[serde(default)]
    network: SandboxNetworkConfig,
    #[serde(skip)]
    has_network_policy: bool,
    #[serde(default)]
    secrets: Vec<SandboxSecret>,
    #[serde(default)]
    volumes: Vec<SandboxVolume>,
    #[serde(default)]
    mounts: Vec<SandboxMount>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct MicrosandboxConfig {
    #[serde(default)]
    runtime: Option<SandboxRuntimeConfig>,
    #[serde(default)]
    assets: MicrosandboxAssetsConfig,
    #[serde(default)]
    patches: Vec<MicrosandboxPatch>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct SandboxRuntimeConfig {
    #[serde(default = "default_image")]
    image: String,
    #[serde(default = "default_memory_mib")]
    memory_mib: u32,
    #[serde(default = "default_cpus")]
    cpus: u8,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct MicrosandboxAssetsConfig {
    #[serde(default)]
    mmux_version: Option<String>,
    #[serde(default)]
    tmux_conf: Option<PathBuf>,
    #[serde(default)]
    scripts_dir: Option<PathBuf>,
    #[serde(default)]
    assets_dir: Option<PathBuf>,
}

impl Default for SandboxRuntimeConfig {
    fn default() -> Self {
        Self {
            image: default_image(),
            memory_mib: default_memory_mib(),
            cpus: default_cpus(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct SandboxNetworkConfig {
    #[serde(default)]
    default_egress: Option<Action>,
    #[serde(default)]
    default_ingress: Option<Action>,
    #[serde(default)]
    egress: Vec<SandboxNetworkRule>,
    #[serde(default)]
    ingress: Vec<SandboxNetworkRule>,
    #[serde(default)]
    trust_host_cas: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct SandboxNetworkRule {
    action: Action,
    #[serde(default)]
    destination_group: Option<DestinationGroup>,
    #[serde(default)]
    destination_domain: Option<String>,
    #[serde(default)]
    destination_domain_suffix: Option<String>,
    #[serde(default)]
    destination_ip: Option<String>,
    #[serde(default)]
    destination_cidr: Option<String>,
    #[serde(default)]
    protocol: Option<Protocol>,
    #[serde(default)]
    ports: Vec<u16>,
}

fn build_network_policy(
    config: &SandboxNetworkConfig,
    controller_url: Option<&str>,
) -> Result<microsandbox::NetworkPolicy, Box<dyn Error + Send + Sync>> {
    let mut policy = build_rule_config_policy(config)?;
    if let Some(controller_url) = controller_url {
        if let Some(port) = controller_host_port(controller_url)? {
            add_host_tcp_port_allow(&mut policy, port);
        }
    }
    Ok(policy)
}

fn build_setup_network_policy() -> Result<microsandbox::NetworkPolicy, Box<dyn Error + Send + Sync>>
{
    NetworkPolicyBuilder::new()
        .default_egress(Action::Allow)
        .default_ingress(Action::Deny)
        .build()
        .map_err(|error| format!("invalid microsandbox setup network policy: {}", error).into())
}

fn build_rule_config_policy(
    config: &SandboxNetworkConfig,
) -> Result<microsandbox::NetworkPolicy, Box<dyn Error + Send + Sync>> {
    let mut builder = NetworkPolicyBuilder::new();
    if let Some(action) = config.default_egress {
        builder = builder.default_egress(action);
    }
    if let Some(action) = config.default_ingress {
        builder = builder.default_ingress(action);
    }
    validate_network_rules("egress", &config.egress)?;
    validate_network_rules("ingress", &config.ingress)?;
    for rule in &config.egress {
        let rule = rule.clone();
        builder = builder.egress(move |egress| apply_network_rule(egress, &rule));
    }
    for rule in &config.ingress {
        let rule = rule.clone();
        builder = builder.ingress(move |ingress| apply_network_rule(ingress, &rule));
    }
    builder
        .build()
        .map_err(|error| format!("invalid microsandbox network policy: {}", error).into())
}

fn validate_network_rules(
    direction: &str,
    rules: &[SandboxNetworkRule],
) -> Result<(), Box<dyn Error + Send + Sync>> {
    for (index, rule) in rules.iter().enumerate() {
        let destination_count = [
            rule.destination_group.is_some(),
            rule.destination_domain.is_some(),
            rule.destination_domain_suffix.is_some(),
            rule.destination_ip.is_some(),
            rule.destination_cidr.is_some(),
        ]
        .into_iter()
        .filter(|selected| *selected)
        .count();
        if destination_count > 1 {
            return Err(format!(
                "invalid microsandbox network {direction} rule #{index}: set only one destination selector"
            )
            .into());
        }
    }
    Ok(())
}

fn apply_network_rule<'a>(
    builder: &'a mut microsandbox_network::policy::RuleBuilder,
    rule: &SandboxNetworkRule,
) -> &'a mut microsandbox_network::policy::RuleBuilder {
    if let Some(protocol) = rule.protocol {
        match protocol {
            Protocol::Tcp => {
                builder.tcp();
            }
            Protocol::Udp => {
                builder.udp();
            }
            Protocol::Icmpv4 => {
                builder.icmpv4();
            }
            Protocol::Icmpv6 => {
                builder.icmpv6();
            }
        }
    }
    for port in &rule.ports {
        builder.port(*port);
    }
    match rule.action {
        Action::Allow => {
            let dest = builder.allow();
            apply_rule_destination(dest, rule);
        }
        Action::Deny => {
            let dest = builder.deny();
            apply_rule_destination(dest, rule);
        }
    }
    builder
}

fn apply_rule_destination(
    builder: microsandbox_network::policy::RuleDestinationBuilder<'_>,
    rule: &SandboxNetworkRule,
) {
    if let Some(group) = rule.destination_group {
        builder.group(group);
    } else if let Some(domain) = rule.destination_domain.as_deref() {
        builder.domain(domain);
    } else if let Some(suffix) = rule.destination_domain_suffix.as_deref() {
        builder.domain_suffix(suffix);
    } else if let Some(ip) = rule.destination_ip.as_deref() {
        builder.ip(ip);
    } else if let Some(cidr) = rule.destination_cidr.as_deref() {
        builder.cidr(cidr);
    } else {
        builder.any();
    }
}

fn add_host_tcp_port_allow(policy: &mut microsandbox::NetworkPolicy, port: u16) {
    let mut rule = Rule::allow_egress(Destination::Group(DestinationGroup::Host));
    rule.protocols.push(Protocol::Tcp);
    rule.ports.push(PortRange::single(port));
    policy.rules.push(rule);
}

fn controller_host_port(controller_url: &str) -> Result<Option<u16>, Box<dyn Error + Send + Sync>> {
    let uri: Uri = controller_url
        .parse()
        .map_err(|error| format!("invalid controller URL '{}': {}", controller_url, error))?;
    let Some(authority) = uri.authority() else {
        return Ok(None);
    };
    if authority.host() != "host.microsandbox.internal" {
        return Ok(None);
    }
    let port = authority
        .port_u16()
        .unwrap_or_else(|| match uri.scheme_str() {
            Some("https") => 443,
            _ => 80,
        });
    Ok(Some(port))
}

#[derive(Debug, Clone)]
struct MicrosandboxScriptAsset {
    name: String,
    content: String,
}

#[derive(Debug, Clone, Deserialize)]
struct SandboxSecret {
    env: String,
    value_from: String,
    allowed_host: String,
    #[serde(default)]
    placeholder: Option<String>,
    #[serde(default = "default_true")]
    inject_headers: bool,
    #[serde(default)]
    inject_basic_auth: bool,
    #[serde(default)]
    inject_query: bool,
    #[serde(default)]
    inject_body: bool,
    #[serde(default = "default_true")]
    require_tls_identity: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct SandboxVolume {
    name: String,
    #[serde(default)]
    quota_mib: Option<u32>,
    #[serde(default)]
    labels: Vec<SandboxLabel>,
}

#[derive(Debug, Clone, Deserialize)]
struct SandboxLabel {
    key: String,
    value: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum SandboxMount {
    Bind {
        guest: String,
        host: PathBuf,
        #[serde(default)]
        readonly: bool,
        #[serde(default)]
        noexec: bool,
        #[serde(default)]
        stat_virtualization: Option<MountStatVirtualization>,
        #[serde(default)]
        host_permissions: Option<MountHostPermissions>,
    },
    Named {
        guest: String,
        name: String,
        #[serde(default)]
        readonly: bool,
        #[serde(default)]
        noexec: bool,
        #[serde(default)]
        stat_virtualization: Option<MountStatVirtualization>,
        #[serde(default)]
        host_permissions: Option<MountHostPermissions>,
    },
    Tmpfs {
        guest: String,
        #[serde(default)]
        size_mib: Option<u32>,
        #[serde(default)]
        readonly: bool,
        #[serde(default)]
        noexec: bool,
    },
    Disk {
        guest: String,
        host: PathBuf,
        #[serde(default)]
        format: Option<DiskImageFormat>,
        #[serde(default)]
        fstype: Option<String>,
        #[serde(default)]
        readonly: bool,
        #[serde(default)]
        noexec: bool,
    },
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum MountStatVirtualization {
    Strict,
    Relaxed,
    Off,
}

impl From<MountStatVirtualization> for StatVirtualization {
    fn from(value: MountStatVirtualization) -> Self {
        match value {
            MountStatVirtualization::Strict => StatVirtualization::Strict,
            MountStatVirtualization::Relaxed => StatVirtualization::Relaxed,
            MountStatVirtualization::Off => StatVirtualization::Off,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum MountHostPermissions {
    Private,
    Mirror,
}

impl From<MountHostPermissions> for HostPermissions {
    fn from(value: MountHostPermissions) -> Self {
        match value {
            MountHostPermissions::Private => HostPermissions::Private,
            MountHostPermissions::Mirror => HostPermissions::Mirror,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum MicrosandboxPatch {
    Text {
        path: String,
        content: String,
        mode: Option<u32>,
        replace: bool,
    },
    Append {
        path: String,
        content: String,
    },
    Mkdir {
        path: String,
        mode: Option<u32>,
    },
    CopyFile {
        src: PathBuf,
        dst: String,
        mode: Option<u32>,
        replace: bool,
    },
    CopyDir {
        src: PathBuf,
        dst: String,
        replace: bool,
    },
    Symlink {
        target: String,
        link: String,
        replace: bool,
    },
    Remove {
        path: String,
    },
}

struct LoadedMicrosandboxConfig {
    base_dir: PathBuf,
    sandbox: SandboxConfig,
    microsandbox: MicrosandboxConfig,
    coder_profiles: Vec<CliProfile>,
}

fn load_microsandbox_config(
    path: &Path,
) -> Result<LoadedMicrosandboxConfig, Box<dyn Error + Send + Sync>> {
    let text = std::fs::read_to_string(path)?;
    let raw: toml::Table = toml::from_str(&text)?;
    let base_dir = path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    let has_network_policy = raw
        .get("sandbox")
        .and_then(|value| value.as_table())
        .and_then(|table| table.get("network"))
        .is_some();
    let mut sandbox: SandboxConfig = match raw.get("sandbox") {
        Some(value) => value.clone().try_into()?,
        None => SandboxConfig::default(),
    };
    sandbox.has_network_policy = has_network_policy;
    let microsandbox = match raw.get("microsandbox") {
        Some(value) => value.clone().try_into()?,
        None => MicrosandboxConfig::default(),
    };
    let coder_profiles = load_coder_profiles(&raw)?;
    Ok(LoadedMicrosandboxConfig {
        base_dir,
        sandbox,
        microsandbox,
        coder_profiles,
    })
}

fn load_coder_profiles(
    table: &toml::Table,
) -> Result<Vec<CliProfile>, Box<dyn Error + Send + Sync>> {
    let mut profiles = mmux_node::default_profiles().read().unwrap().clone();

    if let Some(coder_profiles) = table
        .get("coder_profile")
        .and_then(|value| value.as_table())
    {
        for (name, value) in coder_profiles {
            let profile = load_profile_overlay(name, value, profiles.get(name).cloned())?;
            profiles.insert(name.clone(), profile);
        }
    }

    let mut profiles = profiles.into_values().collect::<Vec<_>>();
    profiles.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(profiles)
}

fn load_profile_overlay(
    name: &str,
    value: &toml::Value,
    base: Option<CliProfile>,
) -> Result<CliProfile, Box<dyn Error + Send + Sync>> {
    let mut merged = match base {
        Some(profile) => toml::Value::try_from(profile)?,
        None => toml::Value::Table(toml::Table::new()),
    };
    merge_toml_value(&mut merged, value.clone());
    let mut profile: CliProfile = merged.try_into()?;
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

fn load_host_scripts(
    config: &MicrosandboxConfig,
    base_dir: &Path,
) -> Result<Vec<MicrosandboxScriptAsset>, Box<dyn Error + Send + Sync>> {
    let Some(scripts_dir) = config.assets.scripts_dir.as_ref() else {
        return Ok(Vec::new());
    };

    let scripts_dir = resolve_host_path(base_dir, scripts_dir.as_path());
    load_script_dir(&scripts_dir, "mmux_sources")
}

fn load_script_dir(
    scripts_dir: &Path,
    scope: &str,
) -> Result<Vec<MicrosandboxScriptAsset>, Box<dyn Error + Send + Sync>> {
    if !scripts_dir.exists() {
        return Ok(Vec::new());
    }

    let mut entries = std::fs::read_dir(scripts_dir)?
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().map(|ty| ty.is_file()).unwrap_or(false))
        .collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.file_name());

    let mut scripts = Vec::with_capacity(entries.len());
    for entry in entries {
        let file_name = entry.file_name().to_string_lossy().to_string();
        let name = format!("{}__{}", scope, file_name);
        let content = std::fs::read_to_string(entry.path())?;
        scripts.push(MicrosandboxScriptAsset { name, content });
    }
    Ok(scripts)
}

fn resolve_host_secret_value(value_from: &str) -> Result<String, Box<dyn Error + Send + Sync>> {
    let env_name = value_from.strip_prefix("host.").ok_or_else(|| {
        format!(
            "unsupported secret source '{}': expected host.ENV_VAR",
            value_from
        )
    })?;
    let value = std::env::var(env_name)
        .map_err(|error| format!("missing host env var {} for secret: {}", env_name, error))?;
    Ok(value)
}

async fn ensure_configured_volumes(
    volumes: &[SandboxVolume],
) -> Result<(), Box<dyn Error + Send + Sync>> {
    for volume in volumes {
        if Volume::get(&volume.name).await.is_ok() {
            continue;
        }
        let mut builder = Volume::builder(&volume.name);
        if let Some(quota_mib) = volume.quota_mib {
            builder = builder.quota(quota_mib);
        }
        for label in &volume.labels {
            builder = builder.label(&label.key, &label.value);
        }
        let _ = builder.create().await?;
    }
    Ok(())
}

fn apply_mount(mount: MountBuilder, spec: &SandboxMount, base_dir: &Path) -> MountBuilder {
    match spec {
        SandboxMount::Bind {
            host,
            readonly,
            noexec,
            stat_virtualization,
            host_permissions,
            ..
        } => {
            let mut mount = mount.bind(resolve_host_path(base_dir, host));
            if *readonly {
                mount = mount.readonly();
            }
            if *noexec {
                mount = mount.noexec();
            }
            if let Some(policy) = stat_virtualization {
                mount = mount.stat_virtualization((*policy).into());
            }
            if let Some(policy) = host_permissions {
                mount = mount.host_permissions((*policy).into());
            }
            mount
        }
        SandboxMount::Named {
            name,
            readonly,
            noexec,
            stat_virtualization,
            host_permissions,
            ..
        } => {
            let mut mount = mount.named(name);
            if *readonly {
                mount = mount.readonly();
            }
            if *noexec {
                mount = mount.noexec();
            }
            if let Some(policy) = stat_virtualization {
                mount = mount.stat_virtualization((*policy).into());
            }
            if let Some(policy) = host_permissions {
                mount = mount.host_permissions((*policy).into());
            }
            mount
        }
        SandboxMount::Tmpfs {
            size_mib,
            readonly,
            noexec,
            ..
        } => {
            let mut mount = mount.tmpfs();
            if let Some(size_mib) = size_mib {
                mount = mount.size(*size_mib);
            }
            if *readonly {
                mount = mount.readonly();
            }
            if *noexec {
                mount = mount.noexec();
            }
            mount
        }
        SandboxMount::Disk {
            host,
            readonly,
            noexec,
            format,
            fstype,
            ..
        } => {
            let mut mount = mount.disk(resolve_host_path(base_dir, host));
            if let Some(format) = format {
                mount = mount.format(*format);
            }
            if let Some(fstype) = fstype {
                mount = mount.fstype(fstype);
            }
            if *readonly {
                mount = mount.readonly();
            }
            if *noexec {
                mount = mount.noexec();
            }
            mount
        }
    }
}

fn mount_guest(spec: &SandboxMount) -> &str {
    match spec {
        SandboxMount::Bind { guest, .. }
        | SandboxMount::Named { guest, .. }
        | SandboxMount::Tmpfs { guest, .. }
        | SandboxMount::Disk { guest, .. } => guest,
    }
}

fn apply_config_patches(
    mut builder: microsandbox::sandbox::PatchBuilder,
    patches: &[MicrosandboxPatch],
    base_dir: &Path,
) -> microsandbox::sandbox::PatchBuilder {
    for patch in patches {
        builder = match patch {
            MicrosandboxPatch::Text {
                path,
                content,
                mode,
                replace,
            } => builder.text(path, content, *mode, *replace),
            MicrosandboxPatch::Append { path, content } => builder.append(path, content),
            MicrosandboxPatch::Mkdir { path, mode } => builder.mkdir(path, *mode),
            MicrosandboxPatch::CopyFile {
                src,
                dst,
                mode,
                replace,
            } => builder.copy_file(resolve_host_path(base_dir, src), dst, *mode, *replace),
            MicrosandboxPatch::CopyDir { src, dst, replace } => {
                builder.copy_dir(resolve_host_path(base_dir, src), dst, *replace)
            }
            MicrosandboxPatch::Symlink {
                target,
                link,
                replace,
            } => builder.symlink(target, link, *replace),
            MicrosandboxPatch::Remove { path } => builder.remove(path),
        };
    }
    builder
}

fn resolve_host_path(base_dir: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base_dir.join(path)
    }
}

fn default_true() -> bool {
    true
}

fn default_memory_mib() -> u32 {
    1024
}

fn default_cpus() -> u8 {
    2
}

fn default_image() -> String {
    DEFAULT_IMAGE.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use microsandbox_network::policy::Direction;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn load_microsandbox_config_reads_launch_dirs() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let base_dir = std::env::temp_dir().join(format!("mmux-msb-test-{}", unique));
        fs::create_dir_all(base_dir.join("mmux_sources/scripts")).unwrap();
        fs::create_dir_all(base_dir.join("mmux_sources/assets")).unwrap();

        let config_path = base_dir.join("mmux-node.toml");
        fs::write(
            &config_path,
            r#"
[microsandbox.assets]
mmux_version = "v0.1.0"
scripts_dir = "./mmux_sources/scripts"
assets_dir = "./mmux_sources/assets"
"#,
        )
        .unwrap();

        let loaded = load_microsandbox_config(&config_path).unwrap();
        assert_eq!(
            loaded.microsandbox.assets.scripts_dir.as_deref(),
            Some(Path::new("./mmux_sources/scripts"))
        );
        assert_eq!(
            loaded.microsandbox.assets.assets_dir.as_deref(),
            Some(Path::new("./mmux_sources/assets"))
        );
        assert!(loaded.microsandbox.assets.tmux_conf.is_none());
        assert_eq!(
            loaded.microsandbox.assets.mmux_version.as_deref(),
            Some("v0.1.0")
        );
        let _ = fs::remove_dir_all(base_dir);
    }

    #[test]
    fn sandbox_config_reads_volumes_and_mounts() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let base_dir = std::env::temp_dir().join(format!("mmux-sandbox-mounts-{}", unique));
        fs::create_dir_all(&base_dir).unwrap();

        let config_path = base_dir.join("mmux-node.toml");
        fs::write(
            &config_path,
            r#"
[[sandbox.volumes]]
name = "my-data"
quota_mib = 5120

[[sandbox.volumes.labels]]
key = "env"
value = "dev"

[[sandbox.mounts]]
kind = "named"
guest = "/data"
name = "my-data"
readonly = true
noexec = true
"#,
        )
        .unwrap();

        let loaded = load_microsandbox_config(&config_path).unwrap();

        let volume = loaded.sandbox.volumes.first().expect("volume loaded");
        assert_eq!(volume.name, "my-data");
        assert_eq!(volume.quota_mib, Some(5120));
        assert_eq!(
            volume.labels.first().map(|label| label.key.as_str()),
            Some("env")
        );
        assert_eq!(
            volume.labels.first().map(|label| label.value.as_str()),
            Some("dev")
        );
        let mount = loaded.sandbox.mounts.first().expect("mount loaded");
        match mount {
            SandboxMount::Named {
                guest,
                name,
                readonly,
                noexec,
                ..
            } => {
                assert_eq!(guest, "/data");
                assert_eq!(name, "my-data");
                assert!(*readonly);
                assert!(*noexec);
            }
            _ => panic!("expected named mount"),
        }

        let _ = fs::remove_dir_all(base_dir);
    }

    #[test]
    fn setup_microsandbox_config_parses_kimi_profile() {
        let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let config_path = manifest_dir
            .join("../../..")
            .join("example-backends/microsandbox/microsandbox-setup.toml");

        let loaded = load_microsandbox_config(&config_path).unwrap();
        let kimi = loaded
            .coder_profiles
            .iter()
            .find(|profile| profile.name == "kimi")
            .expect("kimi profile loaded");

        assert_eq!(kimi.cmd.as_deref(), Some("kimi"));
        assert!(kimi
            .busy_indicators
            .iter()
            .any(|marker| marker == "ctrl-s to steer"));
        assert!(kimi.launch.is_none());
        assert_eq!(
            loaded
                .microsandbox
                .runtime
                .as_ref()
                .map(|runtime| runtime.image.as_str()),
            Some(DEFAULT_IMAGE)
        );
        assert!(!loaded.sandbox.has_network_policy);
        assert!(loaded.sandbox.secrets.is_empty());
        let policy = build_setup_network_policy().unwrap();
        assert_eq!(policy.default_egress, Action::Allow);
        assert_eq!(policy.default_ingress, Action::Deny);
        assert!(policy.rules.is_empty());
    }

    #[test]
    fn root_microsandbox_example_config_parses_runtime_and_setup_assets() {
        let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let config_path = manifest_dir
            .join("../../..")
            .join("mmux-microsandbox.toml.example");

        let loaded = load_microsandbox_config(&config_path).unwrap();
        assert!(loaded
            .coder_profiles
            .iter()
            .all(|profile| profile.launch.is_none()));
        assert_eq!(loaded.sandbox.network.default_egress, Some(Action::Deny));
        assert_eq!(loaded.sandbox.network.default_ingress, Some(Action::Deny));
        assert_eq!(
            loaded
                .sandbox
                .secrets
                .first()
                .map(|secret| secret.allowed_host.as_str()),
            Some("host.microsandbox.internal")
        );
        let policy = build_network_policy(
            &loaded.sandbox.network,
            Some("http://host.microsandbox.internal:3000"),
        )
        .unwrap();
        assert_eq!(policy.rules.len(), 1);
        assert!(policy_has_host_tcp_port_allow(&policy, 3000));
    }

    #[test]
    fn runtime_config_has_controller_only_policy_and_no_setup_scripts() {
        let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let config_path = manifest_dir
            .join("../../..")
            .join("example-backends/microsandbox/mmux.toml");

        let loaded = load_microsandbox_config(&config_path).unwrap();

        assert!(loaded.microsandbox.assets.scripts_dir.is_none());
        assert!(loaded
            .coder_profiles
            .iter()
            .all(|profile| profile.launch.is_none()));
        let policy = build_network_policy(
            &loaded.sandbox.network,
            Some("http://host.microsandbox.internal:3000"),
        )
        .unwrap();
        assert_eq!(policy.rules.len(), 1);
        assert!(policy_has_host_tcp_port_allow(&policy, 3000));
    }

    #[test]
    fn controller_host_rule_uses_controller_url_port_only_for_microsandbox_host() {
        let config = SandboxNetworkConfig {
            default_egress: Some(Action::Deny),
            default_ingress: Some(Action::Deny),
            ..Default::default()
        };

        let policy = build_network_policy(&config, Some("http://host.microsandbox.internal:3210"))
            .expect("policy");
        assert!(policy_has_host_tcp_port_allow(&policy, 3210));

        let policy =
            build_network_policy(&config, Some("http://example.com:3210")).expect("policy");
        assert!(!policy_has_host_tcp_port_allow(&policy, 3210));

        let policy = build_network_policy(&config, None).expect("policy");
        assert!(!policy_has_host_tcp_port_allow(&policy, 3210));
    }

    #[test]
    fn network_rules_support_domain_destinations_and_reject_ambiguous_destinations() {
        let config = SandboxNetworkConfig {
            default_egress: Some(Action::Deny),
            default_ingress: Some(Action::Deny),
            egress: vec![
                SandboxNetworkRule {
                    action: Action::Allow,
                    destination_domain: Some("api.openai.com".into()),
                    protocol: Some(Protocol::Udp),
                    ports: vec![53],
                    ..empty_network_rule()
                },
                SandboxNetworkRule {
                    action: Action::Allow,
                    destination_domain: Some("api.openai.com".into()),
                    protocol: Some(Protocol::Tcp),
                    ports: vec![443],
                    ..empty_network_rule()
                },
                SandboxNetworkRule {
                    action: Action::Allow,
                    destination_domain_suffix: Some(".openai.com".into()),
                    protocol: Some(Protocol::Tcp),
                    ports: vec![443],
                    ..empty_network_rule()
                },
            ],
            ..Default::default()
        };

        let policy = build_network_policy(&config, None).expect("policy");

        assert!(policy_has_domain_port_allow(
            &policy,
            "api.openai.com",
            Protocol::Udp,
            53
        ));
        assert!(policy_has_domain_port_allow(
            &policy,
            "api.openai.com",
            Protocol::Tcp,
            443
        ));
        assert!(policy_has_domain_suffix_port_allow(
            &policy,
            "openai.com",
            Protocol::Tcp,
            443
        ));

        let config = SandboxNetworkConfig {
            egress: vec![SandboxNetworkRule {
                action: Action::Allow,
                destination_group: Some(DestinationGroup::Public),
                destination_domain: Some("api.openai.com".into()),
                ..empty_network_rule()
            }],
            ..Default::default()
        };
        let error = build_network_policy(&config, None).unwrap_err();
        assert!(error
            .to_string()
            .contains("set only one destination selector"));
    }

    fn empty_network_rule() -> SandboxNetworkRule {
        SandboxNetworkRule {
            action: Action::Allow,
            destination_group: None,
            destination_domain: None,
            destination_domain_suffix: None,
            destination_ip: None,
            destination_cidr: None,
            protocol: None,
            ports: Vec::new(),
        }
    }

    fn policy_has_host_tcp_port_allow(policy: &microsandbox::NetworkPolicy, port: u16) -> bool {
        policy.rules.iter().any(|rule| {
            rule.direction == Direction::Egress
                && rule.action == Action::Allow
                && matches!(rule.destination, Destination::Group(DestinationGroup::Host))
                && rule.protocols == vec![Protocol::Tcp]
                && rule.ports == vec![PortRange::single(port)]
        })
    }

    fn policy_has_domain_port_allow(
        policy: &microsandbox::NetworkPolicy,
        domain: &str,
        protocol: Protocol,
        port: u16,
    ) -> bool {
        policy.rules.iter().any(|rule| {
            rule.direction == Direction::Egress
                && rule.action == Action::Allow
                && matches!(
                    &rule.destination,
                    Destination::Domain(name) if name.as_str() == domain
                )
                && rule.protocols == vec![protocol]
                && rule.ports == vec![PortRange::single(port)]
        })
    }

    fn policy_has_domain_suffix_port_allow(
        policy: &microsandbox::NetworkPolicy,
        suffix: &str,
        protocol: Protocol,
        port: u16,
    ) -> bool {
        policy.rules.iter().any(|rule| {
            rule.direction == Direction::Egress
                && rule.action == Action::Allow
                && matches!(
                    &rule.destination,
                    Destination::DomainSuffix(name) if name.as_str() == suffix
                )
                && rule.protocols == vec![protocol]
                && rule.ports == vec![PortRange::single(port)]
        })
    }
}
