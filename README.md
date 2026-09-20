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
- `plan` / `rotate`: `vless-uuid`, `hysteria2-password`, `vless-reality-short-id`, `vless-reality-keypair`, and `hysteria2-obfs-password`;
- `set`: `server`, `server-port`, `server-ports`, and `tls-server-name`, with `--dry-run` for a read-only preview;
- `recover`: preview or roll back an interrupted multi-file transaction without needing sing-box;
- masked passwords, private keys, and short IDs; staged validation, permission-preserving replacement, durable rollback journals, and automatic rollback on ordinary replacement failures.

Shared identities rotate across every discovered occurrence. Short-ID rotation assigns an independent ID to each selected Reality outbound, retains IDs needed by unselected clients, and preserves unattributed accepted IDs. Whole-service operations require a single matching service (use `--inbound-tag` to disambiguate), reject `--client` / `--client-tag`, and use `--clients` to supply the client inventory. They do not edit server listen addresses/ports or automatically enable TLS/obfs.

`set --kind server-ports --value 20000:30000,40000` switches bound Hysteria2 outbounds to port hopping, removes their scalar `server_port`, and writes `["20000:30000", "40000:40000"]` (sing-box requires range syntax). `server-port` rejects port-hopping outbounds rather than silently switching them back.

### Install on Linux (x86-64)

Download a prebuilt binary from [GitHub Releases](https://github.com/jcdiv47/sb-rotate/releases). Rust is not required on the server; operational commands other than `recover` still require sing-box >=1.14.0.

```bash
base="https://github.com/jcdiv47/sb-rotate/releases/download/v0.1.0"
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

- Supply every client that must stay synchronized. The tool cannot update clients it has not been given. With `--clients`, `--client` narrows selection but does not remove other discovered occurrences of a shared identity.
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

```bash
# Discover bindings.
sb-rotate inspect --server ./server.json --clients ./clients/

# Preview changes.
sb-rotate plan --server ./server.json --clients ./clients/ \
  --kind vless-uuid

# Rotate all matched VLESS UUID identity bindings.
sb-rotate rotate --server ./server.json --clients ./clients/ \
  --kind vless-uuid

# Rotate Reality short_id for one client outbound.
sb-rotate rotate --server ./server.json --clients ./clients/ \
  --kind vless-reality-short-id \
  --client ./clients/phone.json \
  --client-tag home

# Rotate a Reality keypair for one VLESS service.
sb-rotate rotate --server ./server.json --clients ./clients/ \
  --kind vless-reality-keypair \
  --inbound-tag vless-home

# Rotate Hysteria2 passwords.
sb-rotate rotate --server ./server.json --clients ./clients/ \
  --kind hysteria2-password

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
