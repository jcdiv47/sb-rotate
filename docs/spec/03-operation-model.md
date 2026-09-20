# Specification: Operation Model

`sb-rotate` generalizes planning and applying edits, but keeps protocol mappings explicit.

## Why not a generic credential DSL

The supported relationships are materially different:

```text
VLESS UUID              equality
Hysteria2 password      equality
Reality short_id        client scalar ∈ server set
Reality keypair         private/public generated pair
server address          client-side service property
TLS server_name         client-side service property
```

Encoding these through generic metadata such as `relation = equality|membership|derived` would add abstraction without reducing protocol code.

Instead, each protocol adapter implements small operation planners that return a generic `RotationPlan`.

## Type-driven composition

`plan` and `rotate --type vless|hysteria2` compose the applicable operations below into one plan against the original inventory. UUID/user-password rotation runs across all selected matched users; each eligible inbound contributes its configured Reality keypair/short-ID or obfs edits. Each inbound's shared secret is generated independently. `--only` restricts the composition to a single material kind but still supports multiple inbounds.

The combined plan is materialized, staged, validated, and committed once. Any planning/generation or staged-validation failure prevents all replacements. Existing per-file atomicity and recovery rules still apply. Discovery ambiguity is checked before splitting work by inbound, so batching never resolves an ambiguous match implicitly.

All-material rotation and shared-inbound secret rotation reject client/outbound narrowing. User credentials and short IDs remain client-selectable via `--only`. Optional features are not enabled, unmatched identities/outbounds are not changed, and unattributed accepted short IDs are preserved. See [CLI semantics](06-cli.md#type-driven-rotation).

Legacy `--kind` uses the original single-operation planners; its service-level operations still require one inbound.

## Operation classes

### Identity rotation

Examples:

- `vless-uuid`
- `hysteria2-password`

Algorithm:

1. select identity bindings;
2. generate one new value per selected identity binding;
3. replace all server references in the identity binding;
4. replace all discovered client references in the identity binding.

If three clients share one UUID, one new UUID is generated and all three move together.

### Service secret/key rotation

Examples:

- `vless-reality-keypair`
- `hysteria2-obfs-password`

Algorithm:

1. select a service binding;
2. generate one service-level replacement;
3. update the server-side field(s);
4. update all bound client outbounds that carry the corresponding field.

### Client-selectable membership rotation

Example:

- `vless-reality-short-id`

Algorithm:

1. select client outbounds inside one VLESS service;
2. generate a new short ID for every selected client outbound;
3. update each selected client;
4. add new IDs to the server accepted-ID set;
5. remove an old ID from the server set only if no unselected bound client still uses it.

This permits:

```text
before
server: [aaa, bbb]
phone: aaa
laptop: aaa
tablet: bbb

rotate phone

server: [aaa, bbb, ccc]
phone: ccc
laptop: aaa
tablet: bbb
```

### Explicit service property update

Examples:

- `server`
- `server-port`
- `server-ports`
- `tls-server-name`

These use `set`, not `rotate`, because the new value is supplied by the user.

The operation updates every bound client outbound in the selected service unless the protocol mapping specifies a corresponding server field to update.

For example, changing the public client `server` hostname usually does not imply any server-side JSON edit.

## Generator ownership

`sb-rotate` uses sing-box generators when available rather than implementing equivalent generators itself.

Initial generators:

```text
VLESS UUID             sing-box generate uuid
Reality keypair        sing-box generate reality-keypair
Reality short_id       sing-box generate rand 8 --hex
Hysteria2 password     sing-box generate rand 32 --base64
Hysteria2 obfs secret  sing-box generate rand 32 --base64
```

The short-ID default is 8 random bytes encoded as 16 hex characters.

## Plan determinism

`plan` may generate replacement secrets so it can display the exact edits that would be applied, but secrets printed in normal output should be masked where displaying them adds little value.

An implementation may alternatively show placeholders during `plan` and generate at `rotate` time. The important invariant is that the edit topology is identical.

The preferred implementation is:

- `plan`: generate values in memory and show masked diffs;
- `rotate`: independently regenerate and apply a fresh plan.

This avoids persisting planned credentials.
