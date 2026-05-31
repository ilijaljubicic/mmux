# Microsandbox mmux Backend Example

This directory contains the example config and Make targets for running an
`mmux node` inside Microsandbox.

The host builds and runs the `mmux-microsandbox-node` launcher crate. That
launcher creates or resumes a Microsandbox instance, injects the node config
and setup assets, then starts `mmux node` inside the guest.

The guest installs `mmux` from source using the `[microsandbox.assets]`
`mmux_source` entry in `mmux.toml`:

```toml
mmux_source = { repo = "https://github.com/ilijaljubicic/mmux.git", ref = "v0.1.0" }
```

Use a branch or commit for development, or a release tag such as `v0.1.0` for
reproducible sandbox launches. If you point the ref at a private repository,
the sandbox needs credentials that can fetch it.

When you want to avoid redoing the guest setup on every launch, create a
snapshot bundle from the prepared sandbox and launch from that bundle
instead. Bundle launches skip the setup scripts and rootfs patches, so the
prepared sandbox must already contain the installed mmux binary, node config,
and guest toolchain state.

## Layout

```text
mmux_sources/
  assets/
    tmux.conf
  scripts/
    00_setup.sh
    05_tmux_plugins.sh
    10_devtools.sh
    20_toolchains.sh
    25_install_mmux.sh

profile_sources/
  codex/
    assets/
    scripts/
      00_install.sh
  opencode/
    assets/
    scripts/
      00_install.sh
  kimi/
    assets/
    scripts/
      00_install.sh
  claude/
    assets/
    scripts/
      00_install.sh
```

`mmux_sources/scripts` contains the shared sandbox setup. The profile-specific
`profile_sources/<name>/scripts` directories contain the per-coder install
steps, so `codex`, `opencode`, `kimi`, and `claude` can be prepared
independently. The backend registers every script and runs them in alphabetical
order, first the shared scripts and then the profile-specific ones.

`mmux_sources/assets/tmux.conf` is copied into `/mmux/tmux.conf` and sourced
from `/etc/tmux.conf`, so a root SSH shell inside the sandbox picks up the same
tmux defaults automatically.

The sandbox also bootstraps `tmux-resurrect` and `tmux-continuum` into
`/mmux/tmux/plugins` from `mmux_sources/scripts/05_tmux_plugins.sh`. That gives
you a simple tmux save/restore POC across stop/start while keeping the config
rooted at `/mmux/tmux.conf`.

## Run

```bash
export MMUX_TOKEN="...controller bearer token..."
cd /mnt/Radni/aitools/mmux/example-backends/microsandbox
make build
make launch CONTROLLER_URL="http://<controller-host>:3000" NODE_CONFIG="mmux.toml"
```

`<controller-host>` is a placeholder, not a checked-in address. Use a DNS name
or host alias that resolves from inside the sandbox. The default
`controller.mmux.local` in the example Makefile and `allowed_host` in
`mmux.toml` are intentionally neutral and should be changed together for your
deployment.

To capture and export it as a bundle:

```bash
make bundle-export SANDBOX=mmux-node SNAPSHOT_NAME=mmux-node-seed BUNDLE=.artifacts/mmux-node-seed.tar.zst
```

`SANDBOX` is required for `bundle-export` because the snapshot is tied to a
specific sandbox instance. If you omit it, the Makefile prints the exact
example invocation above.

To import an exported bundle and launch from that imported snapshot in one go:

```bash
make bundle-launch BUNDLE=.artifacts/mmux-node-seed.tar.zst CONTROLLER_URL="http://<controller-host>:3000" NODE_CONFIG="mmux.toml"
```

## Node config

`mmux.toml` owns the Microsandbox node config plus shared setup and per-profile
launch extensions. It should not restate built-in coder profile fields unless
you intentionally want to replace them:

```toml
[microsandbox.runtime]
memory_mib = 1024
cpus = 2

[microsandbox.assets]
mmux_source = { repo = "https://github.com/ilijaljubicic/mmux.git", ref = "v0.1.0" }
scripts_dir = "./mmux_sources/scripts"
assets_dir = "./mmux_sources/assets"
tmux_conf = "./mmux_sources/assets/tmux.conf"

[coder_profile.codex.launch]
scripts_dir = "./profile_sources/codex/scripts"
assets_dir = "./profile_sources/codex/assets"

[coder_profile.claude.launch]
scripts_dir = "./profile_sources/claude/scripts"
assets_dir = "./profile_sources/claude/assets"
```

Coder interaction profiles are built into mmux. The Microsandbox TOML only
needs backend-specific launch sections for the profiles that require sandbox
setup. Omitted fields keep their built-in values; scalar and list fields that
you set replace the built-in value. The backend loads shared scripts from
`mmux_sources/scripts`, then loads each profile launch directory in
profile-name order. Host-side paths for
`copy_file`, `copy_dir`, and launch directories are resolved relative to the
directory that contains `mmux.toml`.

Secrets also live in `mmux.toml`. Use `[[microsandbox.secrets]]` entries
with a guest env name, a host env reference like `host.MMUX_TOKEN`, and an
allowed host. The Rust backend resolves the host env var and injects the
secret through the Microsandbox SDK. The controller token is injected this way
into `MMUX_CONTROLLER_TOKEN`. For the example launch, export `MMUX_TOKEN` on
the host before running `make launch`, and set `allowed_host` to the hostname
used in `CONTROLLER_URL`.

Volumes are declared separately and mounts consume them:

```toml
[[microsandbox.volumes]]
name = "my-data"
quota_mib = 5120

[[microsandbox.mounts]]
kind = "named"
guest = "/data"
name = "my-data"
readonly = false
noexec = false
```

Patches live under `[[microsandbox.patches]]` and are passed through to the
SDK as `text`, `append`, `mkdir`, `copy_file`, `copy_dir`, `symlink`, and
`remove` operations.

## Host prerequisites

Before building this backend, install the native package that provides
`libcap-ng.so.0`:

```bash
sudo apt update
sudo apt install -y libcap-ng-dev
```
