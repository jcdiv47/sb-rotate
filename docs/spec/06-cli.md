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
recover   restore an interrupted transaction or clean completed transaction metadata
```

## Common input options

```text
--server <PATH>          server JSON file or server config directory
--clients <DIR>          local directory of independent client JSON configs
--client <PATH>          include/select a specific client file; repeatable
--inbound-tag <TAG>      select server inbound tag; repeatable
--outbound-tag <TAG>     select client outbound tag; repeatable (--client-tag alias)
--sing-box <PATH>        explicit sing-box executable
```

At least one client source is required for commands that synchronize server/client state.

`--clients` (alias `--client-config-dir`) discovers direct `.json` files in the directory. Recursive discovery is not required in v1. Each file may contain multiple outbounds of the same or different types. Files are local: the tool does not contact devices, deploy configs, or reload services.

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

## Type-driven rotation

Preferred syntax for both `plan` and `rotate`:

```bash
sb-rotate plan --server ./server.json --clients ./clients/ --type vless
sb-rotate rotate --server ./server.json --clients ./clients/ --type vless
sb-rotate rotate --server ./server.json --clients ./clients/ --type hysteria2

# Limit material and optionally select through an outbound.
sb-rotate rotate --server ./server.json --clients ./clients/ \
  --type vless --only uuid --outbound-tag home

# Limit to one inbound, including its shared secrets.
sb-rotate rotate --server ./server.json --clients ./clients/ \
  --type vless --inbound-tag vless-home
```

`--type` uses sing-box type names, currently `vless` and `hysteria2`. Reality is a TLS option on VLESS, not a separate type.

Without `--only`, all supported, already-configured material with bound outbounds is rotated:

| Type | Default material | Allowed `--only` values |
|---|---|---|
| `vless` | UUIDs, enabled Reality keypairs and short IDs | `uuid`, `reality-keypair`, `reality-short-id` |
| `hysteria2` | User passwords, configured obfs passwords | `password`, `obfs-password` |

Rules:

- Each outbound is matched independently to its inbound/user by the existing discovery rules, not merely by type or tag. Ambiguous matches fail.
- Every matching inbound is eligible, even for keypair/obfs/short-ID rotation. `--inbound-tag` optionally narrows this set.
- All edits are composed from the original inventory, then validated/applied once using one recoverable transaction. Replacements remain atomic per file, not across files.
- Shared user credentials receive one replacement across all discovered occurrences. Shared keypairs/obfs passwords are generated once per inbound. Short IDs are generated per selected Reality outbound.
- An all-material rotation rejects `--client` and `--outbound-tag`, even if optional secrets are absent. Whole-inbound `--only reality-keypair` / `--only obfs-password` also reject these selectors. Use `--clients` for the full inventory.
- `--only uuid`, `--only password`, and `--only reality-short-id` allow client/outbound selection. Selecting a user still updates every discovered occurrence of that user's credential.
- Absent/disabled optional features are not enabled. TLS certificates, addresses, and ports are not rotated. Unmatched users/outbounds and unattributed accepted short IDs are retained. Clients not supplied cannot be updated.
- Explicitly selected material with no eligible bindings fails rather than silently succeeding. A material unsupported by the selected type fails before generation.
- `--only` requires `--type`. Neither can be combined with legacy `--kind`.

The `--kind` examples below remain supported for compatibility; unlike type-driven rotation, their service-scoped operations still require a single matching inbound.

## `inspect`

```bash
sb-rotate inspect --server ./server.json --clients ./clients/
```

Useful filters:

```bash
sb-rotate inspect --server ./server.json --clients ./clients/ \
  --type vless

sb-rotate inspect --server ./server.json --clients ./clients/ \
  --type hysteria2

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

Passwords are masked in output. `--protocol` remains an alias for `--type`; `--client-tag` remains an alias for `--outbound-tag` on commands that accept outbound selectors.

## `plan`

Use the type-driven syntax above, or the legacy single-operation syntax:

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

Use the type-driven syntax above, or the legacy single-operation syntax:

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

Service-level `set` operations apply to all bound clients in the selected service. Client selectors are rejected for service-wide properties in v1 rather than allowing a service to drift accidentally. Supply the client inventory with `--clients`, not `--client`. The same restriction applies to Reality keypair and Hysteria2 obfs rotation.

`set --dry-run` displays the same property edit plan without writing or validating temporary configs. Setting a property to its existing value is a no-op; unchanged files are not rewritten.

`server-ports` accepts comma-delimited ranges/single ports. Single ports are normalized to equal-ended ranges (`40000` becomes `"40000:40000"`) for sing-box compatibility, and scalar `server_port` fields are removed when enabling port hopping.

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

## `recover`

```bash
# Inspect a pending transaction through either affected config directory.
sb-rotate recover --directory ./clients --dry-run

# Roll back an unfinished transaction (or clean already-finalized metadata).
sb-rotate recover --directory ./clients

# Use the exact journal path reported by a failed mutation.
sb-rotate recover --journal /absolute/path/.sb-rotate-transaction-... --dry-run
```

Exactly one of `--directory` and `--journal` is required. This command does not accept the common server/client input selectors or `--sing-box`, and it works without sing-box installed. It restores exact original bytes and permissions (plus owner/group on Unix), not new generated credentials. Restored files may have new inodes and timestamps; extended attributes and ACLs are not restored. It refuses conflicts instead of providing a force option.

Before replacement begins, a transaction publishes a pending marker in each locked directory. These markers block further overlapping mutations (including no-ops) until recovery completes; interruption during publication may leave only some markers, with no config replacements yet performed. A recorded successful commit is never rolled back; recovery only cleans its metadata. See [planning, validation, and writes](07-validation-and-writes.md#explicit-recovery) for safety and durability limits.

## Binary resolution

Resolve the executable in this order:

1. `--sing-box <path>`
2. `SING_BOX`
3. `sing-box` via `PATH`

Every command except `recover` verifies `sing-box version` before protocol work.
