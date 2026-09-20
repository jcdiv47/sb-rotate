# Specification: CLI

## Command shape

```text
sb-rotate <COMMAND> [OPTIONS]
```

Top-level commands:

```text
inspect   discover server/client/service/identity bindings
plan      calculate and display edits without writing
rotate    generate and apply a supported replacement
set       propagate an explicitly supplied service property
check     validate supplied config sets using sing-box
```

## Common input options

```text
--server <PATH>          server JSON file or server config directory
--clients <DIR>          directory containing independent client JSON files
--client <PATH>          include/select a specific client file; repeatable
--inbound-tag <TAG>      select server inbound tag; repeatable
--client-tag <TAG>       select client outbound tag; repeatable
--sing-box <PATH>        explicit sing-box executable
```

At least one client source is required for commands that synchronize server/client state.

`--clients` discovers direct `.json` files in the directory. Recursive discovery is not required in v1.

## Selector semantics

Within one category, repeated selectors are ORed:

```bash
--client phone.json --client laptop.json
```

means phone OR laptop.

Across categories, selectors are ANDed:

```bash
--client phone.json --client-tag home
```

means the `home` outbound(s) inside `phone.json`.

## `inspect`

```bash
sb-rotate inspect --server ./server.json --clients ./clients/
```

Useful filters:

```bash
sb-rotate inspect --server ./server.json --clients ./clients/ \
  --protocol vless

sb-rotate inspect --server ./server.json --clients ./clients/ \
  --protocol hysteria2

sb-rotate inspect --server ./server.json --clients ./clients/ \
  --inbound-tag vless-home
```

Suggested output:

```text
vless inbound=vless-home
  identity uuid=bf000d23-0752-40b4-affe-68f7707a9661 user=alice
    clients/phone.json outbound=home
    clients/macbook.json outbound=home

  reality
    short_ids: 2 accepted / 2 observed
    clients: 2
```

Passwords are masked in output.

## `plan`

Syntax:

```bash
sb-rotate plan --server <PATH> <client options> --kind <KIND> [selectors]
```

Examples:

```bash
sb-rotate plan --server ./server.json --clients ./clients/ \
  --kind vless-uuid

sb-rotate plan --server ./server.json --clients ./clients/ \
  --kind vless-reality-short-id \
  --client ./clients/phone.json

sb-rotate plan --server ./server.json --clients ./clients/ \
  --kind hysteria2-obfs-password \
  --inbound-tag hy2-home
```

`plan` never writes config files.

## `rotate`

Syntax:

```bash
sb-rotate rotate --server <PATH> <client options> --kind <KIND> [selectors]
```

Supported initial kinds:

```text
vless-uuid
vless-reality-short-id
vless-reality-keypair
hysteria2-password
hysteria2-obfs-password
```

Examples:

```bash
# All matched VLESS identities.
sb-rotate rotate --server ./server.json --clients ./clients/ \
  --kind vless-uuid

# Identity selected through one client. If that UUID is shared,
# every occurrence in the identity binding rotates together.
sb-rotate rotate --server ./server.json --clients ./clients/ \
  --kind vless-uuid \
  --client ./clients/phone.json

# One specific outbound in a multi-outbound client config.
sb-rotate rotate --server ./server.json --clients ./clients/ \
  --kind vless-reality-short-id \
  --client ./clients/phone.json \
  --client-tag home

# Several selected client outbounds receive independent short IDs.
sb-rotate rotate --server ./server.json --clients ./clients/ \
  --kind vless-reality-short-id \
  --client ./clients/phone.json \
  --client ./clients/laptop.json \
  --client-tag home

# Whole-service Reality keypair.
sb-rotate rotate --server ./server.json --clients ./clients/ \
  --kind vless-reality-keypair \
  --inbound-tag vless-home

# Hysteria2 identities.
sb-rotate rotate --server ./server.json --clients ./clients/ \
  --kind hysteria2-password

# Shared Hysteria2 obfs secret.
sb-rotate rotate --server ./server.json --clients ./clients/ \
  --kind hysteria2-obfs-password \
  --inbound-tag hy2-home
```

## `set`

Syntax:

```bash
sb-rotate set --server <PATH> <client options> \
  --kind <KIND> --value <VALUE> [selectors]
```

Initial kinds:

```text
server
tls-server-name
server-port
server-ports
```

Examples:

```bash
sb-rotate set --server ./server.json --clients ./clients/ \
  --inbound-tag vless-home \
  --kind server \
  --value new.example.com

sb-rotate set --server ./server.json --clients ./clients/ \
  --inbound-tag vless-home \
  --kind server-port \
  --value 443

sb-rotate set --server ./server.json --clients ./clients/ \
  --inbound-tag vless-home \
  --kind tls-server-name \
  --value new.example.com

sb-rotate set --server ./server.json --clients ./clients/ \
  --inbound-tag hy2-home \
  --kind server-ports \
  --value 20000:30000,40000
```

Service-level `set` operations apply to all bound clients in the selected service. Client selectors are rejected for service-wide properties in v1 rather than allowing a service to drift accidentally.

## `check`

```bash
sb-rotate check --server ./server.json --clients ./clients/
```

The command validates:

- the server config set as one sing-box configuration;
- each client file independently.

Examples:

```bash
sb-rotate check --server ./server-config/ --clients ./clients/

sb-rotate check --server ./server.json --client ./phone.json \
  --sing-box /opt/sing-box/bin/sing-box
```

## Binary resolution

Resolve the executable in this order:

1. `--sing-box <path>`
2. `SING_BOX`
3. `sing-box` via `PATH`

Every command verifies `sing-box version` before protocol work.
