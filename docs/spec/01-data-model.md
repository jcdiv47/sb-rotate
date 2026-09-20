# Specification: Data Model

The data model should be generic below the protocol layer and explicit above it.

## Config references

```rust
struct ConfigRef {
    file: PathBuf,
    pointer: JsonPointer,
}
```

A reference identifies the exact JSON location to edit.

For diagnostics it is useful to retain metadata alongside a reference:

```rust
struct EndpointRef {
    config: ConfigRef,
    side: Side,
    protocol: Protocol,
    tag: Option<String>,
}

enum Side {
    Server,
    Client,
}
```

## Service binding

```rust
struct ServiceBinding {
    protocol: Protocol,
    inbound: InboundRef,
    clients: Vec<ClientBinding>,
    identities: Vec<IdentityBinding>,
}
```

A `ServiceBinding` represents one server inbound. Multiple server users may belong to the same service.

## Client binding

```rust
struct ClientBinding {
    outbound: OutboundRef,
    identity_id: Option<IdentityId>,
}
```

A client config may contain multiple VLESS or Hysteria2 outbounds. Each outbound is discovered and bound independently.

The outbound tag is metadata and a selector. It is not assumed to be globally unique.

## Identity binding

```rust
struct IdentityBinding {
    protocol: Protocol,
    kind: IdentityKind,
    value: SecretValue,
    server_refs: Vec<ConfigRef>,
    client_refs: Vec<ConfigRef>,
}
```

`server_refs` is plural because malformed or intentionally duplicated server entries can contain the same credential more than once. The rotation unit is the complete equality group.

Initial identity kinds:

```rust
enum IdentityKind {
    VlessUuid,
    Hysteria2Password,
}
```

## Operation kind

Operations are explicit rather than expressed through a generic relationship DSL.

```rust
enum OperationKind {
    VlessUuid,
    VlessRealityShortId,
    VlessRealityKeypair,
    Hysteria2Password,
    Hysteria2ObfsPassword,
    Server,
    ServerPort,
    ServerPorts,
    TlsServerName,
}
```

Protocol adapters decide whether an operation is valid for a binding.

## Rotation plan

All protocol-specific logic ends at `RotationPlan`. Type-driven rotation composes multiple operation plans into a single `RotateType(Protocol)` plan; edit paths remain disjoint and are checked together before writes.

```rust
struct RotationPlan {
    operation: OperationKind,
    edits: Vec<Edit>,
}

struct Edit {
    file: PathBuf,
    pointer: JsonPointer,
    old: Option<serde_json::Value>,
    new: Option<serde_json::Value>,
}
```

`None` denotes an absent object member, distinct from JSON `null`. This supports adding optional fields and removing scalar `server_port` when setting Hysteria2 `server_ports`. The parent object must already exist; planners do not invent TLS/obfs blocks. Duplicate or overlapping edit locations are rejected.

The apply/validation layer does not need to know whether an edit came from VLESS, Reality, or Hysteria2.

## Selectors

Selectors narrow the inventory before planning.

```rust
struct Selection {
    inbound_tags: Vec<String>,
    client_paths: Vec<PathBuf>,
    client_tags: Vec<String>,
}
```

Rules:

- multiple values in the same selector category are ORed;
- different selector categories are ANDed;
- no client selector means all clients in the selected service binding;
- all-material and service-scoped secret operations reject client narrowing; identity and short-ID operations allow it;
- `--type` selects the sing-box type; `--outbound-tag` is the preferred name for the client tag selector (`--client-tag` remains an alias).

Example:

```text
--client phone.json --client-tag home
```

means "outbounds tagged `home` inside `phone.json`".

## Config representation

The first implementation should parse supported files into `serde_json::Value` and edit only known paths.

Do not create Rust structs for the complete sing-box configuration schema.

Exact formatting and comments are not preserved in v1. Output is serialized as valid JSON and then validated by sing-box.
