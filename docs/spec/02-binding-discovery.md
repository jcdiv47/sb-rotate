# Specification: Binding Discovery

Binding discovery answers two separate questions:

1. Which server inbound does a client outbound belong to?
2. Which server user identity does that client use?

Addresses, ports, and TLS names are not primary matching keys. They can differ because of DNS, NAT, port forwarding, proxies, or service migration.

## VLESS discovery

For every client outbound where:

```json
{ "type": "vless", "uuid": "..." }
```

find server VLESS user occurrences where:

```text
server.inbounds[type=vless].users[].uuid == client.outbounds[type=vless].uuid
```

A successful match creates:

- an `IdentityBinding` for that UUID;
- a client membership in the `ServiceBinding` associated with the containing inbound.

Multiple different UUIDs can resolve to the same server inbound, producing one service with multiple identities.

Example:

```text
server inbound vless-home
  alice -> uuid-a
  bob   -> uuid-b

phone  -> uuid-a
laptop -> uuid-b

ServiceBinding(vless-home)
  Identity(uuid-a) -> phone
  Identity(uuid-b) -> laptop
```

## Multiple client outbounds

Every outbound is discovered independently.

A single file can contain:

```text
outbound home   -> uuid-a
outbound office -> uuid-b
outbound backup -> uuid-a
```

`home` and `backup` are two client occurrences in the same UUID identity binding if `uuid-a` resolves to the same server inbound.

## Shared identity behavior

If several client outbounds use the same VLESS UUID, they form one identity binding.

Rotating that identity changes:

- every server occurrence of the old UUID in the matched inbound;
- every discovered client outbound using that UUID.

Selecting one of those clients selects the identity binding; it does **not** silently split the shared identity.

Automatic identity splitting is out of scope for v1.

## Hysteria2 discovery

For every client outbound where:

```json
{ "type": "hysteria2", "password": "..." }
```

find server Hysteria2 users where:

```text
server.inbounds[type=hysteria2].users[].password == client.outbounds[type=hysteria2].password
```

The same service/identity grouping rules used for VLESS apply.

## Reality fields

Reality fields are discovered only after the parent VLESS outbound is already bound through its UUID.

Do not attempt to bind a VLESS client to a server solely through:

- `tls.server_name`;
- Reality `short_id`;
- server address;
- port.

Once the VLESS service binding is known, the tool can inspect:

- server `tls.reality.private_key`;
- server `tls.reality.short_id[]`;
- client `tls.reality.public_key`;
- client `tls.reality.short_id`.

## Ambiguity

Discovery is ambiguous when one client identity value matches more than one server inbound.

Example:

```text
server inbound A contains uuid-x
server inbound B contains uuid-x
client outbound contains uuid-x
```

The tool reports the ambiguity and requires a server selector such as:

```bash
--inbound-tag vless-home
```

The tool should not guess using server address or port.

## Unmatched values

An unmatched server user or client outbound is included in `inspect` output but excluded from rotation by default.

Examples:

- server user has no supplied client;
- client UUID/password does not exist in the supplied server configuration.

## Binding IDs

Persistent binding IDs are not required in v1.

Human-facing output should primarily use:

- protocol;
- server file;
- inbound tag;
- client file;
- outbound tag;
- server user name when present.
