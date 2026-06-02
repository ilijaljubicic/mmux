# Microsandbox mmux Backend Example

This directory contains the example config and Make targets for running an
`mmux node` inside Microsandbox.

The host runs the `mmux-microsandbox-node` launcher. In prepare mode, the
launcher creates a Microsandbox instance, injects the node config and
configured assets, runs setup scripts, and finishes without node registration.
In runtime launch mode, it starts `mmux node` inside an already prepared guest.

For first-time setup from a stock image, the guest installs `mmux` from a
GitHub release archive using the `[microsandbox.assets]` `mmux_version` entry in
`microsandbox-setup.toml`:

```toml
mmux_version = "v0.1.0"
```

Use a release tag such as `v0.1.0` for reproducible sandbox launches. For
local development, use `make build-dev` and `make dev-prepare`; that copies the
host-built `mmux` binary into the guest and skips release download.

To avoid installer network access at launch time, use either a prepared
snapshot or a prepared Docker/OCI image. Bundle launches skip setup scripts,
so the prepared sandbox must already contain the installed mmux binary and
guest toolchain state. The runtime config is still copied into the guest before
`mmux node` starts. Image launches use the same `mmux.toml`; set
`[sandbox.runtime].image` to the prepared image.

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
    30_install_codex.sh
    31_install_opencode.sh
    32_install_kimi.sh
    33_install_claude.sh
```

`mmux_sources/scripts` contains the sandbox setup. The backend registers every
script from that directory and runs them in alphabetical order.

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
# First-time preparation from a stock image. make prepare uses open setup
# egress by default so apt/curl/npm and tool installers can run. It does not
# start mmux node and does not need the node wire token.
cd /mnt/Radni/aitools/mmux/example-backends/microsandbox
make prepare NODE_CONFIG="microsandbox-setup.toml"
make bundle-export SANDBOX=mmux-node SNAPSHOT_NAME=mmux-node-seed BUNDLE=.artifacts/mmux-node-seed.tar.zst
```

```bash
# Runtime from the prepared bundle. This is the step that starts mmux node and
# registers it with the controller.
export MMUX_WIRE_TOKEN="...node wire bearer token..."
make bundle-launch BUNDLE=.artifacts/mmux-node-seed.tar.zst NODE_CONFIG="mmux.toml"
```

```bash
# Runtime from a prepared Docker/OCI image. The image already contains tmux,
# mmux, and the coder CLIs, so launch does not open installer network access.
export MMUX_WIRE_TOKEN="...node wire bearer token..."
cd /mnt/Radni/aitools/mmux/example-backends/microsandbox
# Edit [sandbox.runtime].image in mmux.toml.
make launch NODE_CONFIG="mmux.toml"
```

The default `CONTROLLER_URL` is
`http://host.microsandbox.internal:3000`, the Microsandbox-provided alias for
reaching the host machine from inside the sandbox. When `CONTROLLER_URL` uses
that alias, the launcher automatically allows exactly the URL's host TCP port
for controller communication. The node wire token secret is scoped to
`allowed_host = "host.microsandbox.internal"`.

The runtime `mmux.toml` denies egress and ingress by default. First-time release
and profile installation needs installer network access, so `make prepare
NODE_CONFIG="microsandbox-setup.toml"` uses open setup egress by default. Export a
prepared snapshot, then relaunch from the snapshot with `mmux.toml` for
controller-only runtime access.

If you use a prepared Docker/OCI image instead, set `[sandbox.runtime].image`
in `mmux.toml` to that image. The runtime config deliberately omits setup
scripts, so launch only applies runtime policy, writes the selected node config
into the guest, auto-allows the controller URL, and starts `mmux node`.
Installer network access is not opened at launch time.

To capture and export a prepared sandbox as a bundle:

```bash
make bundle-export SANDBOX=mmux-node SNAPSHOT_NAME=mmux-node-seed BUNDLE=.artifacts/mmux-node-seed.tar.zst
```

`bundle-export` expects `SANDBOX` to name an existing sandbox that was already
prepared with `make prepare`. It does not run setup itself; it stops that
sandbox, snapshots it, and writes the bundle. `SANDBOX` is required because the
snapshot is tied to a specific sandbox instance. If you omit it, the Makefile
prints the exact example invocation above.

To import an exported bundle and launch from that imported snapshot in one go:

```bash
make bundle-launch BUNDLE=.artifacts/mmux-node-seed.tar.zst NODE_CONFIG="mmux.toml"
```

## Coder CLI auth

For coder CLIs that support device-flow authentication, authenticate once
inside the prepared sandbox:

```bash
msb ssh mmux-node
# inside the guest, run the coder CLI and choose device-flow auth
codex
```

The auth files are stored in the running sandbox filesystem. Later
mmux-managed coder sessions in that sandbox use the saved auth state until the
CLI's token expires.

## Node config

`mmux.toml` owns the controller-only runtime config for prepared sandboxes.
`microsandbox-setup.toml` owns first-time preparation from a stock image. It
adds shared setup scripts plus `mmux_version`; `make prepare` provides open
setup egress by default. It does not include controller secrets and does not
start `mmux node`. `mmux.toml` is also used for prepared Docker/OCI images by
setting `[sandbox.runtime].image`. None of these files should restate built-in
coder profile fields unless you intentionally want to replace them:

Runtime, network, secret bindings, volumes, and mounts use the backend-agnostic
`[sandbox.*]` namespace. Microsandbox-only setup assets and patches stay under
`[microsandbox.*]`.

```toml
[microsandbox.runtime]
image = "debian:bookworm-slim"
memory_mib = 4096
cpus = 2

[microsandbox.assets]
mmux_version = "v0.1.0"
scripts_dir = "./mmux_sources/scripts"
assets_dir = "./mmux_sources/assets"
```

Coder interaction profiles are built into mmux. TOML files only need profile
sections when you intentionally override or extend those built-ins. Host-side
paths for `copy_file`, `copy_dir`, `scripts_dir`, and `assets_dir` are resolved
relative to the directory that contains the selected config file.

Secrets also live in `mmux.toml`. Use `[[sandbox.secrets]]` entries with a
guest env name, a host env reference like `host.MMUX_WIRE_TOKEN`, and an allowed
host. The Rust backend resolves the host env var and injects the secret through
the Microsandbox SDK. The node wire token is injected this way into
`MMUX_CONTROLLER_TOKEN`. For the example launch, export `MMUX_WIRE_TOKEN` on the
host before running `make launch`; `allowed_host` must match the hostname used
in `CONTROLLER_URL`.

Controller access is not configured in TOML. If `CONTROLLER_URL` targets
`host.microsandbox.internal`, the launcher derives and merges the exact
Host-group TCP port rule from the URL. TOML network rules are for additional
policy that the user intentionally applies.

For service access, prefer exact domain rules over broad public egress. DNS is
also policy-gated under default deny:

```toml
[[sandbox.network.egress]]
action = "allow"
destination_domain = "api.openai.com"
protocol = "udp"
ports = [53]

[[sandbox.network.egress]]
action = "allow"
destination_domain = "api.openai.com"
protocol = "tcp"
ports = [443]
```

The default setup scripts fetch from the internet. Use
`microsandbox-setup.toml` for that one-time preparation run, create/export a
snapshot after setup, then relaunch from the prepared snapshot with
`mmux.toml`.

Volumes are declared separately and mounts consume them:

```toml
[[sandbox.volumes]]
name = "my-data"
quota_mib = 5120

[[sandbox.mounts]]
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

Normal example targets use the installed `mmux-microsandbox-node` binary from
`PATH`. For local development, use the dev targets. `make build-dev` builds the
launcher and a musl-linked Linux `mmux` binary; `make dev-prepare` passes that
binary through `--mmux-binary` so the guest does not compile `mmux` or download
a release:

```bash
make build-dev
make dev-prepare NODE_CONFIG="microsandbox-setup.toml"
```
