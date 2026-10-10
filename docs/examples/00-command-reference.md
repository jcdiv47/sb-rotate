# Example Command Reference

## Rotation workflow

```bash
# --clients is a local directory of sing-box client configs; repeat it for
# configs split across directories. Each file can contain several outbounds.
sb-rotate inspect --server ./server.json --clients ./clients/
sb-rotate plan --server ./server.json --clients ./clients/
sb-rotate rotate --server ./server.json --clients ./clients/

# Only VLESS (UUIDs, Reality keypairs, short IDs) or only Hysteria2
# (user passwords, obfs passwords).
sb-rotate rotate --server ./server.json --clients ./clients/ --type vless
sb-rotate rotate --server ./server.json --clients ./clients/ --type hysteria2

# One inbound, including its configured shared key material.
sb-rotate rotate --server ./server.json --clients ./clients/ \
  --inbound-tag vless-home

# Credentials of the home outbound in phone.json only: its user credential
# (everywhere it is shared) and its short ID. Shared keys and other clients
# stay unchanged.
sb-rotate rotate --server ./server.json --clients ./clients/ \
  --client ./clients/phone.json --outbound-tag home

# Client configs assembled from fragments in several directories.
sb-rotate rotate --server ./server.json \
  --clients ./clients/shared/ --clients ./clients/devices/
```

Rotation covers every supported credential of the selected outbounds, plus the shared keypair/obfs password of inbounds whose bound outbounds are all selected; individual materials cannot be rotated on their own. It does not rotate certificates or enable optional features. Plans span all matching inbounds and are applied as one validated, recoverable transaction. Unmatched users/outbounds and unattributed accepted short IDs are retained; the plan warns when a rotated keypair or obfs password also affects server users without a supplied client. Nothing is remotely deployed or reloaded.

`--client-config-dir` aliases `--clients`. Prefer `--outbound-tag` over its alias `--client-tag`, and `inspect --type` over its alias `--protocol`.

## Inspect

```bash
sb-rotate inspect --server ./server.json --clients ./clients/

sb-rotate inspect --server ./server.json --clients ./clients/ \
  --protocol vless

sb-rotate inspect --server ./server.json --clients ./clients/ \
  --protocol hysteria2

sb-rotate inspect --server ./server.json --clients ./clients/ \
  --inbound-tag vless-home

sb-rotate inspect --server ./server-config/ --clients ./clients/

sb-rotate inspect --server ./server.json --client ./phone.json
```

## Plan

```bash
sb-rotate plan --server ./server.json --clients ./clients/

sb-rotate plan --server ./server.json --clients ./clients/ \
  --type hysteria2 --inbound-tag hy2-home
```

## Rotate selected outbounds

```bash
# One client file.
sb-rotate rotate --server ./server.json --clients ./clients/ \
  --client ./clients/phone.json

# By outbound tag, across every supplied client.
sb-rotate rotate --server ./server.json --clients ./clients/ \
  --outbound-tag home

# One outbound inside a multi-outbound client file.
sb-rotate rotate --server ./server.json --clients ./clients/ \
  --client ./clients/phone.json --outbound-tag home

# Several clients; each selected Reality outbound gets a different short ID.
sb-rotate rotate --server ./server.json --clients ./clients/ \
  --client ./clients/phone.json --client ./clients/laptop.json \
  --outbound-tag home
```

If several outbounds share a selected user credential, the whole identity binding rotates together. A Reality keypair or Hysteria2 obfs password is shared by its inbound, so it rotates only when every bound outbound of that inbound is selected; otherwise the plan notes that it was kept. To cut off one device, rotating its own credentials is enough.

## Change service address

```bash
sb-rotate set --server ./server.json --clients ./clients/ \
  --inbound-tag vless-home \
  --kind server \
  --value new.example.com

sb-rotate set --server ./server.json --clients ./clients/ \
  --inbound-tag hy2-home \
  --kind server \
  --value hy2-new.example.com
```

## Change service port

```bash
sb-rotate set --server ./server.json --clients ./clients/ \
  --inbound-tag vless-home \
  --kind server-port \
  --value 443

sb-rotate set --server ./server.json --clients ./clients/ \
  --inbound-tag hy2-home \
  --kind server-port \
  --value 8443
```

## Change Hysteria2 port-hopping ranges

```bash
sb-rotate set --server ./server.json --clients ./clients/ \
  --inbound-tag hy2-home \
  --kind server-ports \
  --value 20000:30000,40000
```

## Change TLS server name

```bash
sb-rotate set --server ./server.json --clients ./clients/ \
  --inbound-tag vless-home \
  --kind tls-server-name \
  --value new.example.com

sb-rotate set --server ./server.json --clients ./clients/ \
  --inbound-tag hy2-home \
  --kind tls-server-name \
  --value hy2-new.example.com
```

## Build fragment-based client configs

```bash
# All targets in the manifest, then publish them as subscriptions.
sb-rotate build --manifest ./clients/build.json
sb-rotate build --manifest ./clients/build.json --publish
# Root-owned subscription files: sudo-install every output after all builds pass.
sb-rotate build --manifest ./clients/build.json --publish --sudo

# Selected targets.
sb-rotate build --manifest ./clients/build.json phone laptop
```

See [the CLI specification](../spec/06-cli.md#build) for the manifest format.

## Validate

```bash
sb-rotate check --server ./server.json --clients ./clients/

sb-rotate check --server ./server-config/ --clients ./clients/

sb-rotate check --server ./server.json --client ./phone.json
```

## Recover an interrupted mutation

Stop other config writers/reload automation and use the same user as the interrupted command:

```bash
# Preview through either affected config directory.
sb-rotate recover --directory ./clients --dry-run

# Restore an unfinished transaction, or clean already-finalized metadata.
sb-rotate recover --directory ./clients

# Alternatively, use the journal path reported by a failed mutation.
sb-rotate recover --journal /absolute/path/.sb-rotate-transaction-... --dry-run

# Validate restored configs before reloading services.
sb-rotate check --server ./server.json --clients ./clients/
```

`recover` itself does not require sing-box. It refuses detected conflicts; do not delete pending markers or journals to bypass recovery. A committed transaction is never rolled back. See the [recovery safety and durability rules](../spec/07-validation-and-writes.md#explicit-recovery).

## Select sing-box binary

```bash
sb-rotate inspect --server ./server.json --clients ./clients/ \
  --sing-box /opt/sing-box/bin/sing-box

SING_BOX=/opt/sing-box/bin/sing-box \
  sb-rotate inspect --server ./server.json --clients ./clients/
```

## Typical workflow

```bash
sb-rotate inspect --server ./server.json --clients ./clients/

sb-rotate plan --server ./server.json --clients ./clients/

sb-rotate rotate --server ./server.json --clients ./clients/

sb-rotate check --server ./server.json --clients ./clients/
```
