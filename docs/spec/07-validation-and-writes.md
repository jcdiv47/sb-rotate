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
5. write changed documents to temporary paths
6. run sing-box validation against temporary configs
7. replace original changed files
8. exit success
```

If generation, editing, serialization, or validation fails, original files are not replaced.

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

The implementation does not need a backup/archive system in v1.

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
