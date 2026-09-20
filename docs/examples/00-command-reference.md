# Example Command Reference

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
sb-rotate plan --server ./server.json --clients ./clients/ \
  --kind vless-uuid

sb-rotate plan --server ./server.json --clients ./clients/ \
  --kind vless-reality-short-id

sb-rotate plan --server ./server.json --clients ./clients/ \
  --kind vless-reality-keypair \
  --inbound-tag vless-home

sb-rotate plan --server ./server.json --clients ./clients/ \
  --kind hysteria2-password

sb-rotate plan --server ./server.json --clients ./clients/ \
  --kind hysteria2-obfs-password \
  --inbound-tag hy2-home
```

## Rotate VLESS UUIDs

```bash
# Rotate every matched VLESS identity.
sb-rotate rotate --server ./server.json --clients ./clients/ \
  --kind vless-uuid

# Select an identity through one client file.
sb-rotate rotate --server ./server.json --clients ./clients/ \
  --kind vless-uuid \
  --client ./clients/phone.json

# Select by outbound tag.
sb-rotate rotate --server ./server.json --clients ./clients/ \
  --kind vless-uuid \
  --client-tag home

# Narrow by file and tag.
sb-rotate rotate --server ./server.json --clients ./clients/ \
  --kind vless-uuid \
  --client ./clients/phone.json \
  --client-tag home

# Select identities used by several client files.
sb-rotate rotate --server ./server.json --clients ./clients/ \
  --kind vless-uuid \
  --client ./clients/phone.json \
  --client ./clients/laptop.json

# Narrow to one server service.
sb-rotate rotate --server ./server.json --clients ./clients/ \
  --kind vless-uuid \
  --inbound-tag vless-home
```

If several outbounds share the selected UUID, the whole UUID identity binding rotates together.

## Rotate Reality short IDs

```bash
# One client file.
sb-rotate rotate --server ./server.json --clients ./clients/ \
  --kind vless-reality-short-id \
  --client ./clients/phone.json

# One outbound inside a multi-outbound client file.
sb-rotate rotate --server ./server.json --clients ./clients/ \
  --kind vless-reality-short-id \
  --client ./clients/phone.json \
  --client-tag home

# Several clients; each gets a different new short ID.
sb-rotate rotate --server ./server.json --clients ./clients/ \
  --kind vless-reality-short-id \
  --client ./clients/phone.json \
  --client ./clients/laptop.json \
  --client-tag home
```

## Rotate Reality keypair

```bash
sb-rotate rotate --server ./server.json --clients ./clients/ \
  --kind vless-reality-keypair \
  --inbound-tag vless-home
```

## Rotate Hysteria2 authentication passwords

```bash
# Every matched identity.
sb-rotate rotate --server ./server.json --clients ./clients/ \
  --kind hysteria2-password

# Identity selected through one client.
sb-rotate rotate --server ./server.json --clients ./clients/ \
  --kind hysteria2-password \
  --client ./clients/phone.json

# One Hysteria2 service.
sb-rotate rotate --server ./server.json --clients ./clients/ \
  --kind hysteria2-password \
  --inbound-tag hy2-home
```

## Rotate Hysteria2 obfs password

```bash
sb-rotate rotate --server ./server.json --clients ./clients/ \
  --kind hysteria2-obfs-password \
  --inbound-tag hy2-home
```

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

sb-rotate plan --server ./server.json --clients ./clients/ \
  --kind vless-uuid

sb-rotate rotate --server ./server.json --clients ./clients/ \
  --kind vless-uuid

sb-rotate check --server ./server.json --clients ./clients/
```
