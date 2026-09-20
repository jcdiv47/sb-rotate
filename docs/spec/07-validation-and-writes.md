# Specification: Planning, Validation, and Writes

## Invariant

A successful mutating command must never leave only part of its planned config edits applied.

The implementation can stay small while still following one transaction-like sequence.

## Apply sequence

```text
1. load source files
2. discover bindings
3. build RotationPlan
4. apply edits to in-memory documents
5. acquire directory writer locks; reject stale source snapshots
6. write changed documents to temporary paths
7. run sing-box validation against temporary configs
8. recheck writer locks, source paths, directory membership, contents, and metadata
9. persist original-byte backups, manifest, and per-directory pending markers
10. recheck sources and record durable commit-start intent
11. replace original changed files (rechecking each destination immediately before replacement)
12. persist the committed decision; remove pending markers and journal
13. release locks and exit success
```

If generation, editing, serialization, or validation fails, original files are not replaced.

## Concurrent writers and filesystem changes

Nonempty mutations acquire exclusive, non-blocking OS locks on `.sb-rotate.lock` sidecars in the config directories and resolved file-parent directories. Locks cover the complete loaded inventory, including unchanged files used for discovery. Canonical directory ordering prevents lock-order deadlocks, and failure to acquire any lock releases the already-acquired locks. Another writer receives a retry diagnostic before staging or validation. Mutations need access to create/open all sidecars, even in directories whose config files will remain unchanged.

Sidecars are deliberately persistent. Deleting one during a command would allow a second writer to lock a different file at the same path. Process exit releases the kernel lock; leftover sidecars are not stale-lock errors. `plan`, `set --dry-run`, and ordinary no-op mutations do not create sidecars. Recovery previews acquire the recorded writer-lock set and may recreate missing lock sidecars.

The loaded snapshot records original source aliases and both server/client directory layouts. Rechecking catches retargeted input symlinks, newly added or removed client files, and changes to contents or metadata (including inode, link count, ownership, and mode on Unix). This prevents a stale plan from silently dropping a newly supplied client. Mutations refuse hard-linked destination configs on Unix, because replacing a single pathname would sever its other aliases. Read-only Windows destinations are rejected before staging, since they cannot be atomically replaced and read-only temporary files would complicate cleanup.

These locks coordinate `sb-rotate` writers only. Readers do not get snapshot isolation, and external editors/deployers can still race with checks. Parent directories must be trusted. Cross-file reads are not atomic, but persistent journals support explicit rollback after an interrupted commit. Pending transactions block overlapping mutations, including otherwise no-op updates.

## Server validation

If `--server` points to one file, validate the temporary file as one sing-box config.

Conceptually:

```bash
sing-box check -c <temp-server.json>
```

If `--server` points to a config directory, create a temporary directory preserving its filenames and validate it using the same multi-file structure.

Conceptually:

```bash
sing-box check -C <temp-server-directory>
```

The exact sing-box global option spelling should be taken from the installed binary's CLI behavior.

## Client validation

Each client file is an independent config in v1.

Validate each modified client separately:

```bash
sing-box check -c <temp-client.json>
```

Unmodified clients do not need to be revalidated during `rotate`/`set`; `check` validates all supplied clients.

## Atomic replacement

For every changed file:

1. create the final temporary file in the same filesystem when practical;
2. flush/close it;
3. rename it over the destination after every validation succeeds.

Original-byte backups and a versioned manifest are persisted in a `.sb-rotate-transaction-*` directory before any replacement. Every locked directory gets a `.sb-rotate.pending` pointer. The manifest records SHA-256 checksums, file snapshots, native paths, source aliases, directory inventories, and signatures for unchanged discovery inputs. Unix journal directories/files use 0700/0600 permissions; Windows inherits directory ACLs.

An ordinary replacement failure invokes the same conservative rollback used by explicit recovery. All targets/backups are preflighted before the first restoration. Already-restored originals are recognized by bytes and attributes (not their new inode/mtime); installed replacements must match their complete recorded snapshot. An external edit, changed source inventory, or corrupt backup prevents automatic restoration. Failures retain the journal and report its location.

A persisted `committed` decision means all replacements completed; recovery must never undo it. Durability depends on the platform/filesystem guarantees described below. A durable `rolled-back` decision similarly permits cleanup only. Pending markers are removed and directory entries synced before deleting the recovery copies, making interrupted cleanup resumable. Successful operations do not keep an archival backup.

Replacement files preserve source permissions and, on Unix, owner/group; failure to preserve these attributes aborts before replacement. Extended attributes and ACLs are not copied.

## Explicit recovery

```bash
sb-rotate recover --directory ./clients --dry-run
sb-rotate recover --directory ./clients
sb-rotate recover --journal /absolute/path/.sb-rotate-transaction-...
```

Exactly one locator is required. Recovery acquires the full recorded writer-lock set, does not load sing-box or generate replacements, and restores original bytes rather than revalidating them. Its dry run reports statuses/conflicts without editing configs or transaction metadata (lock sidecars may be recreated if missing). Normal read-only commands remain available, but can observe an interrupted mixed state.

Journals without a start decision have not modified originals and need cleanup only, even if marker publication was interrupted. Started, unfinished transactions require all recorded pending markers and a conflict-free inventory before rollback. Completed transactions need only metadata cleanup and do not overwrite later config edits. Missing files, altered staged scratch files, symlinks, incompatible versions/platforms, and foreign/private-directory ownership violations cause recovery to refuse unsafe actions. There is no force or automatic roll-forward mode.

Run recovery as the original command's user on the original host/platform, and trust only tool-created journals. Checksums detect accidental corruption, not maliciously forged metadata. Keep parent directories trusted and stop external writers/reload automation during recovery. Manually reconcile conflicts before retrying; do not delete markers to bypass them. Use `check` before redeploying restored configs.

Files are flushed/closed before publication and replacement. Unix also syncs affected directories; Windows has no portable directory-fsync support here, so its guarantees cover process interruption rather than power loss. Fault tests exercise actual abrupt process exit, not storage-controller/power-loss failures. Crashes before journal publication or during recovery staging can leave unreferenced temporary scratch files; no original replacement precedes durable journal publication. Cleanup never scans/deletes unknown scratch files.

## Multi-file server configs

sing-box supports configuration directories and multiple configuration files. The server directory must be treated as one config set rather than validating each server fragment independently.

Only files containing planned edits need new serialized contents, but validation receives a complete temporary copy/view of the server config set.

## JSON formatting

v1 may reserialize changed JSON documents through `serde_json`.

Consequences:

- indentation/order may change according to serializer behavior;
- comments/non-standard JSON are not preserved;
- unchanged files are not rewritten.

A syntax-preserving JSON/JSONC editor can be added later without changing the binding or operation layers.

## Plan output

A plan should identify edits by semantic context and JSON location.

Example:

```text
VLESS UUID
  server: server.json inbound=vless-home user=alice
  clients:
    clients/phone.json outbound=home
    clients/macbook.json outbound=home
  uuid: bf000d23-... -> 7df92d8a-...
```

Passwords and random shared secrets should normally be masked:

```text
Hysteria2 password
  server: server.json inbound=hy2-home user=alice
  clients: 2
  password: ******** -> ********
```

Reality public keys and UUIDs may be shown because they are useful identifiers and not passwords.

## Validation errors

Surface sing-box stderr/stdout with enough context to identify which temporary config set failed.

Do not translate every sing-box validation error into custom errors. sing-box is the schema/runtime authority.
