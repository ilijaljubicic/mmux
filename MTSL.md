# mTLS Certificates

This guide creates one shared node-client CA and per-node client certificates
for mmux node wire mTLS.

The controller does not need one CA per node. It trusts one node-client CA, and
each node gets its own certificate/key signed by that CA. Revocation can be
added later with CRL support without keeping a node allow-list in the
controller.

## Files

Recommended layout:

```text
certs/
  node-ca.pem
  node-ca-key.pem
  controller-ca.pem
  controller-ca-key.pem
  controller.pem
  controller-key.pem
  nodes/
    msb-1.pem
    msb-1-key.pem
```

`node-ca-key.pem` is the issuer private key. Keep it offline or in a controlled
release/admin workflow. Do not copy it into a node or sandbox.

`msb-1-key.pem` is the node private key. It belongs only to that node.

`node-ca.pem` is public CA material. It is safe to give to the controller.

`controller-ca.pem` is public CA material. Give it to nodes when the controller
uses a private or self-signed CA.

## Create The Node CA

```bash
mkdir -p certs/nodes

openssl genrsa -out certs/node-ca-key.pem 4096

openssl req -x509 -new -nodes \
  -key certs/node-ca-key.pem \
  -sha256 -days 3650 \
  -subj "/CN=mmux node client CA" \
  -out certs/node-ca.pem

chmod 0400 certs/node-ca-key.pem
```

## Create One Node Certificate

Set `NODE_ID` to the exact `--node-id` the node will use.

```bash
NODE_ID=msb-1

openssl genrsa -out "certs/nodes/${NODE_ID}-key.pem" 2048

openssl req -new \
  -key "certs/nodes/${NODE_ID}-key.pem" \
  -subj "/CN=${NODE_ID}" \
  -out "certs/nodes/${NODE_ID}.csr"
```

Create a certificate extension file with a URI SAN. mmux requires URI SAN
`mmux:node:<node-id>` or `spiffe://mmux/node/<node-id>` for node identity.
DNS SAN and CN are ignored for node identity.

```bash
cat > "certs/nodes/${NODE_ID}.ext" <<EOF
basicConstraints=CA:FALSE
keyUsage=digitalSignature
extendedKeyUsage=clientAuth
subjectAltName=URI:mmux:node:${NODE_ID}
EOF
```

Sign the node certificate:

```bash
openssl x509 -req \
  -in "certs/nodes/${NODE_ID}.csr" \
  -CA certs/node-ca.pem \
  -CAkey certs/node-ca-key.pem \
  -CAcreateserial \
  -out "certs/nodes/${NODE_ID}.pem" \
  -days 825 -sha256 \
  -extfile "certs/nodes/${NODE_ID}.ext"

chmod 0400 "certs/nodes/${NODE_ID}-key.pem"
```

The controller extracts the node id from the certificate and rejects wire RPC
requests where the certificate identity does not match the request `node_id`.

## Controller TLS Certificate

The local runtime terminates TLS itself when `--wire-mtls` is selected, so it
also needs a server certificate and key. For local/private deployments, create
a self-signed controller CA and sign the controller server certificate with it.

```bash
openssl genrsa -out certs/controller-ca-key.pem 4096

openssl req -x509 -new -nodes \
  -key certs/controller-ca-key.pem \
  -sha256 -days 3650 \
  -subj "/CN=mmux controller CA" \
  -out certs/controller-ca.pem

openssl genrsa -out certs/controller-key.pem 2048

openssl req -new \
  -key certs/controller-key.pem \
  -subj "/CN=localhost" \
  -out certs/controller.csr

cat > certs/controller.ext <<EOF
basicConstraints=CA:FALSE
keyUsage=digitalSignature,keyEncipherment
extendedKeyUsage=serverAuth
subjectAltName=DNS:localhost,IP:127.0.0.1
EOF

openssl x509 -req \
  -in certs/controller.csr \
  -CA certs/controller-ca.pem \
  -CAkey certs/controller-ca-key.pem \
  -CAcreateserial \
  -out certs/controller.pem \
  -days 825 -sha256 \
  -extfile certs/controller.ext

chmod 0400 certs/controller-key.pem
chmod 0400 certs/controller-ca-key.pem
```

The controller uses `controller.pem` and `controller-key.pem`. Nodes use
`controller-ca.pem` with `--controller-ca` to trust that controller certificate.

## Run The Controller

```bash
mmux controller \
  --wire-mtls \
  --tls-cert ./certs/controller.pem \
  --tls-key ./certs/controller-key.pem \
  --wire-client-ca ./certs/node-ca.pem
```

When `--wire-mtls` is set, `--wire-token`, `--wire-token-file`,
`MMUX_WIRE_TOKEN`, and `--allow-unauthenticated-node-wire` are rejected.

## Run A Node

```bash
mmux node \
  --controller-url https://localhost:3000 \
  --node-id msb-1 \
  --controller-ca ./certs/controller-ca.pem \
  --client-cert ./certs/nodes/msb-1.pem \
  --client-key ./certs/nodes/msb-1-key.pem
```

The node certificate/key are mutually exclusive with `--wire-token` and
`MMUX_WIRE_TOKEN`. `--controller-ca` is independent of node authentication and
only controls HTTPS server certificate trust.

## Private Key Ownership

The controller holds:

- `controller-key.pem`: private server TLS key.
- `node-ca.pem`: public CA certificate used to verify node client certs.

Each node holds:

- `controller-ca.pem`: public CA certificate used to verify the controller.
- `<node-id>-key.pem`: private node client key.
- `<node-id>.pem`: public node client certificate chain.

The CA holder holds:

- `node-ca-key.pem`: private signing key used to issue or rotate node
  certificates.
- `controller-ca-key.pem`: private signing key used to issue or rotate
  controller server certificates.

No node should receive the CA private key. The controller does not need the CA
private key.
