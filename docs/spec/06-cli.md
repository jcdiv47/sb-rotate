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
build     merge client fragments into one validated config per target
recover   restore an interrupted transaction or clean completed transaction metadata
```

## Common input options

```text
--server <PATH>          server JSON file or server config directory
--clients <DIR>          local directory of client JSON configs; repeatable
--client <PATH>          include/select a specific client file; repeatable
--inbound-tag <TAG>      select server inbound tag; repeatable
--outbound-tag <TAG>     select client outbound tag; repeatable (--client-tag alias)
--sing-box <PATH>        explicit sing-box executable
```

At least one client source is required for commands that synchronize server/client state.

`--clients` (alias `--client-config-dir`) discovers direct `.json` files in the directory; repeat it to combine directories, for example client configs assembled from shared and per-device fragments. Recursive discovery is not required in v1. Each file may contain multiple outbounds of the same or different types. Files are local: the tool does not contact devices, deploy configs, or reload services.

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

## Rotation

Syntax for both `plan` and `rotate`:

```bash
sb-rotate plan --server <PATH> <client options> [--type <TYPE>] [selectors]
sb-rotate rotate --server <PATH> <client options> [--type <TYPE>] [selectors]
```

Examples:

```bash
# Every supported type.
sb-rotate rotate --server ./server.json --clients ./clients/

# One type.
sb-rotate rotate --server ./server.json --clients ./clients/ --type vless
sb-rotate rotate --server ./server.json --clients ./clients/ --type hysteria2

# One client's credentials only.
sb-rotate rotate --server ./server.json --clients ./clients/ \
  --client ./clients/phone.json --outbound-tag home

# One inbound, including its shared secrets.
sb-rotate rotate --server ./server.json --clients ./clients/ \
  --inbound-tag vless-home
```

`--type` uses sing-box type names, currently `vless` and `hysteria2`; without it every supported type with selected bound outbounds is rotated. Reality is a TLS option on VLESS, not a separate type.

A rotation replaces the supported, already-configured secrets of the selected outbounds, on the server and on the clients. Individual materials cannot be rotated on their own:

| Type | Material |
|---|---|
| `vless` | user UUIDs, enabled Reality keypairs and short IDs |
| `hysteria2` | user passwords, configured obfs passwords |

Rules:

- Each outbound is matched independently to its inbound/user by the existing discovery rules, not merely by type or tag. Ambiguous matches fail before any value is generated.
- Without selectors, every bound outbound is selected. `--client`, `--outbound-tag`, and `--inbound-tag` narrow the selection. An inbound is fully selected when all of its bound outbounds are selected; `--inbound-tag` alone fully selects its inbounds.
- A selected outbound's user credential receives one replacement across all discovered occurrences, including unselected outbounds sharing it.
- A fully selected inbound's Reality keypair or obfs password is generated once and written to the server and every bound client. For a partially selected inbound it is kept, and the plan says so; this lets one client's credentials rotate without touching other clients. The plan warns when a shared secret change also affects server users that have no supplied client.
- Short IDs are generated per selected Reality outbound. IDs still used by unselected supplied clients and unattributed accepted IDs are retained.
- All edits are composed from the original inventory, then validated/applied once using one recoverable transaction. Replacements remain atomic per file, not across files.
- Absent/disabled optional features are not enabled. TLS certificates, addresses, and ports are not rotated. Unmatched users/outbounds are retained. Clients not supplied cannot be updated.
- A selection with no bound outbounds fails rather than silently succeeding.

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

Uses the [rotation](#rotation) syntax. `plan` generates fresh values and displays the edits (secrets masked), but never writes config files.

## `rotate`

Uses the [rotation](#rotation) syntax. `rotate` generates a new plan, validates the staged server set and changed clients, then replaces the originals.

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

Service-level `set` operations apply to all bound clients in the selected service. Client selectors are rejected for service-wide properties in v1 rather than allowing a service to drift accidentally. Supply the client inventory with `--clients`, not `--client`.

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

## `build`

```bash
sb-rotate build --manifest <PATH> [TARGET...] [--publish [--sudo]] [--sing-box <PATH>]
```

The JSON manifest lists targets and their fragments; relative paths resolve from the manifest's directory:

```json
{
  "output_dir": "out",
  "publish_dir": "/srv/singbox-sub",
  "targets": {
    "phone": {"fragments": ["devices/phone.json", "shared/base.json"], "publish_as": "phone-<random>.json"}
  }
}
```

`output_dir` defaults to `out`; `publish_dir` and `publish_as` are required only with `--publish`. Unknown fields, unsafe target/publish names, missing fragments, and unknown selected targets are rejected.

Rules:

- A scalar set by more than one fragment of a target is rejected, so merge order only affects arrays.
- Fragments are staged under index-prefixed names in a private directory and merged with `sing-box merge`, which orders inputs by path; manifest order therefore becomes array (rule) order.
- Every selected target is merged and validated with `sing-box check` before any output is written.
- Outputs (`<output_dir>/<target>.json`) are replaced atomically; on Unix they are 0600 and a new output directory is 0700.
- `--publish` writes each output to `<publish_dir>/<publish_as>`: atomically with mode 0644 when the directory is writable. Otherwise it writes nothing there, because existing ownership may be deliberate. It reports `sudo install -o root -g root -m 644 <output> <target>` commands and exits non-zero after writing all outputs.
- `--publish --sudo` never writes published files directly: after every target has been merged, validated, and written to `output_dir`, it runs that `sudo install` for each one (sudo may prompt), producing root-owned subscription files.
- A pending rotation transaction in any fragment directory blocks the build.

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
