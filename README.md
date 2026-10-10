# sb-rotate

`sb-rotate` is a small Rust CLI for discovering relationships between sing-box server and client configs, rotating shared credentials/key material, and propagating connection-property changes across the configs that belong to the same service.

The initial target is **sing-box 1.14+** with first-class support for:

- VLESS
- VLESS + Reality
- Hysteria2

The tool intentionally does not model the complete sing-box schema. It understands only the fields it needs, uses the installed `sing-box` binary to generate supported credentials/key material, and uses `sing-box check` as the final config validator.

## Implementation status

Currently supported:

- `inspect`: VLESS/Hysteria2 service and identity discovery, selectors, unmatched identities, ambiguity reporting, and Reality accepted/observed short-ID counts;
- `check`: server file/config-directory validation and independent client validation;
- `plan` / `rotate`: rotate the selected outbounds' credentials on server and clients (plus shared keys of fully selected inbounds), optionally limited with `--type vless|hysteria2` and outbound/inbound selectors;
- `set`: `server`, `server-port`, `server-ports`, and `tls-server-name`, with `--dry-run` for a read-only preview;
- `build`: merge client fragments into one validated config per target from a JSON manifest, optionally publishing them;
- `recover`: preview or roll back an interrupted multi-file transaction without needing sing-box;
- masked passwords, private keys, and short IDs; staged validation, permission-preserving replacement, durable rollback journals, and automatic rollback on ordinary replacement failures.

Rotation never covers a subset of materials. For the selected outbounds it replaces the user credential (across every discovered occurrence) and their Reality short IDs. An inbound's shared Reality keypair or Hysteria2 obfs password rotates only when all of its bound outbounds are selected, so rotating one client never forces other clients to change. Short-ID rotation retains IDs needed by unselected clients and preserves unattributed accepted IDs. One rotation combines all matching inbounds into one validated, recoverable transaction. `set` requires one matching inbound and rejects `--client` / `--outbound-tag`. Operations do not edit server listen addresses/ports or automatically enable TLS/obfs.

`set --kind server-ports --value 20000:30000,40000` switches bound Hysteria2 outbounds to port hopping, removes their scalar `server_port`, and writes `["20000:30000", "40000:40000"]` (sing-box requires range syntax). `server-port` rejects port-hopping outbounds rather than silently switching them back.

### Install on Linux (x86-64)

Download a prebuilt binary from [GitHub Releases](https://github.com/jcdiv47/sb-rotate/releases). Rust is not required on the server; operational commands other than `recover` still require sing-box >=1.14.0.

```bash
base="https://github.com/jcdiv47/sb-rotate/releases/latest/download"
# To pin a version instead: .../releases/download/v0.3.0
curl -fLO "$base/sb-rotate-linux-amd64.tar.gz"
curl -fLO "$base/SHA256SUMS"
sha256sum --check SHA256SUMS && \
  tar -xzf sb-rotate-linux-amd64.tar.gz && \
  sudo install -m 755 sb-rotate /usr/local/bin/sb-rotate
sb-rotate --help
```

This musl build targets `x86_64` Linux (check with `uname -m`), not ARM64.

Maintainers: update the version in `Cargo.toml` and `Cargo.lock`, commit it, then push a matching `v*` tag (for example, `v0.1.0`). The release workflow tests and builds the Linux binary and publishes an archive and SHA-256 checksum to GitHub Releases.

### Build and test

```bash
cargo build --release
cargo test
cargo clippy --all-targets -- -D warnings
./target/release/sb-rotate --help
```

Rust 1.89+ (edition 2024 and standard-library file locking) is required. Every operational command except `recover` requires **sing-box >=1.14.0**; `1.14.0-beta.*` is below this release boundary and is rejected. Default tests use controlled generators/validators and do not require sing-box to be installed.

An opt-in integration test exercises all rotations and service properties against the real generators/validator, using temporary configs and certificates (requires sing-box and `openssl`, no running services):

```bash
cargo test --test real_singbox -- --ignored
# Optionally prefix with SING_BOX=/path/to/sing-box
```

GitHub Actions runs formatting, Clippy, tests, and release builds on Linux, macOS, and Windows. A separate Linux job runs the real-binary integration test using a checksum-pinned sing-box 1.14.0 release.

### Safety and current limitations

- Supply every client that must stay synchronized. The tool cannot update clients it has not been given. With `--clients`, `--client` narrows selection but a shared user credential still updates every discovered occurrence.
- `plan` generates fresh values in memory without writing config files. `rotate` generates a new plan, validates the staged server set and changed clients, then replaces originals. Unchanged files are not rewritten.
- Mutations use fail-fast, cross-process writer locks on the supplied config directories and resolved file-parent directories. Contending `sb-rotate` writers fail with a retry message before staging/validation. Mutations need permission to create/open these sidecars, including in directories containing unchanged source configs. Empty `.sb-rotate.lock` files intentionally persist; **do not delete them while a command is running**. Kernel locks are released when the process exits, including after a crash. Planning previews and no-op updates do not create locks; recovery (including its dry-run) takes writer locks for a consistent assessment.
- Keep backups and avoid other concurrent config writers. The locks are advisory: external editors/deployers and read-only commands do not participate. Input-path retargeting, server/client directory membership changes, and content/metadata changes detected before commit are rejected. This does not provide snapshot isolation for readers or eliminate races with uncooperative writers. Config directories must be trusted; this is not a sandbox for hostile filesystem changes.
- Replacements preserve permissions and, on Unix, owner/group. Mutating a hard-linked config is rejected on Unix instead of silently breaking its aliases; read-only Windows destinations are rejected before staging. Extended attributes/ACLs are not copied. Ordinary replacement failures trigger rollback. Private journals and `.sb-rotate.pending` markers remain if recovery is needed, and overlapping mutations are blocked until recovery completes. Replacements are still atomic **per file**, not across the whole config set: interruption can leave a partial state until you explicitly recover it.
- Files are strict JSON; changed files are reserialized. Validation uses the command's working directory for relative resource paths. sing-box validation diagnostics are forwarded verbatim and may contain config values.

### Recovering an interrupted mutation

Stop other config writers/reload automation, then run as the same user that performed the mutation:

```bash
sb-rotate recover --directory ./clients --dry-run
sb-rotate recover --directory ./clients
# Alternatively: --journal /absolute/path/.sb-rotate-transaction-...
```

Recovery rolls back unfinished commits, resumes an interrupted rollback, or only cleans metadata for already-committed/not-started transactions. It refuses external edits, changed input inventories, missing targets/markers, and corrupt backups instead of overwriting them. There is no force mode. Recovery restores original bytes without generating secrets or invoking sing-box; run `check` afterwards before reloading services.

Journals contain **old credentials**. Unix journals are private (0700 directories/0600 files) and must belong to the recovering user; on Windows they inherit the parent directory's ACL, so use private config directories. Recover only trusted, tool-created journals on the original host/platform. Do not delete/edit pending markers or journals to bypass recovery. Completed journals are removed, not kept as a backup archive.

File data is flushed on all supported platforms; directory entries are also synced on Unix. Windows support covers process interruption, not power-loss durability. Recovery tests simulate abrupt process exit at transaction boundaries; they are not power-loss tests. A crash before journal publication or while staging recovery copies can leave unreferenced `.sb-rotate-*.tmp` scratch files; these need manual cleanup only after confirming no writer is active and all pending transactions are resolved.

## Core model

`sb-rotate` discovers a server **service binding** and the client outbounds that belong to it.

```text
ServiceBinding
├── server inbound
├── client outbound A
│   └── user binding A
├── client outbound B
│   └── user binding B
└── client outbound C
    └── user binding C
```

Operations then apply at one of two useful scopes:

- **identity scope** — a server user credential and every client occurrence using it, for example a VLESS UUID or Hysteria2 password;
- **service scope** — the whole server service and all bound clients, for example a Reality keypair, Hysteria2 obfs password, server address, port, or TLS `server_name`.

Reality `short_id` is a special client-selectable operation: the server accepts a set of short IDs while each client uses one value.

## Example commands

`--clients ./clients/` supplies a local directory of sing-box client JSON configs (direct `.json` files only); repeat it to combine directories, such as configs built from shared and per-device fragments. Each config may contain multiple outbounds, including several of the same type. The tool updates local files; it does not connect to devices or deploy/reload configs. `--client-config-dir` is an alias.

`rotate` replaces the credentials of the selected outbounds on the server and the clients. Without selectors, all bound outbounds are selected; `--type`, `--client`, `--outbound-tag`, and `--inbound-tag` narrow the selection. User credentials and short IDs rotate per selected outbound; an inbound's shared keypair/obfs password rotates only when every bound outbound of that inbound is selected (for example with no selectors or with `--inbound-tag`), and the plan notes when it is kept. Only material with bound outbounds is rotated: unmatched users/outbounds remain unchanged, and unattributed accepted short IDs are retained. A shared keypair/obfs password also changes for server users whose clients were not supplied; the plan warns about them. TLS certificates and connection properties are not rotated. Supply every client config that must stay synchronized.

| Type | Rotated material |
|---|---|
| `vless` | user UUIDs, enabled Reality keypairs and short IDs |
| `hysteria2` | user passwords, configured obfs passwords |

```bash
# Discover inbounds, users, and outbounds.
sb-rotate inspect --server ./server.json --clients ./clients/

# Preview a rotation of everything with bound outbounds.
sb-rotate plan --server ./server.json --clients ./clients/

# Rotate it across every matching inbound and bound outbound.
sb-rotate rotate --server ./server.json --clients ./clients/

# Rotate only VLESS or only Hysteria2.
sb-rotate rotate --server ./server.json --clients ./clients/ --type vless
sb-rotate rotate --server ./server.json --clients ./clients/ --type hysteria2

# Rotate one client's credentials: its user credential (on every outbound that
# shares it) and its short ID. Other clients and shared keys stay unchanged.
sb-rotate rotate --server ./server.json --clients ./clients/ \
  --client ./clients/phone.json --outbound-tag home

# Rotate one inbound and all its bound outbounds.
sb-rotate rotate --server ./server.json --clients ./clients/ \
  --inbound-tag vless-home

# Client configs assembled from fragments in several directories.
sb-rotate rotate --server ./server.json \
  --clients ./clients/shared/ --clients ./clients/devices/

# Change the public server address used by every client in a service.
sb-rotate set --server ./server.json --clients ./clients/ \
  --inbound-tag vless-home \
  --kind server \
  --value new.example.com

# Validate without changing anything.
sb-rotate check --server ./server.json --clients ./clients/

# Use a specific sing-box binary.
sb-rotate inspect --server ./server.json --clients ./clients/ \
  --sing-box /opt/sing-box/bin/sing-box
```

`--client-tag` remains an alias for `--outbound-tag`, and `inspect --protocol` remains an alias for `inspect --type`. Tags are selectors, not globally unique IDs; combine `--client` with `--outbound-tag` to distinguish the same tag in different files. Ambiguous inbound matches are rejected, not guessed.

## Building client configs from fragments

When client configs are assembled from shared and per-device fragments, `build` merges them with `sing-box merge` into one standalone config per target, as listed in a JSON manifest:

```json
{
  "output_dir": "out",
  "publish_dir": "/srv/singbox-sub",
  "targets": {
    "phone": {
      "fragments": ["devices/phone.json", "shared/base.json", "shared/rules.json"],
      "publish_as": "phone-3f9c1a7e5b2d4c60.json"
    }
  }
}
```

Relative paths resolve from the manifest's directory; `output_dir` defaults to `out`. Fragments merge in the listed order: arrays such as rules append in that order, and a scalar may be set by only one fragment (conflicts are rejected). Keep the manifest outside directories passed to `--clients`.

```bash
sb-rotate build --manifest ./clients/build.json              # all targets
sb-rotate build --manifest ./clients/build.json phone laptop # selected targets
sb-rotate build --manifest ./clients/build.json --publish    # also publish
sb-rotate build --manifest ./clients/build.json --publish --sudo  # publish as root:root via sudo
```

Every selected target is merged and checked with `sing-box check` before any output is written, so a failing target leaves all outputs unchanged. Outputs are replaced atomically and are private (0600, in a 0700 directory on Unix). `--publish` copies each output to `publish_dir/publish_as`; every target needs an explicit `publish_as`, so subscription URLs are never derived from guessable target names. Where the publish directory is writable, files are replaced atomically with mode 0644. Otherwise nothing there is touched (not even files you own, whose ownership may be deliberate): the files are listed as `sudo install -o root -g root -m 644` commands and the command exits non-zero. `--publish --sudo` runs those installs itself for every file, after all targets have passed, so subscription files end up root-owned. `build` refuses to run while a fragment directory has a pending rotation transaction, so a half-applied rotation is never published. Relative resource paths (for example `certificate_path`, which `sing-box merge` inlines) resolve from the working directory.

A typical rotation of fragment-based clients:

```bash
sb-rotate rotate --server ./servers/a.json --clients ./clients/shared --clients ./clients/devices
sb-rotate rotate --server ./servers/b.json --clients ./clients/shared --clients ./clients/devices
sb-rotate build --manifest ./clients/build.json --publish --sudo
```

Binary resolution order:

1. `--sing-box <path>`
2. `SING_BOX` environment variable
3. `sing-box` from `PATH`

## Documentation

### Specification

- [Specification index](docs/spec/README.md)
- [Overview and scope](docs/spec/00-overview.md)
- [Data model](docs/spec/01-data-model.md)
- [Binding discovery](docs/spec/02-binding-discovery.md)
- [Operation model](docs/spec/03-operation-model.md)
- [VLESS and Reality](docs/spec/04-vless.md)
- [Hysteria2](docs/spec/05-hysteria2.md)
- [CLI](docs/spec/06-cli.md)
- [Planning, validation, and writes](docs/spec/07-validation-and-writes.md)
- [Versioning and sing-box integration](docs/spec/08-versioning.md)

### Examples

See [`docs/examples/`](docs/examples/README.md) for worked examples and sample configs.

## Design principles

- Keep protocol knowledge explicit and small.
- Generalize the config/editing machinery, not every protocol relationship.
- Match services using strong authentication relationships, not addresses or ports.
- Treat tags as selectors and labels, not as globally unique identity.
- Make `plan` and `rotate` produce the same edit plan; only `rotate` commits it.
- Validate changed configs with the selected sing-box binary before replacing originals.
- Do not build Rust structs for the entire sing-box schema.

## Initial non-goals

- Editing arbitrary sing-box fields.
- Remote deployment or SSH/SFTP.
- Automatic DNS, certificate, firewall, or NAT changes.
- Supporting sing-box versions older than 1.14.
- Automatically splitting a shared VLESS/Hysteria2 identity into separate server users.
- Supporting every protocol in the first release.
- Preserving non-standard JSON formatting/comments in the first implementation.

## References

- sing-box configuration: https://sing-box.sagernet.org/configuration/
- VLESS inbound: https://sing-box.sagernet.org/configuration/inbound/vless/
- VLESS outbound: https://sing-box.sagernet.org/configuration/outbound/vless/
- Hysteria2 inbound: https://sing-box.sagernet.org/configuration/inbound/hysteria2/
- Hysteria2 outbound: https://sing-box.sagernet.org/configuration/outbound/hysteria2/
- TLS / Reality fields: https://sing-box.sagernet.org/configuration/shared/tls/
- JSON Schema: https://sing-box.sagernet.org/configuration/schema/
