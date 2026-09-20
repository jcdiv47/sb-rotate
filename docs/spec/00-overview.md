# Specification: Overview and Scope

## Purpose

`sb-rotate` manages coordinated changes across sing-box server and client configuration files.

The main problems are:

1. discover which client outbounds belong to which server inbound;
2. discover which server user identity a client uses;
3. rotate credentials or key material without leaving related configs inconsistent;
4. propagate service-level connection changes to all clients of a service;
5. validate the resulting configs with sing-box before committing them.

The first supported sing-box version is **1.14.0**.

## Supported protocols in v1

- VLESS
- VLESS with Reality
- Hysteria2

The protocol implementation is intentionally explicit. Adding another protocol means adding another small protocol adapter and its operation mappings.

## Terminology

### Config document

One JSON file that can be read and edited by `sb-rotate`.

### Server config set

The server configuration supplied with `--server`. It may be:

- one JSON file; or
- a directory representing one sing-box multi-file configuration set.

### Client config

A standalone client JSON file discovered under `--clients` or supplied with `--client`.

For v1, a client file is treated as one independent sing-box config. A directory of client files is a collection of independent client configs, not one merged client config set.

### Service binding

A relationship between one server inbound and all client outbounds that authenticate to that inbound.

### Identity binding

A relationship between one server-side user credential and all client outbounds using that same credential.

Examples:

- VLESS server `users[].uuid` ↔ client `outbound.uuid`
- Hysteria2 server `users[].password` ↔ client `outbound.password`

### Rotation plan

A complete, in-memory list of edits that must succeed together.

## High-level architecture

```text
load configs
    ↓
protocol discovery
    ↓
ServiceBinding + IdentityBinding inventory
    ↓
operation-specific planner
    ↓
RotationPlan { edits }
    ↓
write temporary copies
    ↓
sing-box check
    ↓
replace changed files
```

## Scope rules

The tool distinguishes between values that belong to an identity and values that belong to a service.

### Identity-scoped

Examples:

- VLESS UUID
- Hysteria2 authentication password

An identity rotation changes the server user credential and every discovered client occurrence of that credential.

### Service-scoped

Examples:

- Reality private/public keypair
- Hysteria2 obfs password
- client `server`
- client `server_port`
- client TLS `server_name`

A service operation applies to all bound client outbounds unless the operation definition explicitly says otherwise.

### Client-selectable set membership

Reality `short_id` is modeled separately because:

- the server accepts an array of IDs;
- each client uses one ID;
- selected clients can move to new IDs while unselected clients keep existing IDs.

## Non-goals

The initial implementation does not:

- infer deployment topology from DNS, public IP, NAT, or ports;
- manage remote hosts;
- reload or restart sing-box;
- update certificates or DNS records;
- preserve JSON comments or exact formatting;
- split one shared identity credential into multiple server user records automatically;
- attempt to understand fields unrelated to supported operations.
