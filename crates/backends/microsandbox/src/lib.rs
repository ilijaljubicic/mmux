use clap::{Parser, Subcommand};
use microsandbox::sandbox::{DiskImageFormat, HostPermissions, MountBuilder, StatVirtualization};
use microsandbox::{Sandbox, Snapshot, Volume};
use microsandbox::snapshot::ExportOpts;
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
    Launch(LaunchArgs),
    Snapshot(SnapshotArgs),
    SnapshotExport(SnapshotExportArgs),
    SnapshotImport(SnapshotImportArgs),
    Status(SharedArgs),
    Resume(SharedArgs),
    Stop(SharedArgs),
    Logs(SharedArgs),
}

#[derive(Debug, Clone, Parser)]
pub struct SharedArgs {
    #[arg(long, default_value = "mmux-node")]
    pub name: String,
    #[arg(long, default_value = DEFAULT_IMAGE)]
    pub image: String,
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
    let script_assets = if args.snapshot.is_some() {
        Vec::new()
    } else {
        load_host_scripts(&microsandbox_config.config, &microsandbox_config.base_dir)?
    };
    let profile_script_assets = if args.snapshot.is_some() {
        Vec::new()
    } else {
        load_profile_scripts(&microsandbox_config.coder_profiles, &microsandbox_config.base_dir)?
    };
    create_sandbox(
        &args.shared,
        &args.node_config,
        &args.controller_url,
        &args.node_id,
        &args.node_name,
        args.snapshot.as_deref(),
        &microsandbox_config.config,
        &microsandbox_config.coder_profiles,
        &microsandbox_config.base_dir,
        &script_assets,
        &profile_script_assets,
    )
    .await?;

    Ok(LaunchReport {
        name: args.shared.name,
        image: args.shared.image,
        node_id: args.node_id,
        controller_url: args.controller_url,
        snapshot: args.snapshot,
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
    Snapshot::export(snapshot_path.to_string_lossy().as_ref(), &bundle, ExportOpts::default())
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
        if let Ok(output) = sandbox.shell("cat /tmp/mmux-node.log 2>/dev/null || true").await {
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

async fn create_sandbox(
    shared: &SharedArgs,
    node_config: &Path,
    controller_url: &str,
    node_id: &str,
    node_name: &str,
    snapshot: Option<&str>,
    microsandbox_config: &MicrosandboxConfig,
    coder_profiles: &[CliProfile],
    config_base_dir: &Path,
    script_assets: &[MicrosandboxScriptAsset],
    profile_script_assets: &[MicrosandboxScriptAsset],
) -> Result<(), Box<dyn Error + Send + Sync>> {
    ensure_configured_volumes(&microsandbox_config.volumes).await?;
    let mut policy: microsandbox::NetworkPolicy =
        microsandbox_config.network.network_policy.into();
    for domain in &microsandbox_config.network.deny_domain {
        policy = policy
            .deny_domain(domain)
            .map_err(|error| format!("invalid denied domain '{}': {}", domain, error))?;
    }
    for suffix in &microsandbox_config.network.deny_domain_suffix {
        policy = policy
            .deny_domain_suffix(suffix)
            .map_err(|error| format!("invalid denied domain suffix '{}': {}", suffix, error))?;
    }

    let mut builder = Sandbox::builder(&shared.name)
        .memory(microsandbox_config.runtime.memory_mib)
        .cpus(microsandbox_config.runtime.cpus)
        .replace()
        .env("MMUX_CONTROLLER_URL", controller_url)
        .env("MMUX_NODE_ID", node_id)
        .env("MMUX_NODE_NAME", node_name)
        .env("MMUX_NODE_CONFIG", "/etc/mmux/mmux-node.toml")
        .env("MMUX_POLL_INTERVAL_MS", "500");
    if let Some(snapshot_ref) = snapshot {
        builder = builder.from_snapshot(snapshot_ref.to_string());
    } else {
        builder = builder.image(shared.image.clone());
    }
    builder = builder.network(|network| {
        let mut network = network.policy(policy);
        if let Some(max_connections) = microsandbox_config.network.max_connections {
            network = network.max_connections(max_connections);
        }
        network = network.trust_host_cas(microsandbox_config.network.trust_host_cas);
        network
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
    for secret in &microsandbox_config.secrets {
        let value = resolve_host_secret_value(&secret.value_from)?;
        builder = builder.env(&secret.env, value.clone());
        builder = builder.secret(|s| {
            let mut s = s.env(&secret.env).value(value).allow_host(&secret.allowed_host);
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
    for mount in &microsandbox_config.mounts {
        builder = builder.volume(mount_guest(mount), |v| apply_mount(v, mount, config_base_dir));
    }

    if snapshot.is_none() {
        for script in script_assets {
            builder = builder.script(&script.name, &script.content);
        }
        for script in profile_script_assets {
            builder = builder.script(&script.name, &script.content);
        }
    }

    let sandbox = if snapshot.is_some() {
        builder.create_detached().await?
    } else {
        let mmux_binary = resolve_host_path(config_base_dir, &microsandbox_config.assets.mmux_binary);
        let tmux_conf = resolve_host_path(config_base_dir, &microsandbox_config.assets.tmux_conf);
        let mmux_assets_dir = microsandbox_config
            .assets
            .assets_dir
            .as_ref()
            .map(|path| resolve_host_path(config_base_dir, path));
        builder
            .patch(|p: microsandbox::sandbox::PatchBuilder| {
                let p = p
                    .mkdir("/mmux", Some(0o755))
                    .mkdir("/etc/mmux", Some(0o755))
                    .mkdir("/workspace", Some(0o755))
                    .copy_file(mmux_binary, "/usr/local/bin/mmux", Some(0o755), true)
                    .copy_file(node_config, "/etc/mmux/mmux-node.toml", None, true)
                    .copy_file(tmux_conf, "/mmux/tmux.conf", None, true)
                    .text("/etc/tmux.conf", "source-file /mmux/tmux.conf\n", Some(0o644), false);
                let p = if let Some(mmux_assets_dir) = mmux_assets_dir.as_ref() {
                    p.copy_dir(mmux_assets_dir, "/mmux/mmux_sources/assets", true)
                } else {
                    p
                };
                let p = apply_profile_assets(p, coder_profiles, config_base_dir);
                apply_config_patches(p, &microsandbox_config.patches, config_base_dir)
            })
            .create_detached()
            .await?
    };

    if snapshot.is_none() {
        run_scripts(&sandbox, script_assets).await?;
        run_scripts(&sandbox, profile_script_assets).await?;
    }
    launch_mmux_node(&sandbox).await?;
    sandbox.detach().await;

    Ok(())
}

async fn launch_mmux_node(sandbox: &Sandbox) -> Result<(), Box<dyn Error + Send + Sync>> {
    let command = format!(
        r#"set -eu
mkdir -p /tmp
nohup /usr/local/bin/mmux node \
  --controller-url "$MMUX_CONTROLLER_URL" \
  --node-id "$MMUX_NODE_ID" \
  --node-name "$MMUX_NODE_NAME" \
  --node-config "$MMUX_NODE_CONFIG" \
  --poll-interval-ms "$MMUX_POLL_INTERVAL_MS" \
  >/tmp/mmux-node.log 2>&1 &
"#
    );

    sandbox
        .shell(command)
        .await
        .map_err(|error| format!("failed to launch mmux node: {}", error))?;

    let ready_check = r#"for _ in 1 2 3 4 5; do
  if ps -ef 2>/dev/null | grep '[m]mux node' >/dev/null 2>&1; then
    exit 0
  fi
  sleep 1
done
exit 1
"#;
    sandbox
        .shell(ready_check)
        .await
        .map_err(|error| format!("mmux node did not stay running: {}", error))?;

    Ok(())
}

async fn run_scripts(
    sandbox: &Sandbox,
    scripts: &[MicrosandboxScriptAsset],
) -> Result<(), Box<dyn Error + Send + Sync>> {
    for script in scripts {
        sandbox
            .shell(&script.name)
            .await
            .map_err(|error| format!("script '{}' failed: {}", script.name, error))?;
    }
    Ok(())
}

fn default_node_config() -> PathBuf {
    PathBuf::from("mmux-node.toml")
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum NetworkPolicyKind {
    NonLocal,
    PublicOnly,
    AllowAll,
    None,
}

impl Default for NetworkPolicyKind {
    fn default() -> Self {
        Self::NonLocal
    }
}

impl From<NetworkPolicyKind> for microsandbox::NetworkPolicy {
    fn from(value: NetworkPolicyKind) -> Self {
        match value {
            NetworkPolicyKind::NonLocal => microsandbox::NetworkPolicy::non_local(),
            NetworkPolicyKind::PublicOnly => microsandbox::NetworkPolicy::public_only(),
            NetworkPolicyKind::AllowAll => microsandbox::NetworkPolicy::allow_all(),
            NetworkPolicyKind::None => microsandbox::NetworkPolicy::none(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
struct MicrosandboxConfig {
    #[serde(default)]
    runtime: MicrosandboxRuntimeConfig,
    #[serde(default)]
    assets: MicrosandboxAssetsConfig,
    #[serde(default)]
    network: MicrosandboxNetworkConfig,
    #[serde(default)]
    secrets: Vec<MicrosandboxSecret>,
    #[serde(default)]
    volumes: Vec<MicrosandboxVolume>,
    #[serde(default)]
    mounts: Vec<MicrosandboxMount>,
    #[serde(default)]
    patches: Vec<MicrosandboxPatch>,
}

#[derive(Debug, Clone, Deserialize)]
struct MicrosandboxRuntimeConfig {
    #[serde(default = "default_memory_mib")]
    memory_mib: u32,
    #[serde(default = "default_cpus")]
    cpus: u8,
}

#[derive(Debug, Clone, Deserialize)]
struct MicrosandboxAssetsConfig {
    #[serde(default = "default_mmux_binary")]
    mmux_binary: PathBuf,
    #[serde(default = "default_tmux_conf")]
    tmux_conf: PathBuf,
    #[serde(default)]
    scripts_dir: Option<PathBuf>,
    #[serde(default)]
    assets_dir: Option<PathBuf>,
}

impl Default for MicrosandboxAssetsConfig {
    fn default() -> Self {
        Self {
            mmux_binary: default_mmux_binary(),
            tmux_conf: default_tmux_conf(),
            scripts_dir: None,
            assets_dir: None,
        }
    }
}

impl Default for MicrosandboxRuntimeConfig {
    fn default() -> Self {
        Self {
            memory_mib: default_memory_mib(),
            cpus: default_cpus(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
struct MicrosandboxNetworkConfig {
    #[serde(default)]
    network_policy: NetworkPolicyKind,
    #[serde(default)]
    #[serde(alias = "deny_domains")]
    deny_domain: Vec<String>,
    #[serde(default)]
    #[serde(alias = "deny_domain_suffixes")]
    deny_domain_suffix: Vec<String>,
    #[serde(default)]
    max_connections: Option<usize>,
    #[serde(default)]
    trust_host_cas: bool,
}

#[derive(Debug, Clone)]
struct MicrosandboxScriptAsset {
    name: String,
    content: String,
}

#[derive(Debug, Clone, Deserialize)]
struct MicrosandboxSecret {
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
struct MicrosandboxVolume {
    name: String,
    #[serde(default)]
    quota_mib: Option<u32>,
    #[serde(default)]
    labels: Vec<MicrosandboxLabel>,
}

#[derive(Debug, Clone, Deserialize)]
struct MicrosandboxLabel {
    key: String,
    value: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum MicrosandboxMount {
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
    config: MicrosandboxConfig,
    coder_profiles: Vec<CliProfile>,
}

fn load_microsandbox_config(
    path: &Path,
) -> Result<LoadedMicrosandboxConfig, Box<dyn Error + Send + Sync>> {
    let text = std::fs::read_to_string(path)?;
    let raw: toml::Table = toml::from_str(&text)?;
    let base_dir = path.parent().unwrap_or_else(|| Path::new(".")).to_path_buf();
    let config = match raw.get("microsandbox") {
        Some(value) => value.clone().try_into()?,
        None => MicrosandboxConfig::default(),
    };
    let coder_profiles = load_coder_profiles(&raw)?;
    Ok(LoadedMicrosandboxConfig {
        base_dir,
        config,
        coder_profiles,
    })
}

fn load_coder_profiles(
    table: &toml::Table,
) -> Result<Vec<CliProfile>, Box<dyn Error + Send + Sync>> {
    let mut profiles = Vec::new();
    let Some(coder_profiles) = table.get("coder_profile").and_then(|value| value.as_table()) else {
        return Ok(profiles);
    };

    for (name, value) in coder_profiles {
        let mut profile: CliProfile = value.clone().try_into()?;
        if profile.name.is_empty() {
            profile.name = name.clone();
        }
        profiles.push(profile);
    }
    Ok(profiles)
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

fn load_profile_scripts(
    profiles: &[CliProfile],
    base_dir: &Path,
) -> Result<Vec<MicrosandboxScriptAsset>, Box<dyn Error + Send + Sync>> {
    let mut scripts = Vec::new();
    let mut profiles = profiles.to_vec();
    profiles.sort_by(|a, b| a.name.cmp(&b.name));
    for profile in profiles {
        let Some(launch) = profile.launch.as_ref() else {
            continue;
        };
        let Some(scripts_dir) = launch.scripts_dir.as_ref() else {
            continue;
        };
        let scripts_dir = resolve_host_path(base_dir, Path::new(scripts_dir));
        scripts.extend(load_script_dir(&scripts_dir, &format!("profile_{}", profile.name))?);
    }
    Ok(scripts)
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

fn apply_profile_assets(
    mut builder: microsandbox::sandbox::PatchBuilder,
    profiles: &[CliProfile],
    base_dir: &Path,
) -> microsandbox::sandbox::PatchBuilder {
    let mut profiles = profiles.to_vec();
    profiles.sort_by(|a, b| a.name.cmp(&b.name));
    for profile in profiles {
        let Some(launch) = profile.launch.as_ref() else {
            continue;
        };
        let Some(assets_dir) = launch.assets_dir.as_ref() else {
            continue;
        };
        let assets_dir = resolve_host_path(base_dir, Path::new(assets_dir));
        if assets_dir.exists() {
            let guest_dir = format!("/mmux/profile_sources/{}/assets", profile.name);
            builder = builder.copy_dir(assets_dir, &guest_dir, true);
        }
    }
    builder
}

fn resolve_host_secret_value(value_from: &str) -> Result<String, Box<dyn Error + Send + Sync>> {
    let env_name = value_from
        .strip_prefix("host.")
        .ok_or_else(|| format!("unsupported secret source '{}': expected host.ENV_VAR", value_from))?;
    let value = std::env::var(env_name)
        .map_err(|error| format!("missing host env var {} for secret: {}", env_name, error))?;
    Ok(value)
}

async fn ensure_configured_volumes(
    volumes: &[MicrosandboxVolume],
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

fn apply_mount(mount: MountBuilder, spec: &MicrosandboxMount, base_dir: &Path) -> MountBuilder {
    match spec {
        MicrosandboxMount::Bind {
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
        MicrosandboxMount::Named {
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
        MicrosandboxMount::Tmpfs {
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
        MicrosandboxMount::Disk {
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

fn mount_guest(spec: &MicrosandboxMount) -> &str {
    match spec {
        MicrosandboxMount::Bind { guest, .. }
        | MicrosandboxMount::Named { guest, .. }
        | MicrosandboxMount::Tmpfs { guest, .. }
        | MicrosandboxMount::Disk { guest, .. } => guest,
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

fn default_mmux_binary() -> PathBuf {
    PathBuf::from("./.artifacts/mmux")
}

fn default_tmux_conf() -> PathBuf {
    PathBuf::from("./tmux.conf")
}

#[cfg(test)]
mod tests {
    use super::*;
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
        fs::create_dir_all(base_dir.join("profile_sources/codex/scripts")).unwrap();
        fs::create_dir_all(base_dir.join("profile_sources/codex/assets")).unwrap();

        let config_path = base_dir.join("mmux-node.toml");
        fs::write(
            &config_path,
            r#"
[microsandbox.assets]
mmux_binary = "./.artifacts/mmux"
scripts_dir = "./mmux_sources/scripts"
assets_dir = "./mmux_sources/assets"
tmux_conf = "./mmux_sources/assets/tmux.conf"

[coder_profile.codex]
name = "codex"
cmd = "codex"
prompt_indicator = "›"
busy_indicators = ["• Working"]
approve_keys = "y Enter"
reject_keys = "n Enter"
cancel_keys = "C-c"
escape_keys = "Escape"

[coder_profile.codex.launch]
scripts_dir = "./profile_sources/codex/scripts"
assets_dir = "./profile_sources/codex/assets"
"#,
        )
        .unwrap();

        let loaded = load_microsandbox_config(&config_path).unwrap();
        assert_eq!(
            loaded.config.assets.scripts_dir.as_deref(),
            Some(Path::new("./mmux_sources/scripts"))
        );
        assert_eq!(
            loaded.config.assets.assets_dir.as_deref(),
            Some(Path::new("./mmux_sources/assets"))
        );
        assert_eq!(loaded.coder_profiles.len(), 1);
        let profile = &loaded.coder_profiles[0];
        assert_eq!(profile.name, "codex");
        assert_eq!(
            profile
                .launch
                .as_ref()
                .and_then(|launch| launch.scripts_dir.as_deref()),
            Some("./profile_sources/codex/scripts")
        );
        assert_eq!(
            profile
                .launch
                .as_ref()
                .and_then(|launch| launch.assets_dir.as_deref()),
            Some("./profile_sources/codex/assets")
        );
    }
}
