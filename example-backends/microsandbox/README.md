# Microsandbox mmux Connector Example

This directory contains a small Makefile for running:

```bash
mmux node --backend microsandbox
```

against an existing running Microsandbox runtime.

mmux does not manage Microsandbox lifecycle. Use `msb` directly to create,
start, stop, snapshot, import, and export sandboxes. The mmux connector runs on
the host, attaches to an existing running sandbox by name, and keeps controller
credentials on the host.

## Run

Start a controller, then prepare a local sandbox:

```bash
make
```

That target uses `msb create` and then runs the optional scripts in
`mmux_sources/scripts/` inside the sandbox. The example-local `workspace/`
directory is mounted read-write at `/workspace`; the setup assets are mounted
at `/mmux-setup`. Once the sandbox exists and is running, run the connector:

```bash
export MMUX_WIRE_TOKEN="...node wire bearer token..."
make launch SANDBOX=mmux-node
```

The default controller URL is `http://127.0.0.1:3000`. Override it when the
controller runs elsewhere:

```bash
make launch \
  CONTROLLER_URL=https://controller.example.com \
  SANDBOX=mmux-node \
  NODE_ID=msb-1
```

## mTLS

For mTLS node auth, pass the certificate material to the host-side connector:

```bash
mmux node \
  --backend microsandbox \
  --sandbox-name mmux-node \
  --controller-url https://127.0.0.1:3000 \
  --node-id msb-1 \
  --node-name "Microsandbox mmux node" \
  --controller-ca ./certs/controller-ca.pem \
  --client-cert ./certs/nodes/msb-1.pem \
  --client-key ./certs/nodes/msb-1-key.pem
```

Do not place node wire tokens or node private keys inside the sandbox. The
sandbox only needs the workload tools such as `tmux` and coder CLIs.

## Setup Assets

`mmux_sources/` contains optional setup assets you can reuse with `msb` or your
own image build process:

- `assets/tmux.conf` provides tmux defaults.
- `scripts/` installs tmux, common toolchains, and coder CLIs.

These files are not consumed by mmux. They are examples for preparing a
Microsandbox runtime before starting the connector.

The Makefile exposes one local prep target:

```bash
make sandbox-prepare
```

## Workspace

Use Microsandbox or Kubernetes-native mounts for workspace persistence. mmux
does not apply Microsandbox mount config; it only executes tmux/file commands
inside the sandbox that already exists.
