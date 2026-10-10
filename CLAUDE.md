# sb-rotate

Rust CLI that discovers server↔client bindings in sing-box JSON configs, rotates
credentials across them, propagates service properties, and builds client
configs from fragments. Targets sing-box >= 1.14.0 (VLESS, VLESS+Reality,
Hysteria2). It edits local files only: no deployment, no SSH, no reloads.

## Commands

```bash
cargo build --release
cargo test                                         # mocks only; no sing-box needed
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test --test real_singbox -- --ignored        # real sing-box + openssl (CI runs it too)
```

Run all four checks before calling a change done. CI also runs them on macOS/Windows,
so keep Unix-only code behind `#[cfg(unix)]`.

## Layout

- `src/cli.rs` — clap definitions. `src/main.rs` — dispatch.
- `src/config.rs` — `ConfigSet`: loads the server file/dir plus client dirs (`--clients`, repeatable) and files (`--client`).
- `src/binding.rs` — discovery: matches outbounds to inbound users by credential equality (never by address/tag).
- `src/plan.rs` — `rotation()` composes per-protocol planners into one `RotationPlan`; `materialize()` applies edits in memory.
- `src/protocol/{vless,hysteria2,properties}.rs` — protocol planners and `set`.
- `src/build.rs` — `build` subcommand (fragment merge + publish).
- `src/apply.rs`, `fsutil.rs`, `locking.rs`, `recovery.rs` — staged validation, locks, journals, `recover`.
- `src/singbox.rs` — the `SingBox` trait (generators, `check`, `merge`) and the process implementation.
- Specs: `docs/spec/`; worked examples: `docs/examples/`. Keep README, spec, and examples in sync with CLI changes.

## Rotation semantics (deliberate decisions)

- No subset rotation: `--only` and rotation `--kind` were removed. A rotation always covers every supported material for the selection. `set --kind` (properties) is unrelated and still exists.
- `--type vless|hysteria2` is optional; without it every supported type is rotated.
- Selection: without selectors everything bound is selected; `--client`, `--outbound-tag`, `--inbound-tag` narrow it.
  - Per-client material rotates for the selected outbounds: user UUID/password (all occurrences of a shared user rotate together) and Reality short IDs (each selected outbound gets its own; IDs still used by unselected clients and unattributed IDs are kept).
  - Shared inbound secrets (Reality keypair, Hysteria2 obfs password) rotate **only when every bound outbound of that inbound is selected**; otherwise the plan says "shared … kept". So rotating one client never forces other clients to change.
  - When a shared secret changes and the inbound has server users without a supplied client, the plan prints a warning naming them.
- Ambiguity is checked before any generation; nothing is ever guessed or split automatically.
- `set` stays whole-service: it requires one matching inbound and rejects `--client`/`--outbound-tag`.
- Breaking changes in 0.3.0 (vs 0.2.0): removed `--only` and rotation `--kind`; `--type` optional; selectors allowed for rotation; `--clients` repeatable; new `build`.

## `build`

`sb-rotate build --manifest <build.json> [TARGET...] [--publish]`. The manifest maps targets to ordered fragment lists (plus `output_dir`, `publish_dir`, `publish_as`).
Merging shells out to `sing-box merge` on index-prefixed staged copies, because sing-box sorts inputs by path, re-serializes through its typed model, and inlines `*_path` resources relative to the cwd. Do not reimplement the merge in Rust: output must stay byte-identical to `sing-box merge`.
Scalars set by two fragments are rejected. All targets are merged and checked before any output is written. Outputs are written atomically with mode 0600. `--publish` replaces files atomically (0644) only where the publish directory is writable. It never overwrites in place; otherwise it reports `sudo install -o root -g root -m 644` commands and exits non-zero. `--publish --sudo` runs those installs itself, after all targets pass; this is how the owner publishes, because subscription files are deliberately root-owned. Every target needs an explicit `publish_as`, so subscription names are never guessable. A pending rotation transaction in any fragment directory blocks the build.

## Testing conventions

- Every `SingBox` mock (`tests/{operations,safety,type_rotation,workflow,build}.rs`) must implement all trait methods; when adding one, update them all.
- Fixtures use obviously fake values (`alice`, `old-private-key`, `uuid-1`). Never put real credentials in tests, docs, or commits.
- Verify against the owner's real configs only on a scratch copy (`cp -r /etc/sing-box/{clients,servers} "$(mktemp -d)"`), never in place. Delete the copy afterwards; it contains live credentials.

## The owner's deployment (context for real-world checks)

Real configs live on this machine under `/etc/sing-box` (not a git repo). Never print, commit, or publish values from them.
`inspect` and `plan` print UUIDs and Reality public keys unmasked, so redact their output before sharing it anywhere.

- `servers/malibu.json` — Malibu host. Inbound `VLESS-Vision-Reality` with users `caijq`, `gdmm`, `loulou`; inbound `HY2-in` with user `caijq-hy2` and obfs.
- `servers/tiny.json` — Tiny host. One `VLESS-Vision-Reality` inbound with one unnamed user.
- These are two separate servers, not one config directory: pass one `--server` file per run (merging them as a directory fails on duplicate inbound tags).
- `clients/shared/*.json` + `clients/devices/<device>.json` are fragments. `clients/build.json` maps devices (iphone, mbp, windows, openwrt, gdmm) to fragments. `clients/out/` holds the built configs. `sb-rotate build` replaced the old Python `build.py` (verified byte-identical outputs; same root-owned publishing).
- `shared/proxies.json` (Tiny, Malibu, HY2 outbounds) is merged into iphone, mbp, windows, and openwrt, so those four share one set of credentials and cannot be rotated individually. gdmm has its own Malibu user and short ID in `devices/gdmm.json`. loulou's client config is not here. The Shadowsocks `ss-*` credentials (home.json, openwrt inbounds) are unsupported and never rotated.
- Subscriptions are published to `/srv/singbox-sub`, a root-owned directory; subscription files there are meant to be root:root 0644 (`--publish --sudo`).
- Gotcha: Malibu's HY2 inbound reads `/etc/sing-box/cert/{cert,key}.pem`, which exist only on the Malibu host. Without a local (throwaway) pair at that path, `rotate`/`check` of Malibu fail validation here and write nothing.

End-to-end workflow:

```bash
cd /etc/sing-box
sb-rotate plan   --server servers/malibu.json --clients clients/shared --clients clients/devices   # preview
sb-rotate rotate --server servers/malibu.json --clients clients/shared --clients clients/devices   # repeat per server
sb-rotate build --manifest clients/build.json --publish --sudo
# then (outside sb-rotate): copy servers/<name>.json to the host's /etc/sing-box/config.json,
# restart sing-box there, and refresh subscriptions on the affected devices
```

Always pass both fragment directories, or gdmm is invisible to the rotation.

| Case | Selector | Changes | Deploy / refresh |
|---|---|---|---|
| Everything | none (run for malibu and tiny) | all users, short IDs, both keypairs, obfs | both hosts / all devices; send loulou the new Malibu public key |
| Hysteria2 only | `--type hysteria2` (malibu) | caijq-hy2 password + obfs | Malibu / iphone, mbp, windows, openwrt |
| gdmm only | `--client clients/devices/gdmm.json` | gdmm UUID + short ID; keypair kept | Malibu / gdmm |
| Personal devices | `--client clients/shared/proxies.json` (malibu and tiny) | caijq creds + HY2 + Tiny keypair; Malibu keypair kept | both / the four personal devices |
| Malibu keypair | `--inbound-tag VLESS-Vision-Reality` (≡ `--outbound-tag Malibu`) | all Malibu VLESS material | Malibu / all devices + loulou |

The full write-up with sample output is `local/sb-rotate-guide.html` (gitignored). It is published with `postplan upload local/sb-rotate-guide.html`, draft `ee76d71a-2cb`; re-uploading the same path adds a version. Keep it placeholder-only, and before uploading, scan it against the string values in `/etc/sing-box` configs.

`local/deploy.sh` (gitignored) only copies the release binary to the Malibu host; it does not deploy configs.
