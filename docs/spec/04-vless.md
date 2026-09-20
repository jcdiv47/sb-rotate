# Specification: VLESS and Reality

## Relevant sing-box fields

### Server inbound

```text
inbounds[type=vless]
  tag
  users[]
    name
    uuid
    flow
  tls
    reality
      private_key
      short_id[]
```

### Client outbound

```text
outbounds[type=vless]
  tag
  server
  server_port
  uuid
  flow
  tls
    server_name
    reality
      public_key
      short_id
```

## Binding rule

Primary binding:

```text
client.uuid == server.users[].uuid
```

UUID is the discovery key, not the permanent identity of the service. Once matched, the containing server inbound defines the service binding.

## `vless-uuid`

Scope: identity.

Generator:

```bash
sing-box generate uuid
```

For every selected VLESS identity binding:

```text
old UUID
  server users[] occurrence(s)
  client outbound occurrence(s)
        ↓
new UUID
```

All occurrences of one shared UUID identity move together.

Example:

```text
server alice: uuid-a
phone/home:   uuid-a
phone/backup: uuid-a

rotate identity selected by phone/home

server alice: uuid-new
phone/home:   uuid-new
phone/backup: uuid-new
```

The tool does not duplicate the server user to split the identity.

## `vless-reality-short-id`

Scope: selected clients within one service.

Server form:

```json
"short_id": ["0123456789abcdef", "fedcba9876543210"]
```

Client form:

```json
"short_id": "0123456789abcdef"
```

Default generator:

```bash
sing-box generate rand 8 --hex
```

Rules:

1. one new ID per selected client outbound;
2. new IDs are added to the server accepted list;
3. a previous ID is retained if any unselected bound client still uses it;
4. a previous ID is removed if no remaining bound client uses it;
5. unrelated accepted short IDs that cannot be attributed to supplied clients are preserved.

The last rule avoids deleting IDs used by clients not included in the command.

## `vless-reality-keypair`

Scope: service.

Generator:

```bash
sing-box generate reality-keypair
```

The generated private key goes to:

```text
server inbound.tls.reality.private_key
```

The generated public key goes to every bound VLESS client with Reality enabled:

```text
client outbound.tls.reality.public_key
```

This operation is not client-selectable. A Reality keypair is treated as a service-level property.

## `server`

Scope: service, client-side.

Updates:

```text
client outbound.server
```

for every bound client outbound in the selected VLESS service.

No server-side edit is implied.

## `server-port`

Scope: service.

Updates:

```text
client outbound.server_port
```

By default this does not edit server `listen_port`, because public and listen ports can differ through NAT or forwarding.

A future explicit operation may support coordinated listen-port changes.

## `tls-server-name`

Scope: service, client-side.

Updates:

```text
client outbound.tls.server_name
```

for every bound client outbound in the selected service.

No server-side `server_name` match is required.

## Multiple VLESS services

If the server contains multiple VLESS inbounds, operations that would otherwise select more than one service may be narrowed with:

```bash
--inbound-tag <tag>
```

For service-scoped operations that require exactly one service, multiple matches are an error unless the command explicitly supports applying independently to all selected services.

Preferred behavior:

- identity rotations may operate across multiple services;
- service secret/property operations require `--inbound-tag` when more than one matching service exists.

## References

- https://sing-box.sagernet.org/configuration/inbound/vless/
- https://sing-box.sagernet.org/configuration/outbound/vless/
- https://sing-box.sagernet.org/configuration/shared/tls/
