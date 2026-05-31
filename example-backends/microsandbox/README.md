# Microsandbox mmux Backend Example

This directory contains the example config and Make targets for running an
`mmux node` inside Microsandbox.

The host builds and runs the `mmux-microsandbox-node` launcher crate. That
launcher creates or resumes a Microsandbox instance, injects the node config
and configured assets, then starts `mmux node` inside the guest.

For first-time setup from a stock image, the guest installs `mmux` from source
using the `[microsandbox.assets]` `mmux_source` entry in
`mmux-setup.toml`:

```toml
mmux_source = { repo = "https://github.com/ilijaljubicic/mmux.git", ref = "v0.1.0" }
```

Use a branch or commit for development, or a release tag such as `v0.1.0` for
reproducible sandbox launches. If you point the ref at a private repository,
the sandbox needs credentials that can fetch it.

To avoid installer network access at launch time, use either a prepared
snapshot or a prepared Docker/OCI image. Bundle launches skip setup scripts and
rootfs patches, so the prepared sandbox must already contain the installed
mmux binary, node config, and guest toolchain state. Image launches with
`mmux-image.toml.example` also skip setup scripts by config, so the image must
already contain `tmux`, `mmux`, and the coder CLIs you plan to use.

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

Choose one launch path:

```bash
# First-time preparation from a stock image. This temporarily allows DNS plus
# public HTTP/HTTPS so apt/curl/git/npm/cargo installers can run.
export MMUX_TOKEN="...controller bearer token..."
cd /mnt/Radni/aitools/mmux/example-backends/microsandbox
make build
make launch NODE_CONFIG="mmux-setup.toml"
```

```bash
# Runtime from a prepared Docker/OCI image. The image already contains tmux,
# mmux, and the coder CLIs, so launch does not open installer network access.
export MMUX_TOKEN="...controller bearer token..."
cd /mnt/Radni/aitools/mmux/example-backends/microsandbox
make build
cp mmux-image.toml.example mmux-image.toml
# Edit [microsandbox.runtime].image in mmux-image.toml.
make launch NODE_CONFIG="mmux-image.toml"
```

The default `CONTROLLER_URL` is
`http://host.microsandbox.internal:3000`, the Microsandbox-provided alias for
reaching the host machine from inside the sandbox. When `CONTROLLER_URL` uses
that alias, the launcher automatically allows exactly the URL's host TCP port
for controller communication. The controller token secret is scoped to
`allowed_host = "host.microsandbox.internal"`.

The runtime `mmux.toml` denies egress and ingress by default. A fresh source
install from GitHub or profile installer scripts that call external services
need temporary egress during preparation. Use `mmux-setup.toml` for
that preparation launch, export a prepared snapshot, then relaunch from the
snapshot with `mmux.toml` for controller-only runtime access.

If you use a prepared Docker/OCI image instead, use
`mmux-image.toml.example` as the template for a real image config. Set
`[microsandbox.runtime].image` to the prepared image. That config deliberately
omits shared setup scripts and per-profile launch scripts, so launch only
copies static config/assets, auto-allows the controller URL, and starts
`mmux node`. Installer network access is not opened at launch time.

To capture and export it as a bundle:

```bash
make bundle-export SANDBOX=mmux-node SNAPSHOT_NAME=mmux-node-seed BUNDLE=.artifacts/mmux-node-seed.tar.zst
```

`SANDBOX` is required for `bundle-export` because the snapshot is tied to a
specific sandbox instance. If you omit it, the Makefile prints the exact
example invocation above.

To import an exported bundle and launch from that imported snapshot in one go:

```bash
make bundle-launch BUNDLE=.artifacts/mmux-node-seed.tar.zst NODE_CONFIG="mmux.toml"
```

## Node config

`mmux.toml` owns the controller-only runtime config for prepared sandboxes.
`mmux-setup.toml` extends that shape with shared setup and per-profile
launch scripts for first-time preparation. `mmux-image.toml.example` omits
setup scripts and expects a prepared Docker/OCI image. None of these files
should restate built-in coder profile fields unless you intentionally want to
replace them:

```toml
[microsandbox.runtime]
memory_mib = 1024
cpus = 2

[microsandbox.assets]
mmux_source = { repo = "https://github.com/ilijaljubicic/mmux.git", ref = "v0.1.0" }
scripts_dir = "./mmux_sources/scripts"
assets_dir = "./mmux_sources/assets"
tmux_conf = "./mmux_sources/assets/tmux.conf"

[microsandbox.network]
default_egress = "deny"
default_ingress = "deny"

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

Secrets also live in `mmux.toml`. Use `[[microsandbox.secrets]]` entries with a
guest env name, a host env reference like `host.MMUX_TOKEN`, and an allowed
host. The Rust backend resolves the host env var and injects the secret through
the Microsandbox SDK. The controller token is injected this way into
`MMUX_CONTROLLER_TOKEN`. For the example launch, export `MMUX_TOKEN` on the
host before running `make launch`; `allowed_host` must match the hostname used
in `CONTROLLER_URL`.

Controller access is not configured in TOML. If `CONTROLLER_URL` targets
`host.microsandbox.internal`, the launcher derives and merges the exact
Host-group TCP port rule from the URL. TOML network rules are for additional
policy that the user intentionally applies.

The default source/profile install scripts fetch from the internet. Use
`mmux-setup.toml` for that one-time preparation run, create/export a
snapshot after setup, then relaunch from the prepared snapshot with `mmux.toml`.

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
